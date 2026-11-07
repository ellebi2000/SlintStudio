//! Bottom panels: Output (application log), Errori (diagnostics), Terminale.

use egui::text::LayoutJob;
use egui::{
    Align, Color32, Label, Layout, Margin, RichText, ScrollArea, TextFormat, TextStyle, Ui,
};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;
use slint_studio_core::{LogBuffer, LogEntry, LogLevel, Severity};

use super::{empty_state, ico, padded, with_icon};
use crate::app::AppState;
use crate::commands::Command;
use crate::theme::Palette;

/// UI-side cache of the log buffer, refreshed only when it changes.
pub struct LogView {
    entries: Vec<LogEntry>,
    generation: Option<u64>,
    pub auto_scroll: bool,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            generation: None,
            auto_scroll: true,
        }
    }
}

impl LogView {
    fn refresh(&mut self, buffer: &LogBuffer) {
        let generation = buffer.generation();
        if self.generation != Some(generation) {
            self.entries = buffer.snapshot();
            self.generation = Some(generation);
        }
    }
}

pub fn log(ui: &mut Ui, state: &mut AppState) {
    state.log_view.refresh(&state.log);

    egui::Frame::NONE
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(with_icon(icon::BROOM, "Pulisci")).clicked() {
                    state.log.clear();
                }
                ui.checkbox(&mut state.log_view.auto_scroll, "Scorrimento automatico");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let count = state.log_view.entries.len();
                    ui.label(RichText::new(format!("{count} righe")).small().weak());
                });
            });
        });

    let palette = state.palette;
    let view = &state.log_view;
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    egui::Frame::NONE
        .inner_margin(Margin {
            left: 8,
            right: 0,
            top: 2,
            bottom: 0,
        })
        .show(ui, |ui| {
            ScrollArea::both()
                .auto_shrink(false)
                .stick_to_bottom(view.auto_scroll)
                .show_rows(ui, row_height, view.entries.len(), |ui, rows| {
                    for entry in &view.entries[rows] {
                        let line = log_line(ui, entry, &palette);
                        ui.add(Label::new(line).extend());
                    }
                });
        });
}

fn log_line(ui: &Ui, entry: &LogEntry, palette: &Palette) -> LayoutJob {
    let font = TextStyle::Monospace.resolve(ui.style());
    let time = chrono::DateTime::<chrono::Local>::from(entry.timestamp).format("%H:%M:%S%.3f");
    let level_color = match entry.level {
        LogLevel::Error => palette.error,
        LogLevel::Warn => palette.warning,
        LogLevel::Info => palette.accent_secondary,
        LogLevel::Debug | LogLevel::Trace => palette.text_weak,
    };

    let mut job = LayoutJob::default();
    let mut push = |text: &str, color: Color32| {
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
    };
    push(&time.to_string(), palette.text_weak);
    push("  ", palette.text);
    push(entry.level.label(), level_color);
    push("  ", palette.text);
    push(&entry.target, palette.text_weak);
    push("  ", palette.text);
    push(&entry.message, palette.text);
    job
}

pub fn problems(ui: &mut Ui, state: &mut AppState) {
    if state.diagnostics.len() == 0 {
        padded(ui, |ui| {
            empty_state(
                ui,
                icon::CHECK_CIRCLE,
                "Nessun problema rilevato",
                "Errori e avvisi di slint-lsp, dell'anteprima e del compilatore compariranno qui.",
            );
        });
        return;
    }

    let palette = state.palette;
    let diagnostics = state.diagnostics.all();
    let project = state.workspace.project.as_ref();
    let mut open = None;
    egui::Frame::NONE
        .inner_margin(Margin::same(6))
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(24.0))
                .column(Column::initial(460.0).at_least(160.0).clip(true))
                .column(Column::initial(220.0).at_least(80.0).clip(true))
                .column(Column::remainder().at_least(80.0))
                .header(22.0, |mut header| {
                    header.col(|_| {});
                    for title in ["Messaggio", "Posizione", "Origine"] {
                        header.col(|ui| {
                            ui.strong(title);
                        });
                    }
                })
                .body(|body| {
                    body.rows(20.0, diagnostics.len(), |mut row| {
                        let Some(diagnostic) = diagnostics.get(row.index()).copied() else {
                            return;
                        };
                        let (glyph, color) = severity_glyph(diagnostic.severity, &palette);
                        row.col(|ui| {
                            ui.label(ico(glyph).color(color))
                                .on_hover_text(diagnostic.severity.label());
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(&diagnostic.message).selectable(false));
                        });
                        row.col(|ui| {
                            let location = diagnostic.location.as_ref().map_or_else(
                                || "—".to_owned(),
                                |l| {
                                    let path =
                                        project.map_or(l.path.as_path(), |p| p.relative(&l.path));
                                    format!(
                                        "{}:{}:{}",
                                        path.display(),
                                        l.start.line + 1,
                                        l.start.column + 1
                                    )
                                },
                            );
                            ui.add(
                                egui::Label::new(RichText::new(location).weak()).selectable(false),
                            );
                        });
                        row.col(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(diagnostic.source.label()).weak())
                                    .selectable(false),
                            );
                        });
                        if row.response().clicked() {
                            open = diagnostic.location.clone();
                        }
                    });
                });
        });
    if let Some(location) = open {
        state.send(Command::OpenLocation {
            path: location.path,
            position: location.start,
        });
    }
}

fn severity_glyph(severity: Severity, palette: &Palette) -> (&'static str, Color32) {
    match severity {
        Severity::Error => (icon::X_CIRCLE, palette.error),
        Severity::Warning => (icon::WARNING, palette.warning),
        Severity::Info => (icon::INFO, palette.accent),
        Severity::Hint => (icon::LIGHTBULB, palette.text_weak),
    }
}

pub fn terminal(ui: &mut Ui) {
    padded(ui, |ui| {
        empty_state(
            ui,
            icon::TERMINAL_WINDOW,
            "Terminale integrato",
            "Dalla Fase 4 mostrerà l'output di compilazione ed esecuzione del progetto.",
        );
    });
}
