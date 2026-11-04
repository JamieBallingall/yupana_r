//! Styles: records of how cells look, built one on another and checked against their types.
//!
//! A style is a record whose fields its type fixes. One style builds on another by update,
//! `base { … }`, which merges field by field at every depth and only adds: a field the base has
//! already set cannot be set again. So a variation can add to its base but never change it.

use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Pos};
use crate::syntax::{Expr, ExprKind, StyleType};

/// A color, as red, green and blue from 0 to 255.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    /// The color as the file writes it: opaque, then red, green and blue in hexadecimal.
    pub fn argb(self) -> String {
        format!("FF{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

/// A font, each part left out taking the default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Font {
    pub name: Option<String>,
    /// The size in points.
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub color: Option<Rgb>,
}

/// One side of a cell's border.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edge {
    /// The line, as the file names it, such as `thin`.
    pub weight: Option<&'static str>,
    pub color: Option<Rgb>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Border {
    pub left: Option<Edge>,
    pub right: Option<Edge>,
    pub top: Option<Edge>,
    pub bottom: Option<Edge>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alignment {
    /// As the file names it, such as `center`.
    pub horizontal: Option<&'static str>,
    pub vertical: Option<&'static str>,
    pub wrap: Option<bool>,
    pub indent: Option<i64>,
}

/// How a cell looks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellStyle {
    /// A number format, such as `0.0%`.
    pub format: Option<String>,
    pub font: Font,
    /// A solid background.
    pub fill: Option<Rgb>,
    pub border: Border,
    pub align: Alignment,
}

/// What a field holds.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Format,
    FontName,
    Points,
    Logical,
    /// A whole number from 0 to the bound.
    Whole(i64),
    /// One of the constructors, each with how the file names it.
    Choice(&'static [(&'static str, &'static str)]),
    Record(StyleType),
}

const WEIGHTS: &[(&str, &str)] = &[
    ("Thin", "thin"),
    ("Medium", "medium"),
    ("Thick", "thick"),
    ("Double", "double"),
    ("Hair", "hair"),
    ("Dotted", "dotted"),
    ("Dashed", "dashed"),
];
const HORIZONTAL: &[(&str, &str)] = &[
    ("Left", "left"),
    ("Center", "center"),
    ("Right", "right"),
    ("Justify", "justify"),
];
const VERTICAL: &[(&str, &str)] = &[("Top", "top"), ("Center", "center"), ("Bottom", "bottom")];

/// The most a font size can be, in points, and the deepest indent, in steps.
const MAX_POINTS: f64 = 409.0;
const MAX_INDENT: i64 = 250;

/// The fields of each type of style, and what each holds.
fn fields(ty: StyleType) -> &'static [(&'static str, Kind)] {
    match ty {
        StyleType::Cell => &[
            ("format", Kind::Format),
            ("font", Kind::Record(StyleType::Font)),
            ("fill", Kind::Record(StyleType::Color)),
            ("border", Kind::Record(StyleType::Border)),
            ("align", Kind::Record(StyleType::Alignment)),
        ],
        StyleType::Font => &[
            ("name", Kind::FontName),
            ("size", Kind::Points),
            ("bold", Kind::Logical),
            ("italic", Kind::Logical),
            ("color", Kind::Record(StyleType::Color)),
        ],
        StyleType::Border => &[
            ("top", Kind::Record(StyleType::Edge)),
            ("bottom", Kind::Record(StyleType::Edge)),
            ("left", Kind::Record(StyleType::Edge)),
            ("right", Kind::Record(StyleType::Edge)),
        ],
        StyleType::Edge => &[
            ("weight", Kind::Choice(WEIGHTS)),
            ("color", Kind::Record(StyleType::Color)),
        ],
        StyleType::Alignment => &[
            ("horizontal", Kind::Choice(HORIZONTAL)),
            ("vertical", Kind::Choice(VERTICAL)),
            ("wrap", Kind::Logical),
            ("indent", Kind::Whole(MAX_INDENT)),
        ],
        StyleType::Color => &[
            ("red", Kind::Whole(255)),
            ("green", Kind::Whole(255)),
            ("blue", Kind::Whole(255)),
        ],
    }
}

