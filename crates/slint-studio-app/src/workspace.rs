//! The open project and its documents.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use directories::ProjectDirs;
use slint_studio_core::project::{self, FileTree};
use slint_studio_core::{
    Diagnostic, DiagnosticCounts, Document, OpenError, Outline, RecentProjects, app_dirs,
};
use tracing::warn;

use crate::ui::editor::EditorState;

/// Preferred files to open when a project is opened, in order.
const ENTRY_POINTS: &[&str] = &[
    "main.slint",
    "app-window.slint",
    "appwindow.slint",
    "app.slint",
];

pub struct Project {
    pub root: PathBuf,
    pub tree: FileTree,
    /// Expanded directories of the file tree.
    pub expanded: HashSet<PathBuf>,
}

impl Project {
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let tree = project::scan(&root)?;
        // Expand the first level so the structure is visible immediately.
        let expanded = tree
            .root
            .children()
            .iter()
            .filter(|node| node.is_dir())
            .map(|node| node.path.clone())
            .collect();
        Ok(Self {
            root,
            tree,
            expanded,
        })
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        self.tree = project::scan(&self.root)?;
        Ok(())
    }

    /// Expands the directories containing `path` so it is visible in the tree.
    pub fn reveal(&mut self, path: &Path) {
        if let Ok(relative) = path.strip_prefix(&self.root) {
            let mut dir = self.root.clone();
            for component in relative.parent().into_iter().flat_map(Path::components) {
                dir.push(component);
                self.expanded.insert(dir.clone());
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.tree.root.name
    }

    /// `path` relative to the project root, for display.
    pub fn relative<'a>(&self, path: &'a Path) -> &'a Path {
        path.strip_prefix(&self.root).unwrap_or(path)
    }

    /// The `.slint` file to open first: a conventional entry point, else the
    /// first `.slint` file of the tree.
    pub fn entry_point(&self) -> Option<PathBuf> {
        let files = self.tree.root.files();
        let slint = || files.iter().filter(|f| f.is_slint());
        ENTRY_POINTS
            .iter()
            .find_map(|name| slint().find(|f| f.name == *name))
            .or_else(|| slint().next())
            .map(|f| f.path.clone())
    }
}

pub struct OpenDocument {
    pub doc: Document,
    pub editor: EditorState,
    /// Parsed structure, cached per document version.
    outline: RefCell<Option<(i32, Rc<Outline>)>>,
}

impl OpenDocument {
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            editor: EditorState::default(),
            outline: RefCell::new(None),
        }
    }

    /// The document's elements and bindings (reparsed after edits).
    pub fn outline(&self) -> Rc<Outline> {
        let version = self.doc.version();
        let mut cache = self.outline.borrow_mut();
        match &*cache {
            Some((cached, outline)) if *cached == version => Rc::clone(outline),
            _ => {
                let outline = Rc::new(Outline::parse(self.doc.text()));
                *cache = Some((version, Rc::clone(&outline)));
                outline
            }
        }
    }
}

pub struct Workspace {
    pub project: Option<Project>,
    pub documents: Vec<OpenDocument>,
    pub active: Option<usize>,
    pub recent: RecentProjects,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Self {
        let recent = recent_file()
            .map(|file| {
                RecentProjects::load(&file, RecentProjects::DEFAULT_CAPACITY).unwrap_or_else(
                    |err| {
                        warn!("Impossibile leggere i progetti recenti: {err}");
                        RecentProjects::default()
                    },
                )
            })
            .unwrap_or_default();
        Self {
            project: None,
            documents: Vec::new(),
            active: None,
            recent,
        }
    }

    pub fn save_recent(&self) {
        if let Some(file) = recent_file()
            && let Err(err) = self.recent.save(&file)
        {
            warn!("Impossibile salvare i progetti recenti: {err}");
        }
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        self.documents.iter().position(|d| d.doc.path() == path)
    }

    pub fn document(&self, path: &Path) -> Option<&OpenDocument> {
        self.documents.iter().find(|d| d.doc.path() == path)
    }

    pub fn document_mut(&mut self, path: &Path) -> Option<&mut OpenDocument> {
        self.documents.iter_mut().find(|d| d.doc.path() == path)
    }

    pub fn active_document(&self) -> Option<&OpenDocument> {
        self.active.and_then(|i| self.documents.get(i))
    }

    pub fn active_document_mut(&mut self) -> Option<&mut OpenDocument> {
        self.active.and_then(|i| self.documents.get_mut(i))
    }

    /// Opens `path` (or focuses it if already open). Returns whether it was
    /// newly opened.
    pub fn open_document(&mut self, path: &Path) -> Result<bool, OpenError> {
        if let Some(project) = &mut self.project {
            project.reveal(path);
        }
        if let Some(index) = self.find(path) {
            self.active = Some(index);
            return Ok(false);
        }
        let doc = Document::open(path)?;
        self.documents.push(OpenDocument::new(doc));
        self.active = Some(self.documents.len() - 1);
        Ok(true)
    }

    pub fn close_document(&mut self, path: &Path) -> Option<OpenDocument> {
        let index = self.find(path)?;
        let closed = self.documents.remove(index);
        self.active = match self.active {
            _ if self.documents.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.min(self.documents.len() - 1)),
            other => other,
        };
        Some(closed)
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.documents.iter().any(|d| d.doc.is_dirty())
    }

    pub fn unsaved_names(&self) -> Vec<String> {
        self.documents
            .iter()
            .filter(|d| d.doc.is_dirty())
            .map(|d| d.doc.file_name())
            .collect()
    }
}

