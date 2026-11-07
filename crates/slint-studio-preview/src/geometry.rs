//! Rendered geometry of a document's elements, for the designer.
//!
//! **This is the only module that uses Slint's `internal-highlight` API**
//! (`ComponentInstance::component_positions`), which is not part of Slint's
//! stable public API and may change between releases. Slint is pinned to an
//! exact version in the workspace `Cargo.toml`; when upgrading it, this is
//! the file to check.

use std::path::Path;

use slint_interpreter::ComponentInstance;
use slint_studio_core::Outline;

/// An element's rectangle in logical pixels, in the window's coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Rotation in degrees around the centre.
    pub angle: f32,
    /// Position relative to the parent: what `x` and `y` bindings mean.
    pub local_x: f32,
    pub local_y: f32,
}

impl ElementRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    pub fn area(&self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }
}

/// Where an element of the source was drawn. Repeated elements (`for`)
/// and components used several times have several rectangles.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementGeometry {
    /// See [`Outline::tree_path`].
    pub tree_path: Vec<usize>,
    pub rects: Vec<ElementRect>,
}

/// Collects the rectangles of every element of `source` (the text the
/// instance was compiled from, for the file at `path`).
pub(crate) fn collect(
    instance: &ComponentInstance,
    path: &Path,
    source: &str,
) -> Vec<ElementGeometry> {
    let outline = Outline::parse(source);
    outline
        .elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| {
            let offset = u32::try_from(element.key_offset).ok()?;
            let rects: Vec<ElementRect> = instance
                .component_positions(path, offset)
                .iter()
                .map(|highlighted| ElementRect {
                    x: highlighted.rect.origin.x,
                    y: highlighted.rect.origin.y,
                    width: highlighted.rect.size.width,
                    height: highlighted.rect.size.height,
                    angle: highlighted.angle,
                    local_x: highlighted.local_rect.origin.x,
                    local_y: highlighted.local_rect.origin.y,
                })
                .collect();
            (!rects.is_empty()).then(|| ElementGeometry {
                tree_path: outline.tree_path(index),
                rects,
            })
        })
        .collect()
}

/// The element drawn at `(x, y)`: the smallest rectangle containing the
/// point, the deepest element on ties (a component's root and its use
/// site can share a rectangle).
pub fn hit_test(elements: &[ElementGeometry], x: f32, y: f32) -> Option<&ElementGeometry> {
    elements
        .iter()
        .filter_map(|element| {
            element
                .rects
                .iter()
                .filter(|r| r.contains(x, y))
                .map(ElementRect::area)
                .min_by(f32::total_cmp)
                .map(|area| (area, element))
        })
        .min_by(|(a_area, a), (b_area, b)| {
            a_area
                .total_cmp(b_area)
                .then_with(|| b.tree_path.len().cmp(&a.tree_path.len()))
        })
        .map(|(_, element)| element)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> ElementRect {
        ElementRect {
            x,
            y,
            width,
            height,
            angle: 0.0,
            local_x: x,
            local_y: y,
        }
    }

    #[test]
    fn hit_test_prefers_the_smallest_then_deepest_element() {
        let elements = vec![
            ElementGeometry {
                tree_path: vec![0],
                rects: vec![rect(0.0, 0.0, 100.0, 100.0)],
            },
            ElementGeometry {
                tree_path: vec![0, 0],
                rects: vec![rect(10.0, 10.0, 50.0, 50.0)],
            },
            ElementGeometry {
                tree_path: vec![0, 0, 0],
                rects: vec![rect(10.0, 10.0, 50.0, 50.0)],
            },
            ElementGeometry {
                tree_path: vec![0, 1],
                rects: vec![rect(70.0, 0.0, 10.0, 10.0), rect(70.0, 20.0, 10.0, 10.0)],
            },
        ];
        let hit = |x, y| hit_test(&elements, x, y).map(|e| e.tree_path.clone());
        assert_eq!(hit(5.0, 5.0), Some(vec![0]));
        assert_eq!(hit(20.0, 20.0), Some(vec![0, 0, 0]));
        assert_eq!(hit(75.0, 25.0), Some(vec![0, 1]), "second repetition");
        assert_eq!(hit(500.0, 5.0), None);
    }
}