/// A style's value as it is built: a record of fields, each with where it was set, or a single
/// setting.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Record(StyleType, BTreeMap<&'static str, (Value, Pos)>),
    Text(String),
    Number(f64),
    Whole(i64),
    Logical(bool),
    Choice(&'static str),
}

/// A binding whose role is `Style`.
pub struct Declared<'a> {
    pub name: &'a str,
    pub ty: StyleType,
    pub expr: &'a Expr,
}

/// Works out styles, each once, reporting what is wrong where it is written.
pub struct Evaluator<'a> {
    styles: Vec<Declared<'a>>,
    by_name: BTreeMap<&'a str, usize>,
    /// Each style worked out so far; `None` where it is wrong and has been reported.
    done: BTreeMap<usize, Option<Value>>,
    /// The styles being worked out, innermost last, to catch one built on itself.
    visiting: Vec<usize>,
    pub errors: Vec<Diagnostic>,
}

impl<'a> Evaluator<'a> {
    pub fn new(styles: Vec<Declared<'a>>) -> Evaluator<'a> {
        let by_name = styles
            .iter()
            .enumerate()
            .map(|(index, style)| (style.name, index))
            .collect();
        Evaluator {
            styles,
            by_name,
            done: BTreeMap::new(),
            visiting: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Works out every style, so that each is checked whether or not anything names it.
    pub fn check_all(&mut self) {
        for index in 0..self.styles.len() {
            self.declared(index, self.styles[index].expr.pos);
        }
    }

    /// The style a name refers to, worked out, if it is a style of type `ty`.
    pub fn named(&mut self, name: &str, ty: StyleType, pos: Pos) -> Option<Value> {
        let Some(&index) = self.by_name.get(name) else {
            self.errors.push(Diagnostic::error(
                pos,
                format!(
                    "`{name}` is not a style; a style is a binding such as `{name} : Style Cell`"
                ),
            ));
            return None;
        };
        let found = self.styles[index].ty;
        if found != ty {
            self.errors.push(Diagnostic::error(
                pos,
                format!(
                    "`{name}` is a style of type {}, and a {} is needed here",
                    found.word(),
                    ty.word()
                ),
            ));
            return None;
        }
        self.declared(index, pos)
    }

    fn declared(&mut self, index: usize, pos: Pos) -> Option<Value> {
        if let Some(value) = self.done.get(&index) {
            return value.clone();
        }
        if let Some(from) = self.visiting.iter().position(|&visiting| visiting == index) {
            let mut path: Vec<&str> = self.visiting[from..]
                .iter()
                .map(|&visiting| self.styles[visiting].name)
                .collect();
            path.push(self.styles[index].name);
            self.errors.push(Diagnostic::error(
                pos,
                format!("a style is built on itself: {}", path.join(" -> ")),
            ));
            return None;
        }
        self.visiting.push(index);
        let (expr, ty) = (self.styles[index].expr, self.styles[index].ty);
        let value = self.evaluate(expr, Kind::Record(ty));
        self.visiting.pop();
        self.done.insert(index, value.clone());
        value
    }

    /// The value of a record of type `ty`: a record, an update of one, or a style's name.
    pub fn record(&mut self, expr: &Expr, ty: StyleType) -> Option<Value> {
        self.evaluate(expr, Kind::Record(ty))
    }

    fn evaluate(&mut self, expr: &Expr, kind: Kind) -> Option<Value> {
        let wrong = |what: String| Some(Diagnostic::error(expr.pos, what));
        let problem = match (kind, &expr.kind) {
            (Kind::Record(ty), ExprKind::Record(fields)) => {
                let mut values = BTreeMap::new();
                let mut ok = true;
                for field in fields {
                    let Some(&(name, kind)) = self::fields(ty)
                        .iter()
                        .find(|(name, _)| *name == field.name.name)
                    else {
                        let names: Vec<&str> =
                            self::fields(ty).iter().map(|(name, _)| *name).collect();
                        self.errors.push(Diagnostic::error(
                            field.name.pos,
                            format!(
                                "a {} has no field `{}`; its fields are {}",
                                ty.word(),
                                field.name.name,
                                names.join(", ")
                            ),
                        ));
                        ok = false;
                        continue;
                    };
                    if let Some((_, first)) = values.get(name) {
                        self.errors.push(Diagnostic::error(
                            field.name.pos,
                            format!("the field `{name}` is set twice; it is set first at {first}"),
                        ));
                        ok = false;
                        continue;
                    }
                    match self.evaluate(&field.value, kind) {
                        Some(value) => {
                            values.insert(name, (value, field.name.pos));
                        }
                        None => ok = false,
                    }
                }
                return ok.then_some(Value::Record(ty, values));
            }
            (Kind::Record(ty), ExprKind::Update { base, fields }) => {
                let base = self.evaluate(base, Kind::Record(ty));
                let record = Expr {
                    pos: expr.pos,
                    kind: ExprKind::Record(fields.clone()),
                };
                let added = self.evaluate(&record, Kind::Record(ty));
                let (Some(base), Some(added)) = (base, added) else {
                    return None;
                };
                return self.merge(base, added);
            }
            (Kind::Record(ty), ExprKind::Name(name)) => return self.named(name, ty, expr.pos),
            (Kind::Record(ty), _) => wrong(format!(
                "a {} is needed here: a style of that type, or a record such as {}",
                ty.word(),
                example(ty)
            )),
            (Kind::Format, ExprKind::Text(format)) => match format_problem(format) {
                Some(problem) => wrong(problem),
                None => return Some(Value::Text(format.clone())),
            },
            (Kind::FontName, ExprKind::Text(name)) => {
                if name.trim().is_empty() {
                    wrong("a font's name cannot be empty".to_string())
                } else {
                    return Some(Value::Text(name.clone()));
                }
            }
            (Kind::Format | Kind::FontName, ExprKind::Str(_)) => wrong(
                "a format code or a font's name is a setting, in single quotes, such as '0.0%'"
                    .to_string(),
            ),
            (Kind::Format, _) => {
                wrong("a format code is needed here, in single quotes".to_string())
            }
            (Kind::FontName, _) => {
                wrong("a font's name is needed here, in single quotes, such as 'Arial'".to_string())
            }
            (Kind::Points, ExprKind::Number { value, .. }) => {
                if *value >= 1.0 && *value <= MAX_POINTS {
                    return Some(Value::Number(*value));
                }
                wrong(format!("a font's size is from 1 to {MAX_POINTS} points"))
            }
            (Kind::Points, _) => {
                wrong("a font's size is a number of points, such as 11".to_string())
            }
            (Kind::Logical, ExprKind::Bool(value)) => return Some(Value::Logical(*value)),
            (Kind::Logical, _) => wrong("`True` or `False` is needed here".to_string()),
            (Kind::Whole(max), _) => match expr.integer() {
                Some(value) if (0..=max).contains(&value) => return Some(Value::Whole(value)),
                _ => wrong(format!("a whole number from 0 to {max} is needed here")),
            },
            (Kind::Choice(choices), ExprKind::Constructor(word)) => {
                match choices.iter().find(|(name, _)| name == word) {
                    Some((_, written)) => return Some(Value::Choice(written)),
                    None => wrong(choice_problem(choices, word)),
                }
            }
            (Kind::Choice(choices), _) => {
                let names: Vec<&str> = choices.iter().map(|(name, _)| *name).collect();
                wrong(format!("one of {} is needed here", names.join(", ")))
            }
        };
        self.errors.extend(problem);
        None
    }

    /// Adds `added` to `base`, field by field at every depth. Setting a field that is set already
    /// is an error.
    fn merge(&mut self, base: Value, added: Value) -> Option<Value> {
        let (Value::Record(ty, mut fields), Value::Record(_, more)) = (base, added) else {
            unreachable!("only records are merged");
        };
        let mut ok = true;
        for (name, (value, pos)) in more {
            match fields.remove(name) {
                None => {
                    fields.insert(name, (value, pos));
                }
                Some((old @ Value::Record(..), first)) if matches!(value, Value::Record(..)) => {
                    match self.merge(old, value) {
                        Some(merged) => {
                            fields.insert(name, (merged, first));
                        }
                        None => ok = false,
                    }
                }
                Some((old, first)) => {
                    self.errors.push(Diagnostic::error(
                        pos,
                        format!(
                            "`{name}` is set already, at {first}; an update only adds fields, and \
                             never changes one"
                        ),
                    ));
                    fields.insert(name, (old, first));
                    ok = false;
                }
            }
        }
        ok.then_some(Value::Record(ty, fields))
    }
}

/// A record of each type, for messages.
fn example(ty: StyleType) -> &'static str {
    match ty {
        StyleType::Cell => "`{ format = '0.0%' }`",
        StyleType::Font => "`{ bold = True }`",
        StyleType::Border => "`{ top = { weight = Thin } }`",
        StyleType::Edge => "`{ weight = Thin }`",
        StyleType::Alignment => "`{ horizontal = Center }`",
        StyleType::Color => "`{ red = 200, green = 200, blue = 200 }`",
    }
}

fn choice_problem(choices: &[(&str, &str)], word: &str) -> String {
    let names: Vec<&str> = choices.iter().map(|(name, _)| *name).collect();
    format!("`{word}` is not one of {}", names.join(", "))
}

/// A record's field, if it is set.
fn field<'v>(fields: &'v BTreeMap<&'static str, (Value, Pos)>, name: &str) -> Option<&'v Value> {
    fields.get(name).map(|(value, _)| value)
}

fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn logical(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Logical(value)) => Some(*value),
        _ => None,
    }
}

