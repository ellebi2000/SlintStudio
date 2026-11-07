//! Output of build and run processes, as shown in the "Terminale" panel.
//!
//! Processes write text with ANSI escape sequences (cargo and rustc colour
//! their output). [`Terminal`] keeps a bounded list of lines split into
//! styled spans: SGR sequences (colours, bold, …) become [`AnsiStyle`]s and
//! every other control sequence is dropped. The current style carries over
//! from one line to the next, separately for each stream, as in a real
//! terminal.

use std::collections::VecDeque;

/// Lines kept by default; older lines are dropped.
pub const DEFAULT_MAX_LINES: usize = 20_000;

/// Longer lines are cut, so a runaway program cannot stall the UI.
pub const MAX_LINE_CHARS: usize = 8_000;

const TAB_WIDTH: usize = 8;

/// Where a line comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalStream {
    Stdout,
    Stderr,
    /// Messages from Slint Studio itself ("▶ cargo run", exit status, …).
    System,
}

impl TerminalStream {
    const fn index(self) -> usize {
        match self {
            Self::Stdout => 0,
            Self::Stderr => 1,
            Self::System => 2,
        }
    }
}

/// A terminal colour: one of the 256 xterm colours (0–15 are the themable
/// ANSI colours) or a 24-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnsiColor {
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl AnsiColor {
    /// The RGB value of an xterm colour above 15 (the 6×6×6 cube and the
    /// grey ramp). The first 16 depend on the theme and return `None`.
    pub fn xterm_rgb(index: u8) -> Option<(u8, u8, u8)> {
        match index {
            0..=15 => None,
            16..=231 => {
                let i = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Some((level(i / 36), level((i / 6) % 6), level(i % 6)))
            }
            232..=255 => {
                let v = 8 + (index - 232) * 10;
                Some((v, v, v))
            }
        }
    }
}

/// Visual attributes of a span of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AnsiStyle {
    pub foreground: Option<AnsiColor>,
    pub background: Option<AnsiColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

impl AnsiStyle {
    /// Applies the parameters of an SGR (`ESC [ … m`) sequence.
    fn apply_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            *self = Self::default();
            return;
        }
        let mut iter = params.iter().copied();
        while let Some(code) = iter.next() {
            match code {
                0 => *self = Self::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                30..=37 => self.foreground = Some(AnsiColor::Indexed((code - 30) as u8)),
                38 => self.foreground = extended_color(&mut iter),
                39 => self.foreground = None,
                40..=47 => self.background = Some(AnsiColor::Indexed((code - 40) as u8)),
                48 => self.background = extended_color(&mut iter),
                49 => self.background = None,
                90..=97 => self.foreground = Some(AnsiColor::Indexed((code - 90 + 8) as u8)),
                100..=107 => self.background = Some(AnsiColor::Indexed((code - 100 + 8) as u8)),
                _ => {}
            }
        }
    }
}

/// Reads the rest of `38;5;n` or `38;2;r;g;b`.
fn extended_color(iter: &mut impl Iterator<Item = u16>) -> Option<AnsiColor> {
    let byte = |v: u16| u8::try_from(v).unwrap_or(u8::MAX);
    match iter.next()? {
        5 => iter.next().map(|n| AnsiColor::Indexed(byte(n))),
        2 => {
            let r = iter.next()?;
            let g = iter.next()?;
            let b = iter.next()?;
            Some(AnsiColor::Rgb(byte(r), byte(g), byte(b)))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSpan {
    pub text: String,
    pub style: AnsiStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLine {
    pub stream: TerminalStream,
    pub spans: Vec<TerminalSpan>,
}

impl TerminalLine {
    /// The text without styling.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

/// Bounded scrollback of process output.
#[derive(Debug, Clone)]
pub struct Terminal {
    lines: VecDeque<TerminalLine>,
    max_lines: usize,
    styles: [AnsiStyle; 3],
    generation: u64,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_LINES)
    }
}

impl Terminal {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            max_lines: max_lines.max(1),
            styles: [AnsiStyle::default(); 3],
            generation: 0,
        }
    }

    /// Appends output. `text` is normally one line without its terminator;
    /// embedded newlines start new lines.
    pub fn push(&mut self, stream: TerminalStream, text: &str) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        for raw in text.split('\n') {
            let style = &mut self.styles[stream.index()];
            let spans = parse_line(raw, style);
            self.lines.push_back(TerminalLine { stream, spans });
        }
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
        }
        self.generation += 1;
    }

    /// Appends a line with an explicit style (for Slint Studio's own messages).
    pub fn push_styled(&mut self, stream: TerminalStream, text: &str, style: AnsiStyle) {
        let spans = vec![TerminalSpan {
            text: sanitize(text),
            style,
        }];
        self.lines.push_back(TerminalLine { stream, spans });
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
        }
        self.generation += 1;
    }

    /// Forgets the styles left open by the previous process.
    pub fn reset_styles(&mut self) {
        self.styles = [AnsiStyle::default(); 3];
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.reset_styles();
        self.generation += 1;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line(&self, index: usize) -> Option<&TerminalLine> {
        self.lines.get(index)
    }

    pub fn lines(&self) -> impl ExactSizeIterator<Item = &TerminalLine> + DoubleEndedIterator {
        self.lines.iter()
    }

    /// Changes whenever the content changes, so views can cache layouts.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The whole content as plain text, e.g. for copying.
    pub fn plain_text(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            text.push_str(&line.text());
            text.push('\n');
        }
        text
    }
}

