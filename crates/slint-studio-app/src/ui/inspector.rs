//! Right side: component palette and property editor (Design / Split modes).

use std::collections::HashMap;

use egui::{Button, CollapsingHeader, Color32, ComboBox, Id, RichText, Sense, TextEdit, Ui};
use egui_phosphor::regular as icon;
use slint_studio_core::properties::{
    self, PropertyGroup, PropertySpec, ValueKind, format_hex_color, parse_hex_color, quote, unquote,
};
use slint_studio_core::{ComponentCategory, ElementNode, PaletteItem};

use super::{empty_state, ico, padded, with_icon};
use crate::app::AppState;
use crate::commands::Command;
use crate::designer::{ElementRef, PaletteDrag};

pub fn components(ui: &mut Ui, state: &mut AppState) {
    if !state.view_mode.shows_canvas() {
        return canvas_hidden(ui, "la palette dei componenti");
    }
    let mut commands = Vec::new();
    padded(ui, |ui| {
        let target = state.designer.selected.is_some();
        let hint = if target {
            "Clic: inserisci nell'elemento selezionato · trascina nel canvas"
        } else {
            "Clic: inserisci nel componente · trascina nel canvas"
        };
        ui.label(RichText::new(hint).small().weak());
        for category in ComponentCategory::ALL {
            CollapsingHeader::new(RichText::new(category.label()).strong())
                .id_salt(category.label())
                .default_open(true)
                .show(ui, |ui| {
                    for item in PaletteItem::in_category(category) {
                        palette_entry(ui, item, &mut commands);
                    }
                });
        }
    });
    for command in commands {
        state.send(command);
    }
}

fn palette_entry(ui: &mut Ui, item: &'static PaletteItem, commands: &mut Vec<Command>) {
    let mut tooltip = format!(
        "Clic per inserire {}, oppure trascinalo nel canvas",
        item.name
    );
    if let Some(module) = item.import_from() {
        tooltip.push_str(&format!("\nimport {{ {} }} from \"{module}\";", item.name));
    }
    // One widget for both gestures: egui tells a click from a drag by how
    // far the pointer moves.
    let response = ui
        .add(
            Button::new(with_icon(category_icon(item.category), item.name))
                .frame(false)
                .sense(Sense::click_and_drag()),
        )
        .on_hover_text(tooltip);
    if response.clicked() {
        commands.push(Command::InsertComponent {
            item: item.name,
            parent: None,
        });
    }
    if response.drag_started() {
        response.dnd_set_drag_payload(PaletteDrag(item.name));
    }
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            // Ghost label following the pointer.
            let painter = ui
                .ctx()
                .layer_painter(egui::LayerId::new(egui::Order::Tooltip, response.id));
            let galley = painter.layout_no_wrap(
                item.name.to_owned(),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
            let rect = egui::Rect::from_min_size(
                pointer + egui::vec2(12.0, 8.0),
                galley.size() + egui::vec2(12.0, 6.0),
            );
            painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
            painter.galley(rect.min + egui::vec2(6.0, 3.0), galley, Color32::WHITE);
        }
    }
}

fn category_icon(category: ComponentCategory) -> &'static str {
    match category {
        ComponentCategory::Basic => icon::SHAPES,
        ComponentCategory::Layout => icon::LAYOUT,
        ComponentCategory::Widget => icon::CURSOR_CLICK,
    }
}