fn choice(value: Option<&Value>) -> Option<&'static str> {
    match value {
        Some(Value::Choice(choice)) => Some(choice),
        _ => None,
    }
}

fn record(value: Option<&Value>) -> Option<&BTreeMap<&'static str, (Value, Pos)>> {
    match value {
        Some(Value::Record(_, fields)) => Some(fields),
        _ => None,
    }
}

fn whole(fields: &BTreeMap<&'static str, (Value, Pos)>, name: &str) -> i64 {
    match field(fields, name) {
        Some(Value::Whole(value)) => *value,
        _ => 0,
    }
}

/// A color record as a color. A component left out is 0.
pub fn color(value: &Value) -> Rgb {
    let Value::Record(_, fields) = value else {
        unreachable!("a color is a record");
    };
    let component = |name| u8::try_from(whole(fields, name)).expect("the evaluator keeps 0 to 255");
    Rgb {
        red: component("red"),
        green: component("green"),
        blue: component("blue"),
    }
}

fn edge(value: &Value) -> Edge {
    let Value::Record(_, fields) = value else {
        unreachable!("an edge is a record");
    };
    Edge {
        weight: choice(field(fields, "weight")),
        color: field(fields, "color").map(color),
    }
}

/// A font record as a font.
pub fn font(value: Option<&Value>) -> Font {
    let Some(fields) = record(value) else {
        return Font::default();
    };
    Font {
        name: text(field(fields, "name")),
        size: match field(fields, "size") {
            Some(Value::Number(size)) => Some(*size),
            _ => None,
        },
        bold: logical(field(fields, "bold")),
        italic: logical(field(fields, "italic")),
        color: field(fields, "color").map(color),
    }
}

