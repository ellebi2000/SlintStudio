//! Top-level application: owns the state and lays out the window.

use egui_dock::{DockArea, DockState};
use slint_studio_core::{Diagnostic, DiagnosticCounts, LogBuffer, ViewMode};
use tracing::{debug, info};

use crate::commands::Command;
use crate::fonts::{self, FontChoice};
use crate::layout::{self, Tab};
use crate::logging::RepaintSignal;
use crate::theme::{self, Palette};
use crate::ui;

const SLINT_DOCS_URL: &str = "https://docs.slint.dev";

/// State shared by every panel. Panels read it freely but change it mostly
/// through [`AppState::send`], so that every action goes through
/// [`StudioApp::run_commands`].
pub struct AppState {
    pub palette: Palette,
    pub fonts: FontChoice,
    pub view_mode: ViewMode,
    pub live_preview: bool,
    pub log: LogBuffer,
    pub log_view: ui::output::LogView,
    pub diagnostics: Vec<Diagnostic>,
    pub explorer_filter: String,
    pub search_query: String,
    pub show_about: bool,
    commands: Vec<Command>,
}

impl AppState {
    fn new(palette: Palette, fonts: FontChoice, log: LogBuffer) -> Self {
        Self {
            palette,
            fonts,
            view_mode: ViewMode::default(),
            live_preview: true,
            log,
            log_view: ui::output::LogView::default(),
            diagnostics: Vec::new(),
            explorer_filter: String::new(),
            search_query: String::new(),
            show_about: false,
            commands: Vec::new(),
        }
    }

    /// Queues `command` for execution at the end of the current frame.
    pub fn send(&mut self, command: Command) {
        self.commands.push(command);
    }

    pub fn diagnostic_counts(&self) -> DiagnosticCounts {
        self.diagnostics.iter().collect()
    }
}

