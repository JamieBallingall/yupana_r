//! Splits a `.yup` source into tokens.
//!
//! The lexer counts brackets so that a newline inside `(` or `[` does not end a statement. That
//! lets a long expression run over several lines without an offside rule.

use crate::diagnostic::{Diagnostic, Pos};

/// One token of a `.yup` source.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A name that starts with a lower-case letter: a binding, a function or a keyword.
    Name(String),
    /// A word that starts with an upper-case letter, used at the type level (`Input`, `Size`).
    Word(String),
    /// A number in JSON's grammar, without a sign. The text is kept so that a printed file
    /// shows the number as it was written.
    Number(f64, String),
    /// A string value, in double quotes, with its escapes decoded.
    Str(String),
    /// A label or a sheet name, in single quotes, with doubled quotes decoded.
    Text(String),
    /// A comment, without its `//`.
    Comment(String),
    /// The end of a statement: a newline outside brackets.
    Newline,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Colon,
    Equals,
    At,
    Plus,
    PlusPlus,
    Minus,
    Star,
    Slash,
    Caret,
    DotDot,
    EqEq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Eof,
}

impl Token {
    /// How the token reads in a message.
    pub fn describe(&self) -> String {
        let symbol = match self {
            Token::Name(name) | Token::Word(name) => return format!("`{name}`"),
            Token::Number(_, text) => return format!("the number `{text}`"),
            Token::Str(_) => return "a string".to_string(),
            Token::Text(_) => return "a quoted label".to_string(),
            Token::Comment(_) => return "a comment".to_string(),
            Token::Newline => return "the end of the line".to_string(),
            Token::Eof => return "the end of the file".to_string(),
            Token::LParen => "(",
            Token::RParen => ")",
            Token::LBracket => "[",
            Token::RBracket => "]",
            Token::Comma => ",",
            Token::Semicolon => ";",
            Token::Colon => ":",
            Token::Equals => "=",
            Token::At => "@",
            Token::Plus => "+",
            Token::PlusPlus => "++",
            Token::Minus => "-",
            Token::Star => "*",
            Token::Slash => "/",
            Token::Caret => "^",
            Token::DotDot => "..",
            Token::EqEq => "==",
            Token::NotEq => "/=",
            Token::Less => "<",
            Token::LessEq => "<=",
            Token::Greater => ">",
            Token::GreaterEq => ">=",
        };
        format!("`{symbol}`")
    }
}

/// A token and where it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Lexeme {
    pub token: Token,
    pub pos: Pos,
}

/// Splits `source` into tokens, ending with [`Token::Eof`]. The last statement always ends with
/// a [`Token::Newline`], whether or not the file does.
pub fn lex(source: &str) -> Result<Vec<Lexeme>, Diagnostic> {
    Lexer::new(source).run()
}

/// Whether a character can be written into a workbook. The XML the workbook is made of leaves
/// out most control characters, and the code points U+FFFE and U+FFFF.
pub fn writable(c: char) -> bool {
    matches!(c, '\t' | '\n') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')
}

struct Lexer {
    chars: Vec<char>,
    next: usize,
    line: u32,
    column: u32,
    /// The brackets still open, innermost last. While any is open, a newline does not end a
    /// statement.
    open: Vec<(char, Pos)>,
    out: Vec<Lexeme>,
}

impl Lexer {
    fn new(source: &str) -> Lexer {
        // Some editors start a file with a byte-order mark, which carries no meaning here.
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        Lexer {
            chars: source.chars().collect(),
            next: 0,
            line: 1,
            column: 1,
            open: Vec::new(),
            out: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.next).copied()
    }

    fn peek_second(&self) -> Option<char> {
        self.chars.get(self.next + 1).copied()
    }

