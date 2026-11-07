//! Central workspace: code editor, design canvas, or both (split).
//!
//! The canvas shows the live preview (see [`super::canvas`]).

use egui::{Align2, FontId, Sense, Ui, vec2};
use slint_studio_core::ViewMode;

use super::{canvas, editor};
use crate::app::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    match state.view_mode {
        ViewMode::Code => editor_area(ui, state),
        ViewMode::Design => canvas::show(ui, state),
        ViewMode::Split => {
            let height = ui.available_height();
            let min = 100.0_f32.min(height / 2.0);
            egui::Panel::top("workspace_split_editor")
                .resizable(true)
                .default_size(height * 0.6)
                .size_range(min..=(height - min).max(min))
                .frame(egui::Frame::NONE)
                .show(ui, |ui| editor_area(ui, state));
            egui::CentralPanel::no_frame().show(ui, |ui| canvas::show(ui, state));
        }
    }
}

fn editor_area(ui: &mut Ui, state: &mut AppState) {
    let palette = state.palette;
    let mut commands = Vec::new();
    if state.workspace.documents.is_empty() {
        watermark(ui, state);
        return;
    }
    editor::document_tabs(ui, &state.workspace, &palette, &mut commands);

    let workspace = &mut state.workspace;
    let project = workspace.project.as_ref();
    if let Some(document) = workspace
        .active
        .and_then(|i| workspace.documents.get_mut(i))
    {
        let diagnostics = state.diagnostics.for_path(document.doc.path());
        editor::show(ui, document, diagnostics, project, &palette, &mut commands);
    }
    for command in commands {
        state.send(command);
    }
}

fn watermark(ui: &mut Ui, state: &AppState) {
    let palette = state.palette;
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.background);
    if rect.height() < 110.0 || rect.width() < 280.0 {
        return;
    }
    let hint = if state.workspace.project.is_some() {
        "Apri un file dall'albero del progetto"
    } else {
        "Apri una cartella progetto: File › Apri cartella… (Ctrl+O)"
    };
    let center = rect.center();
    painter.text(
        center - vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        "Slint Studio",
        FontId::proportional(28.0),
        palette.text_weak.gamma_multiply(0.45),
    );
    painter.text(
        center + vec2(0.0, 6.0),
        Align2::CENTER_CENTER,
        hint,
        FontId::proportional(13.0),
        palette.text_weak,
    );
    painter.text(
        center + vec2(0.0, 28.0),
        Align2::CENTER_CENTER,
        "Ctrl+S salva · Ctrl+Space completamento · F12 vai alla definizione",
        FontId::proportional(12.0),
        palette.text_weak.gamma_multiply(0.7),
    );
}
