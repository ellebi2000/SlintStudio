//! Text positions: byte offsets, line/column pairs and their encodings.
//!
//! Three coordinate systems meet in an editor talking to a language server:
//! - **byte offsets** into the Rust `String` (what we store and edit);
//! - **char indices** (what egui's text cursor uses);
//! - **line/column pairs**, whose column unit depends on the
//!   [`PositionEncoding`] negotiated with the language server (UTF-8 bytes or
//!   UTF-16 code units) or, for display, Unicode scalar values.
//!
//! [`LineIndex`] converts between them. All lines and columns here are
//! **0-based**; user-facing 1-based numbers are produced at the UI edge.

use std::ops::Range;

/// Unit in which the column of a line/column position is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PositionEncoding {
    /// UTF-8 code units (bytes).
    Utf8,
    /// UTF-16 code units: the LSP default.
    #[default]
    Utf16,
    /// Unicode scalar values (Rust `char`s): what users and egui see.
    Chars,
}

/// A 0-based line/column pair; the column unit depends on the encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct LineCol {
    pub line: u32,
    pub column: u32,
}

impl LineCol {
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

/// Start offsets of every line of a text, for fast position conversions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    /// Byte offset at which each line starts; always contains at least `0`.
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self {
            line_starts,
            len: text.len(),
        }
    }

    /// Number of lines; a trailing newline starts a final, empty line.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte range of `line`, excluding its line terminator (`\n` or `\r\n`).
    pub fn line_range(&self, text: &str, line: usize) -> Option<Range<usize>> {
        let start = *self.line_starts.get(line)?;
        let mut end = self
            .line_starts
            .get(line + 1)
            .map_or(self.len, |next| next - 1);
        if end > start && text.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some(start..end)
    }

    /// Line containing the byte `offset` (clamped to the text).
    pub fn line_of(&self, offset: usize) -> usize {
        let offset = offset.min(self.len);
        match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        }
    }

    /// Converts a position to a byte offset.
    ///
    /// Out-of-range positions are clamped: a line past the end maps to the
    /// end of the text, a column past the end of its line to the end of that
    /// line, and a column inside a multi-byte character to its start.
    pub fn offset(&self, text: &str, position: LineCol, encoding: PositionEncoding) -> usize {
        let Some(range) = self.line_range(text, position.line as usize) else {
            return self.len;
        };
        let line = &text[range.clone()];
        let column = position.column as usize;
        let within = match encoding {
            PositionEncoding::Utf8 => {
                let mut column = column.min(line.len());
                while !line.is_char_boundary(column) {
                    column -= 1;
                }
                column
            }
            PositionEncoding::Utf16 => {
                let mut units = 0;
                line.char_indices()
                    .find(|(_, c)| {
                        units += c.len_utf16();
                        units > column
                    })
                    .map_or(line.len(), |(i, _)| i)
            }
            PositionEncoding::Chars => line
                .char_indices()
                .nth(column)
                .map_or(line.len(), |(i, _)| i),
        };
        range.start + within
    }

    /// Converts a byte offset (clamped, snapped back to a char boundary) to a
    /// position.
    pub fn position(&self, text: &str, offset: usize, encoding: PositionEncoding) -> LineCol {
        let mut offset = offset.min(self.len);
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.line_of(offset);
        let before = &text[self.line_starts[line]..offset];
        let column = match encoding {
            PositionEncoding::Utf8 => before.len(),
            PositionEncoding::Utf16 => before.chars().map(char::len_utf16).sum(),
            PositionEncoding::Chars => before.chars().count(),
        };
        LineCol::new(to_u32(line), to_u32(column))
    }

    /// Re-expresses `position` in another encoding.
    pub fn convert(
        &self,
        text: &str,
        position: LineCol,
        from: PositionEncoding,
        to: PositionEncoding,
    ) -> LineCol {
        if from == to {
            return position;
        }
        self.position(text, self.offset(text, position, from), to)
    }
}

