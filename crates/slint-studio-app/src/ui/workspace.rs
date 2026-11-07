//! Central workspace: code editor, design canvas, or both (split).
//!
//! The rendered preview arrives in Phase 2; until then the canvas is a
//! placeholder.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use slint_studio_core::ViewMode;

use super::editor;
use crate::app::AppState;
use crate::fonts;
use crate::theme::Palette;

const CANVAS_BG: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
const CANVAS_DOT: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);

pub fn show(ui: &mut Ui, state: &mut AppState) {
    match state.view_mode {
        ViewMode::Code => editor_area(ui, state),
        ViewMode::Design => canvas(ui, state),
        ViewMode::Split => {
            let height = ui.available_height();
            let min = 100.0_f32.min(height / 2.0);
            egui::Panel::top("workspace_split_editor")
                .resizable(true)
                .default_size(height * 0.6)
                .size_range(min..=(height - min).max(min))
                .frame(egui::Frame::NONE)
                .show(ui, |ui| editor_area(ui, state));
            egui::CentralPanel::no_frame().show(ui, |ui| canvas(ui, state));
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

fn canvas(ui: &mut Ui, state: &AppState) {
    let palette = state.palette;
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, CANVAS_BG);
    paint_dot_grid(&painter, rect);

    let label_pos = rect.left_top() + vec2(10.0, 8.0);
    painter.text(
        label_pos,
        Align2::LEFT_TOP,
        icon::FRAME_CORNERS,
        FontId::new(12.0, fonts::icon_family()),
        palette.text_weak,
    );
    painter.text(
        label_pos + vec2(18.0, 0.0),
        Align2::LEFT_TOP,
        "Canvas di design",
        FontId::proportional(11.0),
        palette.text_weak,
    );

    let size = vec2(380.0, 210.0)
        .min(rect.size() - vec2(40.0, 40.0))
        .max(vec2(0.0, 0.0));
    if size.x < 200.0 || size.y < 110.0 {
        return;
    }
    let artboard = Rect::from_center_size(rect.center(), size);
    paint_artboard(&painter, artboard, state.live_preview, &palette);
}

fn paint_dot_grid(painter: &egui::Painter, rect: Rect) {
    const SPACING: f32 = 20.0;
    let dot = vec2(1.5, 1.5);
    let mut y = rect.top() + SPACING / 2.0;
    while y < rect.bottom() {
        let mut x = rect.left() + SPACING / 2.0;
        while x < rect.right() {
            painter.rect_filled(
                Rect::from_center_size(Pos2::new(x, y), dot),
                0.0,
                CANVAS_DOT,
            );
            x += SPACING;
        }
        y += SPACING;
    }
}

fn paint_artboard(painter: &egui::Painter, board: Rect, live: bool, palette: &Palette) {
    painter.rect_filled(board, 4.0, palette.background);
    let outline = [
        board.left_top(),
        board.right_top(),
        board.right_bottom(),
        board.left_bottom(),
        board.left_top(),
    ];
    painter.extend(Shape::dashed_line(
        &outline,
        Stroke::new(1.0, palette.accent_secondary.gamma_multiply(0.8)),
        6.0,
        4.0,
    ));

    let (glyph, title, detail) = if live {
        (
            icon::EYE,
            "Anteprima live attiva",
            "Il rendering con slint-interpreter arriva nella Fase 2",
        )
    } else {
        (
            icon::EYE_SLASH,
            "Anteprima live disattivata",
            "Riattivala dalla toolbar per aggiornare il canvas",
        )
    };
    let center = board.center();
    painter.text(
        center - vec2(0.0, 34.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(30.0, fonts::icon_family()),
        palette.accent_secondary,
    );
    painter.text(
        center + vec2(0.0, 6.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(15.0),
        palette.text,
    );
    painter.text(
        center + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        detail,
        FontId::proportional(12.0),
        palette.text_weak,
    );
}
