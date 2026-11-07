//! Syntax highlighting: turns Slint tokens into a coloured egui `LayoutJob`.

use egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontId};
use slint_studio_core::Language;
use slint_studio_core::syntax::{self, TokenKind};

/// Keywords that affect control flow or animation, coloured differently from
/// declarations (as in VS Code's Dark+ theme).
const CONTROL_KEYWORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "return",
    "animate",
    "states",
    "transitions",
    "when",
    "changed",
];

/// Colours of the code editor (VS Code Dark+).
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct CodeTheme {
    pub font: FontId,
    pub text: Color32,
    pub keyword: Color32,
    pub control: Color32,
    pub type_name: Color32,
    pub property: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
}

impl CodeTheme {
    pub fn dark(font: FontId) -> Self {
        Self {
            font,
            text: Color32::from_rgb(0xD4, 0xD4, 0xD4),
            keyword: Color32::from_rgb(0x56, 0x9C, 0xD6),
            control: Color32::from_rgb(0xC5, 0x86, 0xC0),
            type_name: Color32::from_rgb(0x4E, 0xC9, 0xB0),
            property: Color32::from_rgb(0x9C, 0xDC, 0xFE),
            string: Color32::from_rgb(0xCE, 0x91, 0x78),
            number: Color32::from_rgb(0xB5, 0xCE, 0xA8),
            comment: Color32::from_rgb(0x6A, 0x99, 0x55),
        }
    }

    pub fn color(&self, kind: TokenKind, text: &str) -> Color32 {
        match kind {
            TokenKind::Keyword if CONTROL_KEYWORDS.contains(&text) => self.control,
            TokenKind::Keyword => self.keyword,
            TokenKind::AtKeyword => self.control,
            TokenKind::TypeName => self.type_name,
            TokenKind::Property => self.property,
            TokenKind::String | TokenKind::Color => self.string,
            TokenKind::Number => self.number,
            TokenKind::Comment => self.comment,
            TokenKind::Identifier
            | TokenKind::Operator
            | TokenKind::Punctuation
            | TokenKind::Whitespace
            | TokenKind::Unknown => self.text,
        }
    }
}

/// Builds the layout job for `text` (uncached; see [`highlighted`]).
pub fn layout_job(theme: &CodeTheme, text: &str, language: Language) -> LayoutJob {
    let section = |range: std::ops::Range<usize>, color| LayoutSection {
        leading_space: 0.0,
        byte_range: ByteIndex(range.start)..ByteIndex(range.end),
        format: TextFormat::simple(theme.font.clone(), color),
    };
    let sections = match language {
        Language::PlainText => vec![section(0..text.len(), theme.text)],
        Language::Slint => {
            let mut sections: Vec<LayoutSection> = Vec::new();
            for token in syntax::tokenize(text) {
                let color = theme.color(token.kind, &text[token.range.clone()]);
                // Merge runs of the same colour, and whitespace into whatever
                // precedes it, to keep the job small.
                match sections.last_mut() {
                    Some(last) if token.kind == TokenKind::Whitespace => {
                        last.byte_range.end = ByteIndex(token.range.end);
                    }
                    Some(last)
                        if last.format.color == color
                            && last.byte_range.end.0 == token.range.start =>
                    {
                        last.byte_range.end = ByteIndex(token.range.end);
                    }
                    _ => sections.push(section(token.range, color)),
                }
            }
            sections
        }
    };
    LayoutJob {
        text: text.to_owned(),
        sections,
        ..Default::default()
    }
}

#[derive(Default)]
struct Highlighter;

impl egui::cache::ComputerMut<(&CodeTheme, &str, Language), LayoutJob> for Highlighter {
    fn compute(&mut self, (theme, text, language): (&CodeTheme, &str, Language)) -> LayoutJob {
        layout_job(theme, text, language)
    }
}

type HighlightCache = egui::cache::FrameCache<LayoutJob, Highlighter>;

/// Cached [`layout_job`]: the text is only re-tokenised when it changes.
pub fn highlighted(
    ctx: &egui::Context,
    theme: &CodeTheme,
    text: &str,
    language: Language,
) -> LayoutJob {
    ctx.memory_mut(|memory| {
        memory
            .caches
            .cache::<HighlightCache>()
            .get((theme, text, language))
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> CodeTheme {
        CodeTheme::dark(FontId::monospace(13.0))
    }

    #[test]
    fn sections_cover_the_text_and_merge_same_colours() {
        let text = "export component A inherits Window {\n    width: 10px; // c\n}\n";
        let job = layout_job(&theme(), text, Language::Slint);
        let mut end = 0;
        for section in &job.sections {
            assert_eq!(section.byte_range.start.0, end);
            end = section.byte_range.end.0;
        }
        assert_eq!(end, text.len());
        // "export component" (keyword, space, keyword) is a single section.
        assert_eq!(
            &text[job.sections[0].byte_range.start.0..job.sections[0].byte_range.end.0],
            "export component "
        );
    }

    #[test]
    fn colours_follow_token_kinds() {
        let theme = theme();
        let text = "Rectangle { background: #fff; }";
        let job = layout_job(&theme, text, Language::Slint);
        let color_of = |needle: &str| {
            let at = text.find(needle).unwrap();
            job.sections
                .iter()
                .find(|s| (s.byte_range.start.0..s.byte_range.end.0).contains(&at))
                .unwrap()
                .format
                .color
        };
        assert_eq!(color_of("Rectangle"), theme.type_name);
        assert_eq!(color_of("background"), theme.property);
        assert_eq!(color_of("#fff"), theme.string);
        assert_eq!(theme.color(TokenKind::Keyword, "if"), theme.control);
    }

    #[test]
    fn plain_text_is_a_single_section() {
        let job = layout_job(&theme(), "fn main() {}", Language::PlainText);
        assert_eq!(job.sections.len(), 1);
    }
}
