//! Design canvas: the live preview rendered by `slint-interpreter`.

use egui::epaint::Shadow;
use egui::{
    Align, Button, Color32, CornerRadius, Layout, Margin, Pos2, Rect, RichText, ScrollArea, Sense,
    Stroke, Ui, UiBuilder, vec2,
};
use egui_phosphor::regular as icon;

use super::{empty_state, ico, with_icon};
use crate::app::AppState;
use crate::commands::Command;
use crate::preview::Zoom;
use crate::theme::Palette;

const CANVAS_BG: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
const CANVAS_DOT: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
/// Space around the component, in points.
const MARGIN: f32 = 32.0;
/// Errors listed over the canvas before "+N altri".
const MAX_LISTED_ERRORS: usize = 6;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let mut commands = Vec::new();
    toolbar(ui, state, &mut commands);

    let area = ui.available_rect_before_wrap();
    ui.painter().rect_filled(area, 0.0, CANVAS_BG);
    paint_dot_grid(&ui.painter_at(area), area);

    let palette = state.palette;
    if state.preview.image.is_some() {
        image_view(ui, area, state);
    } else {
        let mut content = ui.new_child(UiBuilder::new().max_rect(area));
        placeholder(&mut content, state);
    }
    if let Some(failure) = &state.preview.failure {
        failure_overlay(ui, area, failure, &palette, &mut commands);
    }
    ui.advance_cursor_after_rect(area);
    for command in commands {
        state.send(command);
    }
}

fn toolbar(ui: &mut Ui, state: &mut AppState, commands: &mut Vec<Command>) {
    let palette = state.palette;
    egui::Frame::NONE
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(ico(icon::FRAME_CORNERS).color(palette.text_weak));
                match &state.preview.image {
                    Some(image) => {
                        ui.label(RichText::new(&image.component).strong())
                            .on_hover_text(image.path.display().to_string());
                        ui.label(
                            RichText::new(format!(
                                "{} × {} · {} ms",
                                image.logical_size.x.round(),
                                image.logical_size.y.round(),
                                image.duration.as_millis()
                            ))
                            .small()
                            .color(palette.text_weak),
                        );
                    }
                    None => {
                        ui.label(RichText::new("Anteprima").color(palette.text_weak));
                    }
                }
                if state.preview.is_rendering() {
                    ui.add(egui::Spinner::new().size(12.0));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let has_target = state.preview.target.is_some();
                    if ui
                        .add_enabled(
                            has_target,
                            Button::new(with_icon(icon::ARROWS_CLOCKWISE, "Aggiorna")),
                        )
                        .on_hover_text("Ricompila e ridisegna l'anteprima")
                        .clicked()
                    {
                        commands.push(Command::RefreshPreview);
                    }
                    ui.add_space(8.0);
                    let preview = &mut state.preview;
                    let fit = preview.zoom == Zoom::Fit;
                    if ui
                        .add(Button::selectable(
                            fit,
                            with_icon(icon::CORNERS_OUT, "Adatta"),
                        ))
                        .on_hover_text("Adatta il componente al canvas")
                        .clicked()
                    {
                        preview.zoom = Zoom::Fit;
                    }
                    if ui
                        .add(Button::new(ico(icon::MAGNIFYING_GLASS_PLUS)))
                        .on_hover_text("Ingrandisci")
                        .clicked()
                    {
                        preview.zoom = Zoom::zoom_in(preview.effective_zoom);
                    }
                    let percent = format!("{:.0}%", preview.effective_zoom * 100.0);
                    if ui
                        .add(Button::new(RichText::new(percent).monospace()).frame(false))
                        .on_hover_text("Dimensione reale (100%)")
                        .clicked()
                    {
                        preview.zoom = Zoom::Scale(1.0);
                    }
                    if ui
                        .add(Button::new(ico(icon::MAGNIFYING_GLASS_MINUS)))
                        .on_hover_text("Riduci")
                        .clicked()
                    {
                        preview.zoom = Zoom::zoom_out(preview.effective_zoom);
                    }
                });
            });
        });
}

