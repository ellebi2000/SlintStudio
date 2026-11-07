//! What "Compila" and "Esegui" do for the open folder.

use std::path::{Path, PathBuf};

/// How a project is built and run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectKind {
    /// A Rust crate or workspace: `cargo build` and `cargo run`.
    Cargo { manifest: PathBuf },
    /// Only `.slint` files: checked with the Slint compiler and opened with
    /// `slint-viewer`.
    Slint,
    /// Nothing Slint Studio knows how to build.
    Unsupported,
}

impl ProjectKind {
    /// Detects the kind of the project at `root`: a `Cargo.toml` makes it a
    /// Rust project, otherwise any `.slint` file makes it a Slint project.
    pub fn detect(root: &Path, has_slint_files: bool) -> Self {
        let manifest = root.join("Cargo.toml");
        if manifest.is_file() {
            Self::Cargo { manifest }
        } else if has_slint_files {
            Self::Slint
        } else {
            Self::Unsupported
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Cargo { .. } => "Rust (Cargo)",
            Self::Slint => "Slint",
            Self::Unsupported => "non supportato",
        }
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    /// What "Compila" runs, for tooltips.
    pub fn build_description(&self) -> &'static str {
        match self {
            Self::Cargo { .. } => "cargo build",
            Self::Slint => "controllo dei file .slint con il compilatore Slint",
            Self::Unsupported => "nessun Cargo.toml né file .slint nel progetto",
        }
    }

    /// What "Esegui" runs, for tooltips.
    pub fn run_description(&self) -> &'static str {
        match self {
            Self::Cargo { .. } => "cargo run",
            Self::Slint => "slint-viewer sul file in anteprima",
            Self::Unsupported => "nessun Cargo.toml né file .slint nel progetto",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detection() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ProjectKind::detect(dir.path(), false),
            ProjectKind::Unsupported
        );
        assert_eq!(ProjectKind::detect(dir.path(), true), ProjectKind::Slint);
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let kind = ProjectKind::detect(dir.path(), true);
        assert_eq!(
            kind,
            ProjectKind::Cargo {
                manifest: dir.path().join("Cargo.toml")
            }
        );
        assert!(kind.is_supported());
        assert_eq!(kind.run_description(), "cargo run");
        assert!(!ProjectKind::Unsupported.is_supported());
        // A directory named Cargo.toml is not a manifest.
        let other = tempfile::tempdir().unwrap();
        fs::create_dir(other.path().join("Cargo.toml")).unwrap();
        assert_eq!(
            ProjectKind::detect(other.path(), false),
            ProjectKind::Unsupported
        );
    }
}
