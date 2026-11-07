//! User-level commands.
//!
//! Menus, the toolbar and (from Phase 5) keyboard shortcuts and the command
//! palette never mutate application state directly: they enqueue a
//! [`Command`], which the app executes once per frame after the UI pass.
//! This keeps the borrow of the dock layout simple and gives a single place
//! where every action is implemented and logged.

use slint_studio_core::ViewMode;

use crate::layout::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetViewMode(ViewMode),
    ToggleLivePreview,
    TogglePanel(Tab),
    ResetLayout,
    ShowAbout,
    OpenSlintDocs,
    Quit,
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