/// A cell record as a cell's style.
pub fn cell(value: &Value) -> CellStyle {
    let Value::Record(_, fields) = value else {
        unreachable!("a cell's style is a record");
    };
    let border = record(field(fields, "border"));
    let side = |name| border.and_then(|border| field(border, name)).map(edge);
    let align = record(field(fields, "align"));
    let part = |name| align.and_then(|align| field(align, name));
    CellStyle {
        format: text(field(fields, "format")),
        font: font(field(fields, "font")),
        fill: field(fields, "fill").map(color),
        border: Border {
            left: side("left"),
            right: side("right"),
            top: side("top"),
            bottom: side("bottom"),
        },
        align: Alignment {
            horizontal: choice(part("horizontal")),
            vertical: choice(part("vertical")),
            wrap: logical(part("wrap")),
            indent: match part("indent") {
                Some(Value::Whole(indent)) => Some(*indent),
                _ => None,
            },
        },
    }
}

/// The fields of a cell's style other than the font's name and size, which are all a sheet's or
/// the workbook's style may set, with where each is set.
pub fn beyond_font_face(value: &Value) -> Vec<(String, Pos)> {
    let Value::Record(_, fields) = value else {
        unreachable!("a cell's style is a record");
    };
    let mut beyond = Vec::new();
    for (name, (value, pos)) in fields {
        match (*name, value) {
            ("font", Value::Record(_, font)) => {
                for (part, (_, pos)) in font {
                    if !matches!(*part, "name" | "size") {
                        beyond.push((format!("font.{part}"), *pos));
                    }
                }
            }
            _ => beyond.push(((*name).to_string(), *pos)),
        }
    }
    beyond
}