pub struct StudioApp {
    state: AppState,
    dock: DockState<Tab>,
    dock_style: egui_dock::Style,
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, log: LogBuffer, repaint: RepaintSignal) -> Self {
        let ctx = &cc.egui_ctx;
        repaint.attach(ctx);

        let palette = Palette::DARK;
        theme::apply(ctx, &palette);
        let fonts = fonts::install(ctx);
        info!(
            "Font interfaccia: {}, font codice: {}",
            fonts.ui.as_deref().unwrap_or("predefinito egui"),
            fonts
                .code
                .as_deref()
                .unwrap_or("monospace predefinito egui"),
        );

        let dock_style = theme::dock_style(&ctx.style_of(egui::Theme::Dark), &palette);
        info!("Slint Studio pronto");

        Self {
            state: AppState::new(palette, fonts, log),
            dock: layout::default_dock_state(),
            dock_style,
        }
    }

    fn run_commands(&mut self, ctx: &egui::Context) {
        for command in std::mem::take(&mut self.state.commands) {
            debug!(?command, "esecuzione comando");
            match command {
                Command::SetViewMode(mode) => {
                    if self.state.view_mode != mode {
                        self.state.view_mode = mode;
                        info!("Modalità di visualizzazione: {}", mode.label());
                    }
                }
                Command::ToggleLivePreview => {
                    self.state.live_preview = !self.state.live_preview;
                    let status = if self.state.live_preview {
                        "attivata"
                    } else {
                        "disattivata"
                    };
                    info!("Anteprima live {status}");
                }
                Command::TogglePanel(tab) => layout::toggle(&mut self.dock, tab),
                Command::ResetLayout => {
                    self.dock = layout::default_dock_state();
                    info!("Layout dei pannelli ripristinato");
                }
                Command::ShowAbout => self.state.show_about = true,
                Command::OpenSlintDocs => ctx.open_url(egui::OpenUrl::new_tab(SLINT_DOCS_URL)),
                Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let palette = self.state.palette;

        egui::Panel::top("menu_bar")
            .show_separator_line(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.panel)
                    .inner_margin(egui::Margin::symmetric(6, 2)),
            )
            .show(ui, |ui| ui::menu_bar::show(ui, &mut self.state, &self.dock));

        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::NONE
                    .fill(palette.panel)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| ui::toolbar::show(ui, &mut self.state));

        egui::Panel::bottom("status_bar")
            .show_separator_line(false)
            .exact_size(24.0)
            .frame(
                egui::Frame::NONE
                    .fill(palette.accent)
                    .inner_margin(egui::Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| ui::status_bar::show(ui, &self.state));

        egui::CentralPanel::no_frame().show(ui, |ui| {
            DockArea::new(&mut self.dock)
                .id(egui::Id::new("main_dock"))
                .style(self.dock_style.clone())
                .show_add_buttons(false)
                // Panels close from the tab context menu or "Visualizza › Pannelli",
                // keeping narrow tab strips readable.
                .show_close_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut ui::tabs::StudioTabViewer::new(&mut self.state));
        });

        ui::about::show(ui.ctx(), &mut self.state);

        self.run_commands(ui.ctx());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use egui_kittest::{Harness, Node};
    use egui_phosphor::regular as icon;
    use slint_studio_core::{DiagnosticSource, Severity};

    fn harness() -> Harness<'static, StudioApp> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .build_eframe(|cc| StudioApp::new(cc, LogBuffer::default(), RepaintSignal::default()));
        harness.run();
        harness
    }

    /// Finds a button by its exact label (icon included).
    fn button<'a>(harness: &'a Harness<'_, StudioApp>, label: &'a str) -> Node<'a> {
        harness.get_by_role_and_label(Role::Button, label)
    }

    /// Accessible label of an "icon + text" widget (see `ui::with_icon`).
    fn label(glyph: &str, text: &str) -> String {
        format!("{glyph} {text}")
    }

    fn mode_label(mode: ViewMode) -> String {
        label(ui::view_mode_icon(mode), mode.label())
    }

    #[test]
    fn main_window_shows_menus_toolbar_and_panels() {
        let harness = harness();
        for menu in [
            "File",
            "Modifica",
            "Visualizza",
            "Progetto",
            "Esegui",
            "Strumenti",
            "Aiuto",
        ] {
            button(&harness, menu);
        }
        button(&harness, &label(icon::PLAY, "Esegui"));
        button(&harness, &label(icon::HAMMER, "Compila"));
        for mode in ViewMode::ALL {
            button(&harness, &mode_label(mode));
        }
    }

    #[test]
    fn toolbar_switches_view_mode() {
        let mut harness = harness();
        assert_eq!(harness.state().state.view_mode, ViewMode::Split);

        for mode in [ViewMode::Code, ViewMode::Design, ViewMode::Split] {
            button(&harness, &mode_label(mode)).click();
            harness.run();
            assert_eq!(harness.state().state.view_mode, mode);
        }
    }

    #[test]
    fn toolbar_toggles_live_preview() {
        let mut harness = harness();
        assert!(harness.state().state.live_preview);

        button(&harness, &label(icon::EYE, "Anteprima live")).click();
        harness.run();
        assert!(!harness.state().state.live_preview);

        button(&harness, &label(icon::EYE_SLASH, "Anteprima live")).click();
        harness.run();
        assert!(harness.state().state.live_preview);
    }

    #[test]
    fn unimplemented_actions_are_disabled() {
        let harness = harness();
        for name in [label(icon::PLAY, "Esegui"), label(icon::HAMMER, "Compila")] {
            let node = button(&harness, &name);
            assert!(
                node.accesskit_node().is_disabled(),
                "{name} should be disabled"
            );
        }
    }

    #[test]
    fn panels_can_be_toggled_and_layout_reset() {
        let mut harness = harness();

        harness
            .state_mut()
            .state
            .send(Command::TogglePanel(Tab::Terminal));
        harness.run();
        assert!(!layout::is_open(&harness.state().dock, Tab::Terminal));

        harness.state_mut().state.send(Command::ResetLayout);
        harness.run();
        assert!(layout::is_open(&harness.state().dock, Tab::Terminal));
    }

    #[test]
    fn problems_panel_lists_diagnostics() {
        let mut harness = harness();
        harness.state_mut().state.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            source: DiagnosticSource::Lsp,
            message: "proprietà sconosciuta 'colour'".into(),
            location: None,
        });
        layout::open(&mut harness.state_mut().dock, Tab::Problems);
        harness.run();

        harness.get_by_label("proprietà sconosciuta 'colour'");
        assert_eq!(harness.state().state.diagnostic_counts().errors, 1);
    }
}
