//! Toolbar: run/build, live preview toggle and view mode switcher.

use egui::{Button, Ui};
use egui_phosphor::regular as icon;
use slint_studio_core::ViewMode;

use super::{planned_button, view_mode_icon, with_icon};
use crate::app::AppState;
use crate::commands::{Command, Planned};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        planned_button(
            ui,
            Planned::new("Esegui", 4)
                .icon(icon::PLAY)
                .shortcut("Ctrl+R"),
        );
        planned_button(
            ui,
            Planned::new("Compila", 4)
                .icon(icon::HAMMER)
                .shortcut("Ctrl+B"),
        );

        ui.separator();

        let (glyph, tooltip) = if state.live_preview {
            (
                icon::EYE,
                "Disattiva l'aggiornamento automatico dell'anteprima",
            )
        } else {
            (
                icon::EYE_SLASH,
                "Attiva l'aggiornamento automatico dell'anteprima",
            )
        };
        let live = Button::selectable(state.live_preview, with_icon(glyph, "Anteprima live"));
        if ui.add(live).on_hover_text(tooltip).clicked() {
            state.send(Command::ToggleLivePreview);
        }

        ui.separator();

        // Segmented control: Codice | Split | Design.
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for mode in ViewMode::ALL {
                let button = Button::selectable(
                    state.view_mode == mode,
                    with_icon(view_mode_icon(mode), mode.label()),
                );
                if ui.add(button).on_hover_text(mode_tooltip(mode)).clicked() {
                    state.send(Command::SetViewMode(mode));
                }
            }
        });
    });
}

fn mode_tooltip(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Code => "Solo editor di codice",
        ViewMode::Split => "Editor sopra, anteprima sotto",
        ViewMode::Design => "Solo canvas di design",
    }
}
