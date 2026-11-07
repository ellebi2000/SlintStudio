//! Status bar: problem counts, current mode, build/run state, cursor
//! position, language server and preview state.

use egui::{Align, Button, Layout, RichText, Ui};
use egui_phosphor::regular as icon;
use slint_studio_core::text::char_to_byte;
use slint_studio_core::{Language, PositionEncoding};

use super::{icon_label, view_mode_icon};
use crate::app::AppState;
use crate::commands::Command;
use crate::layout::Tab;
use crate::runner::RunnerStatus;
use slint_studio_compiler::JobOutcome;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.visuals_mut().override_text_color = Some(state.palette.on_accent);
    let counts = state.diagnostics.counts();
    let mut commands = Vec::new();

    ui.horizontal_centered(|ui| {
        let problems = Button::new((
            super::ico(icon::X_CIRCLE),
            counts.errors.to_string(),
            super::ico(icon::WARNING),
            counts.warnings.to_string(),
        ))
        .frame(false);
        if ui
            .add(problems)
            .on_hover_text("Errori e avvisi: apri il pannello")
            .clicked()
        {
            commands.push(Command::ShowPanel(Tab::Problems));
        }
        ui.add_space(12.0);
        icon_label(ui, view_mode_icon(state.view_mode), state.view_mode.label())
            .on_hover_text("Modalità di visualizzazione");

        let status = &state.runner.status;
        if *status != RunnerStatus::Idle {
            ui.add_space(12.0);
            let glyph = match status {
                _ if status.is_working() => icon::HOURGLASS_MEDIUM,
                _ if status.is_failure() => icon::X_CIRCLE,
                RunnerStatus::Finished {
                    outcome: JobOutcome::Stopped,
                    ..
                } => icon::STOP_CIRCLE,
                _ => icon::CHECK_CIRCLE,
            };
            let mut label = RichText::new(status.label());
            if status.is_failure() {
                label = label.strong();
            }
            let tooltip = status
                .summary()
                .unwrap_or_else(|| "Apri il pannello Terminale".to_owned());
            let response = ui
                .add(Button::new((super::ico(glyph), label)).frame(false))
                .on_hover_text(tooltip);
            if response.clicked() {
                commands.push(Command::ShowPanel(Tab::Terminal));
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(concat!("Slint Studio ", env!("CARGO_PKG_VERSION"))).small());
            ui.add_space(12.0);

            let lsp = &state.lsp_status;
            let glyph = if lsp.is_error() {
                icon::LIGHTNING_SLASH
            } else {
                icon::LIGHTNING
            };
            let mut label = RichText::new(lsp.label());
            if lsp.is_error() {
                label = label.strong();
            }
            let response = ui
                .add(Button::new((super::ico(glyph), label)).frame(false))
                .on_hover_text(lsp.tooltip());
            if response.clicked() {
                commands.push(Command::ShowPanel(Tab::Output));
            }
            ui.add_space(12.0);

            let (glyph, text) = if state.live_preview {
                (icon::EYE, "Anteprima live: attiva")
            } else {
                (icon::EYE_SLASH, "Anteprima live: disattivata")
            };
            icon_label(ui, glyph, text);

            if let Some(open) = state.workspace.active_document() {
                ui.add_space(12.0);
                let language = match open.doc.language() {
                    Language::Slint => "Slint",
                    Language::PlainText => "Testo",
                };
                ui.label(language);
                ui.add_space(12.0);
                let text = open.doc.text();
                let byte = char_to_byte(text, open.editor.cursor);
                let position = open
                    .doc
                    .line_index()
                    .position(text, byte, PositionEncoding::Chars);
                ui.label(format!(
                    "Ln {}, Col {}",
                    position.line + 1,
                    position.column + 1
                ));
                if open.doc.is_dirty() {
                    ui.add_space(12.0);
                    icon_label(ui, icon::CIRCLE, "Non salvato");
                }
            }
        });
    });
    for command in commands {
        state.send(command);
    }
}