fn image_view(ui: &mut Ui, area: Rect, state: &mut AppState) {
    let palette = state.palette;
    let preview = &mut state.preview;
    let Some(image) = &preview.image else {
        return;
    };
    let logical = image.logical_size.max(vec2(1.0, 1.0));
    let zoom = match preview.zoom {
        Zoom::Fit => {
            let available = (area.size() - vec2(2.0 * MARGIN, 2.0 * MARGIN)).max(vec2(1.0, 1.0));
            (available.x / logical.x)
                .min(available.y / logical.y)
                .clamp(0.05, 1.0)
        }
        Zoom::Scale(scale) => scale,
    };
    preview.effective_zoom = zoom;
    let size = logical * zoom;
    let stale = preview.failure.as_ref().is_some_and(|f| !f.informational);
    let texture = image.texture.id();

    let mut content = ui.new_child(UiBuilder::new().max_rect(area));
    ScrollArea::both()
        .id_salt("canvas_scroll")
        .auto_shrink(false)
        .show(&mut content, |ui| {
            let canvas = (size + vec2(2.0 * MARGIN, 2.0 * MARGIN)).max(ui.available_size());
            let (rect, _) = ui.allocate_exact_size(canvas, Sense::hover());
            let image_rect = Rect::from_center_size(rect.center(), size);
            let painter = ui.painter();
            painter.add(
                Shadow {
                    offset: [0, 6],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(120),
                }
                .as_shape(image_rect, CornerRadius::ZERO),
            );
            paint_checkerboard(painter, image_rect);
            painter.image(
                texture,
                image_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            painter.rect_stroke(
                image_rect,
                0.0,
                Stroke::new(1.0, palette.border),
                egui::StrokeKind::Outside,
            );
            if stale {
                painter.rect_filled(image_rect, 0.0, Color32::from_black_alpha(150));
            }
        });

    // Ctrl + wheel (or pinch) zooms around the current level.
    let zoom_delta = ui.input(|i| i.zoom_delta());
    if (zoom_delta - 1.0).abs() > f32::EPSILON && ui.rect_contains_pointer(area) {
        preview.zoom = Zoom::Scale((zoom * zoom_delta).clamp(0.1, 8.0));
    }
}

fn placeholder(ui: &mut Ui, state: &AppState) {
    let preview = &state.preview;
    let (glyph, title, detail) = if preview.target.is_none() {
        (
            icon::FRAME_CORNERS,
            "Nessuna anteprima",
            "Apri un file .slint per vederne l'anteprima: verrà aggiornata mentre scrivi.",
        )
    } else if preview.is_rendering() {
        (
            icon::HOURGLASS,
            "Rendering in corso…",
            "Compilazione con slint-interpreter",
        )
    } else if !state.live_preview {
        (
            icon::EYE_SLASH,
            "Anteprima live disattivata",
            "Premi Aggiorna o riattiva l'anteprima live dalla toolbar.",
        )
    } else {
        return;
    };
    empty_state(ui, glyph, title, detail);
}

fn failure_overlay(
    ui: &mut Ui,
    area: Rect,
    failure: &crate::preview::PreviewFailure,
    palette: &Palette,
    commands: &mut Vec<Command>,
) {
    let width = (area.width() - 24.0).clamp(160.0, 640.0);
    let overlay = Rect::from_min_size(
        area.left_top() + vec2(12.0, 12.0),
        vec2(width, area.height() - 24.0),
    );
    let mut content = ui.new_child(UiBuilder::new().max_rect(overlay));
    let (glyph, color, title) = if failure.informational {
        (icon::INFO, palette.accent, failure.message.clone())
    } else {
        (
            icon::X_CIRCLE,
            palette.error,
            format!("Anteprima non aggiornata: {}", failure.message),
        )
    };
    egui::Frame::popup(content.style())
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .show(&mut content, |ui| {
            ui.set_max_width(width);
            ui.horizontal(|ui| {
                ui.label(ico(glyph).color(color));
                ui.label(RichText::new(title).strong());
            });
            let errors: Vec<_> = failure
                .diagnostics
                .iter()
                .filter(|d| d.severity == slint_studio_core::Severity::Error)
                .collect();
            for diagnostic in errors.iter().take(MAX_LISTED_ERRORS) {
                let place = diagnostic.location.as_ref().map_or_else(String::new, |l| {
                    let file = l
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    format!("{file}:{} ", l.start.line + 1)
                });
                let text = RichText::new(format!("{place}{}", diagnostic.message))
                    .monospace()
                    .small();
                let response = ui
                    .add(Button::new(text).frame(false))
                    .on_hover_text("Vai alla riga");
                if response.clicked()
                    && let Some(location) = &diagnostic.location
                {
                    commands.push(Command::OpenLocation {
                        path: location.path.clone(),
                        position: location.start,
                    });
                }
            }
            if errors.len() > MAX_LISTED_ERRORS {
                ui.label(
                    RichText::new(format!(
                        "+{} altri (vedi il pannello Errori)",
                        errors.len() - MAX_LISTED_ERRORS
                    ))
                    .small()
                    .weak(),
                );
            }
        });
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

/// Shows transparent regions of the component (e.g. a root `Rectangle`).
fn paint_checkerboard(painter: &egui::Painter, rect: Rect) {
    const CELL: f32 = 8.0;
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    painter.rect_filled(rect, 0.0, Color32::from_gray(0x3A));
    let light = Color32::from_gray(0x46);
    let columns = (rect.width() / CELL).ceil() as usize;
    let rows = (rect.height() / CELL).ceil() as usize;
    if columns * rows > 40_000 {
        return; // huge zoomed images: plain background is enough
    }
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = rect.min + vec2(column as f32 * CELL, row as f32 * CELL);
            painter.rect_filled(Rect::from_min_size(min, vec2(CELL, CELL)), 0.0, light);
        }
    }
}
