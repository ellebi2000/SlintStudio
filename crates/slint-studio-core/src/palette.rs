//! Catalogue of the components offered by the designer palette.
//!
//! The palette is static data: Phase 3 uses [`PaletteItem::snippet`] to insert
//! a component into the `.slint` source at the chosen position.

/// Grouping used by the palette UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentCategory {
    /// Built-in elements, always available without imports.
    Basic,
    /// Built-in layout elements.
    Layout,
    /// Widgets from `std-widgets.slint`.
    Widget,
}

impl ComponentCategory {
    pub const ALL: [Self; 3] = [Self::Basic, Self::Layout, Self::Widget];

    pub fn label(self) -> &'static str {
        match self {
            Self::Basic => "Elementi di base",
            Self::Layout => "Layout",
            Self::Widget => "Widget standard",
        }
    }
}

/// A component that can be inserted from the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteItem {
    /// Element name as written in `.slint` code.
    pub name: &'static str,
    pub category: ComponentCategory,
    /// Minimal source inserted into the document (without indentation).
    pub snippet: &'static str,
}

impl PaletteItem {
    /// Module the component must be imported from, if any.
    pub fn import_from(&self) -> Option<&'static str> {
        match self.category {
            ComponentCategory::Widget => Some("std-widgets.slint"),
            ComponentCategory::Basic | ComponentCategory::Layout => None,
        }
    }

    /// All palette items, grouped by category in display order.
    pub fn all() -> &'static [PaletteItem] {
        PALETTE
    }

    /// Items of a single category, in display order.
    pub fn in_category(category: ComponentCategory) -> impl Iterator<Item = &'static PaletteItem> {
        PALETTE.iter().filter(move |item| item.category == category)
    }
}

const fn item(
    name: &'static str,
    category: ComponentCategory,
    snippet: &'static str,
) -> PaletteItem {
    PaletteItem {
        name,
        category,
        snippet,
    }
}

use ComponentCategory::{Basic, Layout, Widget};

static PALETTE: &[PaletteItem] = &[
    item(
        "Rectangle",
        Basic,
        "Rectangle {\n    width: 100px;\n    height: 100px;\n    background: #4ec9b0;\n}",
    ),
    item("Text", Basic, "Text {\n    text: \"Testo\";\n}"),
    item("Image", Basic, "Image {\n    source: @image-url(\"\");\n}"),
    item("TouchArea", Basic, "TouchArea {\n    clicked => { }\n}"),
    item("FocusScope", Basic, "FocusScope { }"),
    item("Flickable", Basic, "Flickable { }"),
    item(
        "Path",
        Basic,
        "Path {\n    commands: \"M 0 0 L 100 100\";\n    stroke: black;\n    stroke-width: 1px;\n}",
    ),
    item(
        "HorizontalLayout",
        Layout,
        "HorizontalLayout {\n    spacing: 8px;\n}",
    ),
    item(
        "VerticalLayout",
        Layout,
        "VerticalLayout {\n    spacing: 8px;\n}",
    ),
    item("GridLayout", Layout, "GridLayout {\n    spacing: 8px;\n}"),
    item("Button", Widget, "Button {\n    text: \"Pulsante\";\n}"),
    item("CheckBox", Widget, "CheckBox {\n    text: \"Opzione\";\n}"),
    item("Switch", Widget, "Switch {\n    text: \"Interruttore\";\n}"),
    item(
        "LineEdit",
        Widget,
        "LineEdit {\n    placeholder-text: \"Testo\";\n}",
    ),
    item("TextEdit", Widget, "TextEdit { }"),
    item("SpinBox", Widget, "SpinBox { }"),
    item("Slider", Widget, "Slider { }"),
    item(
        "ComboBox",
        Widget,
        "ComboBox {\n    model: [\"Uno\", \"Due\"];\n}",
    ),
    item(
        "ProgressIndicator",
        Widget,
        "ProgressIndicator {\n    progress: 50%;\n}",
    ),
    item("ScrollView", Widget, "ScrollView { }"),
    item("ListView", Widget, "ListView { }"),
    item("TabWidget", Widget, "TabWidget { }"),
    item("GroupBox", Widget, "GroupBox {\n    title: \"Gruppo\";\n}"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique() {
        let names: HashSet<_> = PaletteItem::all().iter().map(|i| i.name).collect();
        assert_eq!(names.len(), PaletteItem::all().len());
    }

    #[test]
    fn every_category_is_populated() {
        for category in ComponentCategory::ALL {
            assert!(
                PaletteItem::in_category(category).next().is_some(),
                "{category:?}"
            );
        }
    }

    #[test]
    fn snippets_start_with_the_element_name_and_are_balanced() {
        for item in PaletteItem::all() {
            assert!(item.snippet.starts_with(item.name), "{}", item.name);
            let open = item.snippet.matches('{').count();
            let close = item.snippet.matches('}').count();
            assert_eq!(open, close, "unbalanced braces in {}", item.name);
        }
    }

    #[test]
    fn only_std_widgets_need_an_import() {
        for item in PaletteItem::all() {
            let needs_import = item.category == ComponentCategory::Widget;
            assert_eq!(item.import_from().is_some(), needs_import, "{}", item.name);
        }
    }

    #[test]
    fn items_are_grouped_by_category_in_display_order() {
        let order: Vec<_> = PaletteItem::all().iter().map(|i| i.category).collect();
        let mut sorted = order.clone();
        sorted.sort_by_key(|c| ComponentCategory::ALL.iter().position(|x| x == c));
        assert_eq!(order, sorted);
    }
}
