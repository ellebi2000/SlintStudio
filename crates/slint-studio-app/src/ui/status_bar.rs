//! Status bar: problem counts, current mode, language server and preview state.

use egui::{Align, Layout, RichText, Ui};
use egui_phosphor::regular as icon;

use super::{icon_label, view_mode_icon};
use crate::app::AppState;

pub fn show(ui: &mut Ui, state: &AppState) {
    ui.visuals_mut().override_text_color = Some(state.palette.on_accent);
    let counts = state.diagnostic_counts();

    ui.horizontal_centered(|ui| {
        icon_label(ui, icon::X_CIRCLE, counts.errors.to_string()).on_hover_text("Errori");
        icon_label(ui, icon::WARNING, counts.warnings.to_string()).on_hover_text("Avvisi");
        ui.add_space(12.0);
        icon_label(ui, view_mode_icon(state.view_mode), state.view_mode.label())
            .on_hover_text("Modalità di visualizzazione");

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(concat!("Slint Studio ", env!("CARGO_PKG_VERSION"))).small());
            ui.add_space(12.0);
            icon_label(ui, icon::LIGHTNING, "LSP: non avviato")
                .on_hover_text("Il language server slint-lsp verrà integrato nella Fase 1");
            ui.add_space(12.0);
            let (glyph, text) = if state.live_preview {
                (icon::EYE, "Anteprima live: attiva")
            } else {
                (icon::EYE_SLASH, "Anteprima live: disattivata")
            };
            icon_label(ui, glyph, text);
        });
    });
}