/// Diagnostics of every file, from the language server (and, later, the
/// preview and the compiler).
#[derive(Debug, Default)]
pub struct DiagnosticStore {
    by_path: BTreeMap<PathBuf, Vec<Diagnostic>>,
}

impl DiagnosticStore {
    pub fn set(&mut self, path: PathBuf, diagnostics: Vec<Diagnostic>) {
        if diagnostics.is_empty() {
            self.by_path.remove(&path);
        } else {
            self.by_path.insert(path, diagnostics);
        }
    }

    pub fn clear(&mut self) {
        self.by_path.clear();
    }

    pub fn for_path(&self, path: &Path) -> &[Diagnostic] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }

    /// All diagnostics, grouped by path, errors first within each file.
    pub fn all(&self) -> Vec<&Diagnostic> {
        let mut all = Vec::new();
        for diagnostics in self.by_path.values() {
            let mut sorted: Vec<_> = diagnostics.iter().collect();
            sorted.sort_by(|a, b| {
                b.severity.cmp(&a.severity).then_with(|| {
                    let start = |d: &Diagnostic| d.location.as_ref().map(|l| l.start);
                    start(a).cmp(&start(b))
                })
            });
            all.extend(sorted);
        }
        all
    }

    pub fn counts(&self) -> DiagnosticCounts {
        self.by_path.values().flatten().collect()
    }

    pub fn len(&self) -> usize {
        self.by_path.values().map(Vec::len).sum()
    }
}

fn recent_file() -> Option<PathBuf> {
    ProjectDirs::from(
        app_dirs::QUALIFIER,
        app_dirs::ORGANIZATION,
        app_dirs::APPLICATION,
    )
    .map(|dirs| dirs.config_dir().join("recent-projects.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint_studio_core::{DiagnosticSource, LineCol, Location, Severity};
    use std::fs;

    fn project_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ui")).unwrap();
        fs::write(dir.path().join("ui/zeta.slint"), "component Z {}").unwrap();
        fs::write(dir.path().join("ui/main.slint"), "export component M {}").unwrap();
        fs::write(dir.path().join("a.slint"), "component A {}").unwrap();
        dir
    }

    #[test]
    fn entry_point_prefers_conventional_names() {
        let dir = project_dir();
        let project = Project::open(dir.path()).unwrap();
        assert!(project.entry_point().unwrap().ends_with("ui/main.slint"));
        assert!(project.expanded.contains(&project.root.join("ui")));

        let mut project = project;
        project.expanded.clear();
        project.reveal(&project.root.join("ui/deep/file.slint"));
        assert!(project.expanded.contains(&project.root.join("ui")));
        assert!(project.expanded.contains(&project.root.join("ui/deep")));
        project.reveal(Path::new("/elsewhere/file.slint"));
        assert_eq!(project.expanded.len(), 2);

        fs::remove_file(dir.path().join("ui/main.slint")).unwrap();
        let project = Project::open(dir.path()).unwrap();
        assert!(project.entry_point().unwrap().ends_with("ui/zeta.slint"));
    }

    #[test]
    fn open_and_close_documents_keeps_active_index_valid() {
        let dir = project_dir();
        let root = dir.path().canonicalize().unwrap();
        let mut ws = Workspace {
            project: None,
            documents: Vec::new(),
            active: None,
            recent: RecentProjects::default(),
        };
        let a = root.join("a.slint");
        let main = root.join("ui/main.slint");
        let zeta = root.join("ui/zeta.slint");
        assert!(ws.open_document(&a).unwrap());
        assert!(ws.open_document(&main).unwrap());
        assert!(ws.open_document(&zeta).unwrap());
        assert!(
            !ws.open_document(&main).unwrap(),
            "already open: just focused"
        );
        assert_eq!(ws.active, Some(1));

        ws.close_document(&a);
        assert_eq!(ws.active_document().unwrap().doc.path(), main);
        ws.close_document(&main);
        assert_eq!(ws.active_document().unwrap().doc.path(), zeta);
        ws.close_document(&zeta);
        assert_eq!(ws.active, None);
        assert!(ws.open_document(&root.join("missing.slint")).is_err());
    }

    #[test]
    fn diagnostic_store_sorts_and_counts() {
        let diagnostic = |severity, line| Diagnostic {
            severity,
            source: DiagnosticSource::Lsp,
            message: String::new(),
            location: Some(Location::at("/p/a.slint", LineCol::new(line, 0))),
        };
        let mut store = DiagnosticStore::default();
        store.set(
            "/p/a.slint".into(),
            vec![
                diagnostic(Severity::Warning, 1),
                diagnostic(Severity::Error, 9),
                diagnostic(Severity::Error, 2),
            ],
        );
        let all = store.all();
        assert_eq!(all[0].severity, Severity::Error);
        assert_eq!(all[0].location.as_ref().unwrap().start.line, 2);
        assert_eq!(all[2].severity, Severity::Warning);
        assert_eq!(store.counts().errors, 2);

        store.set("/p/a.slint".into(), Vec::new());
        assert_eq!(store.len(), 0);
    }
}
