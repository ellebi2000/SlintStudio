//! Code editor: document tabs, breadcrumb, line numbers, syntax highlighting,
//! diagnostics, completion popup and minimap.
//!
//! Built on egui's `TextEdit`. Positions are char indices (egui's cursor
//! unit); conversions to the language server's encoding happen in the app.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditOutput;
use egui::{
    Align, Align2, Atom, Button, Color32, Id, Key, Margin, Modifiers, Order, Rect, RichText,
    ScrollArea, Sense, Shape, Stroke, TextBuffer, TextEdit, TextStyle, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;
use slint_studio_core::text::{byte_to_char, char_to_byte};
use slint_studio_core::{Diagnostic, Language, PositionEncoding, Severity, syntax};
use slint_studio_lsp::RequestId;
use slint_studio_lsp::convert;
use slint_studio_lsp::lsp_types::{CompletionItem, CompletionItemKind};

use super::highlight::{self, CodeTheme};
use super::ico;
use crate::commands::Command;
use crate::theme::Palette;
use crate::workspace::{OpenDocument, Project, Workspace};

const INDENT: &str = "    ";
const MINIMAP_WIDTH: f32 = 90.0;
const MAX_VISIBLE_COMPLETIONS: usize = 200;

/// Per-document editor state that egui does not keep for us.
#[derive(Default)]
pub struct EditorState {
    /// Primary cursor (char index) as of the last frame.
    pub cursor: usize,
    /// Move the cursor here and scroll it into view on the next frame.
    pub reveal: Option<usize>,
    pub completion: Option<CompletionPopup>,
    /// Vertical scroll offset requested by the minimap.
    scroll_to: Option<f32>,
    minimap: Option<(i32, Vec<MiniLine>)>,
    breadcrumb: Option<((i32, usize), Vec<String>)>,
}

pub struct CompletionPopup {
    /// Pending request; `None` once results arrived.
    pub request: Option<RequestId>,
    /// Start (char index) of the word being completed.
    pub anchor: usize,
    pub items: Vec<CompletionEntry>,
    pub selected: usize,
}

impl CompletionPopup {
    pub fn pending(request: RequestId, anchor: usize) -> Self {
        Self {
            request: Some(request),
            anchor,
            items: Vec::new(),
            selected: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEntry {
    pub label: String,
    pub detail: Option<String>,
    pub insert: String,
    pub kind: Option<CompletionItemKind>,
    sort_key: String,
}

impl CompletionEntry {
    pub fn from_item(item: &CompletionItem) -> Self {
        Self {
            label: item.label.clone(),
            detail: item.detail.clone(),
            insert: convert::insert_text(item),
            kind: item.kind,
            sort_key: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
        }
    }
}

/// Entries matching `prefix` (case-insensitive): prefix matches first, then
/// substring matches, each in the server's sort order.
pub fn filter_completions<'a>(
    items: &'a [CompletionEntry],
    prefix: &str,
) -> Vec<&'a CompletionEntry> {
    let prefix = prefix.to_lowercase();
    let mut matches: Vec<(bool, &CompletionEntry)> = items
        .iter()
        .filter_map(|entry| {
            let label = entry.label.to_lowercase();
            if label.starts_with(&prefix) {
                Some((false, entry))
            } else if label.contains(&prefix) {
                Some((true, entry))
            } else {
                None
            }
        })
        .collect();
    matches.sort_by(|(a_sub, a), (b_sub, b)| {
        a_sub.cmp(b_sub).then_with(|| a.sort_key.cmp(&b.sort_key))
    });
    matches.into_iter().map(|(_, entry)| entry).collect()
}

pub fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Char index where the identifier ending at `cursor` starts.
pub fn word_start(text: &str, cursor: usize) -> usize {
    let before: Vec<char> = text.chars().take(cursor).collect();
    let word_len = before
        .iter()
        .rev()
        .take_while(|c| is_identifier_char(**c))
        .count();
    cursor - word_len
}

/// The characters `range` of `text`.
fn char_slice(text: &str, range: Range<usize>) -> &str {
    let start = char_to_byte(text, range.start);
    let end = char_to_byte(text, range.end.max(range.start));
    &text[start..end]
}

// ---------------------------------------------------------------------------
// Document tabs

pub fn document_tabs(
    ui: &mut Ui,
    workspace: &Workspace,
    palette: &Palette,
    commands: &mut Vec<Command>,
) {
    egui::Frame::NONE.fill(palette.panel).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ScrollArea::horizontal()
            .id_salt("document_tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (index, open) in workspace.documents.iter().enumerate() {
                        document_tab(
                            ui,
                            workspace,
                            open,
                            workspace.active == Some(index),
                            palette,
                            commands,
                        );
                    }
                });
            });
    });
}

