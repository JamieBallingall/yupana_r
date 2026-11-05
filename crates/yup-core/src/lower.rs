//! Step 7, Lower: from ranges to cells, and from `.yup` expressions to the spreadsheet app's
//! formulas. This is the only step that writes A1 references.
//!
//! A formula becomes one dynamic array formula in its anchor cell; the cells it spills into are
//! left for the spreadsheet app to fill when it calculates. A value becomes one value per cell,
//! and a label a text cell.
//!
//! Every formula and value becomes a workbook-scoped defined name, and formulas refer to them by
//! those names. The name of a spilled range refers to the whole spill, which the file writes as
//! `_xlfn.ANCHORARRAY` of the anchor cell; the app shows it as `D3#`. A local has no defined name:
//! each formula that uses it, directly or through other locals, binds it with LET under its own
//! name, so that it is worked out once and reads as it does in the file.
//!
//! An expression whose span is `[a, b)` lowers to a formula whose result has `b - a` positions,
//! the first of them position `a`. That is what keeps `lag`, `tail` and `init` aligned: each is a
//! DROP of the positions that fall away.
//!
//! The cells a formula spills into are not in the file, so they cannot carry a style of their
//! own. Where the extent is known, the file holds them as empty cells with the formula's style.
//! Where it is not, the style goes on the whole row or column the range grows along, and the
//! cells before the range on that row or column are written with the default style, so that the
//! style covers the range and what lies beyond it only.

use std::collections::{BTreeMap, BTreeSet};

use crate::cells::{absolute, cell_name, sheet_reference};
use crate::csv::Field;
use crate::diagnostic::{Diagnostic, Pos};
use crate::functions::Builtin;
use crate::ground::{Cell as At, Grounded};
use crate::infer::Typed;
use crate::resolve::{Binding, Model, Recurrence, Symbol};
use crate::style::{CellStyle, Font, Rgb};
use crate::syntax::{Axis, BinOp, Expr, ExprKind, Program, Role};

/// Names for the LET variables and LAMBDA parameters that the tool writes itself. Each starts
/// with `_`, which no `.yup` name can, so none hides a binding of the file's that the formula
/// around it refers to.
const TOOL_NAMES: [&str; 5] = ["_x", "_a", "_b", "_value", "_position"];

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
    pub styles: Vec<CellStyle>,
    /// The workbook's default font, with its name and size filled in.
    pub font: Font,
}

/// One sheet's cells, by `(row, column)`, its settings, and the styles and sizes of its rows
/// and columns.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetCells {
    pub name: String,
    pub color: Option<Rgb>,
    pub hidden: bool,
    pub cells: BTreeMap<(u32, u32), Cell>,
    /// The style of each row that a formula of unknown width spills along.
    pub row_styles: BTreeMap<u32, usize>,
    /// The style of each column that a formula of unknown height spills down.
    pub column_styles: BTreeMap<u32, usize>,
    /// The width of each column a binding sets, in pixels.
    pub widths: BTreeMap<u32, u32>,
    /// The height of each row a binding sets, in pixels.
    pub heights: BTreeMap<u32, u32>,
    /// The width of every other column, in pixels, where the sheet or the workbook sets one.
    pub column_width: Option<u32>,
    /// The height of every other row, in pixels, where the sheet or the workbook sets one.
    pub row_height: Option<u32>,
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
    /// Nothing but a style: a cell a formula spills into.
    Empty,
}

/// A workbook-scoped defined name and the formula it refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    pub refers_to: String,
}

/// The typeface of the default font. A reader without it substitutes another; the standard's
/// own examples use the same one.
pub const DEFAULT_FONT: &str = "Calibri";
/// The size of the default font, in points, as in the standard's own examples.
pub const DEFAULT_SIZE: f64 = 11.0;

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
    i64::try_from(n).expect("a table in a file has fewer than 2^63 values")
}

fn put(cells: &mut Cells, (row, column): At, value: Value, style: usize) {
    let previous = cells.insert((to_u32(row), to_u32(column)), Cell { value, style });
    debug_assert!(previous.is_none(), "Check keeps cells apart");
}

/// What the names in an expression refer to while it is lowered.
#[derive(Clone, Copy)]
struct Scope<'a> {
    /// The parameters of the lambda whose body this is; the file writes them `_xlpm.name`.
    params: &'a [String],
    /// The recurrence one position of which this is, if any.
    position: Option<&'a Position>,
    /// The locals a LET around the formula binds; the file writes them `_xlpm.name` too.
    lets: &'a BTreeSet<usize>,
}

