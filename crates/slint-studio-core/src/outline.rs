//! Structure of a `.slint` document: its elements and property bindings,
//! with their byte ranges in the text.
//!
//! This is the text ↔ element mapping behind the designer: clicking an
//! element in the canvas finds its source, moving the cursor in the editor
//! finds the element to highlight, and the property editor rewrites
//! bindings in place.
//!
//! Elements are identified across edits by their *tree path* (child
//! indices from the component root), which property edits do not change.
//! Each element also records the offset the Slint compiler uses to identify
//! it ([`ElementNode::key_offset`]), to look up its rendered geometry.

use std::ops::Range;

use crate::syntax::{TokenKind, tokenize};

/// A property binding: `name: value;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub name_range: Range<usize>,
    /// The value expression, without surrounding whitespace and `;`.
    pub value: Range<usize>,
    /// The whole statement, including the `;` when present.
    pub statement: Range<usize>,
}

/// How an element is instantiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instantiation {
    /// The root element of a `component`.
    ComponentRoot,
    Plain,
    /// `if condition : Element { }`
    Conditional,
    /// `for item in model : Element { }`
    Repeated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementNode {
    /// Element type (`Rectangle`, `Button`, a component name). Empty for a
    /// `component X { }` without base type.
    pub type_name: String,
    /// `id` in `id := Type { }`.
    pub id: Option<String>,
    /// Component name, for component roots.
    pub component: Option<String>,
    pub instantiation: Instantiation,
    /// Offset of the type name (or of `{` when there is none): the location
    /// under which the Slint compiler records this element.
    pub key_offset: usize,
    /// Start of the declaration (`export`, `component`, the id, `if`, …).
    pub start: usize,
    pub open_brace: usize,
    /// The matching `}`; `None` for an unterminated element.
    pub close_brace: Option<usize>,
    /// Just after the closing brace (end of text when unterminated).
    pub end: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub bindings: Vec<Binding>,
    pub depth: usize,
}

impl ElementNode {
    /// Display name: `Button (ok)`, `MainWindow` for component roots.
    pub fn label(&self) -> String {
        if let Some(component) = &self.component {
            return component.clone();
        }
        match &self.id {
            Some(id) => format!("{} ({id})", self.type_name),
            None => self.type_name.clone(),
        }
    }

    /// Range between the braces.
    pub fn body(&self) -> Range<usize> {
        self.open_brace + 1..self.close_brace.unwrap_or(self.end)
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    pub fn binding(&self, name: &str) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.name == name)
    }
}

/// All elements of a document, in source (pre-)order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    pub elements: Vec<ElementNode>,
}

impl Outline {
    pub fn parse(text: &str) -> Self {
        Parser::new(text).run()
    }

    /// Top-level elements (component roots), in source order.
    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        self.elements
            .iter()
            .enumerate()
            .filter(|(_, e)| e.parent.is_none())
            .map(|(i, _)| i)
    }

    /// The innermost element whose declaration contains `offset`.
    pub fn element_at(&self, offset: usize) -> Option<usize> {
        // Pre-order: a nested element always comes after its ancestors.
        self.elements.iter().rposition(|e| e.contains(offset))
    }

    /// Child indices from the component root: `[root, child, grandchild…]`.
    pub fn tree_path(&self, index: usize) -> Vec<usize> {
        let mut path = Vec::new();
        let mut current = index;
        while let Some(element) = self.elements.get(current) {
            match element.parent {
                Some(parent) => {
                    let position = self.elements[parent]
                        .children
                        .iter()
                        .position(|c| *c == current)
                        .unwrap_or(0);
                    path.push(position);
                    current = parent;
                }
                None => {
                    path.push(self.roots().position(|r| r == current).unwrap_or(0));
                    break;
                }
            }
        }
        path.reverse();
        path
    }

    pub fn by_tree_path(&self, path: &[usize]) -> Option<usize> {
        let (&root, rest) = path.split_first()?;
        let mut current = self.roots().nth(root)?;
        for &child in rest {
            current = *self.elements[current].children.get(child)?;
        }
        Some(current)
    }
}

