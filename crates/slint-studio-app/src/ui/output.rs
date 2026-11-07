//! Bottom panels: Output (application log), Errori (diagnostics), Terminale
//! (output of "Compila" and "Esegui").

use egui::text::LayoutJob;
use egui::{
    Align, Button, Color32, Label, Layout, Margin, RichText, ScrollArea, Stroke, TextFormat,
    TextStyle, Ui,
};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;
use slint_studio_compiler::JobOutcome;
use slint_studio_core::{
    AnsiColor, LogBuffer, LogEntry, LogLevel, Severity, TerminalLine, TerminalStream,
};

use super::{empty_state, ico, padded, with_icon};
use crate::app::AppState;
use crate::commands::Command;
use crate::runner::RunnerStatus;
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

pub fn terminal(ui: &mut Ui, state: &mut AppState) {
    let palette = state.palette;
    let mut commands = Vec::new();
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let runner = &mut state.runner;
                let can_stop = runner.is_running() && !runner.is_stopping();
                if ui
                    .add_enabled(can_stop, Button::new(with_icon(icon::STOP, "Interrompi")))
                    .on_hover_text("Interrompe la compilazione o il programma (Maiusc+F5)")
                    .clicked()
                {
                    commands.push(Command::StopRun);
                }
                if ui.button(with_icon(icon::BROOM, "Pulisci")).clicked() {
                    commands.push(Command::ClearTerminal);
                }
                let copy = Button::new(with_icon(icon::COPY, "Copia"));
                if ui
                    .add_enabled(!runner.terminal.is_empty(), copy)
                    .on_hover_text("Copia tutto l'output negli appunti")
                    .clicked()
                {
                    ui.ctx().copy_text(runner.terminal.plain_text());
                }
                ui.checkbox(&mut runner.auto_scroll, "Scorrimento automatico");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    runner_status(ui, &runner.status, runner.command.as_deref(), &palette);
                });
            });
        });

    let runner = &state.runner;
    if runner.terminal.is_empty() {
        padded(ui, |ui| {
            empty_state(
                ui,
                icon::TERMINAL_WINDOW,
                "Nessun output",
                "Compila (Ctrl+B) o Esegui (Ctrl+R) il progetto: qui compariranno \
                 l'output di cargo, del compilatore Slint e del programma.",
            );
        });
    } else {
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
                    .id_salt("terminal_output")
                    .auto_shrink(false)
                    .stick_to_bottom(runner.auto_scroll)
                    .show_rows(ui, row_height, runner.terminal.len(), |ui, rows| {
                        for index in rows {
                            if let Some(line) = runner.terminal.line(index) {
                                let job = terminal_line(ui, line, &palette);
                                ui.add(Label::new(job).extend());
                            }
                        }
                    });
            });
    }
    for command in commands {
        state.send(command);
    }
}

/// Icon and text of the current or last job, right-aligned in the header.
fn runner_status(ui: &mut Ui, status: &RunnerStatus, command: Option<&str>, palette: &Palette) {
    let label = status.label();
    let tooltip = status.summary().unwrap_or_else(|| label.clone());
    match status {
        RunnerStatus::Idle => {}
        RunnerStatus::Working { .. } => {
            if let Some(command) = command {
                ui.add(
                    Label::new(RichText::new(command).monospace().weak())
                        .truncate()
                        .selectable(false),
                );
            }
            ui.label(RichText::new(label).color(palette.accent_secondary));
            ui.add(egui::Spinner::new().size(14.0));
        }
        RunnerStatus::Finished { outcome, .. } => {
            let (glyph, color) = match outcome {
                _ if status.is_failure() => (icon::X_CIRCLE, palette.error),
                JobOutcome::Stopped => (icon::STOP_CIRCLE, palette.text_weak),
                _ => (icon::CHECK_CIRCLE, palette.accent_secondary),
            };
            ui.label(RichText::new(label).color(color))
                .on_hover_text(&tooltip);
            ui.label(ico(glyph).color(color)).on_hover_text(tooltip);
        }
    }
}

fn terminal_line(ui: &Ui, line: &TerminalLine, palette: &Palette) -> LayoutJob {
    let font = TextStyle::Monospace.resolve(ui.style());
    let default = match line.stream {
        TerminalStream::Stdout | TerminalStream::Stderr => palette.text,
        TerminalStream::System => palette.accent_secondary,
    };
    let mut job = LayoutJob::default();
    for span in &line.spans {
        let style = span.style;
        let mut color = match style.foreground {
            Some(color) => ansi_color(color, palette),
            None if style.bold => Color32::WHITE,
            None => default,
        };
        if style.dim {
            color = color.gamma_multiply(0.6);
        }
        let mut format = TextFormat::simple(font.clone(), color);
        format.italics = style.italic;
        if style.underline {
            format.underline = Stroke::new(1.0, color);
        }
        if let Some(background) = style.background {
            format.background = ansi_color(background, palette);
        }
        job.append(&span.text, 0.0, format);
    }
    if job.sections.is_empty() {
        // Empty lines still take a row.
        job.append(" ", 0.0, TextFormat::simple(font, default));
    }
    job
}

/// The 16 themable colours follow VS Code's dark terminal, with the IDE's
/// own error and warning colours for red and yellow.
fn ansi_color(color: AnsiColor, palette: &Palette) -> Color32 {
    let rgb = Color32::from_rgb;
    match color {
        AnsiColor::Rgb(r, g, b) => rgb(r, g, b),
        AnsiColor::Indexed(index) => match index {
            0 => rgb(0x66, 0x66, 0x66),
            1 | 9 => palette.error,
            2 => rgb(0x0D, 0xBC, 0x79),
            10 => rgb(0x23, 0xD1, 0x8B),
            3 | 11 => palette.warning,
            4 => rgb(0x24, 0x72, 0xC8),
            12 => rgb(0x3B, 0x8E, 0xEA),
            5 => rgb(0xBC, 0x3F, 0xBC),
            13 => rgb(0xD6, 0x70, 0xD6),
            6 => rgb(0x11, 0xA8, 0xCD),
            14 => rgb(0x29, 0xB8, 0xDB),
            7 => palette.text,
            8 => rgb(0x80, 0x80, 0x80),
            15 => Color32::WHITE,
            other => AnsiColor::xterm_rgb(other).map_or(palette.text, |(r, g, b)| rgb(r, g, b)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_palette() {
        let palette = Palette::DARK;
        assert_eq!(ansi_color(AnsiColor::Indexed(1), &palette), palette.error);
        assert_eq!(
            ansi_color(AnsiColor::Indexed(11), &palette),
            palette.warning
        );
        assert_eq!(
            ansi_color(AnsiColor::Indexed(196), &palette),
            Color32::from_rgb(255, 0, 0)
        );
        assert_eq!(
            ansi_color(AnsiColor::Rgb(1, 2, 3), &palette),
            Color32::from_rgb(1, 2, 3)
        );
    }
}