impl<'a> Scope<'a> {
    /// The scope of a formula with no parameters and not inside a recurrence.
    fn global(lets: &'a BTreeSet<usize>) -> Scope<'a> {
        Scope {
            params: &[],
            position: None,
            lets,
        }
    }
}

/// A recurrence, as one position of it is lowered.
struct Position {
    members: BTreeSet<usize>,
    state: usize,
    axis: Axis,
}

/// What lies along one row or one column of a sheet: the cells covered before the range that
/// grows along it, so that they can be kept from its style.
struct Guard {
    axis: Axis,
    /// The row for a range that grows across, the column for one that grows down.
    line: u32,
    /// Where the range starts along the line.
    start: u32,
}

struct Lowerer<'a> {
    program: &'a Program,
    model: &'a Model,
    typed: &'a Typed,
    grounded: &'a Grounded,
    styles: Vec<CellStyle>,
}

/// Lowers every sheet to cells and every formula and value to a defined name.
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
        styles: vec![CellStyle::default()],
    };
    let mut errors = Vec::new();
    let mut sheets = Vec::new();
    let mut names = Vec::new();
    for (index, sheet) in model.sheets.iter().enumerate() {
        let mut out = SheetCells {
            name: sheet.name.clone(),
            color: sheet.color,
            hidden: sheet.hidden,
            cells: Cells::new(),
            row_styles: BTreeMap::new(),
            column_styles: BTreeMap::new(),
            widths: BTreeMap::new(),
            heights: BTreeMap::new(),
            column_width: sheet.column_width,
            row_height: sheet.row_height,
        };
        let mut guards = Vec::new();
        for &binding in &model.layout {
            if model.bindings[binding].sheet != Some(index) {
                continue;
            }
            match lowerer.binding(binding, &mut out, &mut guards) {
                Ok(Some(name)) => names.push(name),
                Ok(None) => {}
                Err(error) => errors.push(error),
            }
        }
        // The cells before a range whose style covers its whole row or column keep the default.
        for guard in &guards {
            for before in 1..guard.start {
                let key = match guard.axis {
                    Axis::Across => (guard.line, before),
                    Axis::Down => (before, guard.line),
                };
                out.cells.entry(key).or_insert(Cell {
                    value: Value::Empty,
                    style: 0,
                });
            }
        }
        (out.widths, out.heights) = lowerer.sizes(index);
        sheets.push(out);
    }
    for (binding, b) in model.bindings.iter().enumerate() {
        if b.role == Role::Lambda {
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
        font: Font {
            name: Some(
                model
                    .font
                    .name
                    .clone()
                    .unwrap_or_else(|| DEFAULT_FONT.to_string()),
            ),
            size: Some(model.font.size.unwrap_or(DEFAULT_SIZE)),
            ..Font::default()
        },
    })
}

