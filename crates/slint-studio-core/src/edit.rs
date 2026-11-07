//! Source edits made by the designer: setting or removing property
//! bindings, inserting and removing elements, adding imports.
//!
//! Every function returns a [`SourceEdit`] (a replacement of one range)
//! instead of mutating, so the caller decides how to apply and record it.

use std::ops::Range;

use crate::outline::ElementNode;

/// One indentation level of generated code.
pub const INDENT: &str = "    ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEdit {
    pub range: Range<usize>,
    pub replacement: String,
}

impl SourceEdit {
    pub fn apply(&self, text: &str) -> String {
        let mut result = String::with_capacity(text.len() + self.replacement.len());
        result.push_str(&text[..self.range.start]);
        result.push_str(&self.replacement);
        result.push_str(&text[self.range.end..]);
        result
    }
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, offset: usize) -> usize {
    text[offset..].find('\n').map_or(text.len(), |i| offset + i)
}

/// Leading whitespace of the line containing `offset`.
fn indentation(text: &str, offset: usize) -> &str {
    let start = line_start(text, offset);
    let line = &text[start..line_end(text, start)];
    &line[..line.len() - line.trim_start().len()]
}

/// Widens `range` to whole lines when nothing but whitespace surrounds it,
/// so that removing it does not leave blank lines behind.
fn whole_lines(text: &str, range: Range<usize>) -> Range<usize> {
    let start = line_start(text, range.start);
    let end = line_end(text, range.end);
    let blank_before = text[start..range.start].trim().is_empty();
    let blank_after = text[range.end..end].trim().is_empty();
    if blank_before && blank_after {
        start..(end + 1).min(text.len())
    } else {
        range
    }
}

/// Sets, replaces or (with `None`) removes the binding `name` of `element`.
/// Returns `None` when there is nothing to do.
pub fn set_binding(
    text: &str,
    element: &ElementNode,
    name: &str,
    value: Option<&str>,
) -> Option<SourceEdit> {
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    match (element.binding(name), value) {
        (Some(binding), Some(value)) => {
            (text[binding.value.clone()] != *value).then(|| SourceEdit {
                range: binding.value.clone(),
                replacement: value.to_owned(),
            })
        }
        (Some(binding), None) => Some(SourceEdit {
            range: whole_lines(text, binding.statement.clone()),
            replacement: String::new(),
        }),
        (None, Some(value)) => Some(insert_binding(text, element, &format!("{name}: {value};"))),
        (None, None) => None,
    }
}

fn insert_binding(text: &str, element: &ElementNode, statement: &str) -> SourceEdit {
    let body = element.body();
    let multiline = text[body.clone()].contains('\n');
    // After the last binding, else right after `{`.
    let anchor = element
        .bindings
        .last()
        .map_or(element.open_brace + 1, |b| b.statement.end);
    if multiline {
        let indent = format!("{}{INDENT}", indentation(text, element.start));
        SourceEdit {
            range: anchor..anchor,
            replacement: format!("\n{indent}{statement}"),
        }
    } else if text[body.clone()].trim().is_empty() {
        // `Text {}` → `Text { name: value; }`
        SourceEdit {
            range: body,
            replacement: format!(" {statement} "),
        }
    } else {
        SourceEdit {
            range: anchor..anchor,
            replacement: format!(" {statement}"),
        }
    }
}

/// Inserts `snippet` as the last child of `element`. Returns the edit and
/// the offset, in the edited text, where the new element starts.
pub fn insert_child(
    text: &str,
    element: &ElementNode,
    snippet: &str,
) -> Option<(SourceEdit, usize)> {
    let close = element.close_brace?;
    let outer = indentation(text, element.start).to_owned();
    let inner = format!("{outer}{INDENT}");
    let indented: Vec<String> = snippet
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{inner}{line}")
            }
        })
        .collect();
    let block = indented.join("\n");

    let close_line = line_start(text, close);
    if text[close_line..close].trim().is_empty() && close_line > element.open_brace {
        // `}` alone on its line: insert the child on the line(s) before it.
        let edit = SourceEdit {
            range: close_line..close_line,
            replacement: format!("{block}\n"),
        };
        return Some((edit, close_line + inner.len()));
    }
    // `}` after other content on the same line: break the line.
    let mut start = close;
    while start > element.open_brace + 1 && text[..start].ends_with([' ', '\t']) {
        start -= 1;
    }
    let edit = SourceEdit {
        range: start..close,
        replacement: format!("\n{block}\n{outer}"),
    };
    Some((edit, start + 1 + inner.len()))
}