fn document_tab(
    ui: &mut Ui,
    workspace: &Workspace,
    open: &OpenDocument,
    active: bool,
    palette: &Palette,
    commands: &mut Vec<Command>,
) {
    let path = open.doc.path();
    let fill = if active {
        palette.background
    } else {
        palette.panel
    };
    let frame = egui::Frame::NONE.fill(fill).inner_margin(Margin {
        left: 10,
        right: 4,
        top: 4,
        bottom: 4,
    });
    let response = frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let glyph = if open.doc.language() == Language::Slint {
            ico(icon::FILE_CODE).color(palette.accent_secondary)
        } else {
            ico(icon::FILE_TEXT).color(palette.text_weak)
        };
        let name_color = if active {
            Color32::WHITE
        } else {
            palette.text_weak
        };
        let label = Button::new((glyph, RichText::new(open.doc.file_name()).color(name_color)))
            .frame(false);
        let tooltip = workspace.project.as_ref().map_or_else(
            || path.display().to_string(),
            |p| p.relative(path).display().to_string(),
        );
        let response = ui.add(label).on_hover_text(tooltip);
        if response.clicked() {
            commands.push(Command::ActivateDocument(path.to_path_buf()));
        }
        if response.middle_clicked() {
            commands.push(Command::CloseDocument(path.to_path_buf()));
        }
        // A dot marks unsaved changes; the button always closes the tab.
        let (glyph, tip) = if open.doc.is_dirty() {
            (icon::CIRCLE, "Modifiche non salvate · chiudi")
        } else {
            (icon::X, "Chiudi")
        };
        let close = Button::new(ico(glyph).size(11.0).color(palette.text_weak)).frame(false);
        if ui.add(close).on_hover_text(tip).clicked() {
            commands.push(Command::CloseDocument(path.to_path_buf()));
        }
    });
    if active {
        let rect = response.response.rect;
        ui.painter().hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(2.0, palette.accent),
        );
    }
}

// ---------------------------------------------------------------------------
// Editor

/// Id of the `TextEdit` of the document at `path` (its egui state holds the
/// cursor and the undo history).
pub fn text_edit_id(path: &std::path::Path) -> Id {
    Id::new(("editor", path)).with("text")
}

