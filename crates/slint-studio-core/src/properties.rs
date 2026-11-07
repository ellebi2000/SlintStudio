//! Catalogue of the properties shown by the property editor, per element
//! type, and helpers to edit their values.

/// Section of the property editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PropertyGroup {
    Geometry,
    Appearance,
    Behavior,
}

impl PropertyGroup {
    pub const ALL: [Self; 3] = [Self::Geometry, Self::Appearance, Self::Behavior];

    pub fn label(self) -> &'static str {
        match self {
            Self::Geometry => "Geometria",
            Self::Appearance => "Aspetto",
            Self::Behavior => "Comportamento",
        }
    }
}

/// How a value is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// `100px`, `50%`, `parent.width - 10px`
    Length,
    /// Colour or gradient (`#4ec9b0`, `red`, `@linear-gradient(…)`).
    Brush,
    /// String literal, edited without quotes.
    Text,
    Number,
    Bool,
    Enum(&'static [&'static str]),
    Image,
    /// Anything else, edited as raw Slint.
    Expression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertySpec {
    pub name: &'static str,
    pub group: PropertyGroup,
    pub kind: ValueKind,
    /// Example value shown in empty fields.
    pub hint: &'static str,
}

const fn spec(
    name: &'static str,
    group: PropertyGroup,
    kind: ValueKind,
    hint: &'static str,
) -> PropertySpec {
    PropertySpec {
        name,
        group,
        kind,
        hint,
    }
}

use PropertyGroup::{Appearance, Behavior, Geometry};
use ValueKind::{Bool, Brush, Enum, Expression, Image, Length, Number, Text};

const H_ALIGN: &[&str] = &["left", "center", "right"];
const V_ALIGN: &[&str] = &["top", "center", "bottom"];
const LAYOUT_ALIGN: &[&str] = &[
    "stretch",
    "start",
    "center",
    "end",
    "space-between",
    "space-around",
    "space-evenly",
];
const WRAP: &[&str] = &["no-wrap", "word-wrap", "char-wrap"];
const OVERFLOW: &[&str] = &["clip", "elide"];
const IMAGE_FIT: &[&str] = &["fill", "contain", "cover", "preserve"];

const GEOMETRY: &[PropertySpec] = &[
    spec("x", Geometry, Length, "0px"),
    spec("y", Geometry, Length, "0px"),
    spec("width", Geometry, Length, "100px"),
    spec("height", Geometry, Length, "100px"),
    spec("preferred-width", Geometry, Length, "100px"),
    spec("preferred-height", Geometry, Length, "100px"),
    spec("min-width", Geometry, Length, "0px"),
    spec("min-height", Geometry, Length, "0px"),
];

const COMMON_BEHAVIOR: &[PropertySpec] = &[spec("visible", Behavior, Bool, "true")];

const RECTANGLE: &[PropertySpec] = &[
    spec("background", Appearance, Brush, "#4ec9b0"),
    spec("border-color", Appearance, Brush, "#3c3c3c"),
    spec("border-width", Appearance, Length, "1px"),
    spec("border-radius", Appearance, Length, "4px"),
    spec("opacity", Appearance, Number, "1.0"),
    spec("clip", Behavior, Bool, "false"),
];

const WINDOW: &[PropertySpec] = &[
    spec("title", Appearance, Text, "Titolo"),
    spec("background", Appearance, Brush, "#ffffff"),
    spec("default-font-size", Appearance, Length, "14px"),
    spec("no-frame", Behavior, Bool, "false"),
];

const TEXT: &[PropertySpec] = &[
    spec("text", Appearance, Text, "Testo"),
    spec("color", Appearance, Brush, "#000000"),
    spec("font-size", Appearance, Length, "14px"),
    spec("font-weight", Appearance, Number, "400"),
    spec("font-family", Appearance, Text, "Inter"),
    spec("horizontal-alignment", Appearance, Enum(H_ALIGN), "left"),
    spec("vertical-alignment", Appearance, Enum(V_ALIGN), "top"),
    spec("wrap", Behavior, Enum(WRAP), "no-wrap"),
    spec("overflow", Behavior, Enum(OVERFLOW), "clip"),
];