impl Lowerer<'_> {
    /// The index of a binding's style, with the font's name and size cascading from its sheet
    /// and then the workbook.
    fn style(&mut self, binding: usize) -> usize {
        let b = &self.model.bindings[binding];
        let mut style = b.style.clone().unwrap_or_default();
        let sheet = &self.model.sheets[b.sheet.expect("a placed binding is on a sheet")];
        style.font.name = style
            .font
            .name
            .or_else(|| sheet.font.name.clone())
            .or_else(|| self.model.font.name.clone());
        style.font.size = style.font.size.or(sheet.font.size).or(self.model.font.size);
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
        sheet: &mut SheetCells,
        guards: &mut Vec<Guard>,
    ) -> Result<Option<DefinedName>, Diagnostic> {
        // An `Empty` writes nothing: its cells stay empty, and Check has kept everything else
        // out of them.
        if self.model.bindings[binding].role == Role::Empty {
            return Ok(None);
        }
        let style = self.style(binding);
        let b = &self.model.bindings[binding];
        let (row, column) = self.grounded.anchors[&binding];
        let expr = self.model.definition(self.program, binding);
        let cells = &mut sheet.cells;
        match b.role {
            Role::Label => {
                let ExprKind::Str(text) = &expr.kind else {
                    unreachable!("Resolve checks that a label is its text");
                };
                put(cells, (row, column), Value::Text(text.clone()), style);
                return Ok(None);
            }
            Role::Value => match self.model.tables.get(&binding) {
                Some(table) => {
                    for (i, fields) in table.rows.iter().enumerate() {
                        for (j, field) in fields.iter().enumerate() {
                            let value = match field {
                                Field::Empty => Value::Empty,
                                Field::Number(value) => Value::Number(*value),
                                Field::Bool(value) => Value::Bool(*value),
                                Field::Text(text) => Value::Text(text.clone()),
                            };
                            let at = (row + to_i64(i), column + to_i64(j));
                            put(cells, at, value, style);
                        }
                    }
                }
                None => put(cells, (row, column), literal(expr), style),
            },
            Role::Formula => {
                let roots = self.roots(binding);
                let formula =
                    self.with_lets(&roots, &[], |lets| self.definition_formula(binding, lets))?;
                within_limits(b, &formula)?;
                put(cells, (row, column), Value::Formula(formula.text), style);
                let shape = &self.grounded.shapes[binding];
                let rows = shape.rows.size.as_constant();
                let columns = shape.columns.size.as_constant();
                if style != 0 {
                    match (rows, columns) {
                        // The cells the formula spills into, styled like its anchor.
                        (Some(rows), Some(columns)) => {
                            for i in 0..rows {
                                for j in 0..columns {
                                    if (i, j) != (0, 0) {
                                        let at = (row + i, column + j);
                                        put(cells, at, Value::Empty, style);
                                    }
                                }
                            }
                        }
                        (Some(rows), None) => {
                            for i in 0..rows {
                                let line = to_u32(row + i);
                                sheet.row_styles.entry(line).or_insert(style);
                                guards.push(Guard {
                                    axis: Axis::Across,
                                    line,
                                    start: to_u32(column),
                                });
                            }
                        }
                        (None, Some(columns)) => {
                            for j in 0..columns {
                                let line = to_u32(column + j);
                                sheet.column_styles.entry(line).or_insert(style);
                                guards.push(Guard {
                                    axis: Axis::Down,
                                    line,
                                    start: to_u32(row),
                                });
                            }
                        }
                        // A range that grows both ways takes its style in its anchor only.
                        (None, None) => {}
                    }
                }
            }
            _ => unreachable!("only formulas, values and labels have cells"),
        }
        Ok(Some(DefinedName {
            name: b.name.clone(),
            refers_to: self.reference(binding, &sheet.name, (row, column)),
        }))
    }

    /// The definitions a binding's formula is made of: its own, or, for a value carried from one
    /// position to the next, those of every member of its cycle.
    fn roots(&self, binding: usize) -> Vec<&Expr> {
        match self.model.recurrence_of(binding) {
            Some(recurrence) if recurrence.state == binding => recurrence
                .members
                .iter()
                .map(|&member| self.model.definition(self.program, member))
                .collect(),
            _ => vec![self.model.definition(self.program, binding)],
        }
    }

    /// The locals that expressions use, directly or through other locals, each after the locals
    /// it uses. A member of a recurrence is written in place, never bound, but what it uses is.
    fn locals(&self, roots: &[&Expr], params: &[String]) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<(&Expr, bool)> = roots.iter().map(|&root| (root, true)).collect();
        while let Some((expr, in_root)) = stack.pop() {
            if let ExprKind::Name(name) = &expr.kind
                && !(in_root && params.contains(name))
                && let Some(&Symbol::Binding(binding)) = self.model.symbols.get(name)
                && self.model.bindings[binding].role == Role::Local
                && seen.insert(binding)
            {
                let members = match self.model.recurrence_of(binding) {
                    Some(recurrence) => recurrence.members.clone(),
                    None => vec![binding],
                };
                for member in members {
                    seen.insert(member);
                    stack.push((self.model.definition(self.program, member), false));
                }
            }
            stack.extend(expr.children().into_iter().map(|child| (child, in_root)));
        }
        self.model
            .order
            .iter()
            .copied()
            .filter(|binding| {
                seen.contains(binding) && self.model.recurrence_of(*binding).is_none()
            })
            .collect()
    }

    /// The formula `build` gives, inside a LET that binds every local it uses. A local whose name
    /// is also a parameter's is written in place instead, so that its LET cannot hide the
    /// parameter.
    fn with_lets(
        &self,
        roots: &[&Expr],
        params: &[String],
        build: impl FnOnce(&BTreeSet<usize>) -> Result<Formula, Diagnostic>,
    ) -> Result<Formula, Diagnostic> {
        let locals = self.locals(roots, params);
        let clashes = locals
            .iter()
            .any(|&local| params.contains(&self.model.bindings[local].name));
        if locals.is_empty() || clashes {
            return build(&BTreeSet::new());
        }
        let mut bound = BTreeSet::new();
        let mut args = Vec::with_capacity(2 * locals.len() + 1);
        for local in locals {
            let definition = self.model.definition(self.program, local);
            let value = self.formula(definition, Scope::global(&bound))?;
            args.push(Formula::atom(format!(
                "_xlpm.{}",
                self.model.bindings[local].name
            )));
            args.push(value);
            bound.insert(local);
        }
        args.push(build(&bound)?);
        Ok(call("_xlfn.LET", &args))
    }

    /// The formula for a binding's definition, with `lets` bound around it. A value carried from
    /// one position to the next is worked out by a scan.
    fn definition_formula(
        &self,
        binding: usize,
        lets: &BTreeSet<usize>,
    ) -> Result<Formula, Diagnostic> {
        match self.model.recurrence_of(binding) {
            Some(recurrence) if recurrence.state == binding => self.scan(recurrence, lets),
            _ => self.formula(
                self.model.definition(self.program, binding),
                Scope::global(lets),
            ),
        }
    }

    /// A value carried from one position to the next, as a SCAN over the positions. The scan
    /// starts from the value's first value; at each position it works out the value from the
    /// value a position back, with the other members of the cycle written in place, so that the
    /// workbook holds no circular reference.
    fn scan(&self, recurrence: &Recurrence, lets: &BTreeSet<usize>) -> Result<Formula, Diagnostic> {
        let state = recurrence.state;
        let axis = self.typed.carried[&state];
        let position = Position {
            members: recurrence.members.iter().copied().collect(),
            state,
            axis,
        };
        let definitions = || {
            recurrence
                .members
                .iter()
                .map(|&member| self.model.definition(self.program, member))
        };
        let first = definitions()
            .find_map(|definition| self.first_value(definition, state))
            .expect("Resolve finds the carried value's first value");
        let driver = definitions()
            .find_map(|definition| find(definition, self.typed.drivers[&state]))
            .expect("Infer finds the part the scan runs over");
        let driver = self.formula(driver, Scope::global(lets))?;
        let positions = match axis {
            Axis::Across => call(
                "_xlfn.SEQUENCE",
                &[Formula::atom("1"), call("COLUMNS", &[driver])],
            ),
            Axis::Down => call("_xlfn.SEQUENCE", &[call("ROWS", &[driver])]),
        };
        let scope = Scope {
            params: &[],
            position: Some(&position),
            lets,
        };
        let body = self.formula(self.model.definition(self.program, state), scope)?;
        let step = call(
            "_xlfn.LAMBDA",
            &[
                Formula::atom(format!("_xlpm.{}", TOOL_NAMES[3])),
                Formula::atom(format!("_xlpm.{}", TOOL_NAMES[4])),
                body,
            ],
        );
        Ok(call(
            "_xlfn.SCAN",
            &[self.formula(first, Scope::global(lets))?, positions, step],
        ))
    }

    /// The first value in `lag 1 state seed first`, if the expression holds one.
    fn first_value<'e>(&self, expr: &'e Expr, state: usize) -> Option<&'e Expr> {
        if let ExprKind::Apply {
            func,
            args,
            seed: Some(seed),
            ..
        } = &expr.kind
            && func.name == "lag"
            && matches!(&args[1].kind, ExprKind::Name(name)
                if self.model.symbols.get(name) == Some(&Symbol::Binding(state)))
        {
            return Some(seed);
        }
        expr.children()
            .into_iter()
            .find_map(|child| self.first_value(child, state))
    }

    /// Whether an expression uses a member of the recurrence.
    fn mentions(&self, expr: &Expr, position: &Position) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => matches!(
                self.model.symbols.get(name),
                Some(Symbol::Binding(binding)) if position.members.contains(binding)
            ),
            _ => expr
                .children()
                .into_iter()
                .any(|child| self.mentions(child, position)),
        }
    }

    /// A lambda, as a defined name whose value is a LAMBDA of its parameters. It can be called by
    /// name in the workbook as well as in the file.
    fn function(&self, binding: usize) -> Result<DefinedName, Diagnostic> {
        let b = &self.model.bindings[binding];
        let definition = self.model.definition(self.program, binding);
        let body = self.with_lets(&[definition], &b.params, |lets| {
            let scope = Scope {
                params: &b.params,
                position: None,
                lets,
            };
            self.formula(definition, scope)
        })?;
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

    /// What a formula's or a value's defined name refers to.
    fn reference(&self, binding: usize, sheet: &str, (row, column): At) -> String {
        let shape = &self.grounded.shapes[binding];
        let sheet = sheet_reference(sheet);
        let start = absolute(to_u32(row), to_u32(column));
        let rows = shape.rows.size.as_constant();
        let columns = shape.columns.size.as_constant();
        match (self.model.bindings[binding].role, rows, columns) {
            (_, Some(1), Some(1)) => format!("{sheet}!{start}"),
            (Role::Value, Some(rows), Some(columns)) => {
                let end = absolute(to_u32(row + rows - 1), to_u32(column + columns - 1));
                format!("{sheet}!{start}:{end}")
            }
            _ => format!("_xlfn.ANCHORARRAY({sheet}!{start})"),
        }
    }

    /// The way `lag`, `tail` or `init` at `expr` works: as written, or as Infer found.
    fn axis(&self, expr: &Expr, written: Option<Axis>) -> Axis {
        written.unwrap_or_else(|| {
            *self
                .typed
                .axes
                .get(&expr.pos)
                .expect("Infer finds the way every lag, tail and init works")
        })
    }

    /// The formula for an expression, in `scope`.
    fn formula(&self, expr: &Expr, scope: Scope<'_>) -> Result<Formula, Diagnostic> {
        // Inside one position of a recurrence, a part outside the cycle is worked out whole, and
        // read at the position if it runs along the recurrence.
        if let Some(position) = scope.position
            && !self.mentions(expr, position)
        {
            let whole = self.formula(
                expr,
                Scope {
                    position: None,
                    ..scope
                },
            )?;
            if !self.typed.along.contains(&expr.pos) {
                return Ok(whole);
            }
            let at = Formula::atom(format!("_xlpm.{}", TOOL_NAMES[4]));
            let one = Formula::atom("1");
            return Ok(match position.axis {
                Axis::Across => call("INDEX", &[whole, one, at]),
                Axis::Down => call("INDEX", &[whole, at, one]),
            });
        }
        let formula = match &expr.kind {
            ExprKind::Number { value, .. } => Formula::atom(number(*value)),
            ExprKind::Str(value) => {
                if value.chars().count() > MAX_FORMULA_STRING {
                    return Err(Diagnostic::error(
                        expr.pos,
                        format!(
                            "a string in a formula has at most {MAX_FORMULA_STRING} characters; \
                             a longer text can be a value"
                        ),
                    ));
                }
                Formula::atom(format!("\"{}\"", value.replace('"', "\"\"")))
            }
            ExprKind::Bool(value) => Formula::atom(if *value { "TRUE" } else { "FALSE" }),
            ExprKind::Name(name) if scope.params.contains(name) => {
                Formula::atom(format!("_xlpm.{name}"))
            }
            ExprKind::Name(name) => match self.model.symbols.get(name) {
                // Another member of the cycle, at the same position: its definition, in place.
                Some(Symbol::Binding(binding))
                    if scope
                        .position
                        .is_some_and(|position| position.members.contains(binding)) =>
                {
                    self.formula(self.model.definition(self.program, *binding), scope)?
                }
                Some(Symbol::Binding(binding)) if scope.lets.contains(binding) => {
                    Formula::atom(format!("_xlpm.{name}"))
                }
                Some(Symbol::Binding(binding))
                    if self.model.bindings[*binding].role == Role::Local =>
                {
                    self.definition_formula(*binding, scope.lets)?
                }
                Some(Symbol::Binding(_)) => Formula::atom(name),
                Some(Symbol::Size(var)) => Formula::atom(
                    self.typed
                        .fixed(*var)
                        .expect("Resolve lets formulas use fixed sizes only")
                        .to_string(),
                ),
                None => match Builtin::lookup(name) {
                    Some(Builtin::Na) => call("NA", &[]),
                    _ => unreachable!("Resolve reports unknown names"),
                },
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
            ExprKind::Range { first, last } => self.range(first, last, Axis::Across, scope)?,
            ExprKind::Apply {
                func,
                axis,
                args,
                seed,
            } => match Builtin::lookup(&func.name) {
                Some(builtin) => self.apply(expr, builtin, *axis, args, seed.as_deref(), scope)?,
                // A lambda of the file's own is a defined name, called by that name.
                None => {
                    let args = args
                        .iter()
                        .map(|arg| self.formula(arg, scope))
                        .collect::<Result<Vec<_>, _>>()?;
                    call(&func.name, &args)
                }
            },
            ExprKind::Text(_)
            | ExprKind::Constructor(_)
            | ExprKind::Record(_)
            | ExprKind::Update { .. } => unreachable!("Resolve keeps settings out of formulas"),
        };
        Ok(formula)
    }

    /// The whole numbers from `first` to `last`, as a row or as a column.
    fn range(
        &self,
        first: &Expr,
        last: &Expr,
        axis: Axis,
        scope: Scope<'_>,
    ) -> Result<Formula, Diagnostic> {
        let start = first
            .integer()
            .expect("Infer checks that a range starts at a whole number");
        // The count of whole numbers from `start` to `last` is `last - (start - 1)`, worked out
        // here when `last` is written in the file.
        let count = match last.integer() {
            Some(end) => Formula::atom((end - start + 1).to_string()),
            None => {
                let last = self.formula(last, scope)?;
                match start - 1 {
                    0 => last,
                    offset if offset > 0 => {
                        binary(BinOp::Sub, &last, &Formula::atom(offset.to_string()))
                    }
                    offset => binary(BinOp::Add, &last, &Formula::atom((-offset).to_string())),
                }
            }
        };
        let mut args = match axis {
            Axis::Across => vec![Formula::atom("1"), count],
            Axis::Down => vec![count],
        };
        if start != 1 {
            if axis == Axis::Down {
                args.push(Formula::atom("1"));
            }
            args.push(Formula::atom(start.to_string()));
        }
        Ok(call("_xlfn.SEQUENCE", &args))
    }

    fn apply(
        &self,
        expr: &Expr,
        builtin: Builtin,
        written: Option<Axis>,
        args: &[Expr],
        seed: Option<&Expr>,
        scope: Scope<'_>,
    ) -> Result<Formula, Diagnostic> {
        let arg = |index: usize| self.formula(&args[index], scope);
        // DROP takes rows first and columns second; an empty argument drops nothing.
        let drop = |x: Formula, axis: Axis, count: i64| {
            let count = Formula::atom(count.to_string());
            match axis {
                Axis::Across => call("_xlfn.DROP", &[x, Formula::atom(""), count]),
                Axis::Down => call("_xlfn.DROP", &[x, count]),
            }
        };
        if let Some(app) = builtin.app() {
            let args = app
                .order
                .iter()
                .map(|&index| arg(index))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(call(app.name, &args));
        }
        let formula = match builtin {
            // Inside one position of a recurrence, the value a position back is the scan's own.
            Builtin::Lag
                if seed.is_some()
                    && scope.position.is_some_and(|position| {
                        matches!(&args[1].kind, ExprKind::Name(name)
                            if self.model.symbols.get(name) == Some(&Symbol::Binding(position.state)))
                    }) =>
            {
                Formula::atom(format!("_xlpm.{}", TOOL_NAMES[3]))
            }
            Builtin::Lag => {
                let axis = self.axis(expr, written);
                let steps = args[0]
                    .integer()
                    .expect("Infer checks that a lag is a whole number");
                let dropped = drop(arg(1)?, axis, -steps);
                match seed {
                    Some(seed) => {
                        let stack = match axis {
                            Axis::Across => "_xlfn.HSTACK",
                            Axis::Down => "_xlfn.VSTACK",
                        };
                        call(stack, &[self.formula(seed, scope)?, dropped])
                    }
                    None => dropped,
                }
            }
            Builtin::Tail => drop(arg(0)?, self.axis(expr, written), 1),
            Builtin::Init => drop(arg(0)?, self.axis(expr, written), -1),
            Builtin::SumColumns | Builtin::SumRows => {
                let by = if builtin == Builtin::SumColumns {
                    "_xlfn.BYCOL"
                } else {
                    "_xlfn.BYROW"
                };
                let x = Formula::atom(format!("_xlpm.{}", TOOL_NAMES[0]));
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
                    let (x, y) = (
                        Formula::atom(format!("_xlpm.{}", TOOL_NAMES[1])),
                        Formula::atom(format!("_xlpm.{}", TOOL_NAMES[2])),
                    );
                    let choice = call("IF", &[binary(op, &x, &y), x.clone(), y.clone()]);
                    call("_xlfn.LET", &[x, a, y, b, choice])
                }
            }
            // A column of whole numbers is written as one, rather than as a row transposed.
            Builtin::Transpose => match &args[0].kind {
                ExprKind::Range { first, last } => self.range(first, last, Axis::Down, scope)?,
                _ => call("TRANSPOSE", &[arg(0)?]),
            },
            Builtin::Column => {
                let (ExprKind::Str(name), ExprKind::Name(table)) = (&args[0].kind, &args[1].kind)
                else {
                    unreachable!("Resolve checks the column's name and its table");
                };
                let Some(&Symbol::Binding(binding)) = self.model.symbols.get(table) else {
                    unreachable!("Resolve checks the table");
                };
                let index = self.model.tables[&binding]
                    .header
                    .iter()
                    .position(|column| column == name)
                    .expect("Resolve checks the column's name");
                // INDEX with row 0 gives the whole column.
                call(
                    "INDEX",
                    &[
                        arg(1)?,
                        Formula::atom("0"),
                        Formula::atom((index + 1).to_string()),
                    ],
                )
            }
            // The nearest multiple, a half away from zero, whatever the signs of x and m:
            // SIGN(x)*FLOOR.MATH(ABS(x)+ABS(m)/2,ABS(m)). MROUND would say the same of single
            // values, but gives #VALUE! for a range where it takes a number.
            Builtin::RoundTo => {
                let (multiple, x) = (arg(0)?, arg(1)?);
                let size = call("ABS", &[multiple]);
                let nearest = |x: Formula| {
                    let half = binary(BinOp::Div, &size, &Formula::atom("2"));
                    let shifted = binary(BinOp::Add, &call("ABS", std::slice::from_ref(&x)), &half);
                    let floor = call("_xlfn.FLOOR.MATH", &[shifted, size.clone()]);
                    binary(BinOp::Mul, &call("SIGN", &[x]), &floor)
                };
                if x.is_simple() {
                    nearest(x)
                } else {
                    let value = Formula::atom(format!("_xlpm.{}", TOOL_NAMES[1]));
                    call("_xlfn.LET", &[value.clone(), x, nearest(value)])
                }
            }
            Builtin::Na => call("NA", &[]),
            Builtin::Csv => unreachable!("Resolve allows csv only as a value's definition"),
            _ => unreachable!("a function written as the app's is written above"),
        };
        Ok(formula)
    }

    /// The widths of the columns and the heights of the rows that the bindings on a sheet set,
    /// in pixels. Only a binding sets one: nothing is sized to fit its contents. Check has made
    /// sure that each is on a range of known extent and that no two disagree.
    fn sizes(&self, sheet: usize) -> (BTreeMap<u32, u32>, BTreeMap<u32, u32>) {
        let mut widths = BTreeMap::new();
        let mut heights = BTreeMap::new();
        for &binding in &self.model.layout {
            if self.model.bindings[binding].sheet != Some(sheet) {
                continue;
            }
            let sig = self.model.signature(self.program, binding);
            let (row, column) = self.grounded.anchors[&binding];
            let shape = &self.grounded.shapes[binding];
            let lines = |first: i64, size: Option<i64>| {
                let count = size.expect("Check refuses a size on a range of unknown extent");
                to_u32(first)..to_u32(first + count)
            };
            if let Some((pixels, _)) = sig.width {
                for at in lines(column, shape.columns.size.as_constant()) {
                    widths.insert(at, pixels);
                }
            }
            if let Some((pixels, _)) = sig.height {
                for at in lines(row, shape.rows.size.as_constant()) {
                    heights.insert(at, pixels);
                }
            }
        }
        (widths, heights)
    }
}