pub fn show(
    ui: &mut Ui,
    document: &mut OpenDocument,
    diagnostics: &[Diagnostic],
    project: Option<&Project>,
    palette: &Palette,
    commands: &mut Vec<Command>,
) {
    let path = document.doc.path().to_path_buf();
    let id = Id::new(("editor", &path));
    let text_id = text_edit_id(&path);
    let theme = CodeTheme::dark(TextStyle::Monospace.resolve(ui.style()));
    let (row_height, char_width) = ui.fonts_mut(|fonts| {
        (
            fonts.row_height(&theme.font),
            fonts.glyph_width(&theme.font, '0'),
        )
    });

    breadcrumb(ui, document, project, palette, &theme);

    if ui.memory(|m| m.has_focus(text_id)) {
        handle_keys(ui, document, text_id, &path, commands);
    }
    let reveal = document.editor.reveal.take();
    if let Some(target) = reveal {
        set_cursor(ui.ctx(), text_id, target);
        ui.memory_mut(|m| m.request_focus(text_id));
    }

    // Layout: line numbers | text | minimap.
    let area = ui.available_rect_before_wrap();
    let line_count = document.doc.line_index().line_count();
    let gutter_width = line_count.to_string().len().max(3) as f32 * char_width + 24.0;
    let minimap_width = if area.width() > 600.0 {
        MINIMAP_WIDTH
    } else {
        0.0
    };
    let gutter = Rect::from_min_max(area.min, pos2(area.left() + gutter_width, area.bottom()));
    let minimap = Rect::from_min_max(pos2(area.right() - minimap_width, area.top()), area.max);
    let text_rect = Rect::from_min_max(
        pos2(gutter.right(), area.top()),
        pos2(minimap.left(), area.bottom()),
    );

    let language = document.doc.language();
    let mut scroll = ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink(false);
    if let Some(offset) = document.editor.scroll_to.take() {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let mut text_ui = ui.new_child(UiBuilder::new().max_rect(text_rect).layout(*ui.layout()));
    let scroll_output = scroll.show(&mut text_ui, |ui| {
        let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, _wrap_width: f32| {
            let job = highlight::highlighted(ui.ctx(), &theme, buffer.as_str(), language);
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let output = TextEdit::multiline(document.doc.buffer_mut())
            .id(text_id)
            .font(theme.font.clone())
            .code_editor()
            .frame(egui::Frame::NONE)
            .margin(Margin::symmetric(6, 2))
            .desired_width(f32::INFINITY)
            .min_size(ui.available_size())
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        if let Some(target) = reveal {
            let rect = output
                .galley
                .pos_from_cursor(CCursor::new(target))
                .translate(output.galley_pos.to_vec2());
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
        output
    });
    ui.advance_cursor_after_rect(area);
    let geometry = ScrollGeometry {
        content_height: scroll_output.content_size.y.max(1.0),
        view_height: scroll_output.inner_rect.height(),
        offset: scroll_output.state.offset.y,
    };
    let output = scroll_output.inner;

    if let Some(range) = output.cursor_range {
        document.editor.cursor = range.primary.index.0;
    }
    if output.response.changed() {
        document.doc.mark_edited();
        commands.push(Command::DocumentEdited(path.clone()));
        after_edit(document, &path, commands);
    }
    close_stale_completion(document);
    if output.response.clicked() && ui.input(|i| i.modifiers.command) {
        commands.push(Command::RequestDefinition {
            path: path.clone(),
            char_index: document.editor.cursor,
        });
    }

    paint_gutter(ui, gutter, &output, document, diagnostics, palette, &theme);
    let hovered = paint_diagnostics(
        ui,
        text_rect,
        &output,
        document,
        diagnostics,
        palette,
        char_width,
    );
    if !hovered.is_empty() {
        output
            .response
            .response
            .clone()
            .on_hover_text_at_pointer(hovered.join("\n"));
    }
    if minimap_width > 0.0 {
        paint_minimap(
            ui,
            minimap,
            id,
            document,
            geometry,
            row_height,
            &theme,
            palette,
            diagnostics,
        );
    }
    completion_popup(ui, id, text_id, &output, document, &path, palette, commands);
}

enum KeyAction {
    None,
    SelectNext,
    SelectPrevious,
    AcceptCompletion,
    CloseCompletion,
    TriggerCompletion,
    GoToDefinition,
    Indent,
    Newline,
}

fn handle_keys(
    ui: &mut Ui,
    document: &mut OpenDocument,
    text_id: Id,
    path: &std::path::Path,
    commands: &mut Vec<Command>,
) {
    let popup = document.editor.completion.as_ref();
    let popup_visible = popup.is_some_and(|p| !p.items.is_empty());
    let popup_exists = popup.is_some();

    let action = ui.input_mut(|i| {
        if popup_visible && i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            KeyAction::SelectNext
        } else if popup_visible && i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            KeyAction::SelectPrevious
        } else if popup_visible
            && (i.consume_key(Modifiers::NONE, Key::Enter)
                || i.consume_key(Modifiers::NONE, Key::Tab))
        {
            KeyAction::AcceptCompletion
        } else if popup_exists && i.consume_key(Modifiers::NONE, Key::Escape) {
            KeyAction::CloseCompletion
        } else if i.consume_key(Modifiers::CTRL, Key::Space) {
            KeyAction::TriggerCompletion
        } else if i.consume_key(Modifiers::NONE, Key::F12) {
            KeyAction::GoToDefinition
        } else if i.consume_key(Modifiers::NONE, Key::Tab) {
            KeyAction::Indent
        } else if i.consume_key(Modifiers::NONE, Key::Enter) {
            KeyAction::Newline
        } else {
            KeyAction::None
        }
    });

    let ctx = ui.ctx().clone();
    match action {
        KeyAction::None => {}
        KeyAction::SelectNext | KeyAction::SelectPrevious => {
            let cursor = document.editor.cursor;
            let text = document.doc.text().to_owned();
            if let Some(popup) = &mut document.editor.completion {
                let count =
                    filter_completions(&popup.items, char_slice(&text, popup.anchor..cursor)).len();
                if count > 0 {
                    popup.selected = if matches!(action, KeyAction::SelectNext) {
                        (popup.selected + 1) % count
                    } else {
                        (popup.selected + count - 1) % count
                    };
                }
            }
        }
        KeyAction::AcceptCompletion => accept_completion(&ctx, text_id, document, path, commands),
        KeyAction::CloseCompletion => document.editor.completion = None,
        KeyAction::TriggerCompletion => commands.push(Command::RequestCompletion {
            path: path.to_path_buf(),
            char_index: document.editor.cursor,
        }),
        KeyAction::GoToDefinition => commands.push(Command::RequestDefinition {
            path: path.to_path_buf(),
            char_index: document.editor.cursor,
        }),
        KeyAction::Indent => {
            let selection = selection(&ctx, text_id, document);
            let cursor = replace_chars(document, selection, INDENT);
            set_cursor(&ctx, text_id, cursor);
            commands.push(Command::DocumentEdited(path.to_path_buf()));
        }
        KeyAction::Newline => {
            let selection = selection(&ctx, text_id, document);
            let (insert, cursor_offset) =
                newline_with_indent(document.doc.text(), selection.start, selection.end);
            let start = selection.start;
            replace_chars(document, selection, &insert);
            set_cursor(&ctx, text_id, start + cursor_offset);
            commands.push(Command::DocumentEdited(path.to_path_buf()));
        }
    }
}

/// Text inserted for Enter at `start..end` (char indices), and where the
/// cursor goes within it: the current line's indentation is kept, one level
/// is added after `{`/`[`, and `{|}` is split over three lines.
pub fn newline_with_indent(text: &str, start: usize, end: usize) -> (String, usize) {
    let before = char_slice(text, 0..start);
    let line = before.rsplit('\n').next().unwrap_or("");
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let opens = line.trim_end().ends_with(['{', '[']);
    let after = text.chars().skip(end).find(|c| *c != ' ' && *c != '\t');
    let closes = matches!(after, Some('}' | ']'));

    if opens {
        let inner = format!("\n{indent}{INDENT}");
        let cursor = inner.chars().count();
        if closes {
            (format!("{inner}\n{indent}"), cursor)
        } else {
            (inner, cursor)
        }
    } else {
        let insert = format!("\n{indent}");
        let cursor = insert.chars().count();
        (insert, cursor)
    }
}

/// Current selection (char range), from egui's state.
fn selection(ctx: &egui::Context, text_id: Id, document: &OpenDocument) -> Range<usize> {
    TextEdit::load_state(ctx, text_id)
        .and_then(|state| state.cursor.char_range())
        .map_or(document.editor.cursor..document.editor.cursor, |range| {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            a.min(b)..a.max(b)
        })
}

/// Replaces the chars `range` with `insert`; returns the char index after it.
fn replace_chars(document: &mut OpenDocument, range: Range<usize>, insert: &str) -> usize {
    let buffer = document.doc.buffer_mut();
    let start = char_to_byte(buffer, range.start);
    let end = char_to_byte(buffer, range.end);
    buffer.replace_range(start..end, insert);
    document.doc.mark_edited();
    range.start + insert.chars().count()
}

pub fn set_cursor(ctx: &egui::Context, text_id: Id, index: usize) {
    let mut state = TextEdit::load_state(ctx, text_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
    TextEdit::store_state(ctx, text_id, state);
}

/// Opens completion after `.` or at the start of a new word.
fn after_edit(document: &mut OpenDocument, path: &std::path::Path, commands: &mut Vec<Command>) {
    if document.doc.language() != Language::Slint {
        return;
    }
    let cursor = document.editor.cursor;
    let text = document.doc.text();
    let Some(typed) = cursor.checked_sub(1).and_then(|i| text.chars().nth(i)) else {
        return;
    };
    let starts_word = is_identifier_char(typed)
        && word_start(text, cursor) == cursor - 1
        && !typed.is_ascii_digit()
        && document.editor.completion.is_none();
    if typed == '.' || starts_word {
        commands.push(Command::RequestCompletion {
            path: path.to_path_buf(),
            char_index: cursor,
        });
    }
}

/// Closes the popup once the cursor leaves the word being completed.
fn close_stale_completion(document: &mut OpenDocument) {
    let cursor = document.editor.cursor;
    let text = document.doc.text();
    let stale = document.editor.completion.as_ref().is_some_and(|popup| {
        cursor < popup.anchor
            || !char_slice(text, popup.anchor..cursor)
                .chars()
                .all(is_identifier_char)
    });
    if stale {
        document.editor.completion = None;
    }
}

fn accept_completion(
    ctx: &egui::Context,
    text_id: Id,
    document: &mut OpenDocument,
    path: &std::path::Path,
    commands: &mut Vec<Command>,
) {
    let Some(popup) = document.editor.completion.take() else {
        return;
    };
    let cursor = document.editor.cursor.max(popup.anchor);
    let insert = {
        let prefix = char_slice(document.doc.text(), popup.anchor..cursor);
        let filtered = filter_completions(&popup.items, prefix);
        match filtered.get(popup.selected).or(filtered.first()) {
            Some(entry) => entry.insert.clone(),
            None => return,
        }
    };
    let new_cursor = replace_chars(document, popup.anchor..cursor, &insert);
    document.editor.cursor = new_cursor;
    set_cursor(ctx, text_id, new_cursor);
    commands.push(Command::DocumentEdited(path.to_path_buf()));
}

// ---------------------------------------------------------------------------
// Painting

fn breadcrumb(
    ui: &mut Ui,
    document: &mut OpenDocument,
    project: Option<&Project>,
    palette: &Palette,
    theme: &CodeTheme,
) {
    let key = (document.doc.version(), document.editor.cursor);
    let scopes = match &document.editor.breadcrumb {
        Some((cached, scopes)) if *cached == key => scopes.clone(),
        _ => {
            let text = document.doc.text();
            let scopes = if document.doc.language() == Language::Slint {
                syntax::scope_path(text, char_to_byte(text, document.editor.cursor))
            } else {
                Vec::new()
            };
            document.editor.breadcrumb = Some((key, scopes.clone()));
            scopes
        }
    };

    let path = document.doc.path();
    let relative = project.map_or(path, |p| p.relative(path));
    egui::Frame::NONE
        .fill(palette.background)
        .inner_margin(Margin::symmetric(12, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let weak = palette.text_weak;
                let separator = |ui: &mut Ui| {
                    ui.label(ico(icon::CARET_RIGHT).size(10.0).color(weak));
                };
                let components: Vec<_> = relative.iter().map(|c| c.to_string_lossy()).collect();
                for (i, component) in components.iter().enumerate() {
                    if i > 0 {
                        separator(ui);
                    }
                    ui.label(RichText::new(component.as_ref()).small().color(weak));
                }
                for scope in &scopes {
                    separator(ui);
                    ui.label(ico(icon::CUBE).size(10.0).color(theme.type_name));
                    ui.label(RichText::new(scope).small().color(palette.text));
                }
            });
        });
}

