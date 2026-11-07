//! Project folder model: file tree, name filtering and text search.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directories never shown in the tree nor searched.
const IGNORED_DIRS: &[&str] = &["target", "node_modules", "__pycache__"];

/// Upper bound on the number of entries scanned, to keep huge folders (e.g. a
/// home directory opened by mistake) from freezing the UI.
pub const MAX_TREE_ENTRIES: usize = 20_000;

/// Files larger than this are skipped by [`search`].
const MAX_SEARCH_FILE_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Directory { children: Vec<FileNode> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
}

impl FileNode {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Directory { .. })
    }

    pub fn children(&self) -> &[FileNode] {
        match &self.kind {
            NodeKind::Directory { children } => children,
            NodeKind::File => &[],
        }
    }

    pub fn is_slint(&self) -> bool {
        !self.is_dir()
            && Path::new(&self.name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("slint"))
    }

    /// Whether this node, or any descendant, has a name containing `query`
    /// (case-insensitive; `query` must already be lowercase).
    pub fn matches(&self, query_lowercase: &str) -> bool {
        query_lowercase.is_empty()
            || self.name.to_lowercase().contains(query_lowercase)
            || self.children().iter().any(|c| c.matches(query_lowercase))
    }

    /// All files below this node, depth first.
    pub fn files(&self) -> Vec<&FileNode> {
        let mut files = Vec::new();
        self.collect_files(&mut files);
        files
    }

    fn collect_files<'a>(&'a self, out: &mut Vec<&'a FileNode>) {
        match &self.kind {
            NodeKind::File => out.push(self),
            NodeKind::Directory { children } => {
                for child in children {
                    child.collect_files(out);
                }
            }
        }
    }
}

/// The scanned content of a project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTree {
    pub root: FileNode,
    /// `true` when [`MAX_TREE_ENTRIES`] was reached and the tree is partial.
    pub truncated: bool,
}

/// Scans `root` recursively. Hidden entries (starting with '.') and build
/// directories are skipped; unreadable sub-directories are shown empty.
pub fn scan(root: &Path) -> io::Result<FileTree> {
    let mut budget = MAX_TREE_ENTRIES;
    let children = scan_dir(root, &mut budget, true)?;
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    Ok(FileTree {
        root: FileNode {
            name,
            path: root.to_path_buf(),
            kind: NodeKind::Directory { children },
        },
        truncated: budget == 0,
    })
}

fn scan_dir(dir: &Path, budget: &mut usize, is_root: bool) -> io::Result<Vec<FileNode>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if is_root => return Err(err),
        Err(_) => return Ok(Vec::new()),
    };
    let mut nodes = Vec::new();
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // `file_type` does not follow symlinks: linked directories are listed
        // as files, which avoids cycles.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        *budget -= 1;
        if file_type.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            let children = scan_dir(&path, budget, false)?;
            nodes.push(FileNode {
                name,
                path,
                kind: NodeKind::Directory { children },
            });
        } else {
            nodes.push(FileNode {
                name,
                path,
                kind: NodeKind::File,
            });
        }
    }
    nodes.sort_by(compare_nodes);
    Ok(nodes)
}

/// Directories first, then case-insensitive alphabetical order.
fn compare_nodes(a: &FileNode, b: &FileNode) -> Ordering {
    b.is_dir()
        .cmp(&a.is_dir())
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

/// One line matching a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    /// 0-based line.
    pub line: u32,
    /// 0-based column, in characters.
    pub column: u32,
    /// The matching line, trimmed and shortened for display.
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub files_searched: usize,
    /// `true` when `max_hits` was reached.
    pub truncated: bool,
}