const IMAGE: &[PropertySpec] = &[
    spec("source", Appearance, Image, "@image-url(\"icon.png\")"),
    spec("image-fit", Appearance, Enum(IMAGE_FIT), "contain"),
    spec("colorize", Appearance, Brush, "#ffffff"),
];

const TOUCH_AREA: &[PropertySpec] = &[
    spec("enabled", Behavior, Bool, "true"),
    spec("mouse-cursor", Behavior, Expression, "pointer"),
];

const LAYOUT: &[PropertySpec] = &[
    spec("padding", Geometry, Length, "8px"),
    spec("spacing", Geometry, Length, "8px"),
    spec("alignment", Appearance, Enum(LAYOUT_ALIGN), "stretch"),
];

const BUTTON: &[PropertySpec] = &[
    spec("text", Appearance, Text, "Pulsante"),
    spec("primary", Appearance, Bool, "false"),
    spec("enabled", Behavior, Bool, "true"),
    spec("checkable", Behavior, Bool, "false"),
];

const CHECKABLE: &[PropertySpec] = &[
    spec("text", Appearance, Text, "Opzione"),
    spec("checked", Behavior, Bool, "false"),
    spec("enabled", Behavior, Bool, "true"),
];

const LINE_EDIT: &[PropertySpec] = &[
    spec("text", Appearance, Text, ""),
    spec("placeholder-text", Appearance, Text, "Testo"),
    spec("font-size", Appearance, Length, "14px"),
    spec("enabled", Behavior, Bool, "true"),
    spec("read-only", Behavior, Bool, "false"),
];

const RANGE: &[PropertySpec] = &[
    spec("value", Behavior, Number, "0"),
    spec("minimum", Behavior, Number, "0"),
    spec("maximum", Behavior, Number, "100"),
    spec("enabled", Behavior, Bool, "true"),
];

const COMBO_BOX: &[PropertySpec] = &[
    spec("model", Behavior, Expression, "[\"Uno\", \"Due\"]"),
    spec("current-index", Behavior, Number, "0"),
    spec("enabled", Behavior, Bool, "true"),
];

const PROGRESS: &[PropertySpec] = &[
    spec("progress", Behavior, Number, "0.5"),
    spec("indeterminate", Behavior, Bool, "false"),
];

const GROUP_BOX: &[PropertySpec] = &[
    spec("title", Appearance, Text, "Gruppo"),
    spec("enabled", Behavior, Bool, "true"),
];

/// Properties offered for an element of type `type_name`, in display order.
/// Bindings not listed here are shown separately.
pub fn properties_for(type_name: &str) -> Vec<PropertySpec> {
    let specific: &[PropertySpec] = match type_name {
        "Rectangle" | "BorderRectangle" => RECTANGLE,
        "Window" | "Dialog" => WINDOW,
        "Text" => TEXT,
        "Image" => IMAGE,
        "TouchArea" => TOUCH_AREA,
        "HorizontalLayout" | "VerticalLayout" | "GridLayout" | "HorizontalBox" | "VerticalBox"
        | "GridBox" => LAYOUT,
        "Button" => BUTTON,
        "CheckBox" | "Switch" => CHECKABLE,
        "LineEdit" | "TextEdit" => LINE_EDIT,
        "Slider" | "SpinBox" => RANGE,
        "ComboBox" => COMBO_BOX,
        "ProgressIndicator" => PROGRESS,
        "GroupBox" => GROUP_BOX,
        _ => &[],
    };
    let mut all: Vec<PropertySpec> = GEOMETRY.to_vec();
    for spec in specific {
        if let Some(existing) = all.iter_mut().find(|s| s.name == spec.name) {
            *existing = *spec;
        } else {
            all.push(*spec);
        }
    }
    for spec in COMMON_BEHAVIOR {
        if !all.iter().any(|s| s.name == spec.name) {
            all.push(*spec);
        }
    }
    all
}