fn line_of_char(document: &OpenDocument, char_index: usize) -> usize {
    let text = document.doc.text();
    document
        .doc
        .line_index()
        .line_of(char_to_byte(text, char_index))
}

fn severity_color(severity: Severity, palette: &Palette) -> Color32 {
    match severity {
        Severity::Error => palette.error,
        Severity::Warning => palette.warning,
        Severity::Info | Severity::Hint => palette.accent,
    }
}

fn paint_gutter(
    ui: &Ui,
    rect: Rect,
    output: &TextEditOutput,
    document: &OpenDocument,
    diagnostics: &[Diagnostic],
    palette: &Palette,
    theme: &CodeTheme,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.background);

    // Most severe diagnostic per line.
    let mut markers = std::collections::HashMap::new();
    for diagnostic in diagnostics {
        if let Some(location) = &diagnostic.location {
            let entry = markers
                .entry(location.start.line as usize)
                .or_insert(diagnostic.severity);
            *entry = (*entry).max(diagnostic.severity);
        }
    }

    let cursor_line = line_of_char(document, document.editor.cursor);
    let offset = output.galley_pos.to_vec2();
    let mut line = 0;
    let mut starts_line = true;
    for row in &output.galley.rows {
        let row_rect = row.rect().translate(offset);
        if starts_line && row_rect.bottom() >= rect.top() && row_rect.top() <= rect.bottom() {
            let color = if line == cursor_line {
                palette.text
            } else {
                palette.text_weak.gamma_multiply(0.6)
            };
            painter.text(
                pos2(rect.right() - 12.0, row_rect.center().y),
                Align2::RIGHT_CENTER,
                (line + 1).to_string(),
                theme.font.clone(),
                color,
            );
            if let Some(severity) = markers.get(&line) {
                painter.circle_filled(
                    pos2(rect.left() + 7.0, row_rect.center().y),
                    3.0,
                    severity_color(*severity, palette),
                );
            }
        }
        starts_line = row.ends_with_newline;
        if row.ends_with_newline {
            line += 1;
        }
    }
}

