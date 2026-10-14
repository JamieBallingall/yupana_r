//! Reads an input file in CSV: a header row of column names, then one row per record.
//!
//! The dialect is RFC 4180's: fields separated by commas, records by line breaks (LF or CRLF),
//! and a field in double quotes may hold commas, line breaks and doubled quotes. An unquoted
//! field is typed: empty is blank, a number in JSON's grammar is a number, `TRUE` and `FALSE` in
//! any case are logicals, and anything else is text. A quoted field is always text, so that
//! `"007"` keeps its zeros.

use crate::diagnostic::{Diagnostic, Pos};
use crate::lexer::writable;

/// One value read from a file.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Empty,
    Number(f64),
    Bool(bool),
    Text(String),
}

/// A file's column names and its records, every record as long as the header.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<Field>>,
}

/// A field as read, before it is typed.
struct Raw {
    text: String,
    quoted: bool,
    pos: Pos,
}

/// The most characters a cell can hold, from the spreadsheet app's published limits.
const MAX_CELL_TEXT: usize = 32_767;

/// Reads a CSV file. Errors carry their line and column in the file.
pub fn parse(text: &str) -> Result<Table, Diagnostic> {
    let records = split(text)?;
    let mut records = records.into_iter();
    let Some(header) = records.next() else {
        return Err(Diagnostic::error(
            Pos::START,
            "the file is empty; it needs a header row of column names",
        ));
    };
    let mut names: Vec<String> = Vec::new();
    for field in &header {
        if field.text.is_empty() {
            return Err(Diagnostic::error(
                field.pos,
                "a column name cannot be empty",
            ));
        }
        if names.contains(&field.text) {
            return Err(Diagnostic::error(
                field.pos,
                format!("the column name `{}` is given twice", field.text),
            ));
        }
        names.push(field.text.clone());
    }
    let mut rows = Vec::new();
    for record in records {
        if record.len() != names.len() {
            let fields = |n: usize| format!("{n} field{}", if n == 1 { "" } else { "s" });
            return Err(Diagnostic::error(
                record[0].pos,
                format!(
                    "this row has {}, and the header has {}",
                    fields(record.len()),
                    fields(names.len())
                ),
            ));
        }
        rows.push(record.into_iter().map(typed).collect());
    }
    Ok(Table {
        header: names,
        rows,
    })
}

fn typed(raw: Raw) -> Field {
    if raw.quoted {
        return if raw.text.is_empty() {
            Field::Empty
        } else {
            Field::Text(raw.text)
        };
    }
    if raw.text.is_empty() {
        Field::Empty
    } else if let Some(number) = json_number(&raw.text) {
        Field::Number(number)
    } else if raw.text.eq_ignore_ascii_case("true") {
        Field::Bool(true)
    } else if raw.text.eq_ignore_ascii_case("false") {
        Field::Bool(false)
    } else {
        Field::Text(raw.text)
    }
}

/// The value of a number in JSON's grammar, sign included, if the text is one.
fn json_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let start = *i;
        while bytes.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i > start
    };
    if bytes.get(i) == Some(&b'0') {
        i += 1;
    } else if !digits(&mut i) {
        return None;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return None;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return None;
        }
    }
    if i != bytes.len() {
        return None;
    }
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Ends a record. A blank line, read as one empty field, is not a record.
fn finish(records: &mut Vec<Vec<Raw>>, record: &mut Vec<Raw>) {
    let record = std::mem::take(record);
    let blank = record.len() == 1 && record[0].text.is_empty() && !record[0].quoted;
    if !blank {
        records.push(record);
    }
}