/// Whether elements of this type may contain child elements.
pub fn accepts_children(type_name: &str) -> bool {
    !matches!(
        type_name,
        "Text"
            | "Image"
            | "Path"
            | "TextInput"
            | "Button"
            | "CheckBox"
            | "Switch"
            | "LineEdit"
            | "TextEdit"
            | "SpinBox"
            | "Slider"
            | "ComboBox"
            | "ProgressIndicator"
            | "Spinner"
            | "StandardButton"
    )
}

/// Whether children of this type are placed by a layout (so `x`/`y` are
/// ignored).
pub fn is_layout(type_name: &str) -> bool {
    type_name.ends_with("Layout")
        || matches!(type_name, "HorizontalBox" | "VerticalBox" | "GridBox")
}

/// Parses `#rgb`, `#rgba`, `#rrggbb` and `#rrggbbaa` into RGBA bytes.
pub fn parse_hex_color(value: &str) -> Option<[u8; 4]> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|v| v * 17);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => Some([nibble(0)?, nibble(1)?, nibble(2)?, 255]),
        4 => Some([nibble(0)?, nibble(1)?, nibble(2)?, nibble(3)?]),
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn format_hex_color([r, g, b, a]: [u8; 4]) -> String {
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// The content of a plain string literal (`"abc"` → `abc`), or `None` for
/// any other expression (including interpolated strings).
pub fn unquote(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return None,
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                _ => return None, // `\{…}` interpolation or unknown escape
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// A Slint string literal for `text`.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_geometry_and_visibility() {
        for type_name in ["Rectangle", "Text", "Button", "MyComponent", ""] {
            let properties = properties_for(type_name);
            for name in ["x", "y", "width", "height", "visible"] {
                assert!(
                    properties.iter().any(|p| p.name == name),
                    "{type_name}: {name}"
                );
            }
            let mut names: Vec<_> = properties.iter().map(|p| p.name).collect();
            names.sort_unstable();
            let count = names.len();
            names.dedup();
            assert_eq!(names.len(), count, "duplicates for {type_name}");
        }
    }

    #[test]
    fn specific_properties_by_type() {
        let text = properties_for("Text");
        let color = text.iter().find(|p| p.name == "color").unwrap();
        assert_eq!((color.group, color.kind), (Appearance, Brush));
        assert!(
            properties_for("VerticalBox")
                .iter()
                .any(|p| p.name == "spacing")
        );
        assert!(
            !properties_for("Text")
                .iter()
                .any(|p| p.name == "background")
        );
    }

    #[test]
    fn containers_and_layouts() {
        assert!(accepts_children("Rectangle"));
        assert!(accepts_children("MyCard"));
        assert!(!accepts_children("Button"));
        assert!(is_layout("VerticalLayout") && is_layout("HorizontalBox"));
        assert!(!is_layout("Rectangle"));
    }

    #[test]
    fn hex_colours() {
        assert_eq!(parse_hex_color("#4ec9b0"), Some([0x4e, 0xc9, 0xb0, 255]));
        assert_eq!(parse_hex_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_hex_color(" #00000080 "), Some([0, 0, 0, 0x80]));
        assert_eq!(parse_hex_color("red"), None);
        assert_eq!(parse_hex_color("#12345"), None);
        assert_eq!(format_hex_color([0x4e, 0xc9, 0xb0, 255]), "#4ec9b0");
        assert_eq!(format_hex_color([0, 0, 0, 0x80]), "#00000080");
    }

    #[test]
    fn string_literals() {
        assert_eq!(unquote("\"Ciao\"").as_deref(), Some("Ciao"));
        assert_eq!(unquote(r#""a \"b\"\n""#).as_deref(), Some("a \"b\"\n"));
        assert_eq!(unquote(r#""\{root.counter}""#), None);
        assert_eq!(unquote("root.title"), None);
        for text in ["Ciao", "a \"b\" \\ c", "riga\nnuova", ""] {
            assert_eq!(unquote(&quote(text)).as_deref(), Some(text));
        }
    }
}
