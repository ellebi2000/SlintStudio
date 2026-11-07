//! Designer state: the selected element, property editor drafts and the
//! resize gesture.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::CursorIcon;
use slint_studio_preview::ElementRect;

/// An element of a document, identified by its tree path (stable across
/// property edits; see `Outline::tree_path`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ElementRef {
    pub path: PathBuf,
    pub tree_path: Vec<usize>,
}

/// Drag-and-drop payload of a palette item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteDrag(pub &'static str);

/// Resize handles around the selected element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
    ];

    /// Position on the rectangle, as fractions of width and height.
    pub fn anchor(self) -> (f32, f32) {
        match self {
            Self::NorthWest => (0.0, 0.0),
            Self::North => (0.5, 0.0),
            Self::NorthEast => (1.0, 0.0),
            Self::East => (1.0, 0.5),
            Self::SouthEast => (1.0, 1.0),
            Self::South => (0.5, 1.0),
            Self::SouthWest => (0.0, 1.0),
            Self::West => (0.0, 0.5),
        }
    }

    pub fn cursor(self) -> CursorIcon {
        match self {
            Self::North | Self::South => CursorIcon::ResizeVertical,
            Self::East | Self::West => CursorIcon::ResizeHorizontal,
            Self::NorthEast | Self::SouthWest => CursorIcon::ResizeNeSw,
            Self::NorthWest | Self::SouthEast => CursorIcon::ResizeNwSe,
        }
    }

    /// Which edges move: (left, right, top, bottom).
    fn edges(self) -> (bool, bool, bool, bool) {
        let (fx, fy) = self.anchor();
        (fx == 0.0, fx == 1.0, fy == 0.0, fy == 1.0)
    }

    pub fn changes_width(self) -> bool {
        let (left, right, _, _) = self.edges();
        left || right
    }

    pub fn changes_height(self) -> bool {
        let (_, _, top, bottom) = self.edges();
        top || bottom
    }

    pub fn moves_x(self) -> bool {
        self.edges().0
    }

    pub fn moves_y(self) -> bool {
        self.edges().2
    }
}

/// Logical rectangle `(x, y, width, height)`.
pub type Bounds = (f32, f32, f32, f32);

/// The rectangle after dragging `handle` by `(dx, dy)` logical pixels.
/// Sizes never go below one pixel; the opposite edge stays put.
pub fn resized(start: Bounds, handle: Handle, dx: f32, dy: f32) -> Bounds {
    let (x, y, width, height) = start;
    let (left, right, top, bottom) = handle.edges();
    let (mut x0, mut x1, mut y0, mut y1) = (x, x + width, y, y + height);
    if left {
        x0 = (x0 + dx).min(x1 - 1.0);
    }
    if right {
        x1 = (x1 + dx).max(x0 + 1.0);
    }
    if top {
        y0 = (y0 + dy).min(y1 - 1.0);
    }
    if bottom {
        y1 = (y1 + dy).max(y0 + 1.0);
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// An ongoing resize gesture.
#[derive(Debug, Clone, PartialEq)]
pub struct ResizeDrag {
    pub element: ElementRef,
    pub handle: Handle,
    pub start: ElementRect,
    /// Accumulated pointer movement, in logical pixels.
    pub delta: (f32, f32),
}

impl ResizeDrag {
    pub fn bounds(&self) -> Bounds {
        resized(
            (
                self.start.x,
                self.start.y,
                self.start.width,
                self.start.height,
            ),
            self.handle,
            self.delta.0,
            self.delta.1,
        )
    }
}

#[derive(Debug, Default)]
pub struct DesignerState {
    pub selected: Option<ElementRef>,
    /// Text being edited in the property editor, by property name.
    pub drafts: HashMap<String, String>,
    /// Element the drafts belong to.
    pub drafts_for: Option<ElementRef>,
    pub resize: Option<ResizeDrag>,
    /// Editor cursor the selection was last derived from.
    pub synced_cursor: Option<(PathBuf, usize)>,
}

impl DesignerState {
    /// Drops drafts that belong to another element.
    pub fn drafts_for(&mut self, element: &ElementRef) -> &mut HashMap<String, String> {
        if self.drafts_for.as_ref() != Some(element) {
            self.drafts.clear();
            self.drafts_for = Some(element.clone());
        }
        &mut self.drafts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizing_moves_only_the_dragged_edges() {
        let start = (10.0, 20.0, 100.0, 50.0);
        assert_eq!(
            resized(start, Handle::East, 30.0, 99.0),
            (10.0, 20.0, 130.0, 50.0)
        );
        assert_eq!(
            resized(start, Handle::South, 99.0, -10.0),
            (10.0, 20.0, 100.0, 40.0)
        );
        assert_eq!(
            resized(start, Handle::NorthWest, 5.0, 5.0),
            (15.0, 25.0, 95.0, 45.0)
        );
        assert_eq!(
            resized(start, Handle::SouthEast, 10.0, 10.0),
            (10.0, 20.0, 110.0, 60.0)
        );
    }

    #[test]
    fn sizes_never_collapse() {
        let start = (0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            resized(start, Handle::East, -50.0, 0.0),
            (0.0, 0.0, 1.0, 10.0)
        );
        assert_eq!(
            resized(start, Handle::West, 50.0, 0.0),
            (9.0, 0.0, 1.0, 10.0)
        );
    }

    #[test]
    fn handle_properties() {
        assert!(Handle::East.changes_width() && !Handle::East.changes_height());
        assert!(Handle::NorthWest.moves_x() && Handle::NorthWest.moves_y());
        assert!(!Handle::SouthEast.moves_x());
        assert_eq!(Handle::North.cursor(), CursorIcon::ResizeVertical);
    }

    #[test]
    fn drafts_reset_when_the_element_changes() {
        let mut designer = DesignerState::default();
        let a = ElementRef {
            path: "a.slint".into(),
            tree_path: vec![0],
        };
        let b = ElementRef {
            path: "a.slint".into(),
            tree_path: vec![0, 1],
        };
        designer
            .drafts_for(&a)
            .insert("width".into(), "10px".into());
        assert_eq!(designer.drafts_for(&a).len(), 1);
        assert!(designer.drafts_for(&b).is_empty());
    }
}
