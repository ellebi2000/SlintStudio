//! Dockable panel layout.
//!
//! Every panel of the IDE is a [`Tab`] inside an [`egui_dock::DockState`], so
//! the user can rearrange them by dragging. Each panel has a home [`Area`]
//! that is used for the default layout and when a closed panel is reopened.

use egui_dock::{DockState, NodeIndex, Translations};
use egui_phosphor::regular as icon;

/// Every panel the IDE can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// Central area: code editor, design canvas or both (see `ViewMode`).
    Workspace,
    Project,
    Search,
    Recent,
    Components,
    Properties,
    Output,
    Problems,
    Terminal,
}

/// Region of the window a panel belongs to by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Left,
    Center,
    Right,
    Bottom,
}

impl Tab {
    /// Panels the user can show/hide from the "Visualizza" menu.
    pub const PANELS: [Self; 8] = [
        Self::Project,
        Self::Search,
        Self::Recent,
        Self::Components,
        Self::Properties,
        Self::Output,
        Self::Problems,
        Self::Terminal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Workspace => "Area di lavoro",
            Self::Project => "Progetto",
            Self::Search => "Ricerca",
            Self::Recent => "Recenti",
            Self::Components => "Componenti",
            Self::Properties => "Proprietà",
            Self::Output => "Output",
            Self::Problems => "Errori",
            Self::Terminal => "Terminale",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Workspace => icon::FILE_CODE,
            Self::Project => icon::TREE_STRUCTURE,
            Self::Search => icon::MAGNIFYING_GLASS,
            Self::Recent => icon::CLOCK_COUNTER_CLOCKWISE,
            Self::Components => icon::SQUARES_FOUR,
            Self::Properties => icon::SLIDERS_HORIZONTAL,
            Self::Output => icon::LIST_DASHES,
            Self::Problems => icon::WARNING_CIRCLE,
            Self::Terminal => icon::TERMINAL_WINDOW,
        }
    }

    pub fn area(self) -> Area {
        match self {
            Self::Workspace => Area::Center,
            Self::Project | Self::Search | Self::Recent => Area::Left,
            Self::Components | Self::Properties => Area::Right,
            Self::Output | Self::Problems | Self::Terminal => Area::Bottom,
        }
    }

    /// The workspace is the anchor of the layout and cannot be closed.
    pub fn is_closeable(self) -> bool {
        self != Self::Workspace
    }
}

/// Qt Creator–like default: explorer on the left and inspector on the right
/// (both full height), output panes below the workspace.
pub fn default_dock_state() -> DockState<Tab> {
    let mut state = DockState::new(vec![Tab::Workspace]).with_translations(translations());
    let surface = state.main_surface_mut();

    // For left/right splits `fraction` is the share of the left child, for
    // above/below splits the share of the top child.
    let [center, _left] = surface.split_left(
        NodeIndex::root(),
        0.17,
        vec![Tab::Project, Tab::Search, Tab::Recent],
    );
    let [center, right] = surface.split_right(center, 0.76, vec![Tab::Components]);
    surface.split_below(right, 0.55, vec![Tab::Properties]);
    surface.split_below(
        center,
        0.72,
        vec![Tab::Output, Tab::Problems, Tab::Terminal],
    );

    state
}

/// Italian labels for the dock's own UI (tab context menu, tooltips).
fn translations() -> Translations {
    let mut t = Translations::english();
    t.tab_context_menu.close_button = "Chiudi pannello".into();
    t.tab_context_menu.eject_button = "Sgancia".into();
    t.tab_context_menu.hide_tab_bar_button = "Nascondi barra delle schede".into();
    t.tab_context_menu.show_tab_bar_button = "Mostra barra delle schede".into();
    t.leaf.close_button_disabled_tooltip =
        "Questo gruppo contiene pannelli che non possono essere chiusi".into();
    t
}

pub fn is_open(state: &DockState<Tab>, tab: Tab) -> bool {
    state.find_tab(&tab).is_some()
}

/// Closes `tab` if it is open, reopens it otherwise.
pub fn toggle(state: &mut DockState<Tab>, tab: Tab) {
    match state.find_tab(&tab) {
        Some(path) if tab.is_closeable() => {
            state.remove_tab(path);
        }
        Some(_) => {}
        None => open(state, tab),
    }
}

/// Opens `tab` (or focuses it if already open).
///
/// A reopened panel joins a sibling from its home area when one is still
/// docked; otherwise it is split off the workspace towards its home area.
pub fn open(state: &mut DockState<Tab>, tab: Tab) {
    if state.find_tab(&tab).is_none() {
        let sibling = state.find_tab_from(|t| t.area() == tab.area());
        let workspace = state.find_main_surface_tab(&Tab::Workspace);

        match (sibling, workspace) {
            (Some(path), _) => match state.leaf_mut(path.node_path()) {
                Ok(leaf) => leaf.append_tab(tab),
                Err(_) => state.push_to_first_leaf(tab),
            },
            (None, Some((node, _))) => {
                let surface = state.main_surface_mut();
                let tabs = vec![tab];
                match tab.area() {
                    Area::Left => surface.split_left(node, 0.2, tabs),
                    Area::Right => surface.split_right(node, 0.78, tabs),
                    Area::Bottom => surface.split_below(node, 0.72, tabs),
                    Area::Center => surface.split_right(node, 0.5, tabs),
                };
            }
            (None, None) => state.push_to_first_leaf(tab),
        }
    }

    if let Some(path) = state.find_tab(&tab) {
        // The path was just looked up, so this cannot fail.
        let _ = state.set_active_tab(path);
        state.set_focused_node_and_surface(path.node_path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_tabs(state: &DockState<Tab>) -> Vec<Tab> {
        state.iter_all_tabs().map(|(_, tab)| *tab).collect()
    }

    #[test]
    fn default_layout_contains_every_tab_once() {
        let state = default_dock_state();
        let tabs = all_tabs(&state);
        assert!(tabs.contains(&Tab::Workspace));
        for tab in Tab::PANELS {
            assert_eq!(tabs.iter().filter(|t| **t == tab).count(), 1, "{tab:?}");
        }
        assert_eq!(tabs.len(), Tab::PANELS.len() + 1);
    }

    #[test]
    fn toggle_closes_and_reopens_a_panel() {
        let mut state = default_dock_state();
        toggle(&mut state, Tab::Output);
        assert!(!is_open(&state, Tab::Output));
        toggle(&mut state, Tab::Output);
        assert!(is_open(&state, Tab::Output));
        assert_eq!(all_tabs(&state).len(), Tab::PANELS.len() + 1);
    }

    #[test]
    fn reopened_panel_joins_a_sibling_from_its_area() {
        let mut state = default_dock_state();
        toggle(&mut state, Tab::Terminal);
        toggle(&mut state, Tab::Terminal);
        let output = state.find_tab(&Tab::Output).unwrap();
        let terminal = state.find_tab(&Tab::Terminal).unwrap();
        assert_eq!(output.node_path(), terminal.node_path());
    }

    #[test]
    fn reopened_panel_without_siblings_is_docked_again() {
        let mut state = default_dock_state();
        for tab in [Tab::Output, Tab::Problems, Tab::Terminal] {
            toggle(&mut state, tab);
        }
        open(&mut state, Tab::Problems);
        assert!(is_open(&state, Tab::Problems));
        assert!(!is_open(&state, Tab::Output));
    }

    #[test]
    fn workspace_cannot_be_closed() {
        let mut state = default_dock_state();
        toggle(&mut state, Tab::Workspace);
        assert!(is_open(&state, Tab::Workspace));
    }
}
