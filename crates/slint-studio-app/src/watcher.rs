//! Watching the project folder for changes made outside the editor.
//!
//! Directories are watched one by one (non-recursively), following the
//! scanned file tree. Watching the root recursively would also cover build
//! output such as `target/`, which can hold enough directories to exhaust
//! the operating system's watch limit.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use slint_studio_core::FileNode;
use tracing::{debug, warn};

/// Changes observed since the last poll.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileChanges {
    /// Files whose content changed.
    pub modified: BTreeSet<PathBuf>,
    /// Files or directories created, removed or renamed.
    pub structure: BTreeSet<PathBuf>,
}

impl FileChanges {
    pub fn is_empty(&self) -> bool {
        self.modified.is_empty() && self.structure.is_empty()
    }
}

pub fn is_slint(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("slint"))
}

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    watched: HashSet<PathBuf>,
}

impl FileWatcher {
    pub fn new(root: &FileNode, waker: Arc<dyn Fn() + Send + Sync>) -> notify::Result<Self> {
        let (tx, events) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |event| {
            if tx.send(event).is_ok() {
                waker();
            }
        })?;
        let mut watcher = Self {
            watcher,
            events,
            watched: HashSet::new(),
        };
        watcher.sync(root);
        Ok(watcher)
    }

    /// Watches every directory of `root` and stops watching removed ones.
    pub fn sync(&mut self, root: &FileNode) {
        let mut wanted = HashSet::new();
        collect_dirs(root, &mut wanted);
        for gone in self
            .watched
            .difference(&wanted)
            .cloned()
            .collect::<Vec<_>>()
        {
            // The directory may already be gone, which also ends the watch.
            let _ = self.watcher.unwatch(&gone);
            self.watched.remove(&gone);
        }
        for dir in wanted {
            if !self.watched.contains(&dir) {
                match self.watcher.watch(&dir, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        self.watched.insert(dir);
                    }
                    Err(err) => warn!("impossibile osservare {}: {err}", dir.display()),
                }
            }
        }
    }

    pub fn watched_count(&self) -> usize {
        self.watched.len()
    }

    pub fn poll(&self) -> FileChanges {
        let mut changes = FileChanges::default();
        for event in self.events.try_iter() {
            match event {
                Ok(event) => classify(&event, &mut changes),
                Err(err) => debug!("evento di file non valido: {err}"),
            }
        }
        changes
    }
}

fn collect_dirs(node: &FileNode, out: &mut HashSet<PathBuf>) {
    if node.is_dir() {
        out.insert(node.path.clone());
        for child in node.children() {
            collect_dirs(child, out);
        }
    }
}

/// Hidden entries are never shown in the tree, so changes to them (editor
/// swap files, `.git`) are ignored.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with('.') || name.ends_with('~'))
}

fn classify(event: &notify::Event, changes: &mut FileChanges) {
    for path in event.paths.iter().filter(|p| !is_hidden(p)) {
        match event.kind {
            EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Name(_)) => {
                changes.structure.insert(path.clone());
            }
            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Any | ModifyKind::Other)
            | EventKind::Any => {
                changes.modified.insert(path.clone());
            }
            EventKind::Access(_)
            | EventKind::Modify(ModifyKind::Metadata(_))
            | EventKind::Other => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, RenameMode};
    use std::time::{Duration, Instant};

    fn event(kind: EventKind, path: &str) -> notify::Event {
        notify::Event::new(kind).add_path(PathBuf::from(path))
    }

    #[test]
    fn events_are_classified() {
        let mut changes = FileChanges::default();
        classify(
            &event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                "/p/a.slint",
            ),
            &mut changes,
        );
        classify(
            &event(EventKind::Create(CreateKind::File), "/p/b.slint"),
            &mut changes,
        );
        classify(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                "/p/c.md",
            ),
            &mut changes,
        );
        classify(
            &event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                "/p/.a.slint.swp",
            ),
            &mut changes,
        );
        classify(
            &event(
                EventKind::Access(notify::event::AccessKind::Read),
                "/p/d.slint",
            ),
            &mut changes,
        );

        assert_eq!(
            changes.modified.iter().collect::<Vec<_>>(),
            [Path::new("/p/a.slint")]
        );
        assert_eq!(changes.structure.len(), 2);
        assert!(changes.structure.iter().any(|p| is_slint(p)));
    }

    #[test]
    fn detects_changes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ui")).unwrap();
        let file = dir.path().join("ui/main.slint");
        std::fs::write(&file, "A {}").unwrap();
        let tree = slint_studio_core::project::scan(&dir.path().canonicalize().unwrap()).unwrap();
        let watcher = FileWatcher::new(&tree.root, Arc::new(|| {})).unwrap();
        assert_eq!(watcher.watched_count(), 2);

        std::fs::write(&file, "B {}").unwrap();
        let canonical = file.canonicalize().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = FileChanges::default();
        while !seen.modified.contains(&canonical) && Instant::now() < deadline {
            let changes = watcher.poll();
            seen.modified.extend(changes.modified);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen.modified.contains(&canonical), "{seen:?}");
    }
}
