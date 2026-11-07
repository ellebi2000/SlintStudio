//! Colour palette and egui style for the IDE shell.

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, Theme, Vec2};

/// Named colours of the IDE theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Editor and canvas background.
    pub background: Color32,
    /// Side panels, menus, tab bars.
    pub panel: Color32,
    /// Interactive widget background (buttons, inputs).
    pub widget: Color32,
    pub widget_hovered: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub accent: Color32,
    pub accent_secondary: Color32,
    pub selection: Color32,
    pub error: Color32,
    pub warning: Color32,
    /// Text drawn on top of [`Self::accent`] (e.g. the status bar).
    pub on_accent: Color32,
}

impl Palette {
    /// Default dark theme.
    pub const DARK: Self = Self {
        background: Color32::from_rgb(0x1E, 0x1E, 0x1E),
        panel: Color32::from_rgb(0x25, 0x25, 0x26),
        widget: Color32::from_rgb(0x3C, 0x3C, 0x3C),
        widget_hovered: Color32::from_rgb(0x45, 0x49, 0x4E),
        border: Color32::from_rgb(0x3C, 0x3C, 0x3C),
        text: Color32::from_rgb(0xD4, 0xD4, 0xD4),
        text_weak: Color32::from_rgb(0x9D, 0x9D, 0x9D),
        accent: Color32::from_rgb(0x00, 0x7A, 0xCC),
        accent_secondary: Color32::from_rgb(0x4E, 0xC9, 0xB0),
        selection: Color32::from_rgb(0x26, 0x4F, 0x78),
        error: Color32::from_rgb(0xF4, 0x47, 0x47),
        warning: Color32::from_rgb(0xFF, 0xCC, 0x00),
        on_accent: Color32::WHITE,
    };
}

/// Applies `palette` to every egui style and forces the dark theme, regardless
/// of the operating system preference.
pub fn apply(ctx: &egui::Context, palette: &Palette) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.visuals = visuals(palette);

        let spacing = &mut style.spacing;
        spacing.item_spacing = Vec2::new(8.0, 5.0);
        spacing.button_padding = Vec2::new(8.0, 3.0);
        spacing.menu_margin = Margin::same(6);
        spacing.interact_size.y = 22.0;
    });
}

fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let radius = CornerRadius::same(3);

    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(100),
    };
    v.menu_corner_radius = CornerRadius::same(4);
    v.extreme_bg_color = p.background;
    v.code_bg_color = p.background;
    v.faint_bg_color = Color32::from_rgb(0x2A, 0x2D, 0x2E);
    v.hyperlink_color = Color32::from_rgb(0x37, 0x94, 0xFF);
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.widget;
    w.inactive.weak_bg_fill = p.widget;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.widget_hovered;
    w.hovered.weak_bg_fill = p.widget_hovered;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;

    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.accent;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    w.active.corner_radius = radius;

    w.open.bg_fill = Color32::from_rgb(0x37, 0x37, 0x3D);
    w.open.weak_bg_fill = Color32::from_rgb(0x37, 0x37, 0x3D);
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;

    v
}

/// Docking style derived from the egui style, tuned to look like an editor
/// tab strip: flat tabs, accent line above the active tab, thin separators.
pub fn dock_style(egui_style: &egui::Style, p: &Palette) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);

    style.dock_area_padding = None;
    style.main_surface_border_stroke = Stroke::NONE;
    style.main_surface_border_rounding = CornerRadius::ZERO;

    style.separator.width = 1.0;
    style.separator.extra_interact_width = 4.0;
    style.separator.color_idle = Color32::from_rgb(0x2B, 0x2B, 0x2B);
    style.separator.color_hovered = p.accent;
    style.separator.color_dragged = p.accent;

    style.tab_bar.bg_fill = p.panel;
    style.tab_bar.height = 28.0;
    style.tab_bar.hline_color = Color32::from_rgb(0x2B, 0x2B, 0x2B);
    style.tab_bar.corner_radius = CornerRadius::ZERO;

    let tab_radius = CornerRadius::ZERO;
    for (tab, bg, text) in [
        (&mut style.tab.active, p.background, Color32::WHITE),
        (&mut style.tab.focused, p.background, Color32::WHITE),
        (
            &mut style.tab.active_with_kb_focus,
            p.background,
            Color32::WHITE,
        ),
        (
            &mut style.tab.focused_with_kb_focus,
            p.background,
            Color32::WHITE,
        ),
        (&mut style.tab.inactive, p.panel, p.text_weak),
        (&mut style.tab.inactive_with_kb_focus, p.panel, p.text_weak),
        (
            &mut style.tab.hovered,
            Color32::from_rgb(0x2D, 0x2D, 0x2D),
            p.text,
        ),
    ] {
        tab.bg_fill = bg;
        tab.text_color = text;
        tab.outline_color = Color32::TRANSPARENT;
        tab.corner_radius = tab_radius;
    }
    style.tab.hline_below_active_tab_name = false;
    style.tab.tab_body.bg_fill = p.panel;
    style.tab.tab_body.stroke = Stroke::NONE;
    style.tab.tab_body.corner_radius = CornerRadius::ZERO;
    style.tab.tab_body.inner_margin = Margin::same(0);

    style.buttons.close_tab_bg_fill = p.widget_hovered;
    style.buttons.close_tab_color = p.text_weak;
    style.buttons.close_tab_active_color = Color32::WHITE;

    style.overlay.selection_color = p.accent.gamma_multiply(0.35);
    style.overlay.button_color = p.accent;

    style
}
