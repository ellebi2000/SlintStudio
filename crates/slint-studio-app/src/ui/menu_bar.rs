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
        ui.menu_button("Modifica", edit_menu);
        ui.menu_button("Visualizza", |ui| view_menu(ui, state, dock));
        ui.menu_button("Progetto", project_menu);
        ui.menu_button("Esegui", |ui| run_menu(ui, state));
        ui.menu_button("Strumenti", tools_menu);
        ui.menu_button("Aiuto", |ui| help_menu(ui, state));
    });
}

fn file_menu(ui: &mut Ui, state: &mut AppState) {
    planned(
        ui,
        Planned::new("Apri cartella…", 1)
            .icon(icon::FOLDER_OPEN)
            .shortcut("Ctrl+O"),
    );
    planned(
        ui,
        Planned::new("Salva", 1)
            .icon(icon::FLOPPY_DISK)
            .shortcut("Ctrl+S"),
    );
    planned(ui, Planned::new("Salva tutto", 1).shortcut("Ctrl+Shift+S"));
    ui.separator();
    planned(ui, Planned::new("Chiudi progetto", 1));
    ui.separator();
    if ui.button(with_icon(icon::SIGN_OUT, "Esci")).clicked() {
        state.send(Command::Quit);
    }
}

fn edit_menu(ui: &mut Ui) {
    let entries = [
        Planned::new("Annulla", 1)
            .icon(icon::ARROW_COUNTER_CLOCKWISE)
            .shortcut("Ctrl+Z"),
        Planned::new("Ripeti", 1)
            .icon(icon::ARROW_CLOCKWISE)
            .shortcut("Ctrl+Y"),
    ];
    for entry in entries {
        planned(ui, entry);
    }
    ui.separator();
    let entries = [
        Planned::new("Taglia", 1)
            .icon(icon::SCISSORS)
            .shortcut("Ctrl+X"),
        Planned::new("Copia", 1).icon(icon::COPY).shortcut("Ctrl+C"),
        Planned::new("Incolla", 1)
            .icon(icon::CLIPBOARD)
            .shortcut("Ctrl+V"),
    ];
    for entry in entries {
        planned(ui, entry);
    }
    ui.separator();
    planned(
        ui,
        Planned::new("Trova", 1)
            .icon(icon::MAGNIFYING_GLASS)
            .shortcut("Ctrl+F"),
    );
    planned(
        ui,
        Planned::new("Trova nei file", 1).shortcut("Ctrl+Shift+F"),
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

fn project_menu(ui: &mut Ui) {
    planned(
        ui,
        Planned::new("Apri cartella…", 1).icon(icon::FOLDER_OPEN),
    );
    planned(
        ui,
        Planned::new("Aggiorna albero dei file", 1).icon(icon::ARROWS_CLOCKWISE),
    );
    planned(ui, Planned::new("Chiudi progetto", 1));
}

fn run_menu(ui: &mut Ui, state: &mut AppState) {
    planned(
        ui,
        Planned::new("Compila", 4)
            .icon(icon::HAMMER)
            .shortcut("Ctrl+B"),
    );
    planned(
        ui,
        Planned::new("Esegui", 4)
            .icon(icon::PLAY)
            .shortcut("Ctrl+R"),
    );
    planned(ui, Planned::new("Interrompi", 4).icon(icon::STOP));
    ui.separator();
    let mut live = state.live_preview;
    if ui.checkbox(&mut live, "Anteprima live").clicked() {
        state.send(Command::ToggleLivePreview);
    }
}

fn tools_menu(ui: &mut Ui) {
    planned(
        ui,
        Planned::new("Command palette…", 5)
            .icon(icon::COMMAND)
            .shortcut("Ctrl+Shift+P"),
    );
    planned(
        ui,
        Planned::new("Riavvia language server", 1).icon(icon::ARROWS_CLOCKWISE),
    );
    ui.separator();
    planned(ui, Planned::new("Opzioni…", 5).icon(icon::GEAR_SIX));
}

fn help_menu(ui: &mut Ui, state: &mut AppState) {
    if ui
        .button(with_icon(icon::BOOK_OPEN, "Documentazione Slint"))
        .clicked()
    {
        state.send(Command::OpenSlintDocs);
    }
    ui.separator();
    if ui
        .button(with_icon(icon::INFO, "Informazioni su Slint Studio"))
        .clicked()
    {
        state.send(Command::ShowAbout);
    }
}
