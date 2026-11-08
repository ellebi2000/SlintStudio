//! Left side: project tree, project-wide search and recent projects.

use std::path::Path;

use egui::{Button, Key, RichText, TextEdit, Ui};
use egui_phosphor::regular as icon;
use slint_studio_core::project::SearchResults;
use slint_studio_core::{FileNode, LineCol};

use super::{empty_state, ico, padded, with_icon};
use crate::app::AppState;
use crate::commands::Command;
use crate::workspace::Project;

/// Indentation of one tree level, in points.
const TREE_INDENT: f32 = 14.0;

#[derive(Debug, Default)]
pub struct ExplorerState {
    pub filter: String,
    pub search_query: String,
    pub case_sensitive: bool,
    pub results: Option<SearchResults>,
}

pub fn project(ui: &mut Ui, state: &mut AppState) {
    let Some(project) = &state.workspace.project else {
        return no_project(ui, state);
    };
    let mut commands = Vec::new();
    let name = project.name().to_owned();
    let root = project.root.display().to_string();
    let palette = state.palette;
    padded(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(ico(icon::FOLDER_OPEN).color(palette.accent_secondary));
            ui.label(RichText::new(name).strong()).on_hover_text(root);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(Button::new(ico(icon::ARROWS_CLOCKWISE)).frame(false))
                    .on_hover_text("Aggiorna l'albero dei file")
                    .clicked()
                {
                    commands.push(Command::RefreshTree);
                }
            });
        });
        ui.add(
            TextEdit::singleline(&mut state.explorer.filter)
                .hint_text("Filtra file…")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);

        let filter = state.explorer.filter.trim().to_lowercase();
        let active = state
            .workspace
            .active_document()
            .map(|d| d.doc.path().to_path_buf());
        if let Some(project) = &mut state.workspace.project {
            let Project { tree, expanded, .. } = project;
            for node in tree.root.children() {
                tree_node(
                    ui,
                    node,
                    0,
                    &filter,
                    expanded,
                    active.as_deref(),
                    &palette,
                    &mut commands,
                );
            }
            if tree.truncated {
                ui.label(
                    RichText::new("Albero incompleto: troppi file")
                        .small()
                        .weak(),
                );
            }
        }
    });
    for command in commands {
        state.send(command);
    }
}

#[allow(clippy::too_many_arguments)]
fn tree_node(
    ui: &mut Ui,
    node: &FileNode,
    depth: usize,
    filter: &str,
    expanded: &mut std::collections::HashSet<std::path::PathBuf>,
    active: Option<&Path>,
    palette: &crate::theme::Palette,
    commands: &mut Vec<Command>,
) {
    if !node.matches(filter) {
        return;
    }
    let indent = depth as f32 * TREE_INDENT;
    if node.is_dir() {
        // While filtering, every matching directory is shown open.
        let is_open = !filter.is_empty() || expanded.contains(&node.path);
        let caret = if is_open {
            icon::CARET_DOWN
        } else {
            icon::CARET_RIGHT
        };
        let folder = if is_open {
            icon::FOLDER_OPEN
        } else {
            icon::FOLDER
        };
        let response = ui
            .horizontal(|ui| {
                ui.add_space(indent);
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add(
                    Button::new((
                        ico(caret).size(10.0).color(palette.text_weak),
                        ico(folder).color(palette.warning.gamma_multiply(0.8)),
                        node.name.as_str(),
                    ))
                    .frame(false),
                )
            })
            .inner;
        if response.clicked() && filter.is_empty() {
            if is_open {
                expanded.remove(&node.path);
            } else {
                expanded.insert(node.path.clone());
            }
        }
        if is_open {
            for child in node.children() {
                tree_node(
                    ui,
                    child,
                    depth + 1,
                    filter,
                    expanded,
                    active,
                    palette,
                    commands,
                );
            }
        }
    } else {
        let glyph = if node.is_slint() {
            ico(icon::FILE_CODE).color(palette.accent_secondary)
        } else {
            ico(icon::FILE).color(palette.text_weak)
        };
        let selected = active == Some(node.path.as_path());
        let response = ui
            .horizontal(|ui| {
                // Align file icons with the folder icons (past the caret).
                ui.add_space(indent + 14.0);
                ui.add(Button::selectable(selected, (glyph, node.name.as_str())))
            })
            .inner;
        if response.clicked() {
            commands.push(Command::OpenFile(node.path.clone()));
        }
    }
}

