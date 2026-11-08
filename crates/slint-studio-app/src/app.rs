//! Top-level application: owns the state, the dock layout and the language
//! server, lays out the window and executes commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui::{Key, KeyboardShortcut, Modifiers, TextEdit};
use egui_dock::{DockArea, DockState};
use slint_studio_compiler::{JobKind, JobOutcome, JobRequest, ProjectKind};
use slint_studio_core::edit::{self, SourceEdit};
use slint_studio_core::project as core_project;
use slint_studio_core::text::{byte_to_char, char_to_byte};
use slint_studio_core::{
    DiagnosticSource, ElementNode, Language, LineCol, LogBuffer, PaletteItem, PositionEncoding,
    ViewMode, properties,
};
use slint_studio_lsp::{DefinitionTarget, LspEvent, RequestId, convert};
use tracing::{debug, error, info, warn};

use crate::commands::{Command, GuardedAction, UnsavedChoice};
use crate::designer::{DesignerState, ElementRef};
use crate::fonts::{self, FontChoice};
use crate::layout::{self, Tab};
use crate::logging::RepaintSignal;
use crate::lsp::{LspManager, LspStatus};
use crate::preview::{EDIT_DEBOUNCE, PreviewManager};
use crate::runner::{Runner, RunnerUpdate};
use crate::theme::{self, Palette};
use crate::ui;
use crate::ui::editor::{self, CompletionEntry, CompletionPopup};
use crate::ui::explorer::ExplorerState;
use crate::ui::new_project::NewProjectForm;
use crate::watcher::{FileWatcher, is_slint};
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
    /// The "Nuovo progetto" dialog, while open.
    pub new_project: Option<NewProjectForm>,
    pub preview: PreviewManager,
    pub designer: DesignerState,
    /// How the open project is built and run.
    pub project_kind: ProjectKind,
    /// "Compila" / "Esegui" and the terminal output.
    pub runner: Runner,
    commands: Vec<Command>,
}

impl AppState {
    fn new(
        palette: Palette,
        fonts: FontChoice,
        log: LogBuffer,
        preview: PreviewManager,
        runner: Runner,
    ) -> Self {
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
            new_project: None,
            preview,
            designer: DesignerState::default(),
            project_kind: ProjectKind::Unsupported,
            runner,
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
    /// Watches the open project for changes made outside the editor.
    watcher: Option<FileWatcher>,
    /// Wakes the UI from background threads.
    waker: Arc<dyn Fn() + Send + Sync>,
    /// Scale factor the preview was rendered for.
    pixels_per_point: f32,
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
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker_ctx.request_repaint());
        let lsp = LspManager::new(waker.clone());
        let preview = PreviewManager::new(waker.clone());
        let runner = Runner::new(waker.clone());

