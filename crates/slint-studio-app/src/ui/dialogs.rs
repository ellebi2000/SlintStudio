//! Modal dialogs.

use egui::{Id, RichText};
use egui_phosphor::regular as icon;

use super::{ico, with_icon};
use crate::app::AppState;
use crate::commands::{Command, GuardedAction, UnsavedChoice};

/// "Unsaved changes" confirmation, shown while an action is pending.
pub fn unsaved_changes(ctx: &egui::Context, state: &mut AppState) {
    let Some(action) = &state.unsaved else {
        return;
    };
    let names = match action {
        GuardedAction::CloseDocument(path) => state
            .workspace
            .document(path)
            .map(|d| vec![d.doc.file_name()])
            .unwrap_or_default(),
        _ => state.workspace.unsaved_names(),
    };
    let question = match action {
        GuardedAction::CloseDocument(_) => "Salvare le modifiche prima di chiudere il file?",
        GuardedAction::CloseProject | GuardedAction::OpenProject(_) => {
            "Salvare le modifiche prima di chiudere il progetto?"
        }
        GuardedAction::Quit => "Salvare le modifiche prima di uscire?",
    };

    let mut choice = None;
    let modal = egui::Modal::new(Id::new("unsaved_changes")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.horizontal(|ui| {
            ui.label(ico(icon::WARNING).size(22.0).color(state.palette.warning));
            ui.label(RichText::new("Modifiche non salvate").heading());
        });
        ui.add_space(4.0);
        ui.label(question);
        for name in &names {
            ui.label(RichText::new(format!("• {name}")).monospace());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(with_icon(icon::FLOPPY_DISK, "Salva")).clicked() {
                choice = Some(UnsavedChoice::Save);
            }
            if ui.button("Non salvare").clicked() {
                choice = Some(UnsavedChoice::Discard);
            }
            if ui.button("Annulla").clicked() {
                choice = Some(UnsavedChoice::Cancel);
            }
        });
    });
    if modal.should_close() && choice.is_none() {
        choice = Some(UnsavedChoice::Cancel);
    }
    if let Some(choice) = choice {
        state.send(Command::ResolveUnsaved(choice));
    }
}