/// Why a number format might be rejected by the spreadsheet app, if it might. The check is
/// conservative: literal text belongs in double quotes.
pub fn format_problem(format: &str) -> Option<String> {
    if format.is_empty() {
        return Some("a number format cannot be empty".to_string());
    }
    if format.chars().count() > 255 {
        return Some("a number format has at most 255 characters".to_string());
    }
    if format.eq_ignore_ascii_case("general") {
        return None;
    }
    let mut sections = 1;
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if !chars.by_ref().any(|c| c == '"') {
                    return Some("a quoted part of the number format is never closed".to_string());
                }
            }
            '[' => {
                let mut inside = String::new();
                loop {
                    match chars.next() {
                        Some(']') => break,
                        Some(c) => inside.push(c),
                        None => {
                            return Some("a `[` in the number format is never closed".to_string());
                        }
                    }
                }
                if !bracket_allowed(&inside) {
                    return Some(format!(
                        "`[{inside}]` is not a color, a condition or an elapsed time"
                    ));
                }
            }
            '\\' | '_' | '*' => {
                if chars.next().is_none() {
                    return Some(format!("`{c}` must be followed by a character"));
                }
            }
            ';' => {
                sections += 1;
                if sections > 4 {
                    return Some("a number format has at most four sections".to_string());
                }
            }
            '0'..='9'
            | '#'
            | '?'
            | '.'
            | ','
            | '%'
            | 'E'
            | 'e'
            | '+'
            | '-'
            | '/'
            | '('
            | ')'
            | ':'
            | ' '
            | '$'
            | '@'
            | '£'
            | '€'
            | '¥' => {}
            'd' | 'm' | 'y' | 'h' | 's' | 'D' | 'M' | 'Y' | 'H' | 'S' | 'A' | 'P' | 'a' | 'p' => {}
            other => {
                return Some(format!(
                    "`{other}` cannot appear in a number format outside double quotes"
                ));
            }
        }
    }
    None
}