/// Removes every ANSI escape sequence and control character from `text`.
pub fn strip_ansi(text: &str) -> String {
    let mut style = AnsiStyle::default();
    text.split('\n')
        .map(|line| {
            parse_line(line, &mut style)
                .into_iter()
                .map(|span| span.text)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect()
}

/// Splits one line into styled spans, updating `style` with the SGR
/// sequences it contains.
pub fn parse_line(raw: &str, style: &mut AnsiStyle) -> Vec<TerminalSpan> {
    let raw = raw.strip_suffix('\r').unwrap_or(raw);
    let mut spans: Vec<TerminalSpan> = Vec::new();
    let mut current = String::new();
    let mut column = 0;
    let mut truncated = false;

    let flush = |spans: &mut Vec<TerminalSpan>, current: &mut String, style: AnsiStyle| {
        if current.is_empty() {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(current),
            _ => spans.push(TerminalSpan {
                text: current.clone(),
                style,
            }),
        }
        current.clear();
    };

    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    // CSI: parameters, intermediates, then a final byte.
                    let mut params = String::new();
                    let mut final_byte = None;
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            final_byte = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    if final_byte == Some('m') {
                        flush(&mut spans, &mut current, *style);
                        let values: Vec<u16> = if params.is_empty() {
                            Vec::new()
                        } else {
                            params
                                .split([';', ':'])
                                .map(|p| p.parse().unwrap_or(0))
                                .collect()
                        };
                        style.apply_sgr(&values);
                    }
                }
                Some(']') => {
                    // OSC (e.g. hyperlinks): up to BEL or ESC \.
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // `ESC ( B` and similar: intermediates, then a final byte.
                Some(c) if (' '..='/').contains(&c) => {
                    for c in chars.by_ref() {
                        if !(' '..='/').contains(&c) {
                            break;
                        }
                    }
                }
                // Two-character sequences: nothing to show.
                _ => {}
            },
            '\r' => {
                // Carriage return: the rest of the line overwrites it.
                spans.clear();
                current.clear();
                column = 0;
                truncated = false;
            }
            '\t' => {
                let spaces = TAB_WIDTH - column % TAB_WIDTH;
                current.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            c if c.is_control() => {}
            c => {
                if column < MAX_LINE_CHARS {
                    current.push(c);
                    column += 1;
                } else if !truncated {
                    current.push('…');
                    truncated = true;
                }
            }
        }
    }
    flush(&mut spans, &mut current, *style);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> AnsiStyle {
        AnsiStyle {
            foreground: Some(AnsiColor::Indexed(1)),
            ..AnsiStyle::default()
        }
    }

    fn spans(raw: &str) -> Vec<(String, AnsiStyle)> {
        parse_line(raw, &mut AnsiStyle::default())
            .into_iter()
            .map(|span| (span.text, span.style))
            .collect()
    }

    #[test]
    fn plain_text_is_one_span() {
        assert_eq!(
            spans("hello world"),
            vec![("hello world".to_owned(), AnsiStyle::default())]
        );
        assert!(spans("").is_empty());
    }

    #[test]
    fn sgr_colours_and_reset() {
        let result = spans("a\u{1b}[31mred\u{1b}[0m b");
        assert_eq!(
            result,
            vec![
                ("a".to_owned(), AnsiStyle::default()),
                ("red".to_owned(), red()),
                (" b".to_owned(), AnsiStyle::default()),
            ]
        );
    }

    #[test]
    fn cargo_style_bold_bright_green() {
        // `   Compiling` as printed by cargo with --color always.
        let result = spans("\u{1b}[1m\u{1b}[92m   Compiling\u{1b}[0m hello v0.1.0");
        let style = AnsiStyle {
            foreground: Some(AnsiColor::Indexed(10)),
            bold: true,
            ..AnsiStyle::default()
        };
        assert_eq!(result[0], ("   Compiling".to_owned(), style));
        assert_eq!(result[1].0, " hello v0.1.0");
        assert_eq!(result[1].1, AnsiStyle::default());
    }

    #[test]
    fn extended_colours() {
        let mut style = AnsiStyle::default();
        parse_line("\u{1b}[38;5;196;48;2;1;2;3m", &mut style);
        assert_eq!(style.foreground, Some(AnsiColor::Indexed(196)));
        assert_eq!(style.background, Some(AnsiColor::Rgb(1, 2, 3)));
        parse_line("\u{1b}[39;49;1;2;3;4m", &mut style);
        assert_eq!(style.foreground, None);
        assert_eq!(style.background, None);
        assert!(style.bold && style.dim && style.italic && style.underline);
        parse_line("\u{1b}[22;23;24m", &mut style);
        assert_eq!(style, AnsiStyle::default());
        // A truncated sequence does not panic.
        parse_line("\u{1b}[38;2;1m", &mut style);
        parse_line("\u{1b}[", &mut style);
    }

    #[test]
    fn xterm_palette() {
        assert_eq!(AnsiColor::xterm_rgb(1), None);
        assert_eq!(AnsiColor::xterm_rgb(16), Some((0, 0, 0)));
        assert_eq!(AnsiColor::xterm_rgb(196), Some((255, 0, 0)));
        assert_eq!(AnsiColor::xterm_rgb(231), Some((255, 255, 255)));
        assert_eq!(AnsiColor::xterm_rgb(232), Some((8, 8, 8)));
        assert_eq!(AnsiColor::xterm_rgb(255), Some((238, 238, 238)));
    }

    #[test]
    fn other_sequences_and_controls_are_dropped() {
        assert_eq!(strip_ansi("a\u{1b}[2Kb\u{1b}[1;1Hc"), "abc");
        let link = "\u{1b}]8;;https://slint.dev\u{1b}\\Slint\u{1b}]8;;\u{7} ok";
        assert_eq!(strip_ansi(link), "Slint ok");
        assert_eq!(strip_ansi("bell\u{7}\u{8}"), "bell");
        assert_eq!(strip_ansi("x\u{1b}(By"), "xy");
    }

    #[test]
    fn carriage_return_overwrites_and_crlf_is_a_terminator() {
        assert_eq!(strip_ansi("progress 10%\rprogress 100%"), "progress 100%");
        assert_eq!(strip_ansi("windows line\r"), "windows line");
    }

    #[test]
    fn tabs_expand_to_columns() {
        assert_eq!(strip_ansi("a\tb"), "a       b");
        assert_eq!(strip_ansi("abcdefgh\tb"), "abcdefgh        b");
    }

    #[test]
    fn long_lines_are_truncated() {
        let line = "x".repeat(MAX_LINE_CHARS + 50);
        let text = strip_ansi(&line);
        assert_eq!(text.chars().count(), MAX_LINE_CHARS + 1);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn style_carries_over_lines_per_stream() {
        let mut terminal = Terminal::default();
        terminal.push(TerminalStream::Stderr, "\u{1b}[31mstart");
        terminal.push(TerminalStream::Stdout, "plain");
        terminal.push(TerminalStream::Stderr, "still red\u{1b}[0m");
        assert_eq!(
            terminal.line(1).unwrap().spans[0].style,
            AnsiStyle::default()
        );
        assert_eq!(terminal.line(2).unwrap().spans[0].style, red());

        terminal.push(TerminalStream::Stdout, "\u{1b}[31mopen");
        terminal.reset_styles();
        terminal.push(TerminalStream::Stdout, "reset");
        assert_eq!(
            terminal.line(4).unwrap().spans[0].style,
            AnsiStyle::default()
        );
    }

    #[test]
    fn terminal_is_bounded_and_tracks_changes() {
        let mut terminal = Terminal::new(3);
        let start = terminal.generation();
        for i in 0..5 {
            terminal.push(TerminalStream::Stdout, &format!("line {i}"));
        }
        assert_eq!(terminal.len(), 3);
        assert_eq!(terminal.line(0).unwrap().text(), "line 2");
        assert!(terminal.generation() > start);
        assert_eq!(terminal.plain_text(), "line 2\nline 3\nline 4\n");

        terminal.push(TerminalStream::Stdout, "a\nb\n");
        assert_eq!(terminal.line(2).unwrap().text(), "b");
        terminal.push_styled(TerminalStream::System, "▶ cargo\u{7} run", red());
        let last = terminal.lines().last().unwrap();
        assert_eq!(last.stream, TerminalStream::System);
        assert_eq!(last.text(), "▶ cargo run");

        let before = terminal.generation();
        terminal.clear();
        assert!(terminal.is_empty());
        assert_ne!(terminal.generation(), before);
    }
}
