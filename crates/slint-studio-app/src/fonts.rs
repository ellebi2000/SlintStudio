//! Font setup: Inter for the UI and JetBrains Mono for code when they are
//! installed on the system, Phosphor icons in a dedicated font family.
//!
//! Fonts are looked up at runtime rather than bundled, falling back to a
//! common system monospace font and finally to egui's built-in fonts.

use egui::{FontData, FontDefinitions, FontFamily};

/// UI font candidates, by family name. Falls back to egui's default font.
const UI_FAMILIES: &[&str] = &["Inter", "Inter Variable"];

/// Code font candidates: JetBrains Mono first, then common system monospace
/// fonts. Falls back to egui's built-in monospace font (Hack).
const CODE_FAMILIES: &[&str] = &[
    "JetBrains Mono",
    "Cascadia Mono",
    "SF Mono",
    "Menlo",
    "Consolas",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
];

const ICON_FONT: &str = "phosphor";

/// Font family that resolves glyphs from the Phosphor icon font first.
pub fn icon_family() -> FontFamily {
    FontFamily::Name(ICON_FONT.into())
}

/// Font families actually chosen, for logging and the About dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontChoice {
    /// `None` means egui's built-in proportional font.
    pub ui: Option<String>,
    /// `None` means egui's built-in monospace font.
    pub code: Option<String>,
}

/// Installs the font definitions into `ctx` and reports what was chosen.
pub fn install(ctx: &egui::Context) -> FontChoice {
    let mut fonts = FontDefinitions::default();

    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut choice = FontChoice::default();
    if let Some((family, data)) = find_first(&db, UI_FAMILIES) {
        prepend(&mut fonts, FontFamily::Proportional, "ui-font", data);
        choice.ui = Some(family);
    }
    if let Some((family, data)) = find_first(&db, CODE_FAMILIES) {
        prepend(&mut fonts, FontFamily::Monospace, "code-font", data);
        choice.code = Some(family);
    }

    // Icons live in their own family, never in the text fallback chain: text
    // fonts such as Inter define glyphs in the same Private Use Area as
    // Phosphor and would otherwise shadow the icons.
    fonts.font_data.insert(
        ICON_FONT.to_owned(),
        FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );
    // Text fonts follow only to provide egui's replacement glyph ('◻' / '?').
    let mut icon_fonts = vec![ICON_FONT.to_owned()];
    icon_fonts.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts.families.insert(icon_family(), icon_fonts);

    ctx.set_fonts(fonts);
    choice
}

fn find_first(db: &fontdb::Database, families: &[&str]) -> Option<(String, FontData)> {
    families.iter().find_map(|&family| {
        let id = db.query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            ..fontdb::Query::default()
        })?;
        let data = db.with_face_data(id, |bytes, index| {
            let mut data = FontData::from_owned(bytes.to_vec());
            data.index = index;
            data
        })?;
        Some((family.to_owned(), data))
    })
}

fn prepend(fonts: &mut FontDefinitions, family: FontFamily, key: &str, data: FontData) {
    fonts.font_data.insert(key.to_owned(), data.into());
    fonts
        .families
        .entry(family)
        .or_default()
        .insert(0, key.to_owned());
}