        let mut app = Self {
            state: AppState::new(palette, fonts, log, preview, runner),
            dock: layout::default_dock_state(),
            dock_style,
            lsp,
            language_server: options.language_server,
            definition_request: None,
            allow_close: false,
            watcher: None,
            waker,
            pixels_per_point: ctx.pixels_per_point(),
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
                KeyboardShortcut::new(command_shift, Key::N),
                Command::NewProject,
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
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::B),
                Command::Build,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::R),
                Command::Run,
            ),
            (
                KeyboardShortcut::new(Modifiers::SHIFT, Key::F5),
                Command::StopRun,
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
                if self.state.live_preview {
                    self.state.preview.schedule(Duration::ZERO);
                }
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

            Command::NewProject => {
                self.state.new_project = Some(NewProjectForm::new(&self.new_project_location()));
            }
            Command::BrowseNewProjectLocation => {
                if let Some(form) = &mut self.state.new_project {
                    let folder = rfd::FileDialog::new()
                        .set_title("Cartella in cui creare il progetto")
                        .set_directory(form.location.trim())
                        .pick_folder();
                    if let Some(folder) = folder {
                        form.location = folder.display().to_string();
                        form.error = None;
                    }
                }
            }
            Command::CreateProject => self.create_project(ctx),
            Command::CancelNewProject => self.state.new_project = None,
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
                self.update_project_kind();
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
            Command::DocumentEdited(path) => self.document_changed(&path, EDIT_DEBOUNCE),

            Command::SelectElement(element) => self.select_element(element),
            Command::SetProperty {
                element,
                name,
                value,
            } => {
                self.edit_element(&element, |text, node| {
                    edit::set_binding(text, node, &name, value.as_deref()).map(|e| (e, None))
                });
            }
            Command::InsertComponent { item, parent } => self.insert_component(item, parent),
            Command::ResizeElement {
                element,
                x,
                y,
                width,
                height,
            } => self.resize_element(
                &element,
                [("x", x), ("y", y), ("width", width), ("height", height)],
            ),
            Command::RemoveElement(element) => {
                if self.edit_element(&element, |text, node| {
                    Some((edit::remove_element(text, node), None))
                }) {
                    self.state.designer.selected = None;
                }
            }

            Command::RequestCompletion { path, char_index } => {
                self.request_completion(&path, char_index)
            }
            Command::RequestDefinition { path, char_index } => {
                self.request_definition(&path, char_index)
            }
            Command::RestartLsp => {
                self.state.diagnostics.clear_source(DiagnosticSource::Lsp);
                self.lsp.restart();
            }
            Command::RefreshPreview => {
                self.retarget_preview();
                self.render_preview(ctx);
            }

            Command::Build => self.start_job(JobKind::Build),
            Command::Run => self.start_job(JobKind::Run),
            Command::StopRun => self.state.runner.stop(),
            Command::ClearTerminal => self.state.runner.clear(),

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

    /// Where new projects go by default: next to the most recent project,
    /// else in the home folder.
    fn new_project_location(&self) -> PathBuf {
        self.state
            .workspace
            .recent
            .paths()
            .iter()
            .filter_map(|p| p.parent())
            .find(|p| p.is_dir())
            .map(Path::to_path_buf)
            .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn create_project(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.state.new_project else {
            return;
        };
        let parent = PathBuf::from(form.location.trim());
        match slint_studio_core::template::create(&parent, &form.name, form.template) {
            Ok(dir) => {
                info!(
                    "Progetto creato: {} ({})",
                    dir.display(),
                    form.template.label()
                );
                self.state.new_project = None;
                self.guard(ctx, GuardedAction::OpenProject(dir));
            }
            Err(err) => {
                error!("Impossibile creare il progetto: {err}");
                form.error = Some(err.to_string());
            }
        }
    }

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
        self.update_project_kind();
        info!(
            "Tipo di progetto: {} (Compila: {})",
            self.state.project_kind.label(),
            self.state.project_kind.build_description()
        );
        layout::open(&mut self.dock, Tab::Project);
        if self.language_server {
            self.lsp.start(&root);
        }
        if let Some(project) = &self.state.workspace.project {
            match FileWatcher::new(&project.tree.root, self.waker.clone()) {
                Ok(watcher) => {
                    debug!("{} cartelle osservate", watcher.watched_count());
                    self.watcher = Some(watcher);
                }
                Err(err) => error!("Impossibile osservare le modifiche ai file: {err}"),
            }
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
        self.watcher = None;
        self.state.runner.stop();
        self.state.project_kind = ProjectKind::Unsupported;
        self.state.preview.clear();
        self.state.designer = DesignerState::default();
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
            self.state
                .diagnostics
                .set(DiagnosticSource::Lsp, path.to_path_buf(), Vec::new());
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

    // --- Designer -------------------------------------------------------------

    /// After any change to a document's text: inform the language server and
    /// re-render the preview after `delay`.
    fn document_changed(&mut self, path: &Path, delay: Duration) {
        // Build diagnostics point into the old text: they return with the
        // next build.
        self.state
            .diagnostics
            .set(DiagnosticSource::Compiler, path.to_path_buf(), Vec::new());
        if let (Some(client), Some(open)) = (self.lsp.client(), self.state.workspace.document(path))
            && open.doc.language() == Language::Slint
        {
            client.did_change(path, open.doc.text(), open.doc.version());
        }
        if self.state.live_preview && is_slint(path) {
            self.state.preview.schedule(delay);
        }
    }

    /// Selects an element and moves the editor cursor to its declaration.
    fn select_element(&mut self, element: Option<ElementRef>) {
        let Some(element) = element else {
            self.state.designer.selected = None;
            return;
        };
        if !self.open_file(&element.path) {
            return;
        }
        if let Some(open) = self.state.workspace.document_mut(&element.path) {
            let outline = open.outline();
            if let Some(index) = outline.by_tree_path(&element.tree_path) {
                let char_index = byte_to_char(open.doc.text(), outline.elements[index].key_offset);
                open.editor.reveal = Some(char_index);
                open.editor.cursor = char_index;
                self.state.designer.synced_cursor = Some((element.path.clone(), char_index));
            }
        }
        self.state.designer.selected = Some(element);
    }

    /// The selection follows the editor cursor in the previewed file.
    fn sync_selection_with_cursor(&mut self) {
        let Some(open) = self.state.workspace.active_document() else {
            return;
        };
        if open.doc.language() != Language::Slint || !self.state.preview.previews(open.doc.path()) {
            return;
        }
        let key = (open.doc.path().to_path_buf(), open.editor.cursor);
        if self.state.designer.synced_cursor.as_ref() == Some(&key) {
            return;
        }
        let outline = open.outline();
        let byte = char_to_byte(open.doc.text(), open.editor.cursor);
        self.state.designer.selected = outline.element_at(byte).map(|index| ElementRef {
            path: key.0.clone(),
            tree_path: outline.tree_path(index),
        });
        self.state.designer.synced_cursor = Some(key);
    }

    /// Applies an edit computed from `element`'s current source. The edit
    /// may name an offset (in the new text) of an element to select.
    fn edit_element(
        &mut self,
        element: &ElementRef,
        make: impl FnOnce(&str, &ElementNode) -> Option<(SourceEdit, Option<usize>)>,
    ) -> bool {
        let Some(open) = self.state.workspace.document(&element.path) else {
            warn!("Il file {} non è aperto", element.path.display());
            return false;
        };
        let outline = open.outline();
        let Some(index) = outline.by_tree_path(&element.tree_path) else {
            warn!("Elemento non trovato nel codice: aggiorna l'anteprima e riprova");
            return false;
        };
        let Some((change, select)) = make(open.doc.text(), &outline.elements[index]) else {
            return false;
        };
        let text = change.apply(open.doc.text());
        self.replace_text(&element.path, text, select);
        true
    }

    /// Replaces a document's text (one undoable change in the editor) and
    /// optionally selects the element starting at `select`.
    fn replace_text(&mut self, path: &Path, text: String, select: Option<usize>) {
        let Some(open) = self.state.workspace.document_mut(path) else {
            return;
        };
        open.doc.set_text(text);
        if let Some(offset) = select {
            let outline = open.outline();
            if let Some(index) = outline.element_at(offset) {
                let element = ElementRef {
                    path: path.to_path_buf(),
                    tree_path: outline.tree_path(index),
                };
                self.select_element(Some(element));
            }
        }
        // Designer edits are deliberate: show them almost immediately.
        self.document_changed(path, Duration::from_millis(30));
    }

    /// Where palette items go when nothing is selected: the root of the
    /// previewed component (the last one in the file).
    fn default_insertion_target(&self) -> Option<ElementRef> {
        let path = self.state.preview.target.clone()?;
        let open = self.state.workspace.document(&path)?;
        let outline = open.outline();
        let root = outline.roots().last()?;
        Some(ElementRef {
            path,
            tree_path: outline.tree_path(root),
        })
    }

    fn insert_component(&mut self, item: &'static str, parent: Option<ElementRef>) {
        let Some(palette) = PaletteItem::all().iter().find(|p| p.name == item) else {
            return;
        };
        let target = parent
            .or_else(|| self.state.designer.selected.clone())
            .or_else(|| self.default_insertion_target());
        let Some(target) = target else {
            info!("Apri un file .slint per inserire componenti");
            return;
        };
        let Some(open) = self.state.workspace.document(&target.path) else {
            return;
        };
        let outline = open.outline();
        let Some(mut index) = outline.by_tree_path(&target.tree_path) else {
            return;
        };
        // Leaf elements (Text, Button, …) cannot have children: use the parent.
        while !properties::accepts_children(&outline.elements[index].type_name) {
            match outline.elements[index].parent {
                Some(parent) => index = parent,
                None => break,
            }
        }
        let container = &outline.elements[index];
        let text = open.doc.text();
        let Some((insert, mut start)) = edit::insert_child(text, container, palette.snippet) else {
            warn!(
                "Impossibile inserire in {}: elemento senza parentesi di chiusura",
                container.label()
            );
            return;
        };
        let mut new_text = insert.apply(text);
        if let Some(module) = palette.import_from()
            && let Some(import) = edit::ensure_import(&new_text, palette.name, module)
        {
            if import.range.start <= start {
                start = start + import.replacement.len() - import.range.len();
            }
            new_text = import.apply(&new_text);
        }
        info!("Inserito {} in {}", palette.name, container.label());
        let path = target.path.clone();
        self.replace_text(&path, new_text, Some(start));
    }

    /// Writes geometry bindings (`{value}px`) as a single change.
    fn resize_element(&mut self, element: &ElementRef, values: [(&str, Option<i32>); 4]) {
        let Some(open) = self.state.workspace.document(&element.path) else {
            return;
        };
        let mut text = open.doc.text().to_owned();
        for (name, value) in values {
            let Some(value) = value else {
                continue;
            };
            let outline = slint_studio_core::Outline::parse(&text);
            let Some(index) = outline.by_tree_path(&element.tree_path) else {
                return;
            };
            if let Some(change) = edit::set_binding(
                &text,
                &outline.elements[index],
                name,
                Some(&format!("{value}px")),
            ) {
                text = change.apply(&text);
            }
        }
        if text != open.doc.text() {
            self.replace_text(&element.path.clone(), text, None);
        }
    }

    // --- Live preview and file watching ---------------------------------------

    /// Previews the active document when it is a `.slint` file.
    fn retarget_preview(&mut self) {
        let Some(open) = self.state.workspace.active_document() else {
            return;
        };
        if open.doc.language() != Language::Slint || self.state.preview.previews(open.doc.path()) {
            return;
        }
        self.state.preview.target = Some(open.doc.path().to_path_buf());
        if self.state.live_preview {
            self.state.preview.schedule(Duration::ZERO);
        }
    }

    /// Sends the preview request, with the unsaved buffers.
    fn render_preview(&mut self, ctx: &egui::Context) {
        let overrides = self
            .state
            .workspace
            .documents
            .iter()
            .filter(|d| d.doc.is_dirty() && d.doc.language() == Language::Slint)
            .map(|d| (d.doc.path().to_path_buf(), d.doc.text().to_owned()))
            .collect();
        self.pixels_per_point = ctx.pixels_per_point();
        self.state
            .preview
            .render_now(overrides, self.pixels_per_point);
    }

    /// Applies finished renders and starts due ones (debounced).
    fn update_preview(&mut self, ctx: &egui::Context) {
        self.state.preview.poll(ctx);
        self.retarget_preview();
        let moved_to_other_screen = (ctx.pixels_per_point() - self.pixels_per_point).abs() > 0.01;
        if moved_to_other_screen && self.state.preview.image.is_some() {
            self.state.preview.schedule(Duration::ZERO);
        }
        if self.state.preview.is_due() {
            self.render_preview(ctx);
        } else if let Some(wait) = self.state.preview.time_until_due() {
            ctx.request_repaint_after(wait);
        }
    }

    /// Reacts to changes made outside the editor: refreshes the tree,
    /// reloads unmodified documents and updates the preview.
    fn process_file_changes(&mut self) {
        let Some(changes) = self.watcher.as_ref().map(FileWatcher::poll) else {
            return;
        };
        if changes.is_empty() {
            return;
        }
        let mut preview_needed = false;
        if !changes.structure.is_empty() {
            if let Some(project) = &mut self.state.workspace.project {
                match project.refresh() {
                    Ok(()) => {
                        if let Some(watcher) = &mut self.watcher {
                            watcher.sync(&project.tree.root);
                        }
                    }
                    Err(err) => error!("Impossibile aggiornare l'albero dei file: {err}"),
                }
            }
            self.update_project_kind();
            preview_needed |= changes.structure.iter().any(|p| is_slint(p));
        }
        for path in &changes.modified {
            let Some(open) = self.state.workspace.document_mut(path) else {
                // Not open: may be imported by the previewed file.
                preview_needed |= is_slint(path);
                continue;
            };
            if open.doc.is_dirty() {
                warn!(
                    "{} è stato modificato fuori dall'editor: le modifiche non salvate sono state mantenute",
                    open.doc.file_name()
                );
                continue;
            }
            match open.doc.reload() {
                Ok(true) => {
                    info!(
                        "{} ricaricato: modificato fuori dall'editor",
                        open.doc.file_name()
                    );
                    if open.doc.language() == Language::Slint {
                        preview_needed = true;
                        if let Some(client) = self.lsp.client() {
                            client.did_change(path, open.doc.text(), open.doc.version());
                        }
                    }
                }
                Ok(false) => {} // e.g. our own save
                Err(err) => warn!("Impossibile ricaricare {}: {err}", path.display()),
            }
        }
        if preview_needed && self.state.live_preview {
            self.state.preview.schedule(EDIT_DEBOUNCE);
        }
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

    // --- Build & run ---------------------------------------------------------

    fn slint_files(&self) -> Vec<PathBuf> {
        let Some(project) = &self.state.workspace.project else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = project
            .tree
            .root
            .files()
            .into_iter()
            .filter(|node| node.is_slint())
            .map(|node| node.path.clone())
            .collect();
        files.sort();
        files
    }

    fn update_project_kind(&mut self) {
        self.state.project_kind = match &self.state.workspace.project {
            Some(project) => {
                let has_slint = project.tree.root.files().iter().any(|f| f.is_slint());
                ProjectKind::detect(&project.root, has_slint)
            }
            None => ProjectKind::Unsupported,
        };
    }

    /// "Compila" / "Esegui": saves every file (like Qt Creator), forgets the
    /// previous build's diagnostics and starts the job.
    fn start_job(&mut self, kind: JobKind) {
        let Some(project) = &self.state.workspace.project else {
            warn!("{}: nessun progetto aperto", kind.label());
            return;
        };
        let root = project.root.clone();
        let entry = project.entry_point();
        self.update_project_kind();
        if self.state.workspace.has_unsaved_changes() {
            if !self.save_all() {
                error!(
                    "{} annullata: non è stato possibile salvare tutti i file",
                    kind.label()
                );
                return;
            }
            info!(
                "File modificati salvati prima della {}",
                kind.label().to_lowercase()
            );
        }
        self.state
            .diagnostics
            .clear_source(DiagnosticSource::Compiler);
        let request = JobRequest {
            kind,
            project: self.state.project_kind.clone(),
            root,
            slint_files: self.slint_files(),
            run_target: self.state.preview.target.clone().or(entry),
            style: slint_studio_preview::DEFAULT_STYLE.to_owned(),
        };
        info!(
            "{} del progetto ({})",
            kind.label(),
            match kind {
                JobKind::Build => request.project.build_description(),
                JobKind::Run => request.project.run_description(),
            }
        );
        self.state.runner.start(request);
        layout::open(&mut self.dock, Tab::Terminal);
    }

    fn process_runner_updates(&mut self) {
        for update in self.state.runner.poll() {
            match update {
                RunnerUpdate::Diagnostic(diagnostic) => {
                    self.state
                        .diagnostics
                        .add(DiagnosticSource::Compiler, diagnostic);
                }
                // Like Qt Creator's "Issues": errors are where to go next.
                RunnerUpdate::Finished(_, JobOutcome::BuildFailed { errors }) if errors > 0 => {
                    layout::open(&mut self.dock, Tab::Problems);
                }
                RunnerUpdate::Finished(..) => {}
            }
        }
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
                    self.state
                        .diagnostics
                        .set(DiagnosticSource::Lsp, path, converted);
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
        self.process_runner_updates();
        self.handle_shortcuts(&ctx);
        self.guard_window_close(&ctx);
        self.process_file_changes();
        self.update_preview(&ctx);
        self.sync_selection_with_cursor();
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
        ui::new_project::show(&ctx, &mut self.state);
        ui::dialogs::unsaved_changes(&ctx, &mut self.state);

        self.run_commands(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designer::ElementRef;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use egui_kittest::{Harness, Node};
    use egui_phosphor::regular as icon;
    use slint_studio_core::{Diagnostic, Location, Severity};
    use std::fs;

    /// Runs frames (in real time) until no preview render is pending or in
    /// flight: opening a project renders in the background.
    fn settle(harness: &mut Harness<'_, StudioApp>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        // Commands run at the end of a frame and the preview reacts on the
        // next one, so require a few idle frames in a row.
        let mut idle_frames = 0;
        while idle_frames < 3 {
            harness.step();
            let preview = &harness.state().state.preview;
            if !preview.is_rendering() && preview.time_until_due().is_none() {
                idle_frames += 1;
            } else {
                idle_frames = 0;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the preview did not settle"
            );
        }
    }

    fn harness_with(options: StartupOptions) -> Harness<'static, StudioApp> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .build_eframe(move |cc| {
                StudioApp::new(cc, LogBuffer::default(), RepaintSignal::default(), options)
            });
        settle(&mut harness);
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
        settle(&mut harness);
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
        settle(harness);
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
            settle(&mut harness);
            assert_eq!(harness.state().state.view_mode, mode);
        }
    }

    #[test]
    fn toolbar_toggles_live_preview() {
        let mut harness = harness();
        assert!(harness.state().state.live_preview);
        button(&harness, &label(icon::EYE, "Anteprima live")).click();
        settle(&mut harness);
        assert!(!harness.state().state.live_preview);
        button(&harness, &label(icon::EYE_SLASH, "Anteprima live")).click();
        settle(&mut harness);
        assert!(harness.state().state.live_preview);
    }

    fn is_disabled(harness: &Harness<'_, StudioApp>, name: &str) -> bool {
        button(harness, name).accesskit_node().is_disabled()
    }

    /// Whether `tab` is the visible tab of its group.
    fn is_active(dock: &DockState<Tab>, tab: Tab) -> bool {
        dock.find_tab(&tab).is_some_and(|path| {
            dock.leaf(path.node_path())
                .is_ok_and(|leaf| leaf.active == path.tab)
        })
    }

    /// Runs frames until "Compila" / "Esegui" has finished.
    fn wait_for_job(harness: &mut Harness<'_, StudioApp>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while harness.state().state.runner.is_running() {
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert!(std::time::Instant::now() < deadline, "the job did not end");
        }
        settle(harness);
    }

    fn terminal_text(harness: &Harness<'_, StudioApp>) -> String {
        harness.state().state.runner.terminal.plain_text()
    }

    /// Opens the "Nuovo progetto" dialog with Ctrl+Shift+N and fills it in.
    fn fill_new_project(harness: &mut Harness<'_, StudioApp>, parent: &Path, name: &str) {
        harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
        settle(harness);
        let form = harness.state_mut().state.new_project.as_mut().unwrap();
        form.location = parent.display().to_string();
        form.name = name.to_owned();
        settle(harness);
    }

    #[test]
    fn new_project_creates_and_opens_a_rust_project() {
        let parent = tempfile::tempdir().unwrap();
        let mut harness = harness();
        button(&harness, &label(icon::FOLDER_PLUS, "Nuovo progetto…"));
        fill_new_project(&mut harness, parent.path(), "La mia app");
        harness.get_by_label("Nome del pacchetto Cargo: la-mia-app");
        button(&harness, &label(icon::CHECK, "Crea")).click();
        settle(&mut harness);

        let state = &harness.state().state;
        assert!(state.new_project.is_none(), "the dialog closes");
        let project = state.workspace.project.as_ref().expect("project opened");
        let root = parent.path().canonicalize().unwrap().join("La mia app");
        assert_eq!(project.root, root);
        assert_eq!(
            state.project_kind,
            ProjectKind::Cargo {
                manifest: root.join("Cargo.toml")
            }
        );
        assert!(root.join("src/main.rs").is_file());
        let active = state.workspace.active_document().unwrap();
        assert_eq!(active.doc.path(), root.join("ui/app-window.slint"));
        let preview = harness.state().state.preview.image.as_ref();
        assert!(preview.is_some(), "the new project is previewed");
    }

    #[test]
    fn new_project_slint_only_and_validation() {
        let parent = tempfile::tempdir().unwrap();
        fs::create_dir_all(parent.path().join("occupata")).unwrap();
        fs::write(parent.path().join("occupata/file.txt"), "x").unwrap();
        let mut harness = harness();
        fill_new_project(&mut harness, parent.path(), "occupata");
        let create = label(icon::CHECK, "Crea");
        assert!(is_disabled(&harness, &create), "folder not empty");
        harness.get_by_label_contains("esiste già e non è vuota");

        harness.state_mut().state.new_project.as_mut().unwrap().name = "a/b".into();
        settle(&mut harness);
        assert!(is_disabled(&harness, &create));
        harness.get_by_label("il nome non può contenere «/»");

        harness
            .get_by_role_and_label(Role::RadioButton, "Solo Slint")
            .click();
        harness.state_mut().state.new_project.as_mut().unwrap().name = "bozza".into();
        settle(&mut harness);
        button(&harness, &create).click();
        settle(&mut harness);
        let state = &harness.state().state;
        assert_eq!(state.project_kind, ProjectKind::Slint);
        assert!(parent.path().join("bozza/ui/app-window.slint").is_file());
        assert!(!parent.path().join("bozza/Cargo.toml").exists());

        // Annulla closes the dialog without creating anything.
        fill_new_project(&mut harness, parent.path(), "mai-creato");
        button(&harness, "Annulla").click();
        settle(&mut harness);
        assert!(harness.state().state.new_project.is_none());
        assert!(!parent.path().join("mai-creato").exists());
    }

    #[test]
    fn run_actions_need_a_supported_project() {
        let run = label(icon::PLAY, "Esegui");
        let build = label(icon::HAMMER, "Compila");
        let stop = label(icon::STOP, "Interrompi");
        let harness = harness();
        for name in [&run, &build, &stop] {
            assert!(is_disabled(&harness, name), "{name} should be disabled");
        }

        let dir = project_dir();
        let harness = with_project(&dir);
        assert_eq!(harness.state().state.project_kind, ProjectKind::Slint);
        assert!(!is_disabled(&harness, &run));
        assert!(!is_disabled(&harness, &build));
        assert!(is_disabled(&harness, &stop), "nothing is running");

        let empty = tempfile::tempdir().unwrap();
        fs::write(empty.path().join("note.txt"), "x").unwrap();
        let harness = with_project(&empty);
        assert_eq!(harness.state().state.project_kind, ProjectKind::Unsupported);
        assert!(is_disabled(&harness, &run));
    }

    #[test]
    fn cargo_projects_are_detected() {
        let dir = project_dir();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
        let harness = with_project(&dir);
        let root = harness
            .state()
            .state
            .workspace
            .project
            .as_ref()
            .unwrap()
            .root
            .clone();
        assert_eq!(
            harness.state().state.project_kind,
            ProjectKind::Cargo {
                manifest: root.join("Cargo.toml")
            }
        );
    }

    #[test]
    fn build_saves_checks_slint_files_and_shows_errors() {
        let dir = project_dir();
        fs::write(
            dir.path().join("ui/card.slint"),
            "export component Card inherits Rectangle { Buton {} }\n",
        )
        .unwrap();
        let mut harness = with_project(&dir);
        let main = harness
            .state()
            .state
            .workspace
            .active_document()
            .unwrap()
            .doc
            .path()
            .to_path_buf();
        let edited =
            "export component Main inherits Window {\n    Text { text: \"Salvato\"; }\n}\n";
        {
            let doc = &mut harness
                .state_mut()
                .state
                .workspace
                .document_mut(&main)
                .unwrap()
                .doc;
            doc.set_text(edited.to_owned());
        }
        send(&mut harness, Command::DocumentEdited(main.clone()));

        button(&harness, &label(icon::HAMMER, "Compila")).click();
        harness.step();
        assert!(is_active(&harness.state().dock, Tab::Terminal));
        wait_for_job(&mut harness);

        let state = &harness.state().state;
        assert!(
            !state.workspace.has_unsaved_changes(),
            "saved before building"
        );
        assert_eq!(fs::read_to_string(&main).unwrap(), edited);
        let text = terminal_text(&harness);
        assert!(text.contains("Controllo di 2 file .slint"), "{text}");
        assert!(text.contains("errore: Unknown element 'Buton'"), "{text}");
        assert!(state.runner.status.is_failure());

        let card = main.parent().unwrap().join("card.slint");
        let errors: Vec<_> = state
            .diagnostics
            .all()
            .into_iter()
            .filter(|d| d.source == DiagnosticSource::Compiler)
            .cloned()
            .collect();
        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert_eq!(errors[0].location.as_ref().unwrap().path, card);
        assert!(
            is_active(&harness.state().dock, Tab::Problems),
            "build errors open the Errori panel"
        );

        // Editing the file makes its build diagnostics stale.
        send(&mut harness, Command::OpenFile(card.clone()));
        {
            let doc = &mut harness
                .state_mut()
                .state
                .workspace
                .document_mut(&card)
                .unwrap()
                .doc;
            doc.set_text("export component Card inherits Rectangle { }\n".to_owned());
        }
        send(&mut harness, Command::DocumentEdited(card.clone()));
        assert!(harness.state().state.diagnostics.for_path(&card).is_empty());

        // Ctrl+B builds again: everything is fine now.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::B);
        harness.step();
        wait_for_job(&mut harness);
        let state = &harness.state().state;
        assert!(!state.runner.status.is_failure());
        assert_eq!(state.diagnostics.counts().errors, 0);
        assert!(terminal_text(&harness).contains("■ Compilazione riuscita"));
        assert_eq!(
            fs::read_to_string(&card).unwrap(),
            "export component Card inherits Rectangle { }\n"
        );
    }

    #[test]
    fn run_does_not_start_when_the_previewed_file_has_errors() {
        let dir = project_dir();
        fs::write(
            dir.path().join("ui/main.slint"),
            "export component Main inherits Window { Buton {} }\n",
        )
        .unwrap();
        let mut harness = with_project(&dir);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
        harness.step();
        wait_for_job(&mut harness);
        let text = terminal_text(&harness);
        assert!(text.contains("▶ Controllo di ui/main.slint"), "{text}");
        assert!(
            !text.contains("slint-viewer"),
            "the viewer must not start: {text}"
        );
        assert_eq!(harness.state().state.diagnostics.counts().errors, 1);
        assert!(harness.state().state.runner.status.is_failure());
    }

    #[test]
    fn terminal_panel_can_be_cleared() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        send(&mut harness, Command::Build);
        wait_for_job(&mut harness);
        assert!(!harness.state().state.runner.terminal.is_empty());
        harness
            .get_by_role_and_label(Role::Button, &label(icon::BROOM, "Pulisci"))
            .click();
        settle(&mut harness);
        assert!(harness.state().state.runner.terminal.is_empty());
        assert_eq!(
            harness.state().state.runner.status,
            crate::runner::RunnerStatus::Idle
        );
        harness.get_by_label("Nessun output");
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
            DiagnosticSource::Lsp,
            card.clone(),
            vec![Diagnostic {
                severity: Severity::Error,
                source: DiagnosticSource::Lsp,
                message: "proprietà sconosciuta 'colour'".into(),
                location: Some(Location::at(&card, LineCol::new(0, 10))),
            }],
        );
        layout::open(&mut harness.state_mut().dock, Tab::Problems);
        settle(&mut harness);

        harness
            .get_by_label("proprietà sconosciuta 'colour'")
            .click();
        settle(&mut harness);
        let state = &harness.state().state;
        assert_eq!(state.diagnostics.counts().errors, 1);
        assert_eq!(state.workspace.active_document().unwrap().doc.path(), card);
    }

    #[test]
    fn editing_marks_dirty_and_ctrl_s_writes_the_file() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        harness.get_by_role(Role::MultilineTextInput).click();
        settle(&mut harness);
        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("// nota");
        settle(&mut harness);
        let path = {
            let open = harness.state().state.workspace.active_document().unwrap();
            assert!(open.doc.is_dirty());
            assert!(open.doc.text().contains("// nota"));
            open.doc.path().to_path_buf()
        };

        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        settle(&mut harness);
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
    fn tab_indents_with_spaces_and_shift_tab_unindents() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let text = |h: &Harness<'_, StudioApp>| {
            h.state()
                .state
                .workspace
                .active_document()
                .unwrap()
                .doc
                .text()
                .to_owned()
        };
        let original = text(&harness);
        harness.get_by_role(Role::MultilineTextInput).click();
        settle(&mut harness);

        harness.key_press(Key::Tab);
        settle(&mut harness);
        let indented = text(&harness);
        assert_eq!(indented.len(), original.len() + 4);
        assert!(!indented.contains('\t'));

        harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        settle(&mut harness);
        assert_eq!(text(&harness).len(), original.len());
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
        settle(&mut harness);
        assert!(harness.state().state.unsaved.is_none());
        assert!(harness.state().state.workspace.document(&path).is_some());

        send(&mut harness, Command::CloseDocument(path.clone()));
        harness.get_by_label("Non salvare").click();
        settle(&mut harness);
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
        settle(&mut harness);
        let open = harness.state().state.workspace.active_document().unwrap();
        assert!(open.doc.path().ends_with("ui/card.slint"));
        assert_eq!(open.editor.cursor, 10, "cursor moved to the match");
    }

    #[test]
    fn opening_a_project_renders_the_live_preview() {
        let dir = project_dir();
        let harness = with_project(&dir);
        let preview = &harness.state().state.preview;
        let image = preview
            .image
            .as_ref()
            .expect("the entry point is previewed");
        assert_eq!(image.component, "Main");
        assert!(image.logical_size.x > 0.0 && image.logical_size.y > 0.0);
        assert!(preview.failure.is_none());
    }

    #[test]
    fn typing_an_error_shows_it_over_the_preview_without_saving() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let path = {
            let open = harness
                .state_mut()
                .state
                .workspace
                .active_document_mut()
                .unwrap();
            open.doc
                .set_text("export component Main inherits Window { colour: red; }".into());
            open.doc.path().to_path_buf()
        };
        send(&mut harness, Command::DocumentEdited(path.clone()));
        let failure = harness
            .state()
            .state
            .preview
            .failure
            .clone()
            .expect("compile error shown");
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|d| d.message.contains("colour"))
        );
        assert!(
            harness.state().state.preview.image.is_some(),
            "the last good image is kept"
        );
        harness.get_by_label_contains("Unknown property colour");

        // Fixing the buffer (still unsaved) updates the preview again.
        harness
            .state_mut()
            .state
            .workspace
            .active_document_mut()
            .unwrap()
            .doc
            .set_text(
                "export component Main inherits Window { width: 50px; height: 20px; }".into(),
            );
        send(&mut harness, Command::DocumentEdited(path.clone()));
        let preview = &harness.state().state.preview;
        assert!(preview.failure.is_none());
        assert_eq!(
            preview.image.as_ref().unwrap().logical_size,
            egui::vec2(50.0, 20.0)
        );
        assert_ne!(
            fs::read_to_string(&path).unwrap(),
            "export component Main inherits Window { width: 50px; height: 20px; }"
        );
    }

    #[test]
    fn external_changes_reload_unmodified_documents() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let path = harness
            .state()
            .state
            .workspace
            .active_document()
            .unwrap()
            .doc
            .path()
            .to_path_buf();
        let changed = "export component Main inherits Window { width: 30px; height: 30px; }\n";
        fs::write(&path, changed).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while harness
            .state()
            .state
            .workspace
            .document(&path)
            .unwrap()
            .doc
            .text()
            != changed
        {
            assert!(
                std::time::Instant::now() < deadline,
                "external change not picked up"
            );
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        settle(&mut harness);
        let state = &harness.state().state;
        assert!(!state.workspace.document(&path).unwrap().doc.is_dirty());
        assert_eq!(
            state.preview.image.as_ref().unwrap().logical_size,
            egui::vec2(30.0, 30.0)
        );
    }

    fn element(harness: &Harness<'_, StudioApp>, tree_path: &[usize]) -> ElementRef {
        let path = harness
            .state()
            .state
            .workspace
            .active_document()
            .unwrap()
            .doc
            .path()
            .to_path_buf();
        ElementRef {
            path,
            tree_path: tree_path.to_vec(),
        }
    }

    fn active_text(harness: &Harness<'_, StudioApp>) -> String {
        harness
            .state()
            .state
            .workspace
            .active_document()
            .unwrap()
            .doc
            .text()
            .to_owned()
    }

    fn rendered(
        harness: &Harness<'_, StudioApp>,
        tree_path: &[usize],
    ) -> slint_studio_preview::ElementRect {
        let image = harness
            .state()
            .state
            .preview
            .image
            .as_ref()
            .expect("preview");
        image
            .elements
            .iter()
            .find(|g| g.tree_path == tree_path)
            .unwrap_or_else(|| panic!("no geometry for {tree_path:?}"))
            .rects[0]
    }

    #[test]
    fn the_preview_carries_element_geometry() {
        let dir = project_dir();
        let harness = with_project(&dir);
        let root = rendered(&harness, &[0]);
        let text = rendered(&harness, &[0, 0]);
        assert!(root.width >= text.width && root.height >= text.height);
        assert!(text.width > 0.0);
    }

    #[test]
    fn selecting_from_the_canvas_moves_the_editor_cursor_and_vice_versa() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let text_element = element(&harness, &[0, 0]);
        send(
            &mut harness,
            Command::SelectElement(Some(text_element.clone())),
        );
        let state = &harness.state().state;
        assert_eq!(state.designer.selected.as_ref(), Some(&text_element));
        let open = state.workspace.active_document().unwrap();
        let cursor_text: String = open
            .doc
            .text()
            .chars()
            .skip(open.editor.cursor)
            .take(4)
            .collect();
        assert_eq!(cursor_text, "Text", "cursor on the element's declaration");

        // Moving the editor cursor to the root selects the root.
        editor::set_cursor(&harness.ctx, editor::text_edit_id(&text_element.path), 2);
        settle(&mut harness);
        assert_eq!(
            harness.state().state.designer.selected,
            Some(element(&harness, &[0]))
        );
    }

    #[test]
    fn editing_a_property_rewrites_the_source_and_the_preview() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let root = element(&harness, &[0]);
        for (name, value) in [("width", "240px"), ("height", "120px")] {
            send(
                &mut harness,
                Command::SetProperty {
                    element: root.clone(),
                    name: name.into(),
                    value: Some(value.into()),
                },
            );
        }
        let text = active_text(&harness);
        assert!(
            text.contains("    width: 240px;\n    height: 120px;\n"),
            "{text}"
        );
        let rect = rendered(&harness, &[0]);
        assert_eq!((rect.width, rect.height), (240.0, 120.0));

        send(
            &mut harness,
            Command::SetProperty {
                element: root,
                name: "height".into(),
                value: None,
            },
        );
        assert!(!active_text(&harness).contains("height"));
    }

    #[test]
    fn palette_insertion_adds_the_import_and_selects_the_new_element() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        send(
            &mut harness,
            Command::InsertComponent {
                item: "Button",
                parent: None,
            },
        );
        let text = active_text(&harness);
        assert!(
            text.starts_with("import { Button } from \"std-widgets.slint\";"),
            "{text}"
        );
        assert!(
            text.contains("    Button {\n        text: \"Pulsante\";\n    }\n}"),
            "{text}"
        );
        assert_eq!(
            harness.state().state.designer.selected,
            Some(element(&harness, &[0, 1]))
        );
        assert!(
            harness.state().state.preview.failure.is_none(),
            "the inserted code compiles"
        );
        rendered(&harness, &[0, 1]);

        // Inserting "into" a Text (which has no children) goes to its parent.
        let text_element = element(&harness, &[0, 0]);
        send(
            &mut harness,
            Command::InsertComponent {
                item: "Rectangle",
                parent: Some(text_element),
            },
        );
        assert_eq!(
            harness.state().state.designer.selected,
            Some(element(&harness, &[0, 2]))
        );
    }

    #[test]
    fn resize_and_remove_update_the_source() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let text_element = element(&harness, &[0, 0]);
        send(
            &mut harness,
            Command::ResizeElement {
                element: text_element.clone(),
                x: Some(5),
                y: None,
                width: Some(80),
                height: Some(30),
            },
        );
        let text = active_text(&harness);
        assert!(
            text.contains("Text { text: \"Ciao\"; x: 5px; width: 80px; height: 30px; }"),
            "{text}"
        );
        let rect = rendered(&harness, &[0, 0]);
        assert_eq!((rect.x, rect.width, rect.height), (5.0, 80.0, 30.0));

        send(
            &mut harness,
            Command::SelectElement(Some(text_element.clone())),
        );
        send(&mut harness, Command::RemoveElement(text_element));
        assert!(!active_text(&harness).contains("Text"));
        assert!(harness.state().state.designer.selected.is_none());
    }

    #[test]
    fn the_property_panel_shows_the_selected_element() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let text_element = element(&harness, &[0, 0]);
        send(&mut harness, Command::SelectElement(Some(text_element)));
        harness.get_by_label_contains("Geometria");
        harness.get_by_label_contains("Aspetto");
        harness.get_by_label("font-size");
        // The text property is edited without quotes.
        assert!(harness.query_all_by_value("Ciao").count() >= 1);
    }

    #[test]
    fn clicking_a_palette_item_inserts_it() {
        let dir = project_dir();
        let mut harness = with_project(&dir);
        let label = label(egui_phosphor::regular::SHAPES, "Rectangle");
        harness.get_by_role_and_label(Role::Button, &label).click();
        settle(&mut harness);
        let text = active_text(&harness);
        assert!(
            text.contains("    Rectangle {\n        width: 100px;"),
            "{text}"
        );
    }
}