pub fn properties(ui: &mut Ui, state: &mut AppState) {
    if !state.view_mode.shows_canvas() {
        return canvas_hidden(ui, "l'editor delle proprietà");
    }
    let selected = state.designer.selected.clone();
    let element = selected.as_ref().and_then(|selected| {
        let open = state.workspace.document(&selected.path)?;
        let outline = open.outline();
        let index = outline.by_tree_path(&selected.tree_path)?;
        Some((
            outline.elements[index].clone(),
            open.doc.text().to_owned(),
            outline,
        ))
    });
    let (Some(selected), Some((element, text, outline))) = (selected, element) else {
        return padded(ui, |ui| {
            empty_state(
                ui,
                icon::CURSOR_CLICK,
                "Nessun elemento selezionato",
                "Seleziona un elemento nel canvas, oppure porta il cursore su un elemento nell'editor.",
            );
        });
    };

    let mut commands = Vec::new();
    let palette = state.palette;
    let rendered = state
        .preview
        .image
        .as_ref()
        .filter(|image| image.path == selected.path)
        .and_then(|image| {
            image
                .elements
                .iter()
                .find(|g| g.tree_path == selected.tree_path)
        })
        .and_then(|g| g.rects.first().copied());
    let line = text[..element.key_offset].matches('\n').count() + 1;

    padded(ui, |ui| {
        // Header: what is selected and where.
        ui.horizontal(|ui| {
            ui.label(ico(icon::CUBE).color(palette.accent_secondary));
            ui.label(RichText::new(element.label()).strong().size(14.0));
        });
        let file = selected
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut info = format!(
            "{} · {file}:{line}",
            if element.type_name.is_empty() {
                "component"
            } else {
                &element.type_name
            }
        );
        if let Some(rect) = rendered {
            info.push_str(&format!(
                " · {} , {} · {} × {}",
                rect.x.round(),
                rect.y.round(),
                rect.width.round(),
                rect.height.round()
            ));
        }
        ui.label(RichText::new(info).small().weak());
        ui.horizontal(|ui| {
            let parent = element.parent.map(|p| outline.tree_path(p));
            if ui
                .add_enabled(
                    parent.is_some(),
                    Button::new(with_icon(icon::ARROW_UP, "Genitore")),
                )
                .on_hover_text("Seleziona l'elemento che contiene questo")
                .clicked()
                && let Some(tree_path) = parent
            {
                commands.push(Command::SelectElement(Some(ElementRef {
                    path: selected.path.clone(),
                    tree_path,
                })));
            }
            if ui
                .button(with_icon(icon::CODE, "Codice"))
                .on_hover_text("Mostra nell'editor")
                .clicked()
            {
                commands.push(Command::SelectElement(Some(selected.clone())));
            }
            let removable = element.parent.is_some();
            if ui
                .add_enabled(removable, Button::new(with_icon(icon::TRASH, "Elimina")))
                .on_hover_text("Rimuovi l'elemento dal codice (Canc nel canvas)")
                .clicked()
            {
                commands.push(Command::RemoveElement(selected.clone()));
            }
        });
        ui.separator();

        let specs = properties::properties_for(&element.type_name);
        let hint_color = palette.text_weak.gamma_multiply(0.55);
        let computed = |name: &str| {
            let rect = rendered?;
            let value = match name {
                "x" => rect.local_x,
                "y" => rect.local_y,
                "width" => rect.width,
                "height" => rect.height,
                _ => return None,
            };
            Some(format!("{}px", value.round()))
        };
        let drafts = state.designer.drafts_for(&selected);
        for group in PropertyGroup::ALL {
            let in_group: Vec<&PropertySpec> = specs.iter().filter(|s| s.group == group).collect();
            CollapsingHeader::new(RichText::new(group.label()).strong())
                .id_salt(("properties", group.label()))
                .default_open(true)
                .show(ui, |ui| {
                    property_grid(ui, group.label(), |ui| {
                        for spec in in_group {
                            let editor = PropertyEditor {
                                element: &selected,
                                node: &element,
                                text: &text,
                                name: spec.name,
                                kind: spec.kind,
                                hint: spec.hint,
                                computed: computed(spec.name),
                                hint_color,
                            };
                            editor.show(ui, drafts, &mut commands);
                        }
                    });
                });
        }
        let others: Vec<_> = element
            .bindings
            .iter()
            .filter(|b| !specs.iter().any(|s| s.name == b.name))
            .collect();
        if !others.is_empty() {
            CollapsingHeader::new(RichText::new("Altre proprietà").strong())
                .id_salt(("properties", "other"))
                .default_open(true)
                .show(ui, |ui| {
                    property_grid(ui, "other", |ui| {
                        for binding in others {
                            let editor = PropertyEditor {
                                element: &selected,
                                node: &element,
                                text: &text,
                                name: &binding.name,
                                kind: ValueKind::Expression,
                                hint: "",
                                computed: None,
                                hint_color,
                            };
                            editor.show(ui, drafts, &mut commands);
                        }
                    });
                });
        }
    });
    for command in commands {
        state.send(command);
    }
}

fn property_grid(ui: &mut Ui, id: &str, add_rows: impl FnOnce(&mut Ui)) {
    egui::Grid::new(("property_grid", id))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .min_col_width(90.0)
        .show(ui, add_rows);
}

/// One row of the property editor.
struct PropertyEditor<'a> {
    element: &'a ElementRef,
    node: &'a ElementNode,
    text: &'a str,
    name: &'a str,
    kind: ValueKind,
    hint: &'a str,
    /// Value computed by the preview, shown when the property is unset.
    computed: Option<String>,
    hint_color: Color32,
}