/// A token that matters for structure (not whitespace or comment).
#[derive(Debug, Clone)]
struct Sig<'a> {
    kind: TokenKind,
    range: Range<usize>,
    text: &'a str,
}

impl Sig<'_> {
    fn is(&self, text: &str) -> bool {
        self.text == text
    }

    fn is_type_name(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::TypeName | TokenKind::Identifier | TokenKind::Property
        )
    }
}

enum Frame {
    Element(usize),
    Block,
}

struct Parser<'a> {
    text: &'a str,
    sig: Vec<Sig<'a>>,
    elements: Vec<ElementNode>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        let sig = tokenize(text)
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment))
            .map(|t| Sig {
                kind: t.kind,
                text: &text[t.range.clone()],
                range: t.range,
            })
            .collect();
        Self {
            text,
            sig,
            elements: Vec::new(),
        }
    }

    fn run(mut self) -> Outline {
        let mut stack: Vec<Frame> = Vec::new();
        let mut statement_start = 0;
        let mut nesting = 0usize; // parentheses and brackets
        let mut i = 0;
        while i < self.sig.len() {
            let token = &self.sig[i];
            match token.text {
                "(" | "[" => nesting += 1,
                ")" | "]" => nesting = nesting.saturating_sub(1),
                "{" if nesting == 0 => {
                    let frame = self
                        .open_element(&stack, statement_start, i)
                        .map_or(Frame::Block, Frame::Element);
                    stack.push(frame);
                    statement_start = i + 1;
                }
                "{" => stack.push(Frame::Block),
                "}" => {
                    if let Some(Frame::Element(index)) = stack.pop() {
                        let element = &mut self.elements[index];
                        element.close_brace = Some(token.range.start);
                        element.end = token.range.end;
                    }
                    if nesting == 0 {
                        statement_start = i + 1;
                    }
                }
                ";" if nesting == 0 => statement_start = i + 1,
                ":" if nesting == 0 => {
                    if let Some(Frame::Element(index)) = stack.last() {
                        self.binding(*index, statement_start, i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Outline {
            elements: self.elements,
        }
    }

    /// Creates the element opened by the `{` at `brace`, if it is one.
    fn open_element(
        &mut self,
        stack: &[Frame],
        statement_start: usize,
        brace: usize,
    ) -> Option<usize> {
        let statement = &self.sig[statement_start..brace];
        let parent = stack.iter().rev().find_map(|f| match f {
            Frame::Element(index) => Some(*index),
            Frame::Block => None,
        });
        // Inside a non-element block (callback body, struct literal, …)
        // there are no elements.
        if matches!(stack.last(), Some(Frame::Block)) {
            return None;
        }
        let open_brace = self.sig[brace].range.start;
        let start = statement.first().map_or(open_brace, |t| t.range.start);

        let (type_name, key_offset, id, component, instantiation) = if let Some(position) =
            statement
                .iter()
                .position(|t| t.kind == TokenKind::Keyword && t.is("component"))
        {
            let name = statement.get(position + 1)?;
            match statement.get(position + 2) {
                Some(inherits) if inherits.is("inherits") => {
                    let base = statement.get(position + 3)?;
                    (
                        base.text.to_owned(),
                        base.range.start,
                        None,
                        Some(name.text.to_owned()),
                        Instantiation::ComponentRoot,
                    )
                }
                None => (
                    String::new(),
                    open_brace,
                    None,
                    Some(name.text.to_owned()),
                    Instantiation::ComponentRoot,
                ),
                Some(_) => return None,
            }
        } else {
            if statement
                .iter()
                .any(|t| matches!(t.text, "struct" | "enum" | "global" | "interface"))
            {
                return None;
            }
            let ty = statement.last().filter(|t| t.is_type_name())?;
            let before = statement.len().checked_sub(2).map(|i| &statement[i]);
            let first = statement.first().map(|t| t.text);
            match before {
                None => (
                    ty.text.to_owned(),
                    ty.range.start,
                    None,
                    None,
                    Instantiation::Plain,
                ),
                Some(op) if op.is(":=") => {
                    let id = statement
                        .len()
                        .checked_sub(3)
                        .map(|i| statement[i].text.to_owned());
                    // Legacy top-level syntax `Name := Base { }` declares a component.
                    if parent.is_none() {
                        (
                            ty.text.to_owned(),
                            ty.range.start,
                            None,
                            id,
                            Instantiation::ComponentRoot,
                        )
                    } else {
                        (
                            ty.text.to_owned(),
                            ty.range.start,
                            id,
                            None,
                            Instantiation::Plain,
                        )
                    }
                }
                Some(op) if op.is(":") && first == Some("if") => (
                    ty.text.to_owned(),
                    ty.range.start,
                    None,
                    None,
                    Instantiation::Conditional,
                ),
                Some(op) if op.is(":") && first == Some("for") => (
                    ty.text.to_owned(),
                    ty.range.start,
                    None,
                    None,
                    Instantiation::Repeated,
                ),
                Some(_) => return None,
            }
        };
        // Only component roots live at the top level.
        if parent.is_none() && instantiation != Instantiation::ComponentRoot {
            return None;
        }

        let index = self.elements.len();
        let depth = parent.map_or(0, |p| self.elements[p].depth + 1);
        self.elements.push(ElementNode {
            type_name,
            id,
            component,
            instantiation,
            key_offset,
            start,
            open_brace,
            close_brace: None,
            end: self.text.len(),
            parent,
            children: Vec::new(),
            bindings: Vec::new(),
            depth,
        });
        if let Some(parent) = parent {
            self.elements[parent].children.push(index);
        }
        Some(index)
    }

    /// Records `name: value;` when the `:` at `colon` ends a binding name.
    fn binding(&mut self, element: usize, statement_start: usize, colon: usize) {
        let Some(name) = colon.checked_sub(1).map(|i| &self.sig[i]) else {
            return;
        };
        if name.kind != TokenKind::Property || colon <= statement_start {
            return;
        }
        let first = &self.sig[statement_start];
        if matches!(
            first.text,
            "if" | "for"
                | "states"
                | "transitions"
                | "animate"
                | "callback"
                | "function"
                | "changed"
        ) {
            return;
        }
        // The value runs to the `;` (or the element's `}`) at nesting level 0.
        let mut depth = 0usize;
        let mut end_token = None;
        let mut semicolon = None;
        for (j, token) in self.sig.iter().enumerate().skip(colon + 1) {
            match token.text {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" => depth = depth.saturating_sub(1),
                "}" if depth == 0 => break,
                "}" => depth -= 1,
                ";" if depth == 0 => {
                    semicolon = Some(j);
                    break;
                }
                _ => {}
            }
            end_token = Some(j);
        }
        let Some(last) = end_token else {
            return;
        };
        let value = self.sig[colon + 1].range.start..self.sig[last].range.end;
        let statement_end = semicolon.map_or(value.end, |s| self.sig[s].range.end);
        let statement_begin = first.range.start;
        self.elements[element].bindings.push(Binding {
            name: name.text.to_owned(),
            name_range: name.range.clone(),
            value,
            statement: statement_begin..statement_end,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = r#"import { Button } from "std-widgets.slint";

// A card
component Card inherits Rectangle {
    in property <string> title;
    background: #252526;
    Text { text: root.title; }
}

export component MainWindow inherits Window {
    title: "Contatore";
    in-out property <int> counter: 0;
    callback reset();
    reset => { counter = 0; }

    VerticalLayout {
        spacing: 8px;
        card := Card {
            title: "Valore";
            Text { text: "\{counter}"; font-size: 32px; }
        }
        if counter > 0 : Button { text: "Azzera"; clicked => { root.reset(); } }
        for item in [1, 2] : Rectangle { height: 10px; }
        states [ big when counter > 10 : { } ]
    }
    animate background { duration: 100ms; }
}
"#;

    fn labels(outline: &Outline) -> Vec<String> {
        outline.elements.iter().map(ElementNode::label).collect()
    }

    #[test]
    fn finds_elements_in_source_order() {
        let outline = Outline::parse(DEMO);
        assert_eq!(
            labels(&outline),
            [
                "Card",
                "Text",
                "MainWindow",
                "VerticalLayout",
                "Card (card)",
                "Text",
                "Button",
                "Rectangle"
            ]
        );
        let button = &outline.elements[6];
        assert_eq!(button.instantiation, Instantiation::Conditional);
        assert_eq!(outline.elements[7].instantiation, Instantiation::Repeated);
        assert_eq!(outline.elements[2].type_name, "Window");
        assert_eq!(
            outline.elements[2].instantiation,
            Instantiation::ComponentRoot
        );
    }

    #[test]
    fn key_offsets_point_at_the_type_name() {
        let outline = Outline::parse(DEMO);
        let key =
            |i: usize| &DEMO[outline.elements[i].key_offset..outline.elements[i].key_offset + 6];
        assert_eq!(key(0), "Rectan"); // `component Card inherits Rectangle`
        assert_eq!(key(2), "Window");
        assert_eq!(key(4), "Card {");
    }

    #[test]
    fn nesting_and_ranges() {
        let outline = Outline::parse(DEMO);
        let window = &outline.elements[2];
        assert_eq!(window.children, [3]);
        assert_eq!(outline.elements[3].children, [4, 6, 7]);
        assert_eq!(outline.elements[5].parent, Some(4));
        assert_eq!(outline.elements[5].depth, 3);
        assert!(DEMO[window.start..window.end].starts_with("export component MainWindow"));
        assert!(DEMO[window.start..window.end].ends_with('}'));
        assert!(DEMO[outline.elements[4].start..].starts_with("card := Card"));
    }

    #[test]
    fn bindings_exclude_declarations_callbacks_and_conditions() {
        let outline = Outline::parse(DEMO);
        let names = |i: usize| {
            outline.elements[i]
                .bindings
                .iter()
                .map(|b| b.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(0), ["background"]);
        assert_eq!(names(2), ["title", "counter"]);
        assert_eq!(names(3), ["spacing"]);
        assert_eq!(names(5), ["text", "font-size"]);
        assert_eq!(names(6), ["text"]);

        let title = outline.elements[2].binding("title").unwrap();
        assert_eq!(&DEMO[title.value.clone()], "\"Contatore\"");
        assert_eq!(&DEMO[title.statement.clone()], "title: \"Contatore\";");
        let counter = outline.elements[2].binding("counter").unwrap();
        assert_eq!(
            &DEMO[counter.statement.clone()],
            "in-out property <int> counter: 0;"
        );
        assert_eq!(&DEMO[counter.value.clone()], "0");
    }

    #[test]
    fn element_at_finds_the_innermost_element() {
        let outline = Outline::parse(DEMO);
        let at = |needle: &str| {
            outline
                .element_at(DEMO.find(needle).unwrap())
                .map(|i| outline.elements[i].label())
        };
        assert_eq!(at("font-size: 32px").as_deref(), Some("Text"));
        assert_eq!(at("spacing: 8px").as_deref(), Some("VerticalLayout"));
        assert_eq!(at("card := Card").as_deref(), Some("Card (card)"));
        assert_eq!(at("root.reset()").as_deref(), Some("Button"));
        assert_eq!(at("import {"), None);
    }

    #[test]
    fn tree_paths_round_trip() {
        let outline = Outline::parse(DEMO);
        for index in 0..outline.elements.len() {
            let path = outline.tree_path(index);
            assert_eq!(outline.by_tree_path(&path), Some(index), "{path:?}");
        }
        assert_eq!(outline.tree_path(5), [1, 0, 0, 0]);
        assert_eq!(outline.by_tree_path(&[9]), None);
    }

    #[test]
    fn components_without_base_and_unterminated_elements() {
        let text = "component Empty {\n    Rectangle { width: 1px; }\n}\nexport component Open inherits Window {\n    Text {";
        let outline = Outline::parse(text);
        assert_eq!(labels(&outline), ["Empty", "Rectangle", "Open", "Text"]);
        assert_eq!(outline.elements[0].key_offset, text.find('{').unwrap());
        assert_eq!(outline.elements[3].close_brace, None);
        assert_eq!(outline.elements[3].end, text.len());
    }

    #[test]
    fn structs_globals_and_top_level_blocks_are_not_elements() {
        let text = "struct P { x: int }\nglobal Theme { out property <color> c: red; }\nexport component A inherits Window { }";
        assert_eq!(labels(&Outline::parse(text)), ["A"]);
    }
}
