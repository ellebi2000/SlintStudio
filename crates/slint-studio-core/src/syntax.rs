//! A small, error-tolerant lexer for the Slint language, used for syntax
//! highlighting.
//!
//! It never fails: every byte of the input belongs to exactly one token, and
//! malformed input (unterminated strings or comments) simply extends to the
//! end of the text. Semantic accuracy is the language server's job; this lexer
//! only needs to be fast and stable while the user types.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    Comment,
    String,
    /// Number literal, including its unit (`10px`, `50%`, `250ms`).
    Number,
    /// Colour literal (`#ff8800`).
    Color,
    Keyword,
    /// `@image-url`, `@linear-gradient`, `@tr`, `@children`, …
    AtKeyword,
    /// Capitalised identifier: an element or type name (`Rectangle`).
    TypeName,
    /// Identifier followed by `:`, i.e. a property binding (`width: 10px`).
    Property,
    Identifier,
    Operator,
    Punctuation,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range in the source text.
    pub range: Range<usize>,
}

const KEYWORDS: &[&str] = &[
    "animate",
    "as",
    "callback",
    "changed",
    "component",
    "else",
    "enum",
    "export",
    "false",
    "for",
    "from",
    "function",
    "global",
    "if",
    "import",
    "in",
    "in-out",
    "inherits",
    "out",
    "parent",
    "private",
    "property",
    "protected",
    "public",
    "pure",
    "return",
    "root",
    "self",
    "states",
    "struct",
    "transitions",
    "true",
    "when",
];

/// Multi-character operators, longest first.
const OPERATORS: &[&str] = &[
    "<=>", ":=", "=>", "->", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=",
];

pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// Splits `text` into tokens covering it completely, in order.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut lexer = Lexer { text, pos: 0 };
    let mut tokens = Vec::new();
    while lexer.pos < text.len() {
        let start = lexer.pos;
        let kind = lexer.next_kind();
        debug_assert!(lexer.pos > start, "lexer made no progress");
        tokens.push(Token {
            kind,
            range: start..lexer.pos,
        });
    }
    tokens
}

struct Lexer<'a> {
    text: &'a str,
    pos: usize,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.text[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.bump();
        }
    }

    fn next_kind(&mut self) -> TokenKind {
        let Some(c) = self.peek() else {
            return TokenKind::Unknown;
        };
        let rest = self.rest();

        if c.is_whitespace() {
            self.eat_while(char::is_whitespace);
            return TokenKind::Whitespace;
        }
        if rest.starts_with("//") {
            self.eat_while(|c| c != '\n');
            return TokenKind::Comment;
        }
        if rest.starts_with("/*") {
            self.block_comment();
            return TokenKind::Comment;
        }
        if c == '"' {
            self.string();
            return TokenKind::String;
        }
        if c.is_ascii_digit() || (c == '.' && self.peek_nth(1).is_some_and(|d| d.is_ascii_digit()))
        {
            self.number();
            return TokenKind::Number;
        }
        if c == '#' {
            self.bump();
            self.eat_while(|c| c.is_ascii_alphanumeric());
            return TokenKind::Color;
        }
        if c == '@' && self.peek_nth(1).is_some_and(is_identifier_start) {
            self.bump();
            self.eat_while(is_identifier_continue);
            return TokenKind::AtKeyword;
        }
        if is_identifier_start(c) {
            let start = self.pos;
            self.identifier();
            return self.classify_identifier(&self.text[start..self.pos]);
        }
        if let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) {
            self.pos += op.len();
            return TokenKind::Operator;
        }
        self.bump();
        match c {
            '{' | '}' | '(' | ')' | '[' | ']' | ';' | ',' | '.' => TokenKind::Punctuation,
            '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '?' | ':' | '&' | '|' => {
                TokenKind::Operator
            }
            _ => TokenKind::Unknown,
        }
    }

    /// Block comments nest, so `/* a /* b */ c */` is a single comment.
    fn block_comment(&mut self) {
        self.pos += 2;
        let mut depth = 1;
        while depth > 0 && self.pos < self.text.len() {
            let rest = self.rest();
            if rest.starts_with("/*") {
                depth += 1;
                self.pos += 2;
            } else if rest.starts_with("*/") {
                depth -= 1;
                self.pos += 2;
            } else {
                self.bump();
            }
        }
    }

    /// Strings may contain `\{ expression }` interpolations, whose braces (and
    /// any quotes inside nested strings) must not end the string.
    fn string(&mut self) {
        self.bump(); // opening quote
        while let Some(c) = self.bump() {
            match c {
                '"' => return,
                '\\' if self.peek() == Some('{') => {
                    self.bump();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.bump() {
                            Some('{') => depth += 1,
                            Some('}') => depth -= 1,
                            Some(_) => {}
                            None => return,
                        }
                    }
                }
                '\\' => {
                    self.bump();
                }
                _ => {}
            }
        }
    }

    fn number(&mut self) {
        self.eat_while(|c| c.is_ascii_digit());
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|d| d.is_ascii_digit()) {
            self.bump();
            self.eat_while(|c| c.is_ascii_digit());
        }
        // Unit suffix: px, phx, rem, ms, s, deg, rad, turn, %, …
        if self.peek() == Some('%') {
            self.bump();
        } else {
            self.eat_while(|c| c.is_ascii_alphabetic());
        }
    }

    fn identifier(&mut self) {
        self.bump();
        while let Some(c) = self.peek() {
            // A '-' belongs to the identifier (Slint uses kebab-case) only when
            // followed by another identifier character: `a-b` vs `a - b`, `a-1`.
            let continues = is_identifier_continue(c)
                && (c != '-'
                    || self
                        .peek_nth(1)
                        .is_some_and(|n| n.is_alphanumeric() || n == '_'));
            if !continues {
                break;
            }
            self.bump();
        }
    }

    fn classify_identifier(&self, word: &str) -> TokenKind {
        if is_keyword(word) {
            return TokenKind::Keyword;
        }
        if word.starts_with(|c: char| c.is_uppercase()) {
            return TokenKind::TypeName;
        }
        // `name: value` is a binding; `name := Element` declares an element id.
        let after = self.rest().trim_start_matches([' ', '\t']);
        if after.starts_with(':') && !after.starts_with(":=") {
            TokenKind::Property
        } else {
            TokenKind::Identifier
        }
    }
}