fn no_project(ui: &mut Ui, state: &mut AppState) {
    let mut commands = Vec::new();
    padded(ui, |ui| {
        empty_state(
            ui,
            icon::FOLDER_OPEN,
            "Nessun progetto aperto",
            "Crea un nuovo progetto o apri una cartella che contiene file .slint.",
        );
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            if ui
                .button(with_icon(icon::FOLDER_PLUS, "Nuovo progetto…"))
                .on_hover_text("Ctrl+Shift+N")
                .clicked()
            {
                commands.push(Command::NewProject);
            }
            if ui
                .button(with_icon(icon::FOLDER_OPEN, "Apri cartella…"))
                .on_hover_text("Ctrl+O")
                .clicked()
            {
                commands.push(Command::OpenFolderDialog);
            }
        });
        let recent: Vec<_> = state
            .workspace
            .recent
            .paths()
            .iter()
            .take(5)
            .cloned()
            .collect();
        if !recent.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Recenti").small().weak());
            for path in recent {
                recent_entry(ui, &path, &mut commands);
            }
        }
    });
    for command in commands {
        state.send(command);
    }
}

fn recent_entry(ui: &mut Ui, path: &Path, commands: &mut Vec<Command>) {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let exists = path.is_dir();
    ui.horizontal(|ui| {
        let button = Button::new(with_icon(icon::FOLDER, name)).frame(false);
        let response = ui
            .add_enabled(exists, button)
            .on_hover_text(path.display().to_string())
            .on_disabled_hover_text(format!("Cartella non trovata: {}", path.display()));
        if response.clicked() {
            commands.push(Command::OpenProject(path.to_path_buf()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(Button::new(ico(icon::X).size(10.0)).frame(false))
                .on_hover_text("Rimuovi dai recenti")
                .clicked()
            {
                commands.push(Command::RemoveRecent(path.to_path_buf()));
            }
        });
    });
}

pub fn search(ui: &mut Ui, state: &mut AppState) {
    let has_project = state.workspace.project.is_some();
    let mut commands = Vec::new();
    padded(ui, |ui| {
        let response = ui
            .add_enabled(
                has_project,
                TextEdit::singleline(&mut state.explorer.search_query)
                    .id(egui::Id::new("project_search_query"))
                    .hint_text("Cerca nei file del progetto…")
                    .desired_width(f32::INFINITY),
            )
            .on_disabled_hover_text("Apri un progetto per cercare nei suoi file");
        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            commands.push(Command::RunSearch);
        }
        ui.horizontal(|ui| {
            ui.add_enabled(
                has_project,
                egui::Checkbox::new(&mut state.explorer.case_sensitive, "Maiuscole/minuscole"),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        has_project,
                        Button::new(with_icon(icon::MAGNIFYING_GLASS, "Cerca")),
                    )
                    .clicked()
                {
                    commands.push(Command::RunSearch);
                }
            });
        });
        ui.separator();

        match (&state.explorer.results, &state.workspace.project) {
            (Some(results), Some(project)) => search_results(ui, results, project, &mut commands),
            _ => empty_state(
                ui,
                icon::MAGNIFYING_GLASS,
                "Ricerca nel progetto",
                "Cerca testo in tutti i file del progetto aperto (Ctrl+Shift+F).",
            ),
        }
    });
    for command in commands {
        state.send(command);
    }
}

fn search_results(
    ui: &mut Ui,
    results: &SearchResults,
    project: &Project,
    commands: &mut Vec<Command>,
) {
    let files = {
        let mut files: Vec<&Path> = results.hits.iter().map(|h| h.path.as_path()).collect();
        files.dedup();
        files.len()
    };
    let mut summary = format!("{} risultati in {files} file", results.hits.len());
    if results.truncated {
        summary.push_str(" (elenco troncato)");
    }
    ui.label(RichText::new(summary).small().weak());

    let mut current: Option<&Path> = None;
    for hit in &results.hits {
        if current != Some(hit.path.as_path()) {
            current = Some(hit.path.as_path());
            ui.add_space(4.0);
            ui.label(
                RichText::new(project.relative(&hit.path).display().to_string())
                    .small()
                    .strong(),
            );
        }
        let text = format!("{:>4}  {}", hit.line + 1, hit.preview);
        let button = Button::new(RichText::new(text).monospace().small()).frame(false);
        if ui.add(button).clicked() {
            commands.push(Command::OpenLocation {
                path: hit.path.clone(),
                position: LineCol::new(hit.line, hit.column),
            });
        }
    }
}

pub fn recent(ui: &mut Ui, state: &mut AppState) {
    let recent: Vec<_> = state.workspace.recent.paths().to_vec();
    let mut commands = Vec::new();
    padded(ui, |ui| {
        if recent.is_empty() {
            empty_state(
                ui,
                icon::CLOCK_COUNTER_CLOCKWISE,
                "Nessun progetto recente",
                "I progetti aperti di recente compariranno qui.",
            );
        }
        for path in &recent {
            recent_entry(ui, path, &mut commands);
        }
    });
    for command in commands {
        state.send(command);
    }
}
