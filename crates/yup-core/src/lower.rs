//! Step 7, Lower: from ranges to cells, and from `.yup` expressions to the spreadsheet app's
//! formulas. This is the only step that writes A1 references.
//!
//! A range that spills becomes one dynamic array formula in its anchor cell; the cells it spills
//! into are left empty for the spreadsheet app to fill when it calculates. An input becomes one
//! value per cell, and every label a text cell.
//!
//! Every shown binding becomes a workbook-scoped defined name, and formulas refer to shown
//! bindings by those names. The name of a spilled range refers to the whole spill, which the
//! file writes as `_xlfn.ANCHORARRAY` of the anchor cell; the app shows it as `D3#`. A local
//! binding has no name: its formula is written into each formula that uses it.
//!
//! An expression whose span is `[a, b)` lowers to a formula whose result has `b - a` positions,
//! the first of them position `a`. That is what keeps `lag`, `tail` and `init` aligned: each is a
//! DROP of the positions that fall away.

use std::collections::{BTreeMap, BTreeSet};

use crate::cells::{absolute, cell_name, sheet_reference};
use crate::diagnostic::Diagnostic;
use crate::functions::Builtin;
use crate::ground::{Cell as At, Grounded};
use crate::infer::Typed;
use crate::place::{LABEL_OFFSET, VALUE_COLUMN};
use crate::resolve::{Binding, Model, Role, SheetItem, Symbol};
use crate::syntax::{BinOp, Expr, ExprKind, ItemKind, Program};

/// The longest formula the spreadsheet app takes, from its published limits.
const MAX_FORMULA: usize = 8_192;
/// The deepest the spreadsheet app nests functions, from its published limits.
const MAX_NESTING: u32 = 64;
/// The longest string a formula can hold, from the spreadsheet app's published limits.
const MAX_FORMULA_STRING: usize = 255;

/// A workbook as cells, ready to be packaged.
#[derive(Clone, Debug, PartialEq)]
pub struct Workbook {
    pub sheets: Vec<SheetCells>,
    pub names: Vec<DefinedName>,
    /// The styles cells refer to by index; index 0 is the default.
    pub styles: Vec<Style>,
}

/// One sheet's cells, by `(row, column)`, its row styles and its column widths.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetCells {
    pub name: String,
    pub cells: BTreeMap<(u32, u32), Cell>,
    /// The style of each row a formula with a style spills along. The cells a formula spills
    /// into are not in the file, so they take the style of their row.
    pub row_styles: BTreeMap<u32, usize>,
    pub widths: BTreeMap<u32, f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub value: Value,
    pub style: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    /// A dynamic array formula, without its `=`.
    Formula(String),
}

/// A workbook-scoped defined name and the formula it refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    pub refers_to: String,
}

/// How a cell looks.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Style {
    /// A number format, such as `0.0%`.
    pub format: Option<String>,
    pub bold: bool,
    pub italic: bool,
    /// Inputs are blue, as is usual in financial models, so that the cells a reader may change
    /// stand out.
    pub input: bool,
}

/// How tightly a formula binds, in the spreadsheet app's precedence: comparison, `&`, `+` and
/// `-`, `*` and `/`, `^`, then negation, which binds tighter than `^` there.
const COMPARE: u8 = 1;
const CONCAT: u8 = 2;
const ADDITIVE: u8 = 3;
const MULTIPLICATIVE: u8 = 4;
const POWER: u8 = 5;
const NEGATION: u8 = 6;
const ATOM: u8 = 7;

#[derive(Clone, Debug)]
struct Formula {
    text: String,
    precedence: u8,
    /// How deeply functions nest in it.
    depth: u32,
}

impl Formula {
    fn atom(text: impl Into<String>) -> Formula {
        Formula {
            text: text.into(),
            precedence: ATOM,
            depth: 0,
        }
    }

    fn wrapped(&self, context: u8) -> String {
        if self.precedence < context {
            format!("({})", self.text)
        } else {
            self.text.clone()
        }
    }

