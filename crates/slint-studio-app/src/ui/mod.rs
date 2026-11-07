//! UI building blocks of the IDE shell. Each module renders one region.

pub mod about;
pub mod canvas;
pub mod dialogs;
pub mod editor;
pub mod explorer;
pub mod highlight;
pub mod inspector;
pub mod menu_bar;
pub mod output;
pub mod status_bar;
pub mod tabs;
pub mod toolbar;
pub mod workspace;

use egui::{Align, AtomLayout, Button, Layout, RichText, Ui, WidgetText};
use egui_phosphor::regular as icon;
use slint_studio_core::ViewMode;

use crate::commands::Planned;
use crate::fonts;

/// An icon glyph rendered with the dedicated icon font.
///
/// Always render Phosphor glyphs through this function: the text fonts may
/// map the same codepoints to unrelated glyphs.
pub fn ico(glyph: &str) -> RichText {
    RichText::new(glyph).family(fonts::icon_family())
}

/// Icon followed by text, as atoms for buttons, checkboxes and menus.
///
/// The accessible label of the resulting widget is `"{glyph} {text}"`.
pub fn with_icon(glyph: &str, text: impl Into<WidgetText>) -> (RichText, WidgetText) {
    (ico(glyph), text.into())
}

/// A non-interactive "icon + text" label.
pub fn icon_label(ui: &mut Ui, glyph: &str, text: impl Into<WidgetText>) -> egui::Response {
    ui.add(AtomLayout::new(with_icon(glyph, text)))
}

pub fn view_mode_icon(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Code => icon::CODE,
        ViewMode::Split => icon::SQUARE_SPLIT_VERTICAL,
        ViewMode::Design => icon::PENCIL_RULER,
    }
}

/// Lays out `add_contents` with the standard inner margin of side panels.
pub fn padded<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(8))
        .show(ui, add_contents)
        .inner
}

/// Centered icon, title and explanation shown by panels with nothing to show.
pub fn empty_state(ui: &mut Ui, glyph: &str, title: &str, detail: &str) {
    let weak = ui.visuals().weak_text_color();
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() * 0.2).clamp(8.0, 80.0));
        ui.label(ico(glyph).size(30.0).color(weak));
        ui.add_space(4.0);
        ui.label(RichText::new(title).strong());
        ui.add(egui::Label::new(RichText::new(detail).small().color(weak)).wrap());
    });
}

fn planned_widget(planned: &Planned) -> Button<'static> {
    match planned.icon {
        Some(glyph) => Button::new(with_icon(glyph, planned.label)),
        None => Button::new(planned.label),
    }
}

/// A disabled button for a feature that arrives in a later phase.
pub fn planned_button(ui: &mut Ui, planned: Planned) -> egui::Response {
    let mut tooltip = planned.tooltip();
    if let Some(shortcut) = planned.shortcut {
        tooltip = format!("{tooltip} · {shortcut}");
    }
    ui.add_enabled(false, planned_widget(&planned))
        .on_disabled_hover_text(tooltip)
}

/// A disabled menu entry for a feature that arrives in a later phase.
pub fn planned_menu_item(ui: &mut Ui, planned: Planned) -> egui::Response {
    let mut button = planned_widget(&planned);
    if let Some(shortcut) = planned.shortcut {
        button = button.shortcut_text(shortcut);
    }
    ui.add_enabled(false, button)
        .on_disabled_hover_text(planned.tooltip())
}