/// The expression at `pos` within `expr`.
fn find(expr: &Expr, pos: Pos) -> Option<&Expr> {
    if expr.pos == pos {
        return Some(expr);
    }
    expr.children()
        .into_iter()
        .find_map(|child| find(child, pos))
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
            "the formula for `{}` {problem}; make parts of it formulas of their own",
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
        Value::Formula(_) | Value::Empty => unreachable!("a literal is a value"),
    }
}

fn rgb(color: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}

/// A style in words, for a listing.
fn notes(style: &CellStyle) -> Vec<String> {
    let mut notes = Vec::new();
    let font = &style.font;
    if font.bold == Some(true) {
        notes.push("bold".to_string());
    }
    if font.italic == Some(true) {
        notes.push("italic".to_string());
    }
    if let Some(name) = &font.name {
        notes.push(format!("font '{name}'"));
    }
    if let Some(size) = font.size {
        notes.push(format!("size {}", number(size)));
    }
    if let Some(color) = font.color {
        notes.push(format!("color {}", rgb(color)));
    }
    if let Some(fill) = style.fill {
        notes.push(format!("fill {}", rgb(fill)));
    }
    let border = &style.border;
    for (side, edge) in [
        ("left", &border.left),
        ("right", &border.right),
        ("top", &border.top),
        ("bottom", &border.bottom),
    ] {
        if let Some(edge) = edge {
            let mut note = format!("{side} {}", edge.weight.unwrap_or("none"));
            if let Some(color) = edge.color {
                note.push_str(&format!(" {}", rgb(color)));
            }
            notes.push(note);
        }
    }
    let align = &style.align;
    notes.extend(align.horizontal.map(|h| format!("horizontal {h}")));
    notes.extend(align.vertical.map(|v| format!("vertical {v}")));
    if align.wrap == Some(true) {
        notes.push("wrap".to_string());
    }
    notes.extend(align.indent.map(|i| format!("indent {i}")));
    if let Some(format) = &style.format {
        notes.push(format!("format '{format}'"));
    }
    notes
}

