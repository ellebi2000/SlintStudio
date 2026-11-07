//! Most-recently-used project folders, stored as a plain text file with one
//! path per line.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentProjects {
    paths: Vec<PathBuf>,
    capacity: usize,
}

impl Default for RecentProjects {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

impl RecentProjects {
    pub const DEFAULT_CAPACITY: usize = 10;

    pub fn new(capacity: usize) -> Self {
        Self {
            paths: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Most recent first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Moves `path` to the front, dropping the oldest entry when full.
    pub fn touch(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(self.capacity);
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
    }

    /// Loads the list from `file`; a missing file yields an empty list.
    pub fn load(file: &Path, capacity: usize) -> io::Result<Self> {
        let mut recent = Self::new(capacity);
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(recent),
            Err(err) => return Err(err),
        };
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let path = PathBuf::from(line);
            if !recent.paths.contains(&path) && recent.paths.len() < recent.capacity {
                recent.paths.push(path);
            }
        }
        Ok(recent)
    }

    /// Writes the list to `file`, creating its parent directory.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut text = String::new();
        for path in &self.paths {
            text.push_str(&path.to_string_lossy());
            text.push('\n');
        }
        fs::write(file, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_moves_to_front_without_duplicates() {
        let mut recent = RecentProjects::new(3);
        for p in ["/a", "/b", "/c", "/a"] {
            recent.touch(Path::new(p));
        }
        assert_eq!(
            recent.paths(),
            [PathBuf::from("/a"), "/c".into(), "/b".into()]
        );
        recent.touch(Path::new("/d"));
        assert_eq!(recent.paths().len(), 3);
        assert_eq!(recent.paths()[0], PathBuf::from("/d"));
        assert!(!recent.paths().contains(&PathBuf::from("/b")));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config/recent.txt");
        let mut recent = RecentProjects::default();
        recent.touch(Path::new("/progetti/uno"));
        recent.touch(Path::new("/progetti/due con spazi"));
        recent.save(&file).unwrap();

        let loaded = RecentProjects::load(&file, 10).unwrap();
        assert_eq!(loaded, recent);
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = RecentProjects::load(&dir.path().join("none.txt"), 5).unwrap();
        assert!(loaded.paths().is_empty());
    }

    #[test]
    fn load_respects_capacity_and_skips_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recent.txt");
        fs::write(&file, "/a\n\n/b\n/a\n/c\n").unwrap();
        let loaded = RecentProjects::load(&file, 2).unwrap();
        assert_eq!(loaded.paths(), [PathBuf::from("/a"), "/b".into()]);
    }
}
