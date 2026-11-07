//! Bridges [`egui_dock`] to the panel renderers.

use egui::{Id, Ui, WidgetText};
use egui_dock::TabViewer;
use egui_dock::tab_viewer::OnCloseResponse;
use tracing::debug;

use super::{explorer, inspector, output, workspace};
use crate::app::AppState;
use crate::layout::Tab;

pub struct StudioTabViewer<'a> {
    state: &'a mut AppState,
}

impl<'a> StudioTabViewer<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

impl TabViewer for StudioTabViewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("dock_tab", *tab))
    }

    /// Text-only titles keep the tab strips compact (icons appear in menus).
    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        let problems = self.state.diagnostics.len();
        match tab {
            Tab::Problems if problems > 0 => format!("{} ({problems})", tab.title()).into(),
            _ => tab.title().into(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        let state = &mut *self.state;
        match tab {
            Tab::Workspace => workspace::show(ui, state),
            Tab::Project => explorer::project(ui, state),
            Tab::Search => explorer::search(ui, state),
            Tab::Recent => explorer::recent(ui),
            Tab::Components => inspector::components(ui, state),
            Tab::Properties => inspector::properties(ui, state),
            Tab::Output => output::log(ui, state),
            Tab::Problems => output::problems(ui, state),
            Tab::Terminal => output::terminal(ui),
        }
    }

    fn is_closeable(&self, tab: &Tab) -> bool {
        tab.is_closeable()
    }

    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        debug!(?tab, "pannello chiuso");
        OnCloseResponse::Close
    }

    /// Floating egui windows inside the main window feel foreign in an IDE.
    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        false
    }

    /// Panels that manage their own scrolling opt out of the dock's scroll area.
    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::Workspace | Tab::Output | Tab::Problems | Tab::Terminal => [false, false],
            _ => [false, true],
        }
    }

    fn tab_style_override(
        &self,
        tab: &Tab,
        global_style: &egui_dock::TabStyle,
    ) -> Option<egui_dock::TabStyle> {
        // The workspace uses the darker editor background, like an editor area.
        (*tab == Tab::Workspace).then(|| {
            let mut style = global_style.clone();
            style.tab_body.bg_fill = self.state.palette.background;
            style
        })
    }
}
