//! Right side: component palette and property editor (Design / Split modes).

use egui::{AtomLayout, CollapsingHeader, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use slint_studio_core::{ComponentCategory, PaletteItem};

use super::{empty_state, padded, with_icon};
use crate::app::AppState;

pub fn components(ui: &mut Ui, state: &AppState) {
    if !state.view_mode.shows_canvas() {
        return canvas_hidden(ui, "la palette dei componenti");
    }
    padded(ui, |ui| {
        for category in ComponentCategory::ALL {
            CollapsingHeader::new(RichText::new(category.label()).strong())
                .id_salt(category.label())
                .default_open(true)
                .show(ui, |ui| {
                    for item in PaletteItem::in_category(category) {
                        palette_entry(ui, item);
                    }
                });
        }
    });
}

fn palette_entry(ui: &mut Ui, item: &PaletteItem) {
    let mut tooltip = String::from("Inserimento nel canvas disponibile dalla Fase 3");
    if let Some(module) = item.import_from() {
        tooltip.push_str(&format!("\nimport {{ {} }} from \"{module}\";", item.name));
    }
    let entry =
        AtomLayout::new(with_icon(category_icon(item.category), item.name)).sense(Sense::hover());
    ui.add(entry).on_hover_text(tooltip);
}

fn category_icon(category: ComponentCategory) -> &'static str {
    match category {
        ComponentCategory::Basic => icon::SHAPES,
        ComponentCategory::Layout => icon::LAYOUT,
        ComponentCategory::Widget => icon::CURSOR_CLICK,
    }
}

pub fn properties(ui: &mut Ui, state: &AppState) {
    if !state.view_mode.shows_canvas() {
        return canvas_hidden(ui, "l'editor delle proprietà");
    }
    padded(ui, |ui| {
        empty_state(
            ui,
            icon::CURSOR_CLICK,
            "Nessun elemento selezionato",
            "Seleziona un elemento nel canvas per modificarne Geometria, Aspetto e \
             Comportamento (Fase 3).",
        );
    });
}

fn canvas_hidden(ui: &mut Ui, what: &str) {
    padded(ui, |ui| {
        empty_state(
            ui,
            icon::EYE_SLASH,
            "Disponibile in modalità Split e Design",
            &format!("Passa a Split o Design dalla toolbar per usare {what}."),
        );
    });
}