/// Paints wavy underlines; returns the messages under the pointer.
fn paint_diagnostics(
    ui: &Ui,
    clip: Rect,
    output: &TextEditOutput,
    document: &OpenDocument,
    diagnostics: &[Diagnostic],
    palette: &Palette,
    char_width: f32,
) -> Vec<String> {
    let painter = ui.painter_at(clip);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let text = document.doc.text();
    let index = document.doc.line_index();
    let offset = output.galley_pos.to_vec2();
    let mut hovered = Vec::new();

    for diagnostic in diagnostics {
        let Some(location) = &diagnostic.location else {
            continue;
        };
        let start = byte_to_char(
            text,
            index.offset(text, location.start, PositionEncoding::Chars),
        );
        let end = byte_to_char(
            text,
            index.offset(text, location.end, PositionEncoding::Chars),
        )
        .max(start + 1);
        let color = severity_color(diagnostic.severity, palette);

        // One segment per line of the range.
        let mut segments = Vec::new();
        let mut segment_start = start;
        for (i, c) in text.chars().enumerate().skip(start).take(end - start) {
            if c == '\n' {
                segments.push(segment_start..i);
                segment_start = i + 1;
            }
        }
        segments.push(segment_start..end);

        for segment in segments {
            let a = output
                .galley
                .pos_from_cursor(CCursor::new(segment.start))
                .translate(offset);
            let b = output
                .galley
                .pos_from_cursor(CCursor::new(segment.end))
                .translate(offset);
            let x1 = b.left().max(a.left() + char_width * 0.8);
            let baseline = a.bottom() - 1.0;
            wavy_line(&painter, a.left(), x1, baseline, color);
            let hit = Rect::from_x_y_ranges(a.left()..=x1, a.top()..=a.bottom() + 2.0);
            if pointer.is_some_and(|p| hit.contains(p)) {
                hovered.push(format!(
                    "{}: {}",
                    diagnostic.severity.label(),
                    diagnostic.message
                ));
            }
        }
    }
    hovered
}

