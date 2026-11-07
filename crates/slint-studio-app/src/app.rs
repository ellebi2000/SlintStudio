//! Top-level application: owns the state, the dock layout and the language
//! server, lays out the window and executes commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Key, KeyboardShortcut, Modifiers, TextEdit};
use egui_dock::{DockArea, DockState};
use slint_studio_core::project as core_project;
use slint_studio_core::text::{byte_to_char, char_to_byte};
use slint_studio_core::{Language, LineCol, LogBuffer, PositionEncoding, ViewMode};
use slint_studio_lsp::{DefinitionTarget, LspEvent, RequestId, convert};
use tracing::{debug, error, info};

use crate::commands::{Command, GuardedAction, UnsavedChoice};
use crate::fonts::{self, FontChoice};
use crate::layout::{self, Tab};
use crate::logging::RepaintSignal;
use crate::lsp::{LspManager, LspStatus};
use crate::theme::{self, Palette};
use crate::ui;
use crate::ui::editor::{self, CompletionEntry, CompletionPopup};
use crate::ui::explorer::ExplorerState;
use crate::workspace::{DiagnosticStore, Project, Workspace};

const SLINT_DOCS_URL: &str = "https://docs.slint.dev";
/// Maximum number of hits of a project-wide search.
const MAX_SEARCH_HITS: usize = 2_000;

/// How the application starts.
#[derive(Debug, Clone, Default)]
pub struct StartupOptions {
    /// Project folder to open immediately.
    pub project: Option<PathBuf>,
    /// Whether to run the language server (disabled in UI tests).
    pub language_server: bool,
}

/// State shared by every panel. Panels read it freely but change it mostly
/// through [`AppState::send`], so that every action goes through
/// [`StudioApp::execute`].
pub struct AppState {
    pub palette: Palette,
    pub fonts: FontChoice,
    pub view_mode: ViewMode,
    pub live_preview: bool,
    pub log: LogBuffer,
    pub log_view: ui::output::LogView,
    pub workspace: Workspace,
    pub diagnostics: DiagnosticStore,
    pub explorer: ExplorerState,
    pub lsp_status: LspStatus,
    pub show_about: bool,
    /// Action waiting for the "unsaved changes" dialog.
    pub unsaved: Option<GuardedAction>,
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
            workspace: Workspace::new(),
            diagnostics: DiagnosticStore::default(),
            explorer: ExplorerState::default(),
            lsp_status: LspStatus::Idle,
            show_about: false,
            unsaved: None,
            commands: Vec::new(),
        }
    }

    /// Queues `command` for execution at the end of the current frame.
    pub fn send(&mut self, command: Command) {
        self.commands.push(command);
    }
}

pub struct StudioApp {
    state: AppState,
    dock: DockState<Tab>,
    dock_style: egui_dock::Style,
    lsp: LspManager,
    language_server: bool,
    /// Pending go-to-definition request.
    definition_request: Option<RequestId>,
    /// Set once quitting was confirmed despite unsaved changes.
    allow_close: bool,
}

