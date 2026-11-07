//! An open text document: content, edit version and saved state.
//!
//! Phase 3 extends this model with the text ↔ element mapping used to keep
//! the editor and the design canvas in sync.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::text::LineIndex;

/// Files larger than this are not opened in the editor.
pub const MAX_DOCUMENT_SIZE: u64 = 8 * 1024 * 1024;

/// Language of a document, deciding highlighting and language server use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Slint,
    PlainText,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("slint") => Self::Slint,
            _ => Self::PlainText,
        }
    }
}

/// Why a file could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("il file è troppo grande ({size} byte, massimo {MAX_DOCUMENT_SIZE})")]
    TooLarge { size: u64 },
    #[error("il file non è testo UTF-8")]
    NotUtf8,
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
pub struct Document {
    path: PathBuf,
    language: Language,
    text: String,
    /// Incremented on every edit; sent to the language server so that late
    /// diagnostics for an older version can be recognised.
    version: i32,
    dirty: bool,
    line_index: LineIndex,
}

impl Document {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        let path = path.into();
        let text = text.into();
        Self {
            language: Language::from_path(&path),
            line_index: LineIndex::new(&text),
            path,
            text,
            version: 1,
            dirty: false,
        }
    }

    /// Reads a UTF-8 text file from disk.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, OpenError> {
        let path = path.into();
        let size = fs::metadata(&path)?.len();
        if size > MAX_DOCUMENT_SIZE {
            return Err(OpenError::TooLarge { size });
        }
        let text = String::from_utf8(fs::read(&path)?).map_err(|_| OpenError::NotUtf8)?;
        Ok(Self::new(path, text))
    }

    /// Writes the content to disk and marks the document as saved.
    pub fn save(&mut self) -> io::Result<()> {
        fs::write(&self.path, &self.text)?;
        self.dirty = false;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    /// Replaces the whole content (e.g. after a completion was accepted).
    pub fn set_text(&mut self, text: String) {
        if text != self.text {
            self.text = text;
            self.mark_edited();
        }
    }

    /// Mutable access for widgets that edit the buffer in place.
    ///
    /// Callers **must** call [`Self::mark_edited`] when they changed it.
    pub fn buffer_mut(&mut self) -> &mut String {
        &mut self.text
    }

    /// Records an in-place edit made through [`Self::buffer_mut`].
    pub fn mark_edited(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.dirty = true;
        self.line_index = LineIndex::new(&self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_is_detected_from_extension() {
        assert_eq!(
            Language::from_path(Path::new("ui/app.slint")),
            Language::Slint
        );
        assert_eq!(Language::from_path(Path::new("APP.SLINT")), Language::Slint);
        assert_eq!(
            Language::from_path(Path::new("main.rs")),
            Language::PlainText
        );
        assert_eq!(
            Language::from_path(Path::new("Makefile")),
            Language::PlainText
        );
    }

    #[test]
    fn edits_bump_version_and_mark_dirty() {
        let mut doc = Document::new("a.slint", "Text {}");
        assert!(!doc.is_dirty());
        let v1 = doc.version();

        doc.set_text("Text {}".into()); // unchanged: no new version
        assert_eq!(doc.version(), v1);
        assert!(!doc.is_dirty());

        doc.buffer_mut().push('\n');
        doc.mark_edited();
        assert!(doc.version() > v1);
        assert!(doc.is_dirty());
        assert_eq!(doc.line_index().line_count(), 2);
    }

    #[test]
    fn open_and_save_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.slint");
        fs::write(&path, "export component A {}").unwrap();

        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.text(), "export component A {}");
        assert_eq!(doc.file_name(), "main.slint");

        doc.set_text("export component B {}".into());
        assert!(doc.is_dirty());
        doc.save().unwrap();
        assert!(!doc.is_dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "export component B {}");
    }

    #[test]
    fn binary_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        fs::write(&path, [0x89, 0x50, 0xff, 0xfe]).unwrap();
        assert!(matches!(Document::open(&path), Err(OpenError::NotUtf8)));
    }
}
