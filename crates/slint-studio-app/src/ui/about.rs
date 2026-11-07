//! "Informazioni su Slint Studio" dialog.

use egui::{Align2, RichText};

use crate::app::AppState;

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if !state.show_about {
        return;
    }

    let mut open = true;
    egui::Window::new("Informazioni su Slint Studio")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Slint Studio").size(22.0).strong());
                ui.label(RichText::new(concat!("Versione ", env!("CARGO_PKG_VERSION"))).weak());
                ui.add_space(4.0);
                ui.label("IDE per il toolkit Slint, ispirato a Qt Creator.");
            });
            ui.separator();
            egui::Grid::new("about_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Interfaccia").weak());
                    ui.label("egui / eframe");
                    ui.end_row();
                    ui.label(RichText::new("Anteprima").weak());
                    ui.label("slint-interpreter 1.18.1 (renderer software)");
                    ui.end_row();
                    ui.label(RichText::new("Language server").weak());
                    ui.label(format!(
                        "slint-lsp {}",
                        slint_studio_lsp::install::SLINT_LSP_VERSION
                    ));
                    ui.end_row();
                    ui.label(RichText::new("Licenza").weak());
                    ui.label("GNU GPLv3");
                    ui.end_row();
                    ui.label(RichText::new("Font interfaccia").weak());
                    ui.label(state.fonts.ui.as_deref().unwrap_or("predefinito egui"));
                    ui.end_row();
                    ui.label(RichText::new("Font codice").weak());
                    ui.label(
                        state
                            .fonts
                            .code
                            .as_deref()
                            .unwrap_or("monospace predefinito egui"),
                    );
                    ui.end_row();
                });
            ui.separator();
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(
                        "Slint Studio è software libero distribuito sotto GPLv3.\n\
                         Usa Slint di SixtyFPS GmbH, sotto licenza GPLv3.",
                    )
                    .small(),
                );
                ui.horizontal(|ui| {
                    ui.hyperlink_to("Slint", "https://slint.dev");
                    ui.label("·");
                    ui.hyperlink_to("Documentazione", "https://docs.slint.dev");
                    ui.label("·");
                    ui.hyperlink_to("GPLv3", "https://www.gnu.org/licenses/gpl-3.0.html");
                });
            });
        });
    state.show_about = open;
}