    fn pos(&self) -> Pos {
        Pos::new(self.line, self.column)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.next += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn push(&mut self, token: Token, pos: Pos) {
        self.out.push(Lexeme { token, pos });
    }

    fn run(mut self) -> Result<Vec<Lexeme>, Diagnostic> {
        while let Some(c) = self.peek() {
            let pos = self.pos();
            match c {
                // A carriage return is whitespace, so that files with CRLF line endings read
                // the same as files with LF.
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => {
                    self.bump();
                    if self.open.is_empty() {
                        self.push(Token::Newline, pos);
                    }
                }
                '/' if self.peek_second() == Some('/') => self.comment(pos),
                'a'..='z' | 'A'..='Z' | '_' => self.name(pos)?,
                '0'..='9' => self.number(pos)?,
                '"' => self.string(pos)?,
                '\'' => self.text(pos)?,
                _ => self.symbol(c, pos)?,
            }
        }
        if let Some(&(bracket, pos)) = self.open.last() {
            return Err(Diagnostic::error(
                pos,
                format!("this `{bracket}` is never closed"),
            ));
        }
        let pos = self.pos();
        if self.out.last().is_some_and(|l| l.token != Token::Newline) {
            self.push(Token::Newline, pos);
        }
        self.push(Token::Eof, pos);
        Ok(self.out)
    }

    fn comment(&mut self, pos: Pos) {
        self.bump();
        self.bump();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.bump();
            text.push(c);
        }
        // A comment inside brackets sits in the middle of an expression, which the syntax tree
        // has no place for, so it is dropped.
        if self.open.is_empty() {
            let text = text.trim_end_matches('\r').to_string();
            self.push(Token::Comment(text), pos);
        }
    }