impl PropertyEditor<'_> {
    fn current(&self) -> Option<&str> {
        self.node
            .binding(self.name)
            .map(|b| &self.text[b.value.clone()])
    }

    fn set(&self, value: Option<String>, commands: &mut Vec<Command>) {
        commands.push(Command::SetProperty {
            element: self.element.clone(),
            name: self.name.to_owned(),
            value,
        });
    }

    fn show(&self, ui: &mut Ui, drafts: &mut HashMap<String, String>, commands: &mut Vec<Command>) {
        let current = self.current();
        let label = RichText::new(self.name).small();
        ui.label(if current.is_some() {
            label
        } else {
            label.weak()
        })
        .on_hover_text(current.map_or_else(
            || "non impostata".to_owned(),
            |v| format!("{}: {v};", self.name),
        ));

        match self.kind {
            ValueKind::Bool => self.choice(ui, current, &["true", "false"], commands),
            ValueKind::Enum(options) => self.choice(ui, current, options, commands),
            ValueKind::Brush => {
                ui.horizontal(|ui| {
                    let mut color = current
                        .and_then(parse_hex_color)
                        .map_or(Color32::TRANSPARENT, |[r, g, b, a]| {
                            Color32::from_rgba_unmultiplied(r, g, b, a)
                        });
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        let [r, g, b, a] = color.to_srgba_unmultiplied();
                        self.set(Some(format_hex_color([r, g, b, a])), commands);
                    }
                    self.text_field(ui, drafts, current, false, commands);
                });
            }
            ValueKind::Text => {
                let literal = current.is_none_or(|v| unquote(v).is_some());
                self.text_field(ui, drafts, current, literal, commands);
            }
            _ => self.text_field(ui, drafts, current, false, commands),
        }
        ui.end_row();
    }

    fn choice(
        &self,
        ui: &mut Ui,
        current: Option<&str>,
        options: &[&str],
        commands: &mut Vec<Command>,
    ) {
        let unset = "—";
        let shown = current.unwrap_or(unset);
        ComboBox::from_id_salt(("property_choice", self.name))
            .selected_text(RichText::new(shown).monospace().small())
            .width(ui.available_width().max(80.0))
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), unset).clicked() && current.is_some() {
                    self.set(None, commands);
                }
                for option in options {
                    if ui
                        .selectable_label(current == Some(*option), *option)
                        .clicked()
                        && current != Some(*option)
                    {
                        self.set(Some((*option).to_owned()), commands);
                    }
                }
            });
    }

    /// Free-text field: committed on Enter or when focus leaves; an empty
    /// field removes the binding. `literal` edits a string without quotes.
    fn text_field(
        &self,
        ui: &mut Ui,
        drafts: &mut HashMap<String, String>,
        current: Option<&str>,
        literal: bool,
        commands: &mut Vec<Command>,
    ) {
        let shown = match current {
            Some(value) if literal => unquote(value).unwrap_or_else(|| value.to_owned()),
            Some(value) => value.to_owned(),
            None => String::new(),
        };
        let draft = drafts
            .entry(self.name.to_owned())
            .or_insert_with(|| shown.clone());
        let response = ui.add(
            TextEdit::singleline(draft)
                .id(Id::new(("property_field", self.name)))
                .hint_text(match &self.computed {
                    Some(value) => RichText::new(format!("{value} (calcolato)"))
                        .monospace()
                        .italics()
                        .color(self.hint_color),
                    None => RichText::new(self.hint)
                        .monospace()
                        .italics()
                        .color(self.hint_color),
                })
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        if response.lost_focus() {
            let edited = draft.trim().to_owned();
            if edited != shown.trim() {
                // Empty removes the binding, except for text: an empty string.
                let value = match (literal, edited.is_empty()) {
                    (true, _) => Some(quote(&edited)),
                    (false, true) => None,
                    (false, false) => Some(edited),
                };
                self.set(value, commands);
            }
        } else if !response.has_focus() && *draft != shown {
            // The source changed elsewhere (editor, canvas): follow it.
            *draft = shown;
        }
    }
}

fn canvas_hidden(ui: &mut Ui, what: &str) {
    padded(ui, |ui| {
        empty_state(
            ui,
            icon::EYE_SLASH,
            "Disponibile in modalità Split e Design",
            &format!("Passa a Split o Design dalla toolbar per usare {what}."),
        );
    });
}