    /// Whether the formula is a single name or value, cheap to repeat.
    fn is_simple(&self) -> bool {
        self.precedence == ATOM && self.depth == 0
    }
}

fn call(name: &str, args: &[Formula]) -> Formula {
    let texts: Vec<&str> = args.iter().map(|arg| arg.text.as_str()).collect();
    Formula {
        text: format!("{name}({})", texts.join(",")),
        precedence: ATOM,
        depth: 1 + args.iter().map(|arg| arg.depth).max().unwrap_or(0),
    }
}

fn binary(op: BinOp, left: &Formula, right: &Formula) -> Formula {
    let zero = Formula::atom("0");
    let (symbol, precedence) = match op {
        // The app's AND and OR reduce whole ranges to one value, so logical and and or at each
        // position are written with arithmetic on the logicals.
        BinOp::And => return binary(BinOp::Ne, &binary(BinOp::Mul, left, right), &zero),
        BinOp::Or => return binary(BinOp::Ne, &binary(BinOp::Add, left, right), &zero),
        BinOp::Add => ("+", ADDITIVE),
        BinOp::Sub => ("-", ADDITIVE),
        BinOp::Mul => ("*", MULTIPLICATIVE),
        BinOp::Div => ("/", MULTIPLICATIVE),
        BinOp::Pow => ("^", POWER),
        BinOp::Concat => ("&", CONCAT),
        BinOp::Eq => ("=", COMPARE),
        BinOp::Ne => ("<>", COMPARE),
        BinOp::Lt => ("<", COMPARE),
        BinOp::Le => ("<=", COMPARE),
        BinOp::Gt => (">", COMPARE),
        BinOp::Ge => (">=", COMPARE),
    };
    // The app groups every operator to the left, so a right operand of equal precedence keeps
    // its parentheses. That also keeps `.yup`'s `^`, which groups to the right.
    Formula {
        text: format!(
            "{}{symbol}{}",
            left.wrapped(precedence),
            right.wrapped(precedence + 1)
        ),
        precedence,
        depth: left.depth.max(right.depth),
    }
}

/// A number as a formula or a cell writes it: Rust's shortest form that reads back the same.
pub fn number(value: f64) -> String {
    format!("{value}")
}

type Cells = BTreeMap<(u32, u32), Cell>;

fn to_u32(n: i64) -> u32 {
    u32::try_from(n).expect("Check keeps every cell on the sheet")
}

fn to_i64(n: usize) -> i64 {
    i64::try_from(n).expect("a list in a file has fewer than 2^63 values")
}

fn put(cells: &mut Cells, (row, column): At, value: Value, style: usize) {
    let previous = cells.insert((to_u32(row), to_u32(column)), Cell { value, style });
    debug_assert!(previous.is_none(), "Check keeps cells apart");
}

struct Lowerer<'a> {
    program: &'a Program,
    model: &'a Model,
    typed: &'a Typed,
    grounded: &'a Grounded,
    styles: Vec<Style>,
}