    fn name(&mut self, pos: Pos) -> Result<(), Diagnostic> {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                break;
            }
            self.bump();
            text.push(c);
        }
        if text.starts_with('_') {
            return Err(Diagnostic::error(
                pos,
                format!("`{text}` starts with `_`; a name starts with a letter"),
            ));
        }
        if text.starts_with(|c: char| c.is_ascii_uppercase()) {
            self.push(Token::Word(text), pos);
        } else if text.contains(|c: char| c.is_ascii_uppercase()) {
            return Err(Diagnostic::error(
                pos,
                format!(
                    "`{text}` mixes upper and lower case; names are lower case, with words \
                     separated by `_`"
                ),
            ));
        } else {
            self.push(Token::Name(text), pos);
        }
        Ok(())
    }

    fn digits(&mut self, text: &mut String) {
        while let Some(c) = self.peek().filter(char::is_ascii_digit) {
            self.bump();
            text.push(c);
        }
    }

    fn number(&mut self, pos: Pos) -> Result<(), Diagnostic> {
        let mut text = String::new();
        let first = self.bump().expect("the caller saw a digit");
        text.push(first);
        if first == '0' {
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(Diagnostic::error(
                    pos,
                    "a number starts with `0` only if it is zero or a fraction, as in JSON",
                ));
            }
        } else {
            self.digits(&mut text);
        }
        // `1..n` is a range, so a `.` followed by another `.` ends the number.
        if self.peek() == Some('.') && self.peek_second() != Some('.') {
            self.bump();
            text.push('.');
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(Diagnostic::error(
                    self.pos(),
                    "a digit must follow the decimal point",
                ));
            }
            self.digits(&mut text);
        }
        if let Some(e @ ('e' | 'E')) = self.peek() {
            self.bump();
            text.push(e);
            if let Some(sign @ ('+' | '-')) = self.peek() {
                self.bump();
                text.push(sign);
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(Diagnostic::error(
                    self.pos(),
                    "a digit must follow the exponent",
                ));
            }
            self.digits(&mut text);
        }
        if let Some(c) = self.peek()
            && (c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(Diagnostic::error(
                self.pos(),
                format!("unexpected `{c}` after the number `{text}`"),
            ));
        }
        let value: f64 = text
            .parse()
            .expect("JSON's number grammar is part of the grammar Rust parses");
        if !value.is_finite() {
            return Err(Diagnostic::error(
                pos,
                format!("the number `{text}` is too large"),
            ));
        }
        self.push(Token::Number(value, text), pos);
        Ok(())
    }

    fn string(&mut self, pos: Pos) -> Result<(), Diagnostic> {
        self.bump();
        let mut value = String::new();
        loop {
            let here = self.pos();
            match self.bump() {
                None | Some('\n') => {
                    return Err(Diagnostic::error(pos, "this string has no closing `\"`"));
                }
                Some('"') => break,
                Some('\\') => {
                    let c = match self.bump() {
                        Some('"') => '"',
                        Some('\\') => '\\',
                        Some('/') => '/',
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('u') => self.unicode_escape(here)?,
                        _ => {
                            return Err(Diagnostic::error(
                                here,
                                "unknown escape; a string allows \\\", \\\\, \\/, \\n, \\t and \
                                 \\uXXXX",
                            ));
                        }
                    };
                    value.push(c);
                }
                Some(c) if !writable(c) => return Err(unwritable(c, here)),
                Some(c) => value.push(c),
            }
        }
        self.push(Token::Str(value), pos);
        Ok(())
    }

    fn hex4(&mut self, at: Pos) -> Result<u32, Diagnostic> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = self
                .bump()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| Diagnostic::error(at, "`\\u` takes four hexadecimal digits"))?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn unicode_escape(&mut self, at: Pos) -> Result<char, Diagnostic> {
        let first = self.hex4(at)?;
        // Characters beyond the first plane are written as a pair of surrogates, as in JSON.
        let code = if (0xD800..0xDC00).contains(&first) {
            if self.bump() != Some('\\') || self.bump() != Some('u') {
                return Err(Diagnostic::error(
                    at,
                    "a high surrogate is followed by a low one, `\\uDC00` to `\\uDFFF`",
                ));
            }
            let second = self.hex4(at)?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(Diagnostic::error(
                    at,
                    "a high surrogate is followed by a low one, `\\uDC00` to `\\uDFFF`",
                ));
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        let c = char::from_u32(code)
            .ok_or_else(|| Diagnostic::error(at, "`\\u` names a lone surrogate"))?;
        if writable(c) {
            Ok(c)
        } else {
            Err(unwritable(c, at))
        }
    }

    fn text(&mut self, pos: Pos) -> Result<(), Diagnostic> {
        self.bump();
        let mut value = String::new();
        loop {
            let here = self.pos();
            match self.bump() {
                None | Some('\n') => {
                    return Err(Diagnostic::error(pos, "this label has no closing `'`"));
                }
                Some('\'') if self.peek() == Some('\'') => {
                    self.bump();
                    value.push('\'');
                }
                Some('\'') => break,
                Some(c) if !writable(c) => return Err(unwritable(c, here)),
                Some(c) => value.push(c),
            }
        }
        self.push(Token::Text(value), pos);
        Ok(())
    }

    fn symbol(&mut self, c: char, pos: Pos) -> Result<(), Diagnostic> {
        self.bump();
        let next = self.peek();
        let mut two = |token| {
            self.bump();
            token
        };
        let token = match c {
            '(' => Token::LParen,
            ')' => Token::RParen,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            ':' => Token::Colon,
            '@' => Token::At,
            '*' => Token::Star,
            '^' => Token::Caret,
            '-' => Token::Minus,
            '+' if next == Some('+') => two(Token::PlusPlus),
            '+' => Token::Plus,
            '/' if next == Some('=') => two(Token::NotEq),
            '/' => Token::Slash,
            '.' if next == Some('.') => two(Token::DotDot),
            '=' if next == Some('=') => two(Token::EqEq),
            '=' => Token::Equals,
            '<' if next == Some('=') => two(Token::LessEq),
            '<' if next == Some('>') => {
                return Err(Diagnostic::error(
                    pos,
                    "`<>` is the spreadsheet app's; in .yup, not-equal is `/=`",
                ));
            }
            '<' => Token::Less,
            '>' if next == Some('=') => two(Token::GreaterEq),
            '>' => Token::Greater,
            '.' => {
                return Err(Diagnostic::error(
                    pos,
                    "a `.` on its own; a range of whole numbers is written `a .. b`",
                ));
            }
            '!' => {
                return Err(Diagnostic::error(
                    pos,
                    "`!` means nothing in .yup; not-equal is `/=`, and formulas name bindings, \
                     not sheets",
                ));
            }
            '#' => {
                return Err(Diagnostic::error(
                    pos,
                    "`#` does not start a comment in .yup; comments start with `//`",
                ));
            }
            '&' => {
                return Err(Diagnostic::error(
                    pos,
                    "`&` is the spreadsheet app's; in .yup, text is joined with `++`",
                ));
            }
            '{' | '}' => {
                return Err(Diagnostic::error(
                    pos,
                    "a list of values is written with `[` and `]`, rows separated by `;`",
                ));
            }
            c if c.is_alphabetic() => {
                return Err(Diagnostic::error(
                    pos,
                    format!("`{c}`: names are ASCII; any character can go in a quoted label"),
                ));
            }
            c => {
                return Err(Diagnostic::error(
                    pos,
                    format!("unexpected character `{c}`"),
                ));
            }
        };
        match token {
            Token::LParen | Token::LBracket => self.open.push((c, pos)),
            Token::RParen | Token::RBracket => {
                let opening = if c == ')' { '(' } else { '[' };
                match self.open.pop() {
                    Some((open, _)) if open == opening => {}
                    Some((open, at)) => {
                        return Err(Diagnostic::error(
                            pos,
                            format!("this `{c}` does not match the `{open}` at {at}"),
                        ));
                    }
                    None => {
                        return Err(Diagnostic::error(
                            pos,
                            format!("this `{c}` has no matching `{opening}`"),
                        ));
                    }
                }
            }
            _ => {}
        }
        self.push(token, pos);
        Ok(())
    }
}