/// Byte offset of the `char_index`-th character (clamped to the text length).
pub fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(i, _)| i)
}

/// Number of characters before the byte `offset` (clamped).
pub fn byte_to_char(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].chars().count()
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PositionEncoding::{Chars, Utf8, Utf16};

    // "è" is 2 bytes / 1 UTF-16 unit, "😀" is 4 bytes / 2 UTF-16 units.
    const TEXT: &str = "abc\nè😀x\r\n\nlast";

    #[test]
    fn counts_lines_including_trailing_empty_line() {
        assert_eq!(LineIndex::new("").line_count(), 1);
        assert_eq!(LineIndex::new("a\n").line_count(), 2);
        assert_eq!(LineIndex::new(TEXT).line_count(), 4);
    }

    #[test]
    fn line_ranges_exclude_line_terminators() {
        let index = LineIndex::new(TEXT);
        let lines: Vec<_> = (0..index.line_count())
            .map(|l| &TEXT[index.line_range(TEXT, l).unwrap()])
            .collect();
        assert_eq!(lines, ["abc", "è😀x", "", "last"]);
        assert!(index.line_range(TEXT, 4).is_none());
    }

    #[test]
    fn column_units_depend_on_encoding() {
        let index = LineIndex::new(TEXT);
        let x = TEXT.find('x').unwrap();
        assert_eq!(index.position(TEXT, x, Utf8), LineCol::new(1, 6));
        assert_eq!(index.position(TEXT, x, Utf16), LineCol::new(1, 3));
        assert_eq!(index.position(TEXT, x, Chars), LineCol::new(1, 2));
        for encoding in [Utf8, Utf16, Chars] {
            let position = index.position(TEXT, x, encoding);
            assert_eq!(index.offset(TEXT, position, encoding), x, "{encoding:?}");
        }
    }

    #[test]
    fn offsets_are_clamped_and_snapped_to_char_boundaries() {
        let index = LineIndex::new(TEXT);
        let line1 = index.line_range(TEXT, 1).unwrap();
        // Past the end of a line: end of that line, before "\r\n".
        assert_eq!(index.offset(TEXT, LineCol::new(1, 99), Chars), line1.end);
        // Inside the 2-byte "è": snapped back to its start.
        assert_eq!(index.offset(TEXT, LineCol::new(1, 1), Utf8), line1.start);
        // Inside the surrogate pair of "😀": the emoji itself.
        let emoji = TEXT.find('😀').unwrap();
        assert_eq!(index.offset(TEXT, LineCol::new(1, 2), Utf16), emoji);
        // Line past the end: end of text.
        assert_eq!(index.offset(TEXT, LineCol::new(42, 0), Utf16), TEXT.len());
    }

    #[test]
    fn convert_between_encodings() {
        let index = LineIndex::new(TEXT);
        let utf8 = LineCol::new(1, 6);
        assert_eq!(index.convert(TEXT, utf8, Utf8, Chars), LineCol::new(1, 2));
        assert_eq!(index.convert(TEXT, utf8, Utf8, Utf16), LineCol::new(1, 3));
    }

    #[test]
    fn char_and_byte_indices_round_trip() {
        for (char_index, (byte, _)) in TEXT.char_indices().enumerate() {
            assert_eq!(char_to_byte(TEXT, char_index), byte);
            assert_eq!(byte_to_char(TEXT, byte), char_index);
        }
        assert_eq!(char_to_byte(TEXT, 10_000), TEXT.len());
        // Inside "è": counts as before it.
        let e = TEXT.find('è').unwrap();
        assert_eq!(byte_to_char(TEXT, e + 1), byte_to_char(TEXT, e));
    }

    #[test]
    fn line_of_offsets() {
        let index = LineIndex::new(TEXT);
        assert_eq!(index.line_of(0), 0);
        assert_eq!(index.line_of(3), 0); // the '\n' belongs to line 0
        assert_eq!(index.line_of(4), 1);
        assert_eq!(index.line_of(usize::MAX), 3);
    }
}