/// Lowers every sheet to cells and every shown binding to a defined name.
pub fn lower(
    program: &Program,
    model: &Model,
    typed: &Typed,
    grounded: &Grounded,
) -> Result<Workbook, Vec<Diagnostic>> {
    let mut lowerer = Lowerer {
        program,
        model,
        typed,
        grounded,
        styles: vec![Style::default()],
    };
    let mut errors = Vec::new();
    let mut sheets = Vec::new();
    let mut names = Vec::new();
    for sheet in &model.sheets {
        let mut cells = Cells::new();
        let mut row_styles = BTreeMap::new();
        for item in &sheet.items {
            match *item {
                SheetItem::Heading(index) => {
                    let ItemKind::Heading { text, .. } = &program.items[index].kind else {
                        unreachable!("a heading index points at a heading");
                    };
                    let style = lowerer.style(Style {
                        bold: true,
                        ..Style::default()
                    });
                    put(
                        &mut cells,
                        grounded.headings[&index],
                        Value::Text(text.value.clone()),
                        style,
                    );
                }
                SheetItem::Gap(_) => {}
                SheetItem::Binding(binding) => {
                    match lowerer.binding(binding, &sheet.name, &mut cells, &mut row_styles) {
                        Ok(name) => names.push(name),
                        Err(error) => errors.push(error),
                    }
                }
            }
        }
        let widths = widths(&cells);
        sheets.push(SheetCells {
            name: sheet.name.clone(),
            cells,
            row_styles,
            widths,
        });
    }
    for (binding, b) in model.bindings.iter().enumerate() {
        if b.role == Role::Function {
            match lowerer.function(binding) {
                Ok(name) => names.push(name),
                Err(error) => errors.push(error),
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    names.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Workbook {
        sheets,
        names,
        styles: lowerer.styles,
    })
}

impl Lowerer<'_> {
    fn style(&mut self, style: Style) -> usize {
        match self.styles.iter().position(|known| *known == style) {
            Some(index) => index,
            None => {
                self.styles.push(style);
                self.styles.len() - 1
            }
        }
    }

    fn binding(
        &mut self,
        binding: usize,
        sheet: &str,
        cells: &mut Cells,
        row_styles: &mut BTreeMap<u32, usize>,
    ) -> Result<DefinedName, Diagnostic> {
        let b = &self.model.bindings[binding];
        let sig = self
            .model
            .signature(self.program, binding)
            .expect("a shown binding has a signature");
        let label = sig.label.as_ref().expect("a shown binding has a label");
        let (bold, italic) = (sig.bold.is_some(), sig.italic.is_some());
        let label_style = self.style(Style {
            bold,
            italic,
            ..Style::default()
        });
        put(
            cells,
            self.grounded.labels[&binding],
            Value::Text(label.value.clone()),
            label_style,
        );
        let value_style = self.style(Style {
            format: sig.format.as_ref().map(|format| format.value.clone()),
            bold,
            italic,
            input: b.role == Role::Input,
        });
        let (row, column) = self.grounded.anchors[&binding];
        let expr = self.model.definition(self.program, binding);
        if b.role == Role::Input {
            let rows: Vec<Vec<&Expr>> = match &expr.kind {
                ExprKind::List(rows) => rows.iter().map(|row| row.iter().collect()).collect(),
                _ => vec![vec![expr]],
            };
            for (i, values) in rows.iter().enumerate() {
                for (j, value) in values.iter().enumerate() {
                    let at = (row + to_i64(i), column + to_i64(j));
                    put(cells, at, literal(value), value_style);
                }
            }
        } else {
            let formula = self.formula(expr, &[])?;
            within_limits(b, &formula)?;
            put(
                cells,
                (row, column),
                Value::Formula(formula.text),
                value_style,
            );
            // A range of unknown height takes its style in its anchor cell only.
            let shape = &self.grounded.shapes[binding];
            let one_cell = shape.rows.size.as_constant() == Some(1)
                && shape.columns.size.as_constant() == Some(1);
            if value_style != 0
                && !one_cell
                && let Some(rows) = shape.rows.size.as_constant()
            {
                for spilled in row..row + rows {
                    row_styles.entry(to_u32(spilled)).or_insert(value_style);
                }
            }
        }
        Ok(DefinedName {
            name: b.name.clone(),
            refers_to: self.reference(binding, sheet, (row, column)),
        })
    }

    /// A function, as a defined name whose value is a LAMBDA of its parameters. It can be
    /// called by name in the workbook as well as in the file.
    fn function(&self, binding: usize) -> Result<DefinedName, Diagnostic> {
        let b = &self.model.bindings[binding];
        let body = self.formula(self.model.definition(self.program, binding), &b.params)?;
        let mut args: Vec<Formula> = b
            .params
            .iter()
            .map(|param| Formula::atom(format!("_xlpm.{param}")))
            .collect();
        args.push(body);
        let lambda = call("_xlfn.LAMBDA", &args);
        within_limits(b, &lambda)?;
        Ok(DefinedName {
            name: b.name.clone(),
            refers_to: lambda.text,
        })
    }

    /// What a shown binding's defined name refers to.
    fn reference(&self, binding: usize, sheet: &str, (row, column): At) -> String {
        let shape = &self.grounded.shapes[binding];
        let sheet = sheet_reference(sheet);
        let start = absolute(to_u32(row), to_u32(column));
        let rows = shape.rows.size.as_constant();
        let columns = shape.columns.size.as_constant();
        match (self.model.bindings[binding].role, rows, columns) {
            (_, Some(1), Some(1)) => format!("{sheet}!{start}"),
            (Role::Input, Some(rows), Some(columns)) => {
                let end = absolute(to_u32(row + rows - 1), to_u32(column + columns - 1));
                format!("{sheet}!{start}:{end}")
            }
            _ => format!("_xlfn.ANCHORARRAY({sheet}!{start})"),
        }
    }

    /// The formula for an expression. `scope` names the parameters of the function whose body
    /// this is, if any; the file writes them with the prefix `_xlpm.`.
    fn formula(&self, expr: &Expr, scope: &[String]) -> Result<Formula, Diagnostic> {
        let formula = match &expr.kind {
            ExprKind::Number { value, .. } => Formula::atom(number(*value)),
            ExprKind::Str(value) => {
                if value.chars().count() > MAX_FORMULA_STRING {
                    return Err(Diagnostic::error(
                        expr.pos,
                        format!(
                            "a string in a formula has at most {MAX_FORMULA_STRING} characters; \
                             a longer text can be an input"
                        ),
                    ));
                }
                Formula::atom(format!("\"{}\"", value.replace('"', "\"\"")))
            }
            ExprKind::Bool(value) => Formula::atom(if *value { "TRUE" } else { "FALSE" }),
            ExprKind::Name(name) if scope.contains(name) => Formula::atom(format!("_xlpm.{name}")),
            ExprKind::Name(name) => match self.model.symbols.get(name) {
                Some(Symbol::Binding(binding))
                    if self.model.bindings[*binding].role == Role::Local =>
                {
                    self.formula(self.model.definition(self.program, *binding), &[])?
                }
                Some(Symbol::Binding(_)) => Formula::atom(name),
                Some(Symbol::Size(var)) => Formula::atom(
                    self.typed
                        .fixed(*var)
                        .expect("Resolve lets formulas use fixed sizes only")
                        .to_string(),
                ),
                None => unreachable!("Resolve reports unknown names"),
            },
            ExprKind::Neg(operand) => {
                let operand = self.formula(operand, scope)?;
                Formula {
                    text: format!("-{}", operand.wrapped(NEGATION)),
                    precedence: NEGATION,
                    depth: operand.depth,
                }
            }
            ExprKind::Binary { op, left, right } => binary(
                *op,
                &self.formula(left, scope)?,
                &self.formula(right, scope)?,
            ),
            ExprKind::If { cond, then, other } => call(
                "IF",
                &[
                    self.formula(cond, scope)?,
                    self.formula(then, scope)?,
                    self.formula(other, scope)?,
                ],
            ),
            ExprKind::List(rows) => {
                let rows: Vec<String> = rows
                    .iter()
                    .map(|row| {
                        let values: Vec<String> =
                            row.iter().map(|value| constant(&literal(value))).collect();
                        values.join(",")
                    })
                    .collect();
                Formula::atom(format!("{{{}}}", rows.join(";")))
            }
            ExprKind::Range { first, last } => {
                let start = first
                    .integer()
                    .expect("Infer checks that a range starts at a whole number");
                let last = self.formula(last, scope)?;
                // The count of whole numbers from `start` to `last` is `last - (start - 1)`.
                let count = match start - 1 {
                    0 => last,
                    offset if offset > 0 => {
                        binary(BinOp::Sub, &last, &Formula::atom(offset.to_string()))
                    }
                    offset => binary(BinOp::Add, &last, &Formula::atom((-offset).to_string())),
                };
                let mut args = vec![Formula::atom("1"), count];
                if start != 1 {
                    args.push(Formula::atom(start.to_string()));
                }
                call("_xlfn.SEQUENCE", &args)
            }
            ExprKind::Apply { func, args, seed } => match Builtin::lookup(&func.name) {
                Some(builtin) => self.apply(builtin, args, seed.as_deref(), scope)?,
                // A function of the file's own is a defined name, called by that name.
                None => {
                    let args = args
                        .iter()
                        .map(|arg| self.formula(arg, scope))
                        .collect::<Result<Vec<_>, _>>()?;
                    call(&func.name, &args)
                }
            },
        };
        Ok(formula)
    }

    fn apply(
        &self,
        builtin: Builtin,
        args: &[Expr],
        seed: Option<&Expr>,
        scope: &[String],
    ) -> Result<Formula, Diagnostic> {
        let arg = |index: usize| self.formula(&args[index], scope);
        let drop_columns = |x: Formula, columns: i64| {
            call(
                "_xlfn.DROP",
                &[x, Formula::atom(""), Formula::atom(columns.to_string())],
            )
        };
        let formula = match builtin {
            Builtin::Lag => {
                let periods = args[0]
                    .integer()
                    .expect("Infer checks that a lag is a whole number");
                let dropped = drop_columns(arg(1)?, -periods);
                match seed {
                    Some(seed) => call("_xlfn.HSTACK", &[self.formula(seed, scope)?, dropped]),
                    None => dropped,
                }
            }
            Builtin::Tail => drop_columns(arg(0)?, 1),
            Builtin::Init => drop_columns(arg(0)?, -1),
            Builtin::Sum => call("SUM", &[arg(0)?]),
            Builtin::Average => call("AVERAGE", &[arg(0)?]),
            Builtin::Minimum => call("MIN", &[arg(0)?]),
            Builtin::Maximum => call("MAX", &[arg(0)?]),
            Builtin::SumColumns | Builtin::SumRows => {
                let by = if builtin == Builtin::SumColumns {
                    "_xlfn.BYCOL"
                } else {
                    "_xlfn.BYROW"
                };
                let x = Formula::atom("_xlpm.x");
                let sum = call("_xlfn.LAMBDA", &[x.clone(), call("SUM", &[x])]);
                call(by, &[arg(0)?, sum])
            }
            Builtin::Min | Builtin::Max => {
                // The app's MIN and MAX reduce whole ranges to one value, so the minimum at each
                // position is written as a choice between the two.
                let op = if builtin == Builtin::Min {
                    BinOp::Lt
                } else {
                    BinOp::Gt
                };
                let (a, b) = (arg(0)?, arg(1)?);
                if a.is_simple() && b.is_simple() {
                    call("IF", &[binary(op, &a, &b), a, b])
                } else {
                    let (x, y) = (Formula::atom("_xlpm.a"), Formula::atom("_xlpm.b"));
                    let choice = call("IF", &[binary(op, &x, &y), x.clone(), y.clone()]);
                    call("_xlfn.LET", &[x, a, y, b, choice])
                }
            }
            Builtin::Abs => call("ABS", &[arg(0)?]),
            Builtin::Exp => call("EXP", &[arg(0)?]),
            Builtin::Ln => call("LN", &[arg(0)?]),
            Builtin::Sqrt => call("SQRT", &[arg(0)?]),
            Builtin::Not => call("NOT", &[arg(0)?]),
            Builtin::Round => call("ROUND", &[arg(1)?, arg(0)?]),
            Builtin::Transpose => call("TRANSPOSE", &[arg(0)?]),
            Builtin::Lookup => call("_xlfn.XLOOKUP", &[arg(0)?, arg(1)?, arg(2)?]),
        };
        Ok(formula)
    }
}

/// Checks a binding's formula against the spreadsheet app's limits on length and nesting.
fn within_limits(binding: &Binding, formula: &Formula) -> Result<(), Diagnostic> {
    let problem = if formula.text.chars().count() > MAX_FORMULA {
        format!("would be longer than the {MAX_FORMULA} characters the spreadsheet app takes")
    } else if formula.depth > MAX_NESTING {
        format!("would nest functions more than {MAX_NESTING} deep")
    } else {
        return Ok(());
    };
    Err(Diagnostic::error(
        binding.pos,
        format!(
            "the formula for `{}` {problem}; make parts of it shown bindings of their own",
            binding.name
        ),
    ))
}

/// The value of a literal, which Resolve and Infer have checked is one.
fn literal(expr: &Expr) -> Value {
    match &expr.kind {
        ExprKind::Number { value, .. } => Value::Number(*value),
        ExprKind::Str(value) => Value::Text(value.clone()),
        ExprKind::Bool(value) => Value::Bool(*value),
        ExprKind::Neg(operand) => match literal(operand) {
            Value::Number(value) => Value::Number(-value),
            _ => unreachable!("only a number is negated in a literal"),
        },
        _ => unreachable!("Resolve and Infer check that this is a literal"),
    }
}

/// A literal value as an array constant writes it.
fn constant(value: &Value) -> String {
    match value {
        Value::Number(value) => number(*value),
        Value::Text(text) => format!("\"{}\"", text.replace('"', "\"\"")),
        Value::Bool(value) => String::from(if *value { "TRUE" } else { "FALSE" }),
        Value::Formula(_) => unreachable!("a literal is not a formula"),
    }
}

/// Column widths: each column of text wide enough for its longest text, within reason, and the
/// margin and the gap before the values narrow when they are empty.
fn widths(cells: &Cells) -> BTreeMap<u32, f64> {
    let mut longest: BTreeMap<u32, usize> = BTreeMap::new();
    for (&(_, column), cell) in cells {
        if let Value::Text(text) = &cell.value {
            let entry = longest.entry(column).or_insert(0);
            *entry = (*entry).max(text.chars().count());
        }
    }
    let mut widths: BTreeMap<u32, f64> = longest
        .into_iter()
        .map(|(column, chars)| (column, (chars as f64 * 1.1 + 2.0).clamp(10.0, 60.0)))
        .collect();
    let used: BTreeSet<u32> = cells.keys().map(|&(_, column)| column).collect();
    let gap = to_u32(VALUE_COLUMN - 1);
    let margin = to_u32(VALUE_COLUMN - LABEL_OFFSET - 1);
    for column in [margin, gap] {
        if !used.contains(&column) {
            widths.insert(column, 2.7);
        }
    }
    widths
}

/// A style in words, for a listing.
fn notes(style: &Style) -> Vec<String> {
    let mut notes = Vec::new();
    if style.input {
        notes.push("input".to_string());
    }
    if style.bold {
        notes.push("bold".to_string());
    }
    if style.italic {
        notes.push("italic".to_string());
    }
    if let Some(format) = &style.format {
        notes.push(format!("format \"{format}\""));
    }
    notes
}

/// The cells and names of a workbook, as text, for reading what Lower produced.
pub fn listing(workbook: &Workbook) -> String {
    let mut out = String::new();
    for sheet in &workbook.sheets {
        out.push_str(&format!("sheet '{}'\n", sheet.name.replace('\'', "''")));
        for (&(row, column), cell) in &sheet.cells {
            let value = match &cell.value {
                Value::Number(value) => number(*value),
                Value::Text(text) => format!("'{}'", text.replace('\'', "''")),
                Value::Bool(value) => String::from(if *value { "TRUE" } else { "FALSE" }),
                Value::Formula(formula) => format!("={formula}"),
            };
            let notes = notes(&workbook.styles[cell.style]);
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!("  [{}]", notes.join(", "))
            };
            out.push_str(&format!("  {:<6} {value}{notes}\n", cell_name(row, column)));
        }
        for (row, style) in &sheet.row_styles {
            let notes = notes(&workbook.styles[*style]).join(", ");
            out.push_str(&format!("  row {row}  [{notes}]\n"));
        }
    }
    out.push_str("names\n");
    for name in &workbook.names {
        out.push_str(&format!("  {} = {}\n", name.name, name.refers_to));
    }
    out
}