fn unwritable(c: char, pos: Pos) -> Diagnostic {
    Diagnostic::error(
        pos,
        format!(
            "the character U+{:04X} cannot be written to a workbook",
            u32::from(c)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        lex(source)
            .expect("the source lexes")
            .into_iter()
            .map(|l| l.token)
            .collect()
    }

    fn name(text: &str) -> Token {
        Token::Name(text.to_string())
    }

    fn error(source: &str) -> String {
        lex(source).expect_err("the source is wrong").to_string()
    }

    #[test]
    fn positions_count_lines_and_characters() {
        let lexed = lex("yup 0.1\nlabel : 'Café' Input").expect("lexes");
        let positions: Vec<String> = lexed.iter().map(|l| l.pos.to_string()).collect();
        assert_eq!(
            positions,
            [
                "1:1", "1:5", "1:8", "2:1", "2:7", "2:9", "2:16", "2:21", "2:21"
            ]
        );
    }

    #[test]
    fn a_range_is_not_a_fraction() {
        assert_eq!(
            tokens("1..n"),
            [
                Token::Number(1.0, "1".to_string()),
                Token::DotDot,
                name("n"),
                Token::Newline,
                Token::Eof
            ]
        );
    }

    #[test]
    fn numbers_follow_json() {
        assert_eq!(
            tokens("1.5e3")[0],
            Token::Number(1500.0, "1.5e3".to_string())
        );
        assert_eq!(
            error("x = 007"),
            "1:5: error: a number starts with `0` only if it is zero or a fraction, as in JSON"
        );
        assert_eq!(
            error("1."),
            "1:3: error: a digit must follow the decimal point"
        );
        assert_eq!(
            error("2n"),
            "1:2: error: unexpected `n` after the number `2`"
        );
    }

    #[test]
    fn strings_and_labels_decode_their_escapes() {
        assert_eq!(tokens(r#""a\"bé\n""#)[0], Token::Str("a\"bé\n".to_string()));
        assert_eq!(
            tokens("'Prior year''s revenue'")[0],
            Token::Text("Prior year's revenue".to_string())
        );
        assert_eq!(tokens(r#""😀""#)[0], Token::Str("😀".to_string()));
        assert_eq!(error("'open"), "1:1: error: this label has no closing `'`");
        assert_eq!(
            error(r#""\u0007""#),
            "1:2: error: the character U+0007 cannot be written to a workbook"
        );
    }

    #[test]
    fn a_newline_inside_brackets_does_not_end_a_statement() {
        assert_eq!(
            tokens("f (a\n + b) // kept\n[1 // dropped\n]"),
            [
                name("f"),
                Token::LParen,
                name("a"),
                Token::Plus,
                name("b"),
                Token::RParen,
                Token::Comment(" kept".to_string()),
                Token::Newline,
                Token::LBracket,
                Token::Number(1.0, "1".to_string()),
                Token::RBracket,
                Token::Newline,
                Token::Eof
            ]
        );
    }

    #[test]
    fn names_are_lower_case_and_words_upper() {
        assert_eq!(tokens("Input")[0], Token::Word("Input".to_string()));
        assert_eq!(
            error("fooBar"),
            "1:1: error: `fooBar` mixes upper and lower case; names are lower case, with words \
             separated by `_`"
        );
    }

    #[test]
    fn the_spreadsheet_apps_spellings_get_a_hint() {
        assert_eq!(
            error("a <> b"),
            "1:3: error: `<>` is the spreadsheet app's; in .yup, not-equal is `/=`"
        );
        assert_eq!(
            error("a & b"),
            "1:3: error: `&` is the spreadsheet app's; in .yup, text is joined with `++`"
        );
    }

    #[test]
    fn brackets_are_matched() {
        assert_eq!(
            error("x = (1\n+ 2\n"),
            "1:5: error: this `(` is never closed"
        );
        assert_eq!(
            error("x = (1]"),
            "1:7: error: this `]` does not match the `(` at 1:5"
        );
        assert_eq!(error("x = 1)"), "1:6: error: this `)` has no matching `(`");
    }

    #[test]
    fn crlf_reads_as_lf() {
        assert_eq!(tokens("a\r\nb"), tokens("a\nb"));
    }
}