/// Splits the text into records of raw fields.
fn split(text: &str) -> Result<Vec<Vec<Raw>>, Diagnostic> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let chars: Vec<char> = text.chars().collect();
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut i = 0;
    let (mut line, mut column) = (1u32, 1u32);
    loop {
        let pos = Pos::new(line, column);
        let mut field = Raw {
            text: String::new(),
            quoted: false,
            pos,
        };
        if chars.get(i) == Some(&'"') {
            field.quoted = true;
            i += 1;
            column += 1;
            loop {
                match chars.get(i) {
                    None => {
                        return Err(Diagnostic::error(pos, "this quoted field is never closed"));
                    }
                    Some('"') if chars.get(i + 1) == Some(&'"') => {
                        field.text.push('"');
                        i += 2;
                        column += 2;
                    }
                    Some('"') => {
                        i += 1;
                        column += 1;
                        break;
                    }
                    // A line break inside quotes is kept, as LF whichever way it was written.
                    Some('\r') if chars.get(i + 1) == Some(&'\n') => {
                        field.text.push('\n');
                        i += 2;
                        line += 1;
                        column = 1;
                    }
                    Some('\n') => {
                        field.text.push('\n');
                        i += 1;
                        line += 1;
                        column = 1;
                    }
                    Some(&c) => {
                        field.text.push(c);
                        i += 1;
                        column += 1;
                    }
                }
            }
        } else {
            while let Some(&c) = chars.get(i) {
                if matches!(c, ',' | '\n' | '\r') {
                    break;
                }
                if c == '"' {
                    return Err(Diagnostic::error(
                        Pos::new(line, column),
                        "a quote inside a field needs the whole field in quotes, with the quote \
                         doubled",
                    ));
                }
                field.text.push(c);
                i += 1;
                column += 1;
            }
        }
        if let Some(c) = field.text.chars().find(|&c| !writable(c)) {
            return Err(Diagnostic::error(
                pos,
                format!(
                    "the character U+{:04X} cannot be written to a workbook",
                    u32::from(c)
                ),
            ));
        }
        if field.text.chars().count() > MAX_CELL_TEXT {
            return Err(Diagnostic::error(
                pos,
                "a cell holds at most 32,767 characters",
            ));
        }
        record.push(field);
        match chars.get(i) {
            Some(',') => {
                i += 1;
                column += 1;
            }
            Some('\r' | '\n') => {
                i += if chars[i] == '\r' && chars.get(i + 1) == Some(&'\n') {
                    2
                } else {
                    1
                };
                line += 1;
                column = 1;
                finish(&mut records, &mut record);
                if i == chars.len() {
                    break;
                }
            }
            None => {
                finish(&mut records, &mut record);
                break;
            }
            Some(_) => {
                return Err(Diagnostic::error(
                    Pos::new(line, column),
                    "after a closing quote, a field ends with a comma or a line break",
                ));
            }
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(text: &str) -> String {
        parse(text).expect_err("the file is wrong").to_string()
    }

    #[test]
    fn fields_are_typed_unless_quoted() {
        let table = parse("year,name,ok,note\r\n2024,\"Retail, UK\",true,\n-1.5e2,\"007\",FALSE,\"a \"\"b\"\"\"\n")
            .expect("parses");
        assert_eq!(table.header, ["year", "name", "ok", "note"]);
        assert_eq!(
            table.rows,
            [
                vec![
                    Field::Number(2024.0),
                    Field::Text("Retail, UK".to_string()),
                    Field::Bool(true),
                    Field::Empty
                ],
                vec![
                    Field::Number(-150.0),
                    Field::Text("007".to_string()),
                    Field::Bool(false),
                    Field::Text("a \"b\"".to_string())
                ],
            ]
        );
    }

    #[test]
    fn blank_lines_are_not_records() {
        let table = parse("a,b\n\n1,2\n\n").expect("parses");
        assert_eq!(table.rows, [vec![Field::Number(1.0), Field::Number(2.0)]]);
    }

    #[test]
    fn a_quoted_field_can_hold_a_line_break() {
        let table = parse("a,b\n\"one\ntwo\",2").expect("parses");
        assert_eq!(table.rows[0][0], Field::Text("one\ntwo".to_string()));
        assert_eq!(table.rows[0][1], Field::Number(2.0));
    }

    #[test]
    fn mistakes_are_reported_where_they_are() {
        assert_eq!(
            error("a,b\n1,2,3\n"),
            "2:1: error: this row has 3 fields, and the header has 2 fields"
        );
        assert_eq!(
            error("a,b\n1\n"),
            "2:1: error: this row has 1 field, and the header has 2 fields"
        );
        assert_eq!(
            error("a,a\n"),
            "1:3: error: the column name `a` is given twice"
        );
        assert_eq!(
            error("a\n\"open\n"),
            "2:1: error: this quoted field is never closed"
        );
        assert_eq!(
            error("a\n\"x\"y\n"),
            "2:4: error: after a closing quote, a field ends with a comma or a line break"
        );
        assert_eq!(
            error(""),
            "1:1: error: the file is empty; it needs a header row of column names"
        );
    }

    #[test]
    fn numbers_follow_json() {
        assert_eq!(json_number("-0.5"), Some(-0.5));
        assert_eq!(json_number("1e3"), Some(1000.0));
        assert_eq!(json_number("007"), None);
        assert_eq!(json_number("1."), None);
        assert_eq!(json_number("1,000"), None);
        assert_eq!(json_number("Infinity"), None);
    }
}