impl StudioApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        log: LogBuffer,
        repaint: RepaintSignal,
        options: StartupOptions,
    ) -> Self {
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
        let waker_ctx = ctx.clone();
        let lsp = LspManager::new(Arc::new(move || waker_ctx.request_repaint()));

        let mut app = Self {
            state: AppState::new(palette, fonts, log),
            dock: layout::default_dock_state(),
            dock_style,
            lsp,
            language_server: options.language_server,
            definition_request: None,
            allow_close: false,
        };
        if let Some(project) = options.project {
            app.state.send(Command::OpenProject(project));
        }
        info!("Slint Studio pronto");
        app
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let command_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // Most specific first: extra Shift is ignored when matching.
        let shortcuts = [
            (
                KeyboardShortcut::new(command_shift, Key::S),
                Command::SaveAll,
            ),
            (
                KeyboardShortcut::new(command_shift, Key::F),
                Command::ShowPanel(Tab::Search),
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::S),
                Command::Save,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::O),
                Command::OpenFolderDialog,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::W),
                Command::CloseActiveDocument,
            ),
        ];
        for (shortcut, command) in shortcuts {
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.state.send(command);
            }
        }
    }

    /// Intercepts the window's close button while there are unsaved changes.
    fn guard_window_close(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && !self.allow_close && self.state.workspace.has_unsaved_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.state.unsaved = Some(GuardedAction::Quit);
        }
    }

    fn run_commands(&mut self, ctx: &egui::Context) {
        // Commands may queue further commands; bound the cascade.
        for _ in 0..8 {
            let commands = std::mem::take(&mut self.state.commands);
            if commands.is_empty() {
                break;
            }
            for command in commands {
                debug!(?command, "esecuzione comando");
                self.execute(ctx, command);
            }
            // Commands run after the UI pass: show their effect right away
            // instead of on the next input event.
            ctx.request_repaint();
        }
    }

    fn execute(&mut self, ctx: &egui::Context, command: Command) {
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
            Command::ShowPanel(tab) => {
                layout::open(&mut self.dock, tab);
                if tab == Tab::Search {
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("project_search_query")));
                }
            }
            Command::ResetLayout => {
                self.dock = layout::default_dock_state();
                info!("Layout dei pannelli ripristinato");
            }
            Command::ShowAbout => self.state.show_about = true,
            Command::OpenSlintDocs => ctx.open_url(egui::OpenUrl::new_tab(SLINT_DOCS_URL)),
            Command::Quit => self.guard(ctx, GuardedAction::Quit),

            Command::OpenFolderDialog => {
                let folder = rfd::FileDialog::new()
                    .set_title("Apri cartella progetto")
                    .pick_folder();
                if let Some(folder) = folder {
                    self.guard(ctx, GuardedAction::OpenProject(folder));
                }
            }
            Command::OpenProject(path) => self.guard(ctx, GuardedAction::OpenProject(path)),
            Command::CloseProject => self.guard(ctx, GuardedAction::CloseProject),
            Command::RefreshTree => {
                if let Some(project) = &mut self.state.workspace.project {
                    match project.refresh() {
                        Ok(()) => info!("Albero dei file aggiornato"),
                        Err(err) => error!("Impossibile aggiornare l'albero dei file: {err}"),
                    }
                }
            }
            Command::RemoveRecent(path) => {
                self.state.workspace.recent.remove(&path);
                self.state.workspace.save_recent();
            }
            Command::RunSearch => self.run_search(),

            Command::OpenFile(path) => {
                self.open_file(&path);
            }
            Command::OpenLocation { path, position } => {
                if self.open_file(&path) {
                    self.reveal(&path, position);
                }
            }
            Command::ActivateDocument(path) => {
                if let Some(index) = self.state.workspace.find(&path) {
                    self.state.workspace.active = Some(index);
                }
            }
            Command::CloseDocument(path) => {
                let dirty = self
                    .state
                    .workspace
                    .document(&path)
                    .is_some_and(|d| d.doc.is_dirty());
                if dirty {
                    self.state.unsaved = Some(GuardedAction::CloseDocument(path));
                } else {
                    self.close_document(&path);
                }
            }
            Command::CloseActiveDocument => {
                if let Some(open) = self.state.workspace.active_document() {
                    let path = open.doc.path().to_path_buf();
                    self.execute(ctx, Command::CloseDocument(path));
                }
            }
            Command::Save => {
                if let Some(open) = self.state.workspace.active_document() {
                    let path = open.doc.path().to_path_buf();
                    self.save_document(&path);
                }
            }
            Command::SaveAll => {
                self.save_all();
            }
            Command::Undo => self.undo_redo(ctx, false),
            Command::Redo => self.undo_redo(ctx, true),
            Command::DocumentEdited(path) => {
                if let (Some(client), Some(open)) =
                    (self.lsp.client(), self.state.workspace.document(&path))
                    && open.doc.language() == Language::Slint
                {
                    client.did_change(&path, open.doc.text(), open.doc.version());
                }
            }

            Command::RequestCompletion { path, char_index } => {
                self.request_completion(&path, char_index)
            }
            Command::RequestDefinition { path, char_index } => {
                self.request_definition(&path, char_index)
            }
            Command::RestartLsp => {
                self.state.diagnostics.clear();
                self.lsp.restart();
            }

            Command::ResolveUnsaved(choice) => self.resolve_unsaved(ctx, choice),
        }
    }

    // --- Guarded actions ----------------------------------------------------

    fn guard(&mut self, ctx: &egui::Context, action: GuardedAction) {
        if self.state.workspace.has_unsaved_changes() {
            self.state.unsaved = Some(action);
        } else {
            self.perform(ctx, action);
        }
    }

    fn perform(&mut self, ctx: &egui::Context, action: GuardedAction) {
        match action {
            GuardedAction::CloseDocument(path) => self.close_document(&path),
            GuardedAction::CloseProject => self.close_project(),
            GuardedAction::OpenProject(path) => self.open_project(&path),
            GuardedAction::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn resolve_unsaved(&mut self, ctx: &egui::Context, choice: UnsavedChoice) {
        let Some(action) = self.state.unsaved.take() else {
            return;
        };
        match choice {
            UnsavedChoice::Cancel => {}
            UnsavedChoice::Discard => self.perform(ctx, action),
            UnsavedChoice::Save => {
                let saved = match &action {
                    GuardedAction::CloseDocument(path) => self.save_document(path),
                    _ => self.save_all(),
                };
                if saved {
                    self.perform(ctx, action);
                }
            }
        }
    }

    // --- Project & documents ------------------------------------------------

    fn open_project(&mut self, path: &Path) {
        let project = match Project::open(path) {
            Ok(project) => project,
            Err(err) => {
                error!("Impossibile aprire il progetto {}: {err}", path.display());
                return;
            }
        };
        self.close_project();
        let root = project.root.clone();
        let entry = project.entry_point();
        info!(
            "Progetto aperto: {} ({} file)",
            root.display(),
            project.tree.root.files().len()
        );
        self.state.workspace.recent.touch(&root);
        self.state.workspace.save_recent();
        self.state.workspace.project = Some(project);
        layout::open(&mut self.dock, Tab::Project);
        if self.language_server {
            self.lsp.start(&root);
        }
        if let Some(entry) = entry {
            self.open_file(&entry);
        }
    }

    fn close_project(&mut self) {
        if self.state.workspace.project.is_none() {
            return;
        }
        self.lsp.stop();
        let workspace = &mut self.state.workspace;
        workspace.documents.clear();
        workspace.active = None;
        workspace.project = None;
        self.state.diagnostics.clear();
        self.state.explorer.results = None;
        self.definition_request = None;
        info!("Progetto chiuso");
    }

    /// Opens (or focuses) a file; returns whether it is now open.
    fn open_file(&mut self, path: &Path) -> bool {
        match self.state.workspace.open_document(path) {
            Ok(newly_opened) => {
                if newly_opened
                    && let (Some(client), Some(open)) =
                        (self.lsp.client(), self.state.workspace.document(path))
                    && open.doc.language() == Language::Slint
                {
                    client.did_open(path, open.doc.text(), open.doc.version());
                }
                true
            }
            Err(err) => {
                error!("Impossibile aprire {}: {err}", path.display());
                false
            }
        }
    }

    /// Moves the cursor of an open document to `position` (columns in chars).
    fn reveal(&mut self, path: &Path, position: LineCol) {
        if let Some(open) = self.state.workspace.document_mut(path) {
            let text = open.doc.text();
            let byte = open
                .doc
                .line_index()
                .offset(text, position, PositionEncoding::Chars);
            open.editor.reveal = Some(byte_to_char(text, byte));
        }
    }

    fn close_document(&mut self, path: &Path) {
        if let Some(closed) = self.state.workspace.close_document(path)
            && closed.doc.language() == Language::Slint
        {
            if let Some(client) = self.lsp.client() {
                client.did_close(path);
            }
            self.state.diagnostics.set(path.to_path_buf(), Vec::new());
        }
    }

    fn save_document(&mut self, path: &Path) -> bool {
        let Some(open) = self.state.workspace.document_mut(path) else {
            return false;
        };
        match open.doc.save() {
            Ok(()) => {
                info!("Salvato {}", path.display());
                if open.doc.language() == Language::Slint
                    && let Some(client) = self.lsp.client()
                {
                    client.did_save(path);
                }
                true
            }
            Err(err) => {
                error!("Impossibile salvare {}: {err}", path.display());
                false
            }
        }
    }

    /// Saves every modified document; returns whether all succeeded.
    fn save_all(&mut self) -> bool {
        let dirty: Vec<PathBuf> = self
            .state
            .workspace
            .documents
            .iter()
            .filter(|d| d.doc.is_dirty())
            .map(|d| d.doc.path().to_path_buf())
            .collect();
        // Try every file even if one fails (no short-circuit).
        let mut all_saved = true;
        for path in &dirty {
            all_saved &= self.save_document(path);
        }
        all_saved
    }

    /// Undo/redo from the menu, using the history egui keeps for the editor.
    fn undo_redo(&mut self, ctx: &egui::Context, redo: bool) {
        let Some(open) = self.state.workspace.active_document_mut() else {
            return;
        };
        let path = open.doc.path().to_path_buf();
        let id = editor::text_edit_id(&path);
        let Some(mut state) = TextEdit::load_state(ctx, id) else {
            return;
        };
        let mut undoer = state.undoer();
        let current = (
            state.cursor.char_range().unwrap_or_default(),
            open.doc.text().to_owned(),
        );
        let restored = if redo {
            undoer.redo(&current)
        } else {
            undoer.undo(&current)
        }
        .cloned();
        if let Some((range, text)) = restored {
            open.doc.set_text(text);
            state.cursor.set_char_range(Some(range));
            state.set_undoer(undoer);
            TextEdit::store_state(ctx, id, state);
            self.state.send(Command::DocumentEdited(path));
        }
    }

    fn run_search(&mut self) {
        let Some(project) = &self.state.workspace.project else {
            return;
        };
        let query = self.state.explorer.search_query.trim().to_owned();
        if query.is_empty() {
            self.state.explorer.results = None;
            return;
        }
        let results = core_project::search(
            &project.tree,
            &query,
            self.state.explorer.case_sensitive,
            MAX_SEARCH_HITS,
        );
        info!(
            "Ricerca \"{query}\": {} risultati in {} file esaminati",
            results.hits.len(),
            results.files_searched
        );
        self.state.explorer.results = Some(results);
        layout::open(&mut self.dock, Tab::Search);
    }

    // --- Language server ----------------------------------------------------

    fn request_completion(&mut self, path: &Path, char_index: usize) {
        let Some(client) = self.lsp.client() else {
            debug!("completamento ignorato: language server non pronto");
            return;
        };
        let encoding = self.lsp.encoding();
        let Some(open) = self.state.workspace.document_mut(path) else {
            return;
        };
        if open.doc.language() != Language::Slint {
            return;
        }
        let text = open.doc.text();
        let position =
            open.doc
                .line_index()
                .position(text, char_to_byte(text, char_index), encoding);
        if let Some(request) = client.completion(path, position) {
            let anchor = editor::word_start(text, char_index);
            open.editor.completion = Some(CompletionPopup::pending(request, anchor));
        }
    }

    fn request_definition(&mut self, path: &Path, char_index: usize) {
        let Some(client) = self.lsp.client() else {
            info!("Vai alla definizione: language server non pronto");
            return;
        };
        let Some(open) = self.state.workspace.document(path) else {
            return;
        };
        let text = open.doc.text();
        let position = open.doc.line_index().position(
            text,
            char_to_byte(text, char_index),
            self.lsp.encoding(),
        );
        self.definition_request = client.definition(path, position);
    }

    fn process_lsp_events(&mut self) {
        for event in self.lsp.poll() {
            match event {
                LspEvent::Ready { .. } => self.open_documents_in_server(),
                LspEvent::Diagnostics {
                    path, diagnostics, ..
                } => {
                    let text = self.state.workspace.document(&path).map(|d| d.doc.text());
                    let converted = convert::to_core_diagnostics(
                        &path,
                        &diagnostics,
                        text,
                        self.lsp.encoding(),
                    );
                    self.state.diagnostics.set(path, converted);
                }
                LspEvent::Completion { request, items } => {
                    for open in &mut self.state.workspace.documents {
                        let Some(popup) = &mut open.editor.completion else {
                            continue;
                        };
                        if popup.request == Some(request) {
                            popup.request = None;
                            popup.items = items.iter().map(CompletionEntry::from_item).collect();
                            popup.selected = 0;
                            if popup.items.is_empty() {
                                open.editor.completion = None;
                            }
                        }
                    }
                }
                LspEvent::Definition { request, targets } => {
                    if self.definition_request == Some(request) {
                        self.definition_request = None;
                        self.go_to_definition(&targets);
                    }
                }
                LspEvent::RequestFailed { request, message } => {
                    debug!("{message}");
                    if self.definition_request == Some(request) {
                        self.definition_request = None;
                    }
                    for open in &mut self.state.workspace.documents {
                        if open
                            .editor
                            .completion
                            .as_ref()
                            .is_some_and(|p| p.request == Some(request))
                        {
                            open.editor.completion = None;
                        }
                    }
                }
                LspEvent::Message { .. } | LspEvent::Exited { .. } => {}
            }
        }
        self.state.lsp_status = self.lsp.status().clone();
    }

    fn open_documents_in_server(&self) {
        let Some(client) = self.lsp.client() else {
            return;
        };
        for open in &self.state.workspace.documents {
            if open.doc.language() == Language::Slint {
                client.did_open(open.doc.path(), open.doc.text(), open.doc.version());
            }
        }
    }

    fn go_to_definition(&mut self, targets: &[DefinitionTarget]) {
        let Some(target) = targets.first() else {
            info!("Nessuna definizione trovata");
            return;
        };
        let Some(path) = target.path.clone() else {
            info!(
                "La definizione si trova in un modulo integrato di Slint ({})",
                target.uri
            );
            return;
        };
        if !self.open_file(&path) {
            return;
        }
        let encoding = self.lsp.encoding();
        let position = self.state.workspace.document(&path).map(|open| {
            let text = open.doc.text();
            open.doc
                .line_index()
                .convert(text, target.position, encoding, PositionEncoding::Chars)
        });
        if let Some(position) = position {
            self.reveal(&path, position);
        }
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_lsp_events();
        self.handle_shortcuts(&ctx);
        self.guard_window_close(&ctx);
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
            .show(ui, |ui| ui::status_bar::show(ui, &mut self.state));

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

        ui::about::show(&ctx, &mut self.state);
        ui::dialogs::unsaved_changes(&ctx, &mut self.state);

        self.run_commands(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use egui_kittest::{Harness, Node};
    use egui_phosphor::regular as icon;
    use slint_studio_core::{Diagnostic, DiagnosticSource, Location, Severity};
    use std::fs;

    fn harness_with(options: StartupOptions) -> Harness<'static, StudioApp> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .build_eframe(move |cc| {
                StudioApp::new(cc, LogBuffer::default(), RepaintSignal::default(), options)
            });
        harness.run();
        harness
    }

    fn harness() -> Harness<'static, StudioApp> {
        harness_with(StartupOptions::default())
    }

    fn with_project(dir: &tempfile::TempDir) -> Harness<'static, StudioApp> {
        let mut harness = harness_with(StartupOptions {
            project: Some(dir.path().to_path_buf()),
            language_server: false,
        });
        harness.run();
        harness
    }

    fn project_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ui")).unwrap();
        fs::write(
            dir.path().join("ui/main.slint"),
            "export component Main inherits Window {\n    Text { text: \"Ciao\"; }\n}\n",
        )
        .unwrap();
        fs::write(dir.path().join("ui/card.slint"), "component Card {}\n").unwrap();
        dir
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

    fn send(harness: &mut Harness<'_, StudioApp>, command: Command) {
        harness.state_mut().state.send(command);
        harness.run();
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
        button(&harness, &label(icon::FOLDER_OPEN, "Apri cartella…"));
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
        send(&mut harness, Command::TogglePanel(Tab::Terminal));
        assert!(!layout::is_open(&harness.state().dock, Tab::Terminal));
        send(&mut harness, Command::ResetLayout);
        assert!(layout::is_open(&harness.state().dock, Tab::Terminal));
    }

    #[test]
    fn opening_a_project_shows_the_tree_and_opens_the_entry_point() {
        let dir = project_dir();
        let harness = with_project(&dir);
        let state = &harness.state().state;
        let project = state.workspace.project.as_ref().unwrap();
        let active = state.workspace.active_document().unwrap();
        assert!(active.doc.path().ends_with("ui/main.slint"));
        assert_eq!(state.workspace.recent.paths()[0], project.root);
        harness.get_by_label_contains("card.slint");
    }

    #[test]
    fn problems_panel_lists_diagnostics_and_opens_them() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let card = harness
            .state()
            .state
            .workspace
            .project
            .as_ref()
            .unwrap()
            .root
            .join("ui/card.slint");
        harness.state_mut().state.diagnostics.set(
            card.clone(),
            vec![Diagnostic {
                severity: Severity::Error,
                source: DiagnosticSource::Lsp,
                message: "proprietà sconosciuta 'colour'".into(),
                location: Some(Location::at(&card, LineCol::new(0, 10))),
            }],
        );
        layout::open(&mut harness.state_mut().dock, Tab::Problems);
        harness.run();

        harness
            .get_by_label("proprietà sconosciuta 'colour'")
            .click();
        harness.run();
        let state = &harness.state().state;
        assert_eq!(state.diagnostics.counts().errors, 1);
        assert_eq!(state.workspace.active_document().unwrap().doc.path(), card);
    }

    #[test]
    fn editing_marks_dirty_and_ctrl_s_writes_the_file() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        harness.get_by_role(Role::MultilineTextInput).click();
        harness.run();
        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("// nota");
        harness.run();
        let path = {
            let open = harness.state().state.workspace.active_document().unwrap();
            assert!(open.doc.is_dirty());
            assert!(open.doc.text().contains("// nota"));
            open.doc.path().to_path_buf()
        };

        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        harness.run();
        assert!(
            !harness
                .state()
                .state
                .workspace
                .active_document()
                .unwrap()
                .doc
                .is_dirty()
        );
        assert!(fs::read_to_string(path).unwrap().contains("// nota"));
    }

    #[test]
    fn closing_a_modified_document_asks_for_confirmation() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let path = {
            let open = harness
                .state_mut()
                .state
                .workspace
                .active_document_mut()
                .unwrap();
            open.doc.set_text("modificato".into());
            open.doc.path().to_path_buf()
        };

        send(&mut harness, Command::CloseDocument(path.clone()));
        assert_eq!(
            harness.state().state.unsaved,
            Some(GuardedAction::CloseDocument(path.clone()))
        );
        harness.get_by_label("Modifiche non salvate");

        harness.get_by_label("Annulla").click();
        harness.run();
        assert!(harness.state().state.unsaved.is_none());
        assert!(harness.state().state.workspace.document(&path).is_some());

        send(&mut harness, Command::CloseDocument(path.clone()));
        harness.get_by_label("Non salvare").click();
        harness.run();
        assert!(harness.state().state.workspace.document(&path).is_none());
        assert_ne!(fs::read_to_string(&path).unwrap(), "modificato");
    }

    #[test]
    fn search_finds_text_and_reveals_the_hit() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        harness.state_mut().state.explorer.search_query = "Card".into();
        send(&mut harness, Command::RunSearch);
        let hits = harness
            .state()
            .state
            .explorer
            .results
            .as_ref()
            .unwrap()
            .hits
            .clone();
        assert_eq!(hits.len(), 1);
        send(
            &mut harness,
            Command::OpenLocation {
                path: hits[0].path.clone(),
                position: LineCol::new(hits[0].line, hits[0].column),
            },
        );
        harness.run();
        let open = harness.state().state.workspace.active_document().unwrap();
        assert!(open.doc.path().ends_with("ui/card.slint"));
        assert_eq!(open.editor.cursor, 10, "cursor moved to the match");
    }
}