/// Removes `element` (its whole declaration, and its lines when they hold
/// nothing else).
pub fn remove_element(text: &str, element: &ElementNode) -> SourceEdit {
    SourceEdit {
        range: whole_lines(text, element.start..element.end),
        replacement: String::new(),
    }
}

/// Makes sure `name` is imported from `module` (e.g. `Button` from
/// `std-widgets.slint`).
pub fn ensure_import(text: &str, name: &str, module: &str) -> Option<SourceEdit> {
    let quoted = format!("\"{module}\"");
    let mut search = 0;
    while let Some(found) = text[search..].find(&quoted) {
        let at = search + found;
        search = at + quoted.len();
        let statement_start = text[..at]
            .rfind("import")
            .filter(|s| !text[*s..at].contains(';'));
        let Some(statement_start) = statement_start else {
            continue;
        };
        let (Some(open), Some(close)) = (
            text[statement_start..at].find('{'),
            text[statement_start..at].rfind('}'),
        ) else {
            continue;
        };
        let (open, close) = (statement_start + open, statement_start + close);
        let names: Vec<&str> = text[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .collect();
        if names
            .iter()
            .any(|n| n.split_whitespace().next() == Some(name))
        {
            return None;
        }
        let mut insert_at = close;
        while text[..insert_at].ends_with([' ', '\t', ',']) {
            insert_at -= 1;
        }
        let replacement = if names.is_empty() {
            format!(" {name} ")
        } else {
            format!(", {name} ")
        };
        return Some(SourceEdit {
            range: insert_at..close,
            replacement,
        });
    }
    // No import from that module yet: add one after the last import.
    let statement = format!("import {{ {name} }} from {quoted};\n");
    let after_imports = text
        .match_indices("import ")
        .filter(|(i, _)| line_start(text, *i) == *i)
        .filter_map(|(i, _)| text[i..].find(';').map(|e| line_end(text, i + e) + 1))
        .max()
        .map(|end| end.min(text.len()));
    Some(match after_imports {
        Some(end) => SourceEdit {
            range: end..end,
            replacement: statement,
        },
        None => SourceEdit {
            range: 0..0,
            replacement: format!("{statement}\n"),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::Outline;

    const TEXT: &str = "export component Main inherits Window {\n    width: 100px;\n    VerticalLayout {\n        Text { text: \"a\"; }\n    }\n}\n";

    fn element<'a>(outline: &'a Outline, label: &str) -> &'a ElementNode {
        outline
            .elements
            .iter()
            .find(|e| e.label() == label)
            .unwrap()
    }

    fn edit(text: &str, f: impl Fn(&str, &Outline) -> Option<SourceEdit>) -> String {
        let outline = Outline::parse(text);
        f(text, &outline).unwrap().apply(text)
    }

    #[test]
    fn replaces_an_existing_binding() {
        let result = edit(TEXT, |t, o| {
            set_binding(t, element(o, "Main"), "width", Some(" 200px "))
        });
        assert!(result.contains("    width: 200px;\n"));
        let outline = Outline::parse(TEXT);
        assert_eq!(
            set_binding(TEXT, element(&outline, "Main"), "width", Some("100px")),
            None
        );
    }

    #[test]
    fn adds_a_binding_after_the_existing_ones() {
        let result = edit(TEXT, |t, o| {
            set_binding(t, element(o, "Main"), "height", Some("50px"))
        });
        assert!(
            result.contains("    width: 100px;\n    height: 50px;\n    VerticalLayout"),
            "{result}"
        );
        let result = edit(TEXT, |t, o| {
            set_binding(t, element(o, "VerticalLayout"), "spacing", Some("8px"))
        });
        assert!(
            result.contains("    VerticalLayout {\n        spacing: 8px;\n        Text"),
            "{result}"
        );
    }

    #[test]
    fn adds_bindings_to_single_line_elements() {
        let result = edit(TEXT, |t, o| {
            set_binding(t, element(o, "Text"), "color", Some("red"))
        });
        assert!(
            result.contains("Text { text: \"a\"; color: red; }"),
            "{result}"
        );
        let empty = "component A inherits Window { Rectangle {} }";
        let result = edit(empty, |t, o| {
            set_binding(t, element(o, "Rectangle"), "width", Some("1px"))
        });
        assert!(result.contains("Rectangle { width: 1px; }"), "{result}");
    }

    #[test]
    fn removes_a_binding_and_its_line() {
        let result = edit(TEXT, |t, o| {
            set_binding(t, element(o, "Main"), "width", None)
        });
        assert!(
            result.starts_with("export component Main inherits Window {\n    VerticalLayout {"),
            "{result}"
        );
        let result = edit(TEXT, |t, o| {
            set_binding(t, element(o, "Text"), "text", Some(""))
        });
        assert!(result.contains("Text {  }"), "{result}");
    }

    #[test]
    fn inserts_children_before_the_closing_brace() {
        let outline = Outline::parse(TEXT);
        let (edit, start) = insert_child(
            TEXT,
            element(&outline, "VerticalLayout"),
            "Button {\n    text: \"OK\";\n}",
        )
        .unwrap();
        let result = edit.apply(TEXT);
        assert!(
            result.contains("        Text { text: \"a\"; }\n        Button {\n            text: \"OK\";\n        }\n    }"),
            "{result}"
        );
        assert!(result[start..].starts_with("Button {"));
        assert_eq!(
            Outline::parse(&result)
                .element_at(start)
                .map(|i| Outline::parse(&result).elements[i].label())
                .as_deref(),
            Some("Button")
        );
    }

    #[test]
    fn inserts_children_into_single_line_elements() {
        let outline = Outline::parse(TEXT);
        let (edit, start) = insert_child(TEXT, element(&outline, "Text"), "Rectangle {}").unwrap();
        let result = edit.apply(TEXT);
        assert!(
            result.contains("Text { text: \"a\";\n            Rectangle {}\n        }"),
            "{result}"
        );
        assert!(result[start..].starts_with("Rectangle {}"));
    }

    #[test]
    fn removes_elements_with_their_lines() {
        let outline = Outline::parse(TEXT);
        let result = remove_element(TEXT, element(&outline, "Text")).apply(TEXT);
        assert_eq!(
            result,
            "export component Main inherits Window {\n    width: 100px;\n    VerticalLayout {\n    }\n}\n"
        );
    }

    #[test]
    fn imports_are_extended_or_added() {
        let with_import = "import { Button } from \"std-widgets.slint\";\ncomponent A {}";
        assert_eq!(
            ensure_import(with_import, "Button", "std-widgets.slint"),
            None
        );
        let result = ensure_import(with_import, "Slider", "std-widgets.slint")
            .unwrap()
            .apply(with_import);
        assert!(
            result.starts_with("import { Button, Slider } from \"std-widgets.slint\";"),
            "{result}"
        );

        let result = ensure_import(TEXT, "Button", "std-widgets.slint")
            .unwrap()
            .apply(TEXT);
        assert!(
            result.starts_with("import { Button } from \"std-widgets.slint\";\n\nexport component"),
            "{result}"
        );

        let other = "import { Card } from \"card.slint\";\ncomponent A {}";
        let result = ensure_import(other, "Button", "std-widgets.slint")
            .unwrap()
            .apply(other);
        assert!(result.starts_with("import { Card } from \"card.slint\";\nimport { Button } from \"std-widgets.slint\";\n"), "{result}");
    }
}
