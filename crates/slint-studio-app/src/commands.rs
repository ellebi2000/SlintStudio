//! User-level commands.
//!
//! Menus, the toolbar, keyboard shortcuts and widgets never mutate application
//! state directly: they enqueue a [`Command`], which the app executes once per
//! frame after the UI pass. This keeps borrows simple (the dock layout and the
//! language server are owned outside the UI state) and gives a single place
//! where every action is implemented and logged.

use std::path::PathBuf;

use slint_studio_core::{LineCol, ViewMode};

use crate::designer::ElementRef;
use crate::layout::Tab;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    SetViewMode(ViewMode),
    ToggleLivePreview,
    TogglePanel(Tab),
    ShowPanel(Tab),
    ResetLayout,
    ShowAbout,
    OpenSlintDocs,
    Quit,

    // Project
    OpenFolderDialog,
    OpenProject(PathBuf),
    CloseProject,
    RefreshTree,
    RemoveRecent(PathBuf),
    RunSearch,

    // Documents
    OpenFile(PathBuf),
    /// Opens a file and moves the cursor to `position` (0-based, columns in
    /// characters).
    OpenLocation {
        path: PathBuf,
        position: LineCol,
    },
    ActivateDocument(PathBuf),
    CloseDocument(PathBuf),
    CloseActiveDocument,
    Save,
    SaveAll,
    Undo,
    Redo,
    /// The buffer of an open document changed (sent by the editor).
    DocumentEdited(PathBuf),

    // Language server
    RequestCompletion {
        path: PathBuf,
        char_index: usize,
    },
    RequestDefinition {
        path: PathBuf,
        char_index: usize,
    },
    RestartLsp,
    /// Re-renders the preview now, even with live preview off.
    RefreshPreview,

    // Designer
    /// Selects an element (from the canvas) and shows it in the editor.
    SelectElement(Option<ElementRef>),
    /// Sets (or with `None` removes) a property binding.
    SetProperty {
        element: ElementRef,
        name: String,
        value: Option<String>,
    },
    /// Inserts a palette item as a child of `parent` (default: the selected
    /// element, else the previewed component's root).
    InsertComponent {
        item: &'static str,
        parent: Option<ElementRef>,
    },
    /// Writes a new geometry, in whole logical pixels.
    ResizeElement {
        element: ElementRef,
        x: Option<i32>,
        y: Option<i32>,
        width: Option<i32>,
        height: Option<i32>,
    },
    RemoveElement(ElementRef),

    /// Answer to the "unsaved changes" dialog.
    ResolveUnsaved(UnsavedChoice),
}

/// An action that would discard unsaved changes and needs confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardedAction {
    CloseDocument(PathBuf),
    CloseProject,
    OpenProject(PathBuf),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

/// A feature shown in the UI but implemented in a later phase. Used to render
/// disabled menu entries and buttons with an explanatory tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Planned {
    pub label: &'static str,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    pub phase: u8,
}

impl Planned {
    pub const fn new(label: &'static str, phase: u8) -> Self {
        Self {
            label,
            icon: None,
            shortcut: None,
            phase,
        }
    }

    pub const fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub const fn shortcut(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    pub fn tooltip(&self) -> String {
        format!("Disponibile dalla Fase {}", self.phase)
    }
}
