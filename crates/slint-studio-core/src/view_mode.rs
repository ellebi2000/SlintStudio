//! Layout modes of the central workspace area.

/// How the central workspace area is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewMode {
    /// Code editor only.
    Code,
    /// Design canvas only.
    Design,
    /// Editor on top, design canvas below, kept in sync.
    #[default]
    Split,
}

impl ViewMode {
    /// All modes, in the order they are presented in the UI.
    pub const ALL: [Self; 3] = [Self::Code, Self::Split, Self::Design];

    /// Human readable (Italian) label used in menus and the toolbar.
    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "Codice",
            Self::Design => "Design",
            Self::Split => "Split",
        }
    }

    /// Whether the code editor is visible in this mode.
    pub fn shows_editor(self) -> bool {
        matches!(self, Self::Code | Self::Split)
    }

    /// Whether the design canvas (live preview) is visible in this mode.
    ///
    /// The component palette and property editor are only meaningful when the
    /// canvas is visible, so the UI uses this to decide what they show.
    pub fn shows_canvas(self) -> bool {
        matches!(self, Self::Design | Self::Split)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_is_the_default_mode() {
        assert_eq!(ViewMode::default(), ViewMode::Split);
    }

    #[test]
    fn every_mode_shows_something() {
        for mode in ViewMode::ALL {
            assert!(mode.shows_editor() || mode.shows_canvas(), "{mode:?}");
        }
    }

    #[test]
    fn split_shows_both_editor_and_canvas() {
        assert!(ViewMode::Split.shows_editor());
        assert!(ViewMode::Split.shows_canvas());
        assert!(!ViewMode::Code.shows_canvas());
        assert!(!ViewMode::Design.shows_editor());
    }

    #[test]
    fn labels_are_unique() {
        let mut labels: Vec<_> = ViewMode::ALL.iter().map(|m| m.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), ViewMode::ALL.len());
    }
}