/// Searches every text file of `tree` for `query` (first match per line).
pub fn search(
    tree: &FileTree,
    query: &str,
    case_sensitive: bool,
    max_hits: usize,
) -> SearchResults {
    let mut results = SearchResults {
        hits: Vec::new(),
        files_searched: 0,
        truncated: false,
    };
    if query.is_empty() {
        return results;
    }
    let needle = if case_sensitive {
        query.to_owned()
    } else {
        query.to_lowercase()
    };

    'files: for file in tree.root.files() {
        let too_big = fs::metadata(&file.path).map_or(true, |m| m.len() > MAX_SEARCH_FILE_SIZE);
        if too_big {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file.path) else {
            continue; // binary or unreadable
        };
        results.files_searched += 1;
        for (line_number, line) in text.lines().enumerate() {
            let haystack = if case_sensitive {
                line.to_owned()
            } else {
                line.to_lowercase()
            };
            let Some(byte) = haystack.find(&needle) else {
                continue;
            };
            if results.hits.len() == max_hits {
                results.truncated = true;
                break 'files;
            }
            // Lowercasing may change byte lengths; count chars in the haystack.
            let column = haystack[..byte].chars().count();
            results.hits.push(SearchHit {
                path: file.path.clone(),
                line: u32::try_from(line_number).unwrap_or(u32::MAX),
                column: u32::try_from(column).unwrap_or(u32::MAX),
                preview: preview(line),
            });
        }
    }
    results
}

fn preview(line: &str) -> String {
    const MAX_CHARS: usize = 160;
    let trimmed = line.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        trimmed.to_owned()
    } else {
        let mut short: String = trimmed.chars().take(MAX_CHARS).collect();
        short.push('…');
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("ui/widgets")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join("ui/main.slint"),
            "export component Main {\n    Text { text: \"Ciao\"; }\n}\n",
        )
        .unwrap();
        fs::write(root.join("ui/widgets/Card.slint"), "component Card { }\n").unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(root.join("README.md"), "Una demo con TEXT maiuscolo\n").unwrap();
        fs::write(root.join("target/debug/out.slint"), "Text {}").unwrap();
        fs::write(root.join(".git/config"), "x").unwrap();
        dir
    }

    fn names(nodes: &[FileNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.name.as_str()).collect()
    }

    #[test]
    fn scan_skips_hidden_and_build_dirs_and_sorts_dirs_first() {
        let dir = project();
        let tree = scan(dir.path()).unwrap();
        assert!(!tree.truncated);
        assert_eq!(
            names(tree.root.children()),
            ["ui", "Cargo.toml", "README.md"]
        );
        let ui = &tree.root.children()[0];
        assert_eq!(names(ui.children()), ["widgets", "main.slint"]);
        assert!(ui.children()[1].is_slint());
        assert_eq!(tree.root.files().len(), 4);
    }

    #[test]
    fn scanning_a_missing_folder_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn matches_looks_into_descendants() {
        let dir = project();
        let tree = scan(dir.path()).unwrap();
        let ui = &tree.root.children()[0];
        assert!(ui.matches("card"));
        assert!(ui.matches(""));
        assert!(!ui.matches("cargo"));
    }

    #[test]
    fn search_is_case_insensitive_by_default() {
        let dir = project();
        let tree = scan(dir.path()).unwrap();
        let results = search(&tree, "text", false, 100);
        assert_eq!(results.files_searched, 4);
        assert_eq!(results.hits.len(), 2);
        let main = results
            .hits
            .iter()
            .find(|h| h.path.ends_with("main.slint"))
            .unwrap();
        assert_eq!((main.line, main.column), (1, 4));
        assert_eq!(main.preview, "Text { text: \"Ciao\"; }");

        let exact = search(&tree, "TEXT", true, 100);
        assert_eq!(exact.hits.len(), 1);
        assert!(exact.hits[0].path.ends_with("README.md"));
    }

    #[test]
    fn search_stops_at_max_hits() {
        let dir = project();
        let tree = scan(dir.path()).unwrap();
        let results = search(&tree, "e", false, 2);
        assert_eq!(results.hits.len(), 2);
        assert!(results.truncated);
        assert!(search(&tree, "", false, 10).hits.is_empty());
    }
}
