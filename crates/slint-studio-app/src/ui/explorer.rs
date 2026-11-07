//! Left side: project tree, project-wide search and recent projects.

use egui::{TextEdit, Ui};
use egui_phosphor::regular as icon;

use super::{empty_state, padded, planned_button};
use crate::app::AppState;
use crate::commands::Planned;

const NEEDS_PROJECT: &str = "Disponibile dopo l'apertura di un progetto (Fase 1)";

pub fn project(ui: &mut Ui, state: &mut AppState) {
    padded(ui, |ui| {
        let filter = TextEdit::singleline(&mut state.explorer_filter)
            .hint_text("Filtra file…")
            .desired_width(f32::INFINITY);
        ui.add_enabled(false, filter)
            .on_disabled_hover_text(NEEDS_PROJECT);

        empty_state(
            ui,
            icon::FOLDER_OPEN,
            "Nessun progetto aperto",
            "Apri una cartella che contiene file .slint per esplorarne i file.",
        );
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            planned_button(
                ui,
                Planned::new("Apri cartella…", 1).icon(icon::FOLDER_OPEN),
            );
        });
    });
}

pub fn search(ui: &mut Ui, state: &mut AppState) {
    padded(ui, |ui| {
        let query = TextEdit::singleline(&mut state.search_query)
            .hint_text("Cerca nei file del progetto…")
            .desired_width(f32::INFINITY);
        ui.add_enabled(false, query)
            .on_disabled_hover_text(NEEDS_PROJECT);

        empty_state(
            ui,
            icon::MAGNIFYING_GLASS,
            "Ricerca nel progetto",
            "Cerca testo in tutti i file .slint del progetto aperto.",
        );
    });
}

pub fn recent(ui: &mut Ui) {
    padded(ui, |ui| {
        empty_state(
            ui,
            icon::CLOCK_COUNTER_CLOCKWISE,
            "Nessun progetto recente",
            "I progetti aperti di recente compariranno qui.",
        );
    });
}