fn bracket_allowed(inside: &str) -> bool {
    const COLORS: [&str; 8] = [
        "black", "blue", "cyan", "green", "magenta", "red", "white", "yellow",
    ];
    let lower = inside.to_ascii_lowercase();
    let all = |unit: u8| !lower.is_empty() && lower.bytes().all(|b| b == unit);
    let condition = ["<=", ">=", "<>", "<", ">", "="]
        .iter()
        .find_map(|op| lower.strip_prefix(op))
        .is_some_and(|number| {
            !number.is_empty()
                && number
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
        });
    COLORS.contains(&lower.as_str())
        || lower
            .strip_prefix("color")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        || all(b'h')
        || all(b'm')
        || all(b's')
        || condition
        || lower.starts_with('$')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::syntax::ItemKind;

    /// Works out the style called `last` among the definitions in `source`, each of whose names
    /// says its type: a name starting `cell` is a Cell, `border` a Border, and `color` a Color.
    fn evaluate(source: &str, last: &str) -> Result<CellStyle, Vec<String>> {
        let program = parse(&format!("yup 0.2\n{source}")).expect("parses");
        let mut declared = Vec::new();
        for item in &program.items {
            if let ItemKind::Definition { name, expr, .. } = &item.kind {
                let ty = if name.name.starts_with("border") {
                    StyleType::Border
                } else if name.name.starts_with("color") {
                    StyleType::Color
                } else {
                    StyleType::Cell
                };
                declared.push(Declared {
                    name: &name.name,
                    ty,
                    expr,
                });
            }
        }
        let mut evaluator = Evaluator::new(declared);
        evaluator.check_all();
        let value = evaluator.named(last, StyleType::Cell, Pos::START);
        if !evaluator.errors.is_empty() {
            return Err(evaluator.errors.iter().map(ToString::to_string).collect());
        }
        Ok(cell(&value.expect("no errors, so a value")))
    }

    #[test]
    fn an_update_adds_at_every_depth() {
        let style = evaluate(
            "color_red = { red = 255 }\ncell_base = { format = '0%', border = { bottom = { weight \
             = Double } } }\ncell_total = cell_base { font = { bold = True }, border = { top = { \
             weight = Thin, color = color_red } } }\n",
            "cell_total",
        )
        .expect("evaluates");
        assert_eq!(style.format.as_deref(), Some("0%"));
        assert_eq!(style.font.bold, Some(true));
        assert_eq!(style.border.bottom.and_then(|e| e.weight), Some("double"));
        let top = style.border.top.expect("a top edge");
        assert_eq!(top.weight, Some("thin"));
        assert_eq!(top.color.map(Rgb::argb).as_deref(), Some("FFFF0000"));
    }

    #[test]
    fn an_update_never_changes_a_field() {
        assert_eq!(
            evaluate(
                "cell_base = { border = { bottom = { weight = Double } } }\ncell_total = cell_base \
                 { border = { bottom = { weight = Thin } } }\n",
                "cell_total"
            ),
            Err(vec![
                "3:48: error: `weight` is set already, at 2:37; an update only adds fields, and \
                 never changes one"
                    .to_string()
            ])
        );
    }

    #[test]
    fn records_are_checked_against_their_types() {
        assert_eq!(
            evaluate(
                "cell_a = { colour = 1, format = 'x', fill = border_b, font = { size = 0 } }\n\
                 border_b = { top = { weight = Heavy } }\ncell_c = cell_c { format = '0' }\n",
                "cell_a"
            ),
            Err(vec![
                "2:12: error: a Cell has no field `colour`; its fields are format, font, fill, \
                 border, align"
                    .to_string(),
                "2:33: error: `x` cannot appear in a number format outside double quotes"
                    .to_string(),
                "2:45: error: `border_b` is a style of type Border, and a Color is needed here"
                    .to_string(),
                "2:71: error: a font's size is from 1 to 409 points".to_string(),
                "3:31: error: `Heavy` is not one of Thin, Medium, Thick, Double, Hair, Dotted, \
                 Dashed"
                    .to_string(),
                "4:10: error: a style is built on itself: cell_c -> cell_c".to_string(),
            ])
        );
    }

    #[test]
    fn number_formats_are_checked_conservatively() {
        for good in [
            "0.0%",
            "#,##0",
            "#,##0;[Red]-#,##0",
            "\"£\"#,##0.00",
            "yyyy-mm-dd",
            "[h]:mm",
            "General",
            "0.0E+00",
            "@",
        ] {
            assert_eq!(format_problem(good), None, "{good}");
        }
        assert!(format_problem("0.0 x").is_some());
        assert!(format_problem("[Orange]0").is_some());
        assert!(format_problem("0;0;0;0;0").is_some());
    }
}