fn wavy_line(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        points.push(pos2(x, if up { y - 1.5 } else { y + 0.5 }));
        x += 2.0;
        up = !up;
    }
    if points.len() > 1 {
        painter.add(Shape::line(points, Stroke::new(1.0, color)));
    }
}

/// Vertical geometry of the editor's scroll area.
#[derive(Clone, Copy)]
struct ScrollGeometry {
    content_height: f32,
    view_height: f32,
    offset: f32,
}

#[derive(Clone, Copy)]
struct MiniLine {
    indent: u16,
    len: u16,
    color: Color32,
}

fn minimap_lines(text: &str, language: Language, theme: &CodeTheme) -> Vec<MiniLine> {
    text.split('\n')
        .map(|line| {
            let indent = line
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum::<usize>();
            let trimmed = line.trim();
            let color = match language {
                Language::Slint => syntax::tokenize(trimmed).first().map_or(theme.text, |t| {
                    theme.color(t.kind, &trimmed[t.range.clone()])
                }),
                Language::PlainText => theme.text,
            };
            MiniLine {
                indent: u16::try_from(indent).unwrap_or(u16::MAX),
                len: u16::try_from(trimmed.chars().count()).unwrap_or(u16::MAX),
                color,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn paint_minimap(
    ui: &Ui,
    rect: Rect,
    id: Id,
    document: &mut OpenDocument,
    scroll: ScrollGeometry,
    row_height: f32,
    theme: &CodeTheme,
    palette: &Palette,
    diagnostics: &[Diagnostic],
) {
    let version = document.doc.version();
    if document
        .editor
        .minimap
        .as_ref()
        .is_none_or(|(v, _)| *v != version)
    {
        let lines = minimap_lines(document.doc.text(), document.doc.language(), theme);
        document.editor.minimap = Some((version, lines));
    }
    let Some((_, lines)) = &document.editor.minimap else {
        return;
    };

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(0x1A, 0x1A, 0x1A));
    painter.vline(
        rect.left(),
        rect.y_range(),
        Stroke::new(1.0, Color32::from_rgb(0x2B, 0x2B, 0x2B)),
    );

    let count = lines.len().max(1) as f32;
    let line_height = (rect.height() / count).clamp(0.5, 3.0);
    let column_width = 1.0;
    for (i, line) in lines.iter().enumerate() {
        let y = rect.top() + i as f32 * line_height;
        if y > rect.bottom() {
            break;
        }
        if line.len == 0 {
            continue;
        }
        let x = rect.left() + 6.0 + f32::from(line.indent) * column_width;
        let width = (f32::from(line.len) * column_width).min(rect.right() - x - 4.0);
        if width > 0.0 {
            let bar = Rect::from_min_size(pos2(x, y), vec2(width, (line_height * 0.7).max(1.0)));
            painter.rect_filled(bar, 0.0, line.color.gamma_multiply(0.75));
        }
    }
    for diagnostic in diagnostics {
        if let Some(location) = &diagnostic.location {
            let y = rect.top() + location.start.line as f32 * line_height;
            let marker =
                Rect::from_min_size(pos2(rect.right() - 4.0, y), vec2(3.0, line_height.max(2.0)));
            painter.rect_filled(marker, 0.0, severity_color(diagnostic.severity, palette));
        }
    }

    // Visible region.
    let ScrollGeometry {
        content_height,
        view_height,
        offset,
    } = scroll;
    let total = count * line_height;
    let top = rect.top() + offset / content_height * total;
    let height = (view_height / content_height * total).clamp(8.0, rect.height());
    let viewport = Rect::from_min_max(
        pos2(rect.left() + 1.0, top),
        pos2(rect.right(), top + height),
    );
    painter.rect_filled(viewport, 0.0, Color32::from_white_alpha(14));

    let response = ui.interact(rect, id.with("minimap"), Sense::click_and_drag());
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let line = ((pos.y - rect.top()) / line_height).max(0.0);
        document.editor.scroll_to = Some((line * row_height - view_height / 2.0).max(0.0));
        ui.ctx().request_repaint();
    }
}

#[allow(clippy::too_many_arguments)]
fn completion_popup(
    ui: &Ui,
    id: Id,
    text_id: Id,
    output: &TextEditOutput,
    document: &mut OpenDocument,
    path: &std::path::Path,
    palette: &Palette,
    commands: &mut Vec<Command>,
) {
    let cursor = document.editor.cursor;
    let Some(popup) = &mut document.editor.completion else {
        return;
    };
    if popup.items.is_empty() {
        return;
    }
    let text = document.doc.text();
    let filtered = filter_completions(&popup.items, char_slice(text, popup.anchor..cursor));
    if filtered.is_empty() {
        return;
    }
    popup.selected = popup.selected.min(filtered.len() - 1);
    let selected = popup.selected;
    let anchor = output
        .galley
        .pos_from_cursor(CCursor::new(popup.anchor))
        .translate(output.galley_pos.to_vec2());

    let mut clicked = None;
    egui::Area::new(id.with("completion"))
        .order(Order::Foreground)
        .fixed_pos(anchor.left_bottom() + vec2(-6.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                ui.set_max_width(460.0);
                ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for (i, entry) in filtered.iter().take(MAX_VISIBLE_COMPLETIONS).enumerate() {
                        let detail = entry.detail.clone().unwrap_or_default();
                        let button = Button::selectable(
                            i == selected,
                            (
                                ico(kind_icon(entry.kind)).color(palette.accent_secondary),
                                RichText::new(&entry.label).color(palette.text),
                                Atom::grow(),
                                RichText::new(detail).small().color(palette.text_weak),
                            ),
                        )
                        .min_size(vec2(ui.available_width(), 0.0));
                        let response = ui.add(button);
                        if i == selected {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        });

    if let Some(index) = clicked {
        if let Some(popup) = &mut document.editor.completion {
            popup.selected = index;
        }
        accept_completion(ui.ctx(), text_id, document, path, commands);
    }
}

fn kind_icon(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(
            CompletionItemKind::PROPERTY | CompletionItemKind::FIELD | CompletionItemKind::VARIABLE,
        ) => icon::TAG,
        Some(
            CompletionItemKind::FUNCTION | CompletionItemKind::METHOD | CompletionItemKind::EVENT,
        ) => icon::FUNCTION,
        Some(
            CompletionItemKind::CLASS
            | CompletionItemKind::STRUCT
            | CompletionItemKind::MODULE
            | CompletionItemKind::INTERFACE,
        ) => icon::CUBE,
        Some(CompletionItemKind::ENUM | CompletionItemKind::ENUM_MEMBER) => icon::LIST_NUMBERS,
        Some(CompletionItemKind::KEYWORD) => icon::KEY,
        Some(CompletionItemKind::COLOR) => icon::PALETTE,
        Some(
            CompletionItemKind::VALUE | CompletionItemKind::CONSTANT | CompletionItemKind::UNIT,
        ) => icon::HASH,
        Some(CompletionItemKind::SNIPPET) => icon::BRACKETS_CURLY,
        Some(CompletionItemKind::TYPE_PARAMETER) => icon::BRACKETS_ANGLE,
        _ => icon::TEXT_AA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, sort: Option<&str>) -> CompletionEntry {
        CompletionEntry {
            label: label.into(),
            detail: None,
            insert: label.into(),
            kind: None,
            sort_key: sort.unwrap_or(label).into(),
        }
    }

    #[test]
    fn completion_filter_prefers_prefix_matches_then_server_order() {
        let items = [
            entry("x", None),
            entry("border-color", Some("#border-color")),
            entry("background", Some("#background")),
            entry("drop-shadow-color", None),
        ];
        let labels = |prefix: &str| -> Vec<String> {
            filter_completions(&items, prefix)
                .iter()
                .map(|e| e.label.clone())
                .collect()
        };
        assert_eq!(labels("b"), ["background", "border-color"]);
        assert_eq!(labels("COL"), ["border-color", "drop-shadow-color"]);
        assert_eq!(labels("").len(), 4);
        assert!(labels("zzz").is_empty());
    }

    #[test]
    fn word_start_handles_kebab_case() {
        let text = "Rectangle { border-co";
        assert_eq!(word_start(text, text.chars().count()), 12);
        assert_eq!(word_start("a.", 2), 2);
        assert_eq!(word_start("", 0), 0);
    }

    #[test]
    fn enter_keeps_indentation() {
        let text = "    width: 1px;";
        let (insert, cursor) = newline_with_indent(text, 15, 15);
        assert_eq!(insert, "\n    ");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn enter_after_brace_indents_and_splits_braces() {
        let text = "Rectangle {}";
        let (insert, cursor) = newline_with_indent(text, 11, 11);
        assert_eq!(insert, "\n    \n");
        assert_eq!(cursor, 5);

        let text = "  Text {";
        let (insert, _) = newline_with_indent(text, 8, 8);
        assert_eq!(insert, "\n      ");
    }

    #[test]
    fn minimap_lines_measure_indent_and_length() {
        let theme = CodeTheme::dark(egui::FontId::monospace(13.0));
        let lines = minimap_lines("component A {\n\tText {}\n\n", Language::Slint, &theme);
        assert_eq!(lines.len(), 4);
        assert_eq!((lines[0].indent, lines[0].len), (0, 13));
        assert_eq!((lines[1].indent, lines[1].len), (4, 7));
        assert_eq!(lines[0].color, theme.keyword);
        assert_eq!(lines[1].color, theme.type_name);
        assert_eq!(lines[2].len, 0);
    }
}
