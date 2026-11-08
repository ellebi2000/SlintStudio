//! Main menu bar: File, Modifica, Visualizza, Progetto, Esegui, Strumenti, Aiuto.

use egui::{Button, RichText, Ui};
use egui_dock::DockState;
use egui_phosphor::regular as icon;
use slint_studio_core::ViewMode;

use super::{planned_menu_item as planned, view_mode_icon, with_icon};
use crate::app::AppState;
use crate::commands::{Command, Planned};
use crate::layout::{self, Tab};

pub fn show(ui: &mut Ui, state: &mut AppState, dock: &DockState<Tab>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, state));
        ui.menu_button("Modifica", |ui| edit_menu(ui, state));
        ui.menu_button("Visualizza", |ui| view_menu(ui, state, dock));
        ui.menu_button("Progetto", |ui| project_menu(ui, state));
        ui.menu_button("Esegui", |ui| run_menu(ui, state));
        ui.menu_button("Strumenti", |ui| tools_menu(ui, state));
        ui.menu_button("Aiuto", |ui| help_menu(ui, state));
    });
}

/// A menu entry that sends `command` when clicked.
fn item(
    ui: &mut Ui,
    state: &mut AppState,
    enabled: bool,
    glyph: Option<&str>,
    label: &str,
    shortcut: Option<&str>,
    command: Command,
) {
    let mut button = match glyph {
        Some(glyph) => Button::new(with_icon(glyph, label)),
        None => Button::new(label),
    };
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(shortcut);
    }
    if ui.add_enabled(enabled, button).clicked() {
        state.send(command);
    }
}

fn file_menu(ui: &mut Ui, state: &mut AppState) {
    let has_project = state.workspace.project.is_some();
    let active = state.workspace.active_document();
    let active_dirty = active.is_some_and(|d| d.doc.is_dirty());
    let has_document = active.is_some();
    let any_dirty = state.workspace.has_unsaved_changes();

    item(
        ui,
        state,
        true,
        Some(icon::FOLDER_PLUS),
        "Nuovo progetto…",
        Some("Ctrl+Shift+N"),
        Command::NewProject,
    );
    item(
        ui,
        state,
        true,
        Some(icon::FOLDER_OPEN),
        "Apri cartella…",
        Some("Ctrl+O"),
        Command::OpenFolderDialog,
    );
    let recent: Vec<_> = state.workspace.recent.paths().to_vec();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button(
            with_icon(icon::CLOCK_COUNTER_CLOCKWISE, "Progetti recenti"),
            |ui| {
                for path in recent {
                    let label = path.display().to_string();
                    if ui.add_enabled(path.is_dir(), Button::new(label)).clicked() {
                        state.send(Command::OpenProject(path));
                    }
                }
            },
        );
    });
    ui.separator();
    item(
        ui,
        state,
        active_dirty,
        Some(icon::FLOPPY_DISK),
        "Salva",
        Some("Ctrl+S"),
        Command::Save,
    );
    item(
        ui,
        state,
        any_dirty,
        None,
        "Salva tutto",
        Some("Ctrl+Shift+S"),
        Command::SaveAll,
    );
    ui.separator();
    item(
        ui,
        state,
        has_document,
        None,
        "Chiudi file",
        Some("Ctrl+W"),
        Command::CloseActiveDocument,
    );
    item(
        ui,
        state,
        has_project,
        None,
        "Chiudi progetto",
        None,
        Command::CloseProject,
    );
    ui.separator();
    item(
        ui,
        state,
        true,
        Some(icon::SIGN_OUT),
        "Esci",
        None,
        Command::Quit,
    );
}

fn edit_menu(ui: &mut Ui, state: &mut AppState) {
    let has_document = state.workspace.active_document().is_some();
    let is_slint = state
        .workspace
        .active_document()
        .is_some_and(|d| d.doc.language() == slint_studio_core::Language::Slint);
    item(
        ui,
        state,
        has_document,
        Some(icon::ARROW_COUNTER_CLOCKWISE),
        "Annulla",
        Some("Ctrl+Z"),
        Command::Undo,
    );
    item(
        ui,
        state,
        has_document,
        Some(icon::ARROW_CLOCKWISE),
        "Ripeti",
        Some("Ctrl+Y"),
        Command::Redo,
    );
    ui.separator();
    // Clipboard commands from the menu need a clipboard API egui does not
    // expose; the keyboard shortcuts work in the editor.
    for (label, glyph, shortcut) in [
        ("Taglia", icon::SCISSORS, "Ctrl+X"),
        ("Copia", icon::COPY, "Ctrl+C"),
        ("Incolla", icon::CLIPBOARD, "Ctrl+V"),
    ] {
        planned(ui, Planned::new(label, 5).icon(glyph).shortcut(shortcut))
            .on_disabled_hover_text(format!("Nell'editor usa {shortcut}"));
    }
    ui.separator();
    let cursor = state
        .workspace
        .active_document()
        .map(|d| (d.doc.path().to_path_buf(), d.editor.cursor));
    if let Some((path, char_index)) = cursor.clone() {
        item(
            ui,
            state,
            is_slint,
            Some(icon::TEXT_AA),
            "Suggerisci completamento",
            Some("Ctrl+Space"),
            Command::RequestCompletion { path, char_index },
        );
    }
    if let Some((path, char_index)) = cursor {
        item(
            ui,
            state,
            is_slint,
            Some(icon::ARROW_SQUARE_OUT),
            "Vai alla definizione",
            Some("F12"),
            Command::RequestDefinition { path, char_index },
        );
    }
    ui.separator();
    planned(
        ui,
        Planned::new("Trova", 5)
            .icon(icon::MAGNIFYING_GLASS)
            .shortcut("Ctrl+F"),
    );
    let has_project = state.workspace.project.is_some();
    item(
        ui,
        state,
        has_project,
        None,
        "Trova nei file",
        Some("Ctrl+Shift+F"),
        Command::ShowPanel(Tab::Search),
    );
}