/// Names of the elements and components enclosing the byte `offset`,
/// outermost first: `component Main {`, `Card {` and `ok := Button {` give
/// `["Main", "Card", "Button (ok)"]`. Other blocks (callbacks, `if`, `for`
/// bodies, `animate`, …) are anonymous and omitted.
pub fn scope_path(text: &str, offset: usize) -> Vec<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut recent: Vec<(TokenKind, &str)> = Vec::new();
    for token in tokenize(text) {
        if token.range.start >= offset {
            break;
        }
        let slice = &text[token.range];
        match token.kind {
            TokenKind::Whitespace | TokenKind::Comment => {}
            TokenKind::Punctuation if slice == "{" => {
                stack.push(block_label(&recent));
                recent.clear();
            }
            TokenKind::Punctuation if slice == "}" => {
                stack.pop();
                recent.clear();
            }
            TokenKind::Punctuation if slice == ";" => recent.clear(),
            kind => recent.push((kind, slice)),
        }
    }
    stack.into_iter().flatten().collect()
}

/// Label of a block from the significant tokens preceding its `{`.
fn block_label(tokens: &[(TokenKind, &str)]) -> Option<String> {
    use TokenKind::{Identifier, Keyword, Operator, Property, TypeName};
    if let Some(i) = tokens.iter().position(|(kind, s)| {
        *kind == Keyword && matches!(*s, "component" | "struct" | "global" | "enum")
    }) {
        return tokens.get(i + 1).map(|(_, name)| (*name).to_owned());
    }
    match tokens {
        [
            ..,
            (Identifier | Property, id),
            (Operator, ":="),
            (TypeName, ty),
        ] => Some(format!("{ty} ({id})")),
        [.., (_, prev), (TypeName, ty)] if !matches!(*prev, "=>" | "->") => Some((*ty).to_owned()),
        [(TypeName, ty)] => Some((*ty).to_owned()),
        _ => None,
    }
}