/// The cells and names of a workbook, as text, for reading what Lower produced.
pub fn listing(workbook: &Workbook) -> String {
    let mut out = String::new();
    let bracketed = |notes: Vec<String>| {
        if notes.is_empty() {
            String::new()
        } else {
            format!("  [{}]", notes.join(", "))
        }
    };
    for sheet in &workbook.sheets {
        let mut settings = Vec::new();
        if let Some(color) = sheet.color {
            settings.push(format!("tab {}", rgb(color)));
        }
        if sheet.hidden {
            settings.push("hidden".to_string());
        }
        if let Some(width) = sheet.column_width {
            settings.push(format!("column width {width}"));
        }
        if let Some(height) = sheet.row_height {
            settings.push(format!("row height {height}"));
        }
        out.push_str(&format!(
            "sheet '{}'{}\n",
            sheet.name.replace('\'', "''"),
            bracketed(settings)
        ));
        for (&(row, column), cell) in &sheet.cells {
            let value = match &cell.value {
                Value::Number(value) => number(*value),
                Value::Text(text) => format!("\"{}\"", text.replace('"', "\"\"")),
                Value::Bool(value) => String::from(if *value { "TRUE" } else { "FALSE" }),
                Value::Formula(formula) => format!("={formula}"),
                Value::Empty => "_".to_string(),
            };
            let notes = bracketed(notes(&workbook.styles[cell.style]));
            out.push_str(&format!("  {:<6} {value}{notes}\n", cell_name(row, column)));
        }
        // A row or column with a style or a size of its own, with each.
        let line = |style: Option<&usize>, size: Option<&u32>, noun: &str| {
            let mut said = style.map_or_else(Vec::new, |style| notes(&workbook.styles[*style]));
            said.extend(size.map(|size| format!("{noun} {size}")));
            bracketed(said)
        };
        let rows: BTreeSet<u32> = sheet
            .row_styles
            .keys()
            .chain(sheet.heights.keys())
            .copied()
            .collect();
        for row in rows {
            let said = line(
                sheet.row_styles.get(&row),
                sheet.heights.get(&row),
                "height",
            );
            out.push_str(&format!("  row {row}{said}\n"));
        }
        let columns: BTreeSet<u32> = sheet
            .column_styles
            .keys()
            .chain(sheet.widths.keys())
            .copied()
            .collect();
        for column in columns {
            let said = line(
                sheet.column_styles.get(&column),
                sheet.widths.get(&column),
                "width",
            );
            out.push_str(&format!(
                "  column {}{said}\n",
                crate::cells::column_name(column)
            ));
        }
    }
    out.push_str("names\n");
    for name in &workbook.names {
        out.push_str(&format!("  {} = {}\n", name.name, name.refers_to));
    }
    out
}