fn view_menu(ui: &mut Ui, state: &mut AppState, dock: &DockState<Tab>) {
    ui.label(RichText::new("Modalità").small().weak());
    for mode in ViewMode::ALL {
        let button = Button::selectable(
            state.view_mode == mode,
            with_icon(view_mode_icon(mode), mode.label()),
        );
        if ui.add(button).clicked() {
            state.send(Command::SetViewMode(mode));
        }
    }
    ui.separator();
    ui.menu_button(with_icon(icon::LAYOUT, "Pannelli"), |ui| {
        for tab in Tab::PANELS {
            let mut open = layout::is_open(dock, tab);
            if ui
                .checkbox(&mut open, with_icon(tab.icon(), tab.title()))
                .clicked()
            {
                state.send(Command::TogglePanel(tab));
            }
        }
    });
    if ui
        .button(with_icon(icon::ARROWS_CLOCKWISE, "Ripristina layout"))
        .clicked()
    {
        state.send(Command::ResetLayout);
    }
    ui.separator();
    planned(ui, Planned::new("Tema chiaro", 5).icon(icon::SUN));
}

fn project_menu(ui: &mut Ui, state: &mut AppState) {
    let has_project = state.workspace.project.is_some();
    item(
        ui,
        state,
        true,
        Some(icon::FOLDER_PLUS),
        "Nuovo progetto…",
        Some("Ctrl+Shift+N"),
        Command::NewProject,
    );
    item(
        ui,
        state,
        true,
        Some(icon::FOLDER_OPEN),
        "Apri cartella…",
        Some("Ctrl+O"),
        Command::OpenFolderDialog,
    );
    item(
        ui,
        state,
        has_project,
        Some(icon::ARROWS_CLOCKWISE),
        "Aggiorna albero dei file",
        None,
        Command::RefreshTree,
    );
    item(
        ui,
        state,
        has_project,
        None,
        "Chiudi progetto",
        None,
        Command::CloseProject,
    );
}

fn run_menu(ui: &mut Ui, state: &mut AppState) {
    let enabled = state.workspace.project.is_some() && state.project_kind.is_supported();
    let can_stop = state.runner.is_running() && !state.runner.is_stopping();
    item(
        ui,
        state,
        enabled,
        Some(icon::HAMMER),
        "Compila",
        Some("Ctrl+B"),
        Command::Build,
    );
    item(
        ui,
        state,
        enabled,
        Some(icon::PLAY),
        "Esegui",
        Some("Ctrl+R"),
        Command::Run,
    );
    item(
        ui,
        state,
        can_stop,
        Some(icon::STOP),
        "Interrompi",
        Some("Maiusc+F5"),
        Command::StopRun,
    );
    ui.separator();
    if state.workspace.project.is_some() {
        ui.label(
            RichText::new(format!("Progetto: {}", state.project_kind.label()))
                .small()
                .weak(),
        );
        ui.separator();
    }
    let mut live = state.live_preview;
    if ui.checkbox(&mut live, "Anteprima live").clicked() {
        state.send(Command::ToggleLivePreview);
    }
}

fn tools_menu(ui: &mut Ui, state: &mut AppState) {
    planned(
        ui,
        Planned::new("Command palette…", 5)
            .icon(icon::COMMAND)
            .shortcut("Ctrl+Shift+P"),
    );
    let has_project = state.workspace.project.is_some();
    item(
        ui,
        state,
        has_project,
        Some(icon::ARROWS_CLOCKWISE),
        "Riavvia language server",
        None,
        Command::RestartLsp,
    );
    ui.separator();
    planned(ui, Planned::new("Opzioni…", 5).icon(icon::GEAR_SIX));
}

fn help_menu(ui: &mut Ui, state: &mut AppState) {
    item(
        ui,
        state,
        true,
        Some(icon::BOOK_OPEN),
        "Documentazione Slint",
        None,
        Command::OpenSlintDocs,
    );
    ui.separator();
    item(
        ui,
        state,
        true,
        Some(icon::INFO),
        "Informazioni su Slint Studio",
        None,
        Command::ShowAbout,
    );
}
