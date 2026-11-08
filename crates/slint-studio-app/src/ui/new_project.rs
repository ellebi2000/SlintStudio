//! "Nuovo progetto" dialog: template, name and location of a new project.

use std::path::{Path, PathBuf};

use egui::{Id, Key, RichText, TextEdit};
use egui_phosphor::regular as icon;
use slint_studio_core::template::{self, CreateError, ProjectTemplate};

use super::{ico, with_icon};
use crate::app::AppState;
use crate::commands::Command;

/// Name suggested for a new project (made unique in the chosen folder).
const DEFAULT_NAME: &str = "nuovo-progetto";

/// What the user typed in the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProjectForm {
    pub template: ProjectTemplate,
    pub name: String,
    /// Parent folder, as typed.
    pub location: String,
    /// Why the last "Crea" failed.
    pub error: Option<String>,
    focus_requested: bool,
}

impl NewProjectForm {
    pub fn new(location: &Path) -> Self {
        Self {
            template: ProjectTemplate::default(),
            name: unique_name(location),
            location: location.display().to_string(),
            error: None,
            focus_requested: false,
        }
    }

    /// The folder that "Crea" would create, or why it cannot.
    pub fn target(&self) -> Result<PathBuf, CreateError> {
        template::target_dir(Path::new(self.location.trim()), &self.name, self.template)
    }
}

/// `nuovo-progetto`, or `nuovo-progetto-2`, … if taken in `location`.
fn unique_name(location: &Path) -> String {
    (1..100)
        .map(|n| match n {
            1 => DEFAULT_NAME.to_owned(),
            n => format!("{DEFAULT_NAME}-{n}"),
        })
        .find(|name| !location.join(name).exists())
        .unwrap_or_else(|| DEFAULT_NAME.to_owned())
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let palette = state.palette;
    let Some(form) = &mut state.new_project else {
        return;
    };
    let mut commands = Vec::new();
    let target = form.target();

    let modal = egui::Modal::new(Id::new("new_project")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.horizontal(|ui| {
            ui.label(ico(icon::FOLDER_PLUS).size(22.0).color(palette.accent));
            ui.label(RichText::new("Nuovo progetto").heading());
        });
        ui.add_space(8.0);

        ui.label(RichText::new("Tipo").strong());
        for kind in ProjectTemplate::ALL {
            ui.radio_value(&mut form.template, kind, kind.label());
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.add(egui::Label::new(RichText::new(kind.description()).small().weak()).wrap());
            });
        }
        ui.add_space(8.0);

        egui::Grid::new("new_project_fields")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Nome");
                let name = ui.add(
                    TextEdit::singleline(&mut form.name)
                        .desired_width(380.0)
                        .hint_text(DEFAULT_NAME),
                );
                if !form.focus_requested {
                    name.request_focus();
                    form.focus_requested = true;
                }
                if name.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && target.is_ok() {
                    commands.push(Command::CreateProject);
                }
                ui.end_row();

                ui.label("Cartella");
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut form.location).desired_width(290.0));
                    if ui
                        .button(with_icon(icon::FOLDER_OPEN, "Sfoglia…"))
                        .clicked()
                    {
                        commands.push(Command::BrowseNewProjectLocation);
                    }
                });
                ui.end_row();
            });
        ui.add_space(6.0);

        match &target {
            Ok(dir) => {
                ui.label(
                    RichText::new(format!("Verrà creato in {}", dir.display()))
                        .small()
                        .weak(),
                );
                if form.template == ProjectTemplate::RustSlint
                    && let Some(package) = template::package_name(&form.name)
                    && package != form.name.trim()
                {
                    ui.label(
                        RichText::new(format!("Nome del pacchetto Cargo: {package}"))
                            .small()
                            .weak(),
                    );
                }
            }
            Err(err) => {
                ui.label(
                    RichText::new(err.to_string())
                        .small()
                        .color(palette.warning),
                );
            }
        }
        if let Some(error) = &form.error {
            ui.label(RichText::new(error).small().color(palette.error));
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    target.is_ok(),
                    egui::Button::new(with_icon(icon::CHECK, "Crea")),
                )
                .clicked()
            {
                commands.push(Command::CreateProject);
            }
            if ui.button("Annulla").clicked() {
                commands.push(Command::CancelNewProject);
            }
        });
    });
    if modal.should_close() && commands.is_empty() {
        commands.push(Command::CancelNewProject);
    }
    for command in commands {
        state.send(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_names_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_name(dir.path()), "nuovo-progetto");
        std::fs::create_dir(dir.path().join("nuovo-progetto")).unwrap();
        std::fs::create_dir(dir.path().join("nuovo-progetto-2")).unwrap();
        assert_eq!(unique_name(dir.path()), "nuovo-progetto-3");

        let form = NewProjectForm::new(dir.path());
        assert_eq!(form.target().unwrap(), dir.path().join("nuovo-progetto-3"));
    }
}