fn is_identifier_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_identifier_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    /// Tokens without whitespace, as (kind, text) pairs.
    fn lex(text: &str) -> Vec<(TokenKind, &str)> {
        tokenize(text)
            .into_iter()
            .filter(|t| t.kind != Whitespace)
            .map(|t| (t.kind, &text[t.range]))
            .collect()
    }

    #[test]
    fn tokens_cover_the_whole_input() {
        let text = "export component A inherits Window { /* é */ w: 1px; \"x\\{a}\" }\n";
        let tokens = tokenize(text);
        let mut pos = 0;
        for token in &tokens {
            assert_eq!(token.range.start, pos);
            pos = token.range.end;
        }
        assert_eq!(pos, text.len());
    }

    #[test]
    fn component_declaration() {
        assert_eq!(
            lex("export component MainWindow inherits Window {"),
            [
                (Keyword, "export"),
                (Keyword, "component"),
                (TypeName, "MainWindow"),
                (Keyword, "inherits"),
                (TypeName, "Window"),
                (Punctuation, "{"),
            ]
        );
    }

    #[test]
    fn properties_values_and_units() {
        assert_eq!(
            lex("preferred-width: 10.5px; opacity: 50%; background: #ff8800;"),
            [
                (Property, "preferred-width"),
                (Operator, ":"),
                (Number, "10.5px"),
                (Punctuation, ";"),
                (Property, "opacity"),
                (Operator, ":"),
                (Number, "50%"),
                (Punctuation, ";"),
                (Property, "background"),
                (Operator, ":"),
                (Color, "#ff8800"),
                (Punctuation, ";"),
            ]
        );
    }

    #[test]
    fn property_declarations_and_bindings() {
        assert_eq!(
            lex("in-out property <int> count: 0; b <=> a; btn := Button {}"),
            [
                (Keyword, "in-out"),
                (Keyword, "property"),
                (Operator, "<"),
                (Identifier, "int"),
                (Operator, ">"),
                (Property, "count"),
                (Operator, ":"),
                (Number, "0"),
                (Punctuation, ";"),
                (Identifier, "b"),
                (Operator, "<=>"),
                (Identifier, "a"),
                (Punctuation, ";"),
                (Identifier, "btn"),
                (Operator, ":="),
                (TypeName, "Button"),
                (Punctuation, "{"),
                (Punctuation, "}"),
            ]
        );
    }

    #[test]
    fn dash_is_part_of_identifiers_only_between_identifier_chars() {
        assert_eq!(lex("a-b")[0], (Identifier, "a-b"));
        assert_eq!(
            lex("a - b"),
            [(Identifier, "a"), (Operator, "-"), (Identifier, "b")]
        );
        assert_eq!(lex("x-")[0], (Identifier, "x"));
    }

    #[test]
    fn comments_nest_and_tolerate_missing_end() {
        assert_eq!(
            lex("/* a /* b */ c */ x"),
            [(Comment, "/* a /* b */ c */"), (Identifier, "x")]
        );
        assert_eq!(lex("// line\nx"), [(Comment, "// line"), (Identifier, "x")]);
        assert_eq!(lex("/* open"), [(Comment, "/* open")]);
    }

    #[test]
    fn strings_with_escapes_and_interpolation() {
        assert_eq!(
            lex(r#""a\"b" x"#),
            [(String, r#""a\"b""#), (Identifier, "x")]
        );
        assert_eq!(
            lex(r#""n: \{ count + 1 } {" y"#),
            [(String, r#""n: \{ count + 1 } {""#), (Identifier, "y")]
        );
        assert_eq!(lex(r#""unterminated"#), [(String, r#""unterminated"#)]);
    }

    #[test]
    fn at_keywords_and_callbacks() {
        assert_eq!(
            lex("source: @image-url(\"a.png\"); clicked => { root.n += 1; }"),
            [
                (Property, "source"),
                (Operator, ":"),
                (AtKeyword, "@image-url"),
                (Punctuation, "("),
                (String, "\"a.png\""),
                (Punctuation, ")"),
                (Punctuation, ";"),
                (Identifier, "clicked"),
                (Operator, "=>"),
                (Punctuation, "{"),
                (Keyword, "root"),
                (Punctuation, "."),
                (Identifier, "n"),
                (Operator, "+="),
                (Number, "1"),
                (Punctuation, ";"),
                (Punctuation, "}"),
            ]
        );
    }

    #[test]
    fn scope_path_follows_nested_elements() {
        let text = "export component Main inherits Window {\n    Card {\n        ok := Button {\n            clicked => { root.x = 1; }\n        }\n        if cond : Text { }\n    }\n}\n";
        let at = |needle: &str| scope_path(text, text.find(needle).unwrap());
        assert_eq!(at("Card"), ["Main"]);
        assert_eq!(at("ok :="), ["Main", "Card"]);
        assert_eq!(at("clicked"), ["Main", "Card", "Button (ok)"]);
        // Inside the anonymous callback block, the element is still the scope.
        assert_eq!(at("root.x"), ["Main", "Card", "Button (ok)"]);
        assert_eq!(at("if cond"), ["Main", "Card"]);
        assert!(scope_path(text, text.len()).is_empty());
    }

    #[test]
    fn scope_path_of_structs_and_globals() {
        let text = "struct Point { x: int }\nglobal Theme { out property <color> c: red; }";
        assert_eq!(scope_path(text, text.find("x:").unwrap()), ["Point"]);
        assert_eq!(scope_path(text, text.find("out").unwrap()), ["Theme"]);
    }

    #[test]
    fn non_ascii_text_is_handled() {
        let tokens = lex("Text { text: \"perché ✓\"; } ¤");
        assert_eq!(tokens[4], (String, "\"perché ✓\""));
        assert_eq!(tokens.last(), Some(&(Unknown, "¤")));
    }
}
