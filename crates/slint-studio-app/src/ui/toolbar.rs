//! Toolbar: run/build/stop, live preview toggle and view mode switcher.

use egui::{Button, Ui};
use egui_phosphor::regular as icon;
use slint_studio_core::ViewMode;

use super::{ico, view_mode_icon, with_icon};
use crate::app::AppState;
use crate::commands::Command;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        run_controls(ui, state);
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

/// Esegui, Compila and Interrompi, with tooltips saying what they run.
fn run_controls(ui: &mut Ui, state: &mut AppState) {
    let has_project = state.workspace.project.is_some();
    let kind = &state.project_kind;
    let enabled = has_project && kind.is_supported();
    let disabled_reason = if has_project {
        "Il progetto non ha né un Cargo.toml né file .slint"
    } else {
        "Apri un progetto per compilarlo ed eseguirlo"
    };
    let runner = &state.runner;
    let mut commands = Vec::new();

    let play = ico(icon::PLAY).color(if enabled {
        state.palette.accent_secondary
    } else {
        ui.visuals().weak_text_color()
    });
    let run = ui
        .add_enabled(
            enabled,
            Button::new((play, egui::WidgetText::from("Esegui"))),
        )
        .on_hover_text(format!("{} · Ctrl+R", kind.run_description()))
        .on_disabled_hover_text(disabled_reason);
    if run.clicked() {
        commands.push(Command::Run);
    }
    let build = ui
        .add_enabled(enabled, Button::new(with_icon(icon::HAMMER, "Compila")))
        .on_hover_text(format!("{} · Ctrl+B", kind.build_description()))
        .on_disabled_hover_text(disabled_reason);
    if build.clicked() {
        commands.push(Command::Build);
    }
    let can_stop = runner.is_running() && !runner.is_stopping();
    let stop = ui
        .add_enabled(can_stop, Button::new(with_icon(icon::STOP, "Interrompi")))
        .on_hover_text("Interrompe la compilazione o il programma · Maiusc+F5")
        .on_disabled_hover_text("Niente in esecuzione");
    if stop.clicked() {
        commands.push(Command::StopRun);
    }
    for command in commands {
        state.send(command);
    }
}
