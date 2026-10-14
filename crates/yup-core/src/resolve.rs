//! Step 2, Resolve: what each name refers to, which sheet each item belongs to, and the order
//! in which definitions depend on each other.
//!
//! A binding is shown in the workbook when its signature has a label. Without a label it is
//! local: it has no cells of its own, and its formula is written into the formulas that use it.

use std::collections::{BTreeMap, BTreeSet};

use crate::Inputs;
use crate::cells::{looks_like_cell, looks_like_r1c1};
use crate::csv::{self, Table};
use crate::diagnostic::{Diagnostic, Pos};
use crate::functions::Builtin;
use crate::printer::expression;
use crate::syntax::{Expr, ExprKind, Ident, ItemKind, Program, Signature, SizeKind, Text};

/// The sheet that holds items written before any `sheet` statement.
pub const DEFAULT_SHEET: &str = "Sheet1";

/// The most characters a cell can hold, from the spreadsheet app's published limits.
const MAX_CELL_TEXT: usize = 32_767;

/// What a file's names refer to, and how its items are laid out on sheets.
#[derive(Clone, Debug)]
pub struct Model {
    pub sheets: Vec<Sheet>,
    pub bindings: Vec<Binding>,
    pub sizes: Vec<SizeDecl>,
    pub symbols: BTreeMap<String, Symbol>,
    /// Every binding, each after the bindings it uses.
    pub order: Vec<usize>,
    /// The table each input defined by `csv "file.csv"` reads, by binding.
    pub tables: BTreeMap<usize, Table>,
    /// The balances carried from one period to the next.
    pub recurrences: Vec<Recurrence>,
}

/// Bindings that depend on each other in a cycle, broken by `lag 1 state from seed`: a balance
/// carried from one period to the next. Each period's value depends on the period before.
#[derive(Clone, Debug)]
pub struct Recurrence {
    /// The bindings in the cycle, each after the others it uses in the same period.
    pub members: Vec<usize>,
    /// The binding carried from one period to the next.
    pub state: usize,
}

impl Model {
    /// The recurrence a binding belongs to, if any.
    pub fn recurrence_of(&self, binding: usize) -> Option<&Recurrence> {
        self.recurrences
            .iter()
            .find(|recurrence| recurrence.members.contains(&binding))
    }
}

/// A binding that a definition uses. A use inside `lag 1 x from seed`, in `x`, is delayed: it
/// reads earlier periods only, so it does not stop a cycle being worked out period by period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Use {
    binding: usize,
    delayed: bool,
}

/// What a name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Binding(usize),
    Size(usize),
}

/// A sheet and the items laid out on it, in order.
#[derive(Clone, Debug)]
pub struct Sheet {
    pub name: String,
    pub pos: Pos,
    /// The index of the `sheet` statement, or `None` for the sheet that holds items written
    /// before any `sheet` statement.
    pub statement: Option<usize>,
    pub items: Vec<SheetItem>,
}

/// An item in a sheet's layout: a heading or a gap (by item index), or a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetItem {
    Heading(usize),
    Gap(usize),
    Binding(usize),
}

/// Whether a binding has cells of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Shown with its label, its values computed by a formula.
    Shown,
    /// Shown with its label, its values written into the cells for the workbook's user to
    /// change.
    Input,
    /// Not shown; its formula is written into the formulas that use it.
    Local,
    /// A function of its parameters, written to the workbook as a LAMBDA in a defined name.
    Function,
}

/// A name defined by `name = …`.
#[derive(Clone, Debug)]
pub struct Binding {
    pub name: String,
    pub pos: Pos,
    /// The index of the signature item, if there is one.
    pub signature: Option<usize>,
    /// The index of the definition item.
    pub definition: usize,
    pub role: Role,
    /// The sheet a shown binding is laid out on.
    pub sheet: Option<usize>,
    /// A function's parameters; empty for any other binding.
    pub params: Vec<String>,
}

/// A size variable declared by `name : Size …`.
#[derive(Clone, Debug)]
pub struct SizeDecl {
    pub name: String,
    pub pos: Pos,
    pub kind: SizeKind,
    /// The value of a fixed size.
    pub value: Option<i64>,
}

impl Model {
    /// A binding's signature, if it has one.
    pub fn signature<'p>(&self, program: &'p Program, binding: usize) -> Option<&'p Signature> {
        let index = self.bindings[binding].signature?;
        Some(signature_at(program, index))
    }

    /// A binding's definition.
    pub fn definition<'p>(&self, program: &'p Program, binding: usize) -> &'p Expr {
        match &program.items[self.bindings[binding].definition].kind {
            ItemKind::Definition { expr, .. } => expr,
            _ => unreachable!("a binding's definition index points at a definition"),
        }
    }
}

fn signature_at(program: &Program, index: usize) -> &Signature {
    match &program.items[index].kind {
        ItemKind::Signature(sig) => sig,
        _ => unreachable!("a signature index points at a signature"),
    }
}

fn twice(name: &str, pos: Pos, first: Pos) -> Diagnostic {
    Diagnostic::error(pos, format!("`{name}` is already declared at {first}"))
}

/// Resolves every name in `program`.
pub fn resolve(program: &Program, inputs: &Inputs) -> Result<Model, Vec<Diagnostic>> {
    let mut errors = Vec::new();

    // Each name is declared once: as a size, or by a signature, a definition or both.
    let mut sizes: Vec<SizeDecl> = Vec::new();
    let mut size_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut signatures: BTreeMap<String, usize> = BTreeMap::new();
    let mut definitions: BTreeMap<String, usize> = BTreeMap::new();
    for (index, item) in program.items.iter().enumerate() {
        match &item.kind {
            ItemKind::Size { name, kind } => {
                errors.extend(name_error(&name.name, name.pos));
                if let Some(&other) = size_index.get(&name.name) {
                    errors.push(twice(&name.name, name.pos, sizes[other].pos));
                } else if let Some(&other) = signatures.get(&name.name) {
                    errors.push(twice(&name.name, name.pos, program.items[other].pos));
                } else {
                    size_index.insert(name.name.clone(), sizes.len());
                    sizes.push(SizeDecl {
                        name: name.name.clone(),
                        pos: name.pos,
                        kind: *kind,
                        value: None,
                    });
                }
            }
            ItemKind::Signature(sig) => {
                let name = &sig.name;
                errors.extend(name_error(&name.name, name.pos));
                if let Some(&other) = signatures.get(&name.name) {
                    errors.push(twice(&name.name, name.pos, program.items[other].pos));
                } else if size_index.contains_key(&name.name) {
                    errors.push(Diagnostic::error(
                        name.pos,
                        format!(
                            "`{0}` is a size variable; it takes a value, such as `{0} = 4`, \
                             not a signature",
                            name.name
                        ),
                    ));
                } else {
                    signatures.insert(name.name.clone(), index);
                }
                errors.extend(sig.label.as_ref().and_then(text_error));
                if let Some(format) = &sig.format
                    && let Some(problem) = format_problem(&format.value)
                {
                    errors.push(Diagnostic::error(format.pos, problem));
                }
            }
            ItemKind::Definition { name, .. } => {
                errors.extend(name_error(&name.name, name.pos));
                if let Some(&other) = definitions.get(&name.name) {
                    errors.push(Diagnostic::error(
                        name.pos,
                        format!(
                            "`{}` is already defined at {}",
                            name.name, program.items[other].pos
                        ),
                    ));
                } else {
                    definitions.insert(name.name.clone(), index);
                }
            }
            ItemKind::Heading { text, .. } => errors.extend(text_error(text)),
            _ => {}
        }
    }

    // A fixed size takes its value from a definition; an unknown size never has one.
    for size in &mut sizes {
        let definition = definitions.remove(&size.name);
        match (size.kind, definition) {
            (SizeKind::Fixed, Some(index)) => {
                let ItemKind::Definition { expr, .. } = &program.items[index].kind else {
                    unreachable!("a definition index points at a definition");
                };
                match expr.integer() {
                    Some(value) if value >= 1 => size.value = Some(value),
                    _ => errors.push(Diagnostic::error(
                        expr.pos,
                        format!(
                            "a fixed size is a whole number, at least 1, such as `{} = 4`",
                            size.name
                        ),
                    )),
                }
            }
            (SizeKind::Fixed, None) => errors.push(Diagnostic::error(
                size.pos,
                format!(
                    "the fixed size `{0}` needs a value, such as `{0} = 4`",
                    size.name
                ),
            )),
            (SizeKind::Unknown, Some(index)) => errors.push(Diagnostic::error(
                program.items[index].pos,
                format!(
                    "`{}` is an unknown size: the workbook decides its value, so it cannot be \
                     given one here",
                    size.name
                ),
            )),
            (SizeKind::Unknown, None) => {}
        }
    }

    let mut symbols: BTreeMap<String, Symbol> = BTreeMap::new();
    for (index, size) in sizes.iter().enumerate() {
        symbols.insert(size.name.clone(), Symbol::Size(index));
    }

    // Bindings are numbered in the order their definitions appear.
    let mut in_order: Vec<(usize, String)> = definitions
        .iter()
        .map(|(name, &index)| (index, name.clone()))
        .collect();
    in_order.sort();
    let mut bindings: Vec<Binding> = Vec::new();
    for (index, name) in in_order {
        let ItemKind::Definition { params, .. } = &program.items[index].kind else {
            unreachable!("a definition index points at a definition");
        };
        let signature = signatures.get(&name).copied();
        let sig = signature.map(|index| signature_at(program, index));
        let role = match sig {
            _ if !params.is_empty() => Role::Function,
            Some(sig) if sig.input.is_some() => Role::Input,
            Some(sig) if sig.label.is_some() => Role::Shown,
            _ => Role::Local,
        };
        if let Some(sig) = sig {
            errors.extend(role_errors(sig, role));
        }
        for (i, param) in params.iter().enumerate() {
            errors.extend(name_error(&param.name, param.pos));
            if params[..i].iter().any(|other| other.name == param.name) {
                errors.push(Diagnostic::error(
                    param.pos,
                    format!("the parameter `{}` is given twice", param.name),
                ));
            }
        }
        symbols.insert(name.clone(), Symbol::Binding(bindings.len()));
        bindings.push(Binding {
            name,
            pos: program.items[index].pos,
            signature,
            definition: index,
            role,
            sheet: None,
            params: params.iter().map(|param| param.name.clone()).collect(),
        });
    }
    for (name, &index) in &signatures {
        if !definitions.contains_key(name) {
            errors.push(Diagnostic::error(
                program.items[index].pos,
                format!("`{name}` has a signature but no definition, such as `{name} = …`"),
            ));
        }
    }

    let sheets = lay_out(program, &symbols, &mut bindings, &mut errors);

    // An input defined as `csv "file.csv"` is a table, read from the file the caller gave.
    let mut tables = BTreeMap::new();
    let mut table_bindings = BTreeSet::new();
    for (index, binding) in bindings.iter().enumerate() {
        let ItemKind::Definition { expr, .. } = &program.items[binding.definition].kind else {
            unreachable!("a binding's definition index points at a definition");
        };
        let Some((file, pos)) = csv_file(expr) else {
            continue;
        };
        if binding.role != Role::Input {
            continue;
        }
        table_bindings.insert(index);
        match inputs.get(file) {
            None => errors.push(Diagnostic::error(
                pos,
                format!("the input file `{file}` was not given"),
            )),
            Some(text) => match csv::parse(text) {
                Ok(table) => {
                    tables.insert(index, table);
                }
                Err(error) => errors.push(error.in_file(file)),
            },
        }
    }

    let mut uses: Vec<Vec<Use>> = Vec::with_capacity(bindings.len());
    for (index, binding) in bindings.iter().enumerate() {
        let ItemKind::Definition { expr, .. } = &program.items[binding.definition].kind else {
            unreachable!("a binding's definition index points at a definition");
        };
        let mut found = Vec::new();
        if !table_bindings.contains(&index) {
            let scope = Scope {
                symbols: &symbols,
                sizes: &sizes,
                bindings: &bindings,
                params: &binding.params,
                tables: &tables,
                table_bindings: &table_bindings,
            };
            scope.resolve_expr(expr, false, &mut found, &mut errors);
            if binding.role == Role::Input
                && let Err(error) = check_input(expr, binding.pos)
            {
                errors.push(error);
            }
        }
        // A function's uses are never delayed, so a cycle through a function stays an error.
        if binding.role == Role::Function {
            for found in &mut found {
                found.delayed = false;
            }
        }
        uses.push(found);
    }
    let (order, cycles) = match dependency_order(&bindings, &uses) {
        Ok(ordered) => ordered,
        Err(error) => {
            errors.push(error);
            (Vec::new(), Vec::new())
        }
    };
    let mut recurrences = Vec::new();
    for members in cycles {
        match recurrence(program, &symbols, &bindings, &uses, members) {
            Ok(recurrence) => recurrences.push(recurrence),
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Ok(Model {
            sheets,
            bindings,
            sizes,
            symbols,
            order,
            tables,
            recurrences,
        })
    } else {
        Err(errors)
    }
}

/// The file an input reads, and where its name is written, when the definition is
/// `csv "file.csv"`.
fn csv_file(expr: &Expr) -> Option<(&str, Pos)> {
    let ExprKind::Apply { func, args, .. } = &expr.kind else {
        return None;
    };
    match (func.name.as_str(), args.as_slice()) {
        (
            "csv",
            [
                Expr {
                    kind: ExprKind::Str(file),
                    pos,
                },
            ],
        ) => Some((file, *pos)),
        _ => None,
    }
}

/// Places headings, gaps and shown bindings on the sheet whose section they are written in.
fn lay_out(
    program: &Program,
    symbols: &BTreeMap<String, Symbol>,
    bindings: &mut [Binding],
    errors: &mut Vec<Diagnostic>,
) -> Vec<Sheet> {
    let mut sheets: Vec<Sheet> = Vec::new();
    let mut current: Option<usize> = None;
    for (index, item) in program.items.iter().enumerate() {
        let layout = match &item.kind {
            ItemKind::Sheet(name) => {
                errors.extend(sheet_name_error(&name.value, name.pos));
                if let Some(other) = sheets
                    .iter()
                    .find(|sheet| sheet.name.to_lowercase() == name.value.to_lowercase())
                {
                    errors.push(Diagnostic::error(
                        name.pos,
                        format!(
                            "there is already a sheet called '{}' at {}; sheet names ignore case",
                            other.name, other.pos
                        ),
                    ));
                }
                sheets.push(Sheet {
                    name: name.value.clone(),
                    pos: name.pos,
                    statement: Some(index),
                    items: Vec::new(),
                });
                current = Some(sheets.len() - 1);
                continue;
            }
            ItemKind::Heading { .. } => SheetItem::Heading(index),
            ItemKind::Gap => SheetItem::Gap(index),
            ItemKind::Signature(sig) => match symbols.get(&sig.name.name) {
                Some(&Symbol::Binding(b))
                    if bindings[b].signature == Some(index) && bindings[b].role != Role::Local =>
                {
                    SheetItem::Binding(b)
                }
                _ => continue,
            },
            _ => continue,
        };
        let sheet = *current.get_or_insert_with(|| {
            sheets.push(Sheet {
                name: DEFAULT_SHEET.to_string(),
                pos: item.pos,
                statement: None,
                items: Vec::new(),
            });
            sheets.len() - 1
        });
        if let SheetItem::Binding(b) = layout {
            bindings[b].sheet = Some(sheet);
        }
        sheets[sheet].items.push(layout);
    }
    // A workbook has at least one sheet, even when nothing is shown.
    if sheets.is_empty() {
        sheets.push(Sheet {
            name: DEFAULT_SHEET.to_string(),
            pos: Pos::START,
            statement: None,
            items: Vec::new(),
        });
    }
    sheets
}

/// Attributes that only make sense for what is shown, on a binding that is not.
fn role_errors(sig: &Signature, role: Role) -> Vec<Diagnostic> {
    let name = &sig.name.name;
    let mut errors = Vec::new();
    if role == Role::Function {
        errors.push(Diagnostic::error(
            sig.name.pos,
            format!(
                "`{name}` is a function, which has no cells and so no signature; its size is \
                 worked out where it is used"
            ),
        ));
        return errors;
    }
    if role == Role::Input && sig.label.is_none() {
        errors.push(Diagnostic::error(
            sig.name.pos,
            format!("an input is shown with a label, such as `{name} : 'Forecast horizon' Input`"),
        ));
    }
    if role == Role::Local {
        let placed = [
            (sig.at.as_ref().map(|at| at.pos), "`@`"),
            (sig.format.as_ref().map(|format| format.pos), "`Format`"),
            (sig.bold, "`Bold`"),
            (sig.italic, "`Italic`"),
        ];
        for (pos, what) in placed {
            if let Some(pos) = pos {
                errors.push(Diagnostic::error(
                    pos,
                    format!("{what} applies to a shown binding, and `{name}` has no label"),
                ));
            }
        }
    }
    errors
}

/// What the names in a definition can refer to: the file's names, and, inside a function, its
/// parameters, which come first.
struct Scope<'a> {
    symbols: &'a BTreeMap<String, Symbol>,
    sizes: &'a [SizeDecl],
    bindings: &'a [Binding],
    params: &'a [String],
    /// The tables that were read.
    tables: &'a BTreeMap<usize, Table>,
    /// Every input defined by `csv`, whether or not its file could be read.
    table_bindings: &'a BTreeSet<usize>,
}

/// How a function is called, for messages: `name a b`.
fn call_usage(binding: &Binding) -> String {
    let mut usage = binding.name.clone();
    for param in &binding.params {
        usage.push(' ');
        usage.push_str(param);
    }
    usage
}

impl Scope<'_> {
    fn resolve_expr(
        &self,
        expr: &Expr,
        delayed: bool,
        uses: &mut Vec<Use>,
        errors: &mut Vec<Diagnostic>,
    ) {
        match &expr.kind {
            ExprKind::Name(name) if self.params.contains(name) => {}
            ExprKind::Name(name) => match self.symbols.get(name) {
                Some(Symbol::Binding(b)) if self.bindings[*b].role == Role::Function => {
                    errors.push(Diagnostic::error(
                        expr.pos,
                        format!(
                            "`{name}` is a function; give it its arguments: `{}`",
                            call_usage(&self.bindings[*b])
                        ),
                    ));
                }
                Some(Symbol::Binding(b)) => uses.push(Use {
                    binding: *b,
                    delayed,
                }),
                Some(Symbol::Size(s)) => {
                    if self.sizes[*s].kind == SizeKind::Unknown {
                        errors.push(Diagnostic::error(
                            expr.pos,
                            format!(
                                "`{name}` is an unknown size; a formula cannot use its value, but \
                                 can use the range it is the size of"
                            ),
                        ));
                    }
                }
                None => {
                    let message = match Builtin::lookup(name) {
                        Some(builtin) => format!(
                            "`{name}` is a function; give it its arguments: `{}`",
                            builtin.usage()
                        ),
                        None => format!("unknown name `{name}`"),
                    };
                    errors.push(Diagnostic::error(expr.pos, message));
                }
            },
            ExprKind::Apply { func, args, seed } => {
                self.resolve_call(func, args.len(), uses, errors);
                match (func.name.as_str(), args.as_slice()) {
                    ("csv", _) => errors.push(Diagnostic::error(
                        func.pos,
                        "`csv` reads a file into an input, as the whole of its definition: \
                         `history = csv \"history.csv\"`, with `Input` in its signature",
                    )),
                    ("column", [name, table]) => errors.extend(self.column_error(name, table)),
                    // What `lag 1 x from seed` lags is read only in earlier periods.
                    ("lag", [periods, lagged]) if seed.is_some() => {
                        self.resolve_expr(periods, delayed, uses, errors);
                        self.resolve_expr(lagged, true, uses, errors);
                        if let Some(seed) = seed {
                            self.resolve_expr(seed, delayed, uses, errors);
                        }
                        return;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        for child in expr.children() {
            self.resolve_expr(child, delayed, uses, errors);
        }
    }

    /// Checks `column "name" table`: a name in quotes, of a column the table has.
    fn column_error(&self, name: &Expr, table: &Expr) -> Option<Diagnostic> {
        let ExprKind::Str(column) = &name.kind else {
            return Some(Diagnostic::error(
                name.pos,
                "`column` takes the column's name in double quotes: `column \"revenue\" history`",
            ));
        };
        let binding = match &table.kind {
            ExprKind::Name(table_name) if !self.params.contains(table_name) => {
                match self.symbols.get(table_name) {
                    Some(Symbol::Binding(b)) if self.table_bindings.contains(b) => *b,
                    _ => {
                        return Some(Diagnostic::error(
                            table.pos,
                            format!(
                                "`column` takes a table, an input defined by `csv`, and \
                                 `{table_name}` is not one"
                            ),
                        ));
                    }
                }
            }
            _ => {
                return Some(Diagnostic::error(
                    table.pos,
                    "`column` takes a table by its name: an input defined by `csv`",
                ));
            }
        };
        // A table whose file could not be read has been reported already.
        let table = self.tables.get(&binding)?;
        (!table.header.contains(column)).then(|| {
            Diagnostic::error(
                name.pos,
                format!(
                    "the table `{}` has no column `{column}`; its columns are {}",
                    self.bindings[binding].name,
                    table.header.join(", ")
                ),
            )
        })
    }

    fn resolve_call(
        &self,
        func: &Ident,
        given: usize,
        uses: &mut Vec<Use>,
        errors: &mut Vec<Diagnostic>,
    ) {
        let (expected, usage) = if let Some(builtin) = Builtin::lookup(&func.name) {
            (builtin.arity(), builtin.usage().to_string())
        } else if self.params.contains(&func.name) {
            errors.push(Diagnostic::error(
                func.pos,
                format!(
                    "`{}` is a parameter, and a parameter is a value, not a function",
                    func.name
                ),
            ));
            return;
        } else {
            match self.symbols.get(&func.name) {
                Some(Symbol::Binding(b)) if self.bindings[*b].role == Role::Function => {
                    uses.push(Use {
                        binding: *b,
                        delayed: false,
                    });
                    let binding = &self.bindings[*b];
                    (binding.params.len(), call_usage(binding))
                }
                Some(_) => {
                    errors.push(Diagnostic::error(
                        func.pos,
                        format!(
                            "`{}` is not a function, so it cannot take arguments",
                            func.name
                        ),
                    ));
                    return;
                }
                None => {
                    errors.push(Diagnostic::error(
                        func.pos,
                        format!("unknown function `{}`", func.name),
                    ));
                    return;
                }
            }
        };
        if given != expected {
            let plural = if expected == 1 { "" } else { "s" };
            errors.push(Diagnostic::error(
                func.pos,
                format!(
                    "`{}` takes {expected} argument{plural}: `{usage}`",
                    func.name
                ),
            ));
        }
    }
}

/// Checks that an input's definition is written out in the file. Errors about the whole
/// definition point at the binding's name, `pos`.
fn check_input(expr: &Expr, pos: Pos) -> Result<(), Diagnostic> {
    if expr.is_literal() {
        return Ok(());
    }
    if let ExprKind::List(rows) = &expr.kind {
        return match rows.iter().flatten().find(|value| !value.is_literal()) {
            Some(value) => Err(Diagnostic::error(
                value.pos,
                "an input list holds literal values only",
            )),
            None => Ok(()),
        };
    }
    Err(Diagnostic::error(
        pos,
        "an input's value is written in the file: a number, a string, `true`, `false`, or a list \
         of these",
    ))
}

/// Orders the bindings so that each comes after the bindings it uses, and finds the cycles that
/// delayed uses break.
///
/// Bindings that use each other in a cycle form a group, and the groups come in the order of
/// their dependencies. Within a group, a use that is not delayed must not close a cycle: that
/// would be a value that depends on itself in the same period. A group with a delayed use inside
/// it is a recurrence, returned with its members in order.
fn dependency_order(
    bindings: &[Binding],
    uses: &[Vec<Use>],
) -> Result<(Vec<usize>, Vec<Vec<usize>>), Diagnostic> {
    let edges: Vec<Vec<usize>> = uses
        .iter()
        .map(|uses| uses.iter().map(|used| used.binding).collect())
        .collect();
    let mut order = Vec::with_capacity(bindings.len());
    let mut cycles = Vec::new();
    for group in strongly_connected(&edges) {
        let members: BTreeSet<usize> = group.iter().copied().collect();
        let same_period = |binding: usize| -> Vec<usize> {
            uses[binding]
                .iter()
                .filter(|used| !used.delayed && members.contains(&used.binding))
                .map(|used| used.binding)
                .collect()
        };
        let sorted =
            topological(&group, same_period).map_err(|cycle| circular(bindings, &cycle))?;
        let carried = group.iter().any(|&binding| {
            uses[binding]
                .iter()
                .any(|used| used.delayed && members.contains(&used.binding))
        });
        if carried {
            cycles.push(sorted.clone());
        }
        order.extend(sorted);
    }
    Ok((order, cycles))
}

fn circular(bindings: &[Binding], cycle: &[usize]) -> Diagnostic {
    let path: Vec<&str> = cycle
        .iter()
        .map(|&binding| bindings[binding].name.as_str())
        .collect();
    Diagnostic::error(
        bindings[cycle[0]].pos,
        format!(
            "circular definition: {}; a value cannot depend on itself in the same period, but \
             `lag 1 x from first` carries a balance from one period to the next",
            path.join(" -> ")
        ),
    )
}

/// The groups of nodes that reach each other, by Tarjan's algorithm, each group after the
/// groups its nodes use.
fn strongly_connected(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNSEEN: usize = usize::MAX;
    let count = edges.len();
    let mut index = vec![UNSEEN; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut groups = Vec::new();
    for root in 0..count {
        if index[root] != UNSEEN {
            continue;
        }
        // Each entry is a node and how many of its edges have been followed.
        let mut calls = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&(node, followed)) = calls.last() {
            if let Some(&target) = edges[node].get(followed) {
                calls.last_mut().expect("the call is there").1 += 1;
                if index[target] == UNSEEN {
                    index[target] = next;
                    low[target] = next;
                    next += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    calls.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(index[target]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut group = Vec::new();
                loop {
                    let member = stack.pop().expect("the group's nodes are on the stack");
                    on_stack[member] = false;
                    group.push(member);
                    if member == node {
                        break;
                    }
                }
                group.sort_unstable();
                groups.push(group);
            }
        }
    }
    groups
}

/// The nodes in an order where each comes after those `edges` leads to, or a cycle, its first
/// node repeated at the end.
fn topological(
    nodes: &[usize],
    edges: impl Fn(usize) -> Vec<usize>,
) -> Result<Vec<usize>, Vec<usize>> {
    // A node is absent until seen, `false` while its edges are followed, and `true` once done.
    let mut done: BTreeMap<usize, bool> = BTreeMap::new();
    let mut order = Vec::with_capacity(nodes.len());
    for &start in nodes {
        if done.contains_key(&start) {
            continue;
        }
        done.insert(start, false);
        let mut stack = vec![(start, edges(start), 0)];
        while let Some((node, targets, followed)) = stack.last_mut() {
            let Some(&target) = targets.get(*followed) else {
                let node = *node;
                done.insert(node, true);
                order.push(node);
                stack.pop();
                continue;
            };
            *followed += 1;
            match done.get(&target) {
                None => {
                    done.insert(target, false);
                    stack.push((target, edges(target), 0));
                }
                Some(false) => {
                    let from = stack
                        .iter()
                        .position(|(node, _, _)| *node == target)
                        .expect("a node being followed is on the stack");
                    let mut cycle: Vec<usize> =
                        stack[from..].iter().map(|(node, _, _)| *node).collect();
                    cycle.push(target);
                    return Err(cycle);
                }
                Some(true) => {}
            }
        }
    }
    Ok(order)
}

/// Checks a cycle that a delayed use breaks, and finds the balance it carries.
///
/// Each period of a recurrence is worked out from the period before, so inside it the balance
/// and the bindings it depends on are taken one period at a time. Only what works position by
/// position can take them; `lag` takes the balance itself, one period back, always with the same
/// first value.
fn recurrence(
    program: &Program,
    symbols: &BTreeMap<String, Symbol>,
    bindings: &[Binding],
    uses: &[Vec<Use>],
    members: Vec<usize>,
) -> Result<Recurrence, Diagnostic> {
    let in_cycle: BTreeSet<usize> = members.iter().copied().collect();
    let carried: BTreeSet<usize> = members
        .iter()
        .flat_map(|&binding| &uses[binding])
        .filter(|used| used.delayed && in_cycle.contains(&used.binding))
        .map(|used| used.binding)
        .collect();
    let first = &bindings[members[0]];
    if carried.len() > 1 {
        let names: Vec<String> = carried
            .iter()
            .map(|&binding| format!("`{}`", bindings[binding].name))
            .collect();
        return Err(Diagnostic::error(
            first.pos,
            format!(
                "{} are each carried from one period to the next in the same cycle; coupled \
                 balances are not supported yet",
                names.join(" and ")
            ),
        ));
    }
    let state = *carried
        .iter()
        .next()
        .expect("a recurrence has a delayed use inside it");
    let mut check = RecurrenceCheck {
        symbols,
        members: &in_cycle,
        state: &bindings[state].name,
        seed: None,
    };
    for &binding in &members {
        let ItemKind::Definition { expr, .. } = &program.items[bindings[binding].definition].kind
        else {
            unreachable!("a binding's definition index points at a definition");
        };
        check.check(expr)?;
    }
    Ok(Recurrence { members, state })
}

struct RecurrenceCheck<'a> {
    symbols: &'a BTreeMap<String, Symbol>,
    members: &'a BTreeSet<usize>,
    state: &'a str,
    /// The first value, as written, of the first `lag 1 state from first` seen.
    seed: Option<String>,
}

impl RecurrenceCheck<'_> {
    /// Whether an expression uses a binding in the cycle.
    fn mentions(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => matches!(
                self.symbols.get(name),
                Some(Symbol::Binding(binding)) if self.members.contains(binding)
            ),
            _ => expr
                .children()
                .into_iter()
                .any(|child| self.mentions(child)),
        }
    }

    fn check(&mut self, expr: &Expr) -> Result<(), Diagnostic> {
        if !self.mentions(expr) {
            return Ok(());
        }
        let state = self.state;
        match &expr.kind {
            ExprKind::Apply { func, args, seed } if func.name == "lag" => {
                let carries_state = matches!(&args[1].kind, ExprKind::Name(name) if name == state);
                let (Some(seed), true, Some(1)) = (seed, carries_state, args[0].integer()) else {
                    return Err(Diagnostic::error(
                        func.pos,
                        format!(
                            "inside a cycle, `lag` carries the balance itself one period, with \
                             its first value: `lag 1 {state} from first`"
                        ),
                    ));
                };
                if self.mentions(seed) {
                    return Err(Diagnostic::error(
                        seed.pos,
                        "the first value of a balance cannot depend on the balance",
                    ));
                }
                let text = expression(seed);
                match &self.seed {
                    Some(first) if *first != text => Err(Diagnostic::error(
                        seed.pos,
                        format!(
                            "every `lag 1 {state} from …` gives the same first value, and another \
                             gives `{first}`"
                        ),
                    )),
                    _ => {
                        self.seed = Some(text);
                        Ok(())
                    }
                }
            }
            ExprKind::Apply { func, args, .. } => {
                let position_by_position = matches!(
                    Builtin::lookup(&func.name),
                    Some(
                        Builtin::Min
                            | Builtin::Max
                            | Builtin::Abs
                            | Builtin::Exp
                            | Builtin::Ln
                            | Builtin::Sqrt
                            | Builtin::Round
                            | Builtin::Not
                    )
                );
                if !position_by_position {
                    return Err(Diagnostic::error(
                        func.pos,
                        format!(
                            "`{}` cannot take part in a balance carried from one period to the \
                             next, which is worked out one period at a time",
                            func.name
                        ),
                    ));
                }
                args.iter().try_for_each(|arg| self.check(arg))
            }
            ExprKind::List(_) | ExprKind::Range { .. } => Err(Diagnostic::error(
                expr.pos,
                "a list or a range cannot hold a balance carried from one period to the next",
            )),
            _ => expr
                .children()
                .into_iter()
                .try_for_each(|child| self.check(child)),
        }
    }
}

/// Why a binding or size cannot have this name, if it cannot. The rules are those for defined
/// names, so that exporting a binding never forces a rename.
fn name_error(name: &str, pos: Pos) -> Option<Diagnostic> {
    let problem = if Builtin::lookup(name).is_some() {
        "is the name of a function"
    } else if name.len() > 255 {
        "is longer than the 255 characters a defined name can have"
    } else if looks_like_cell(name) {
        "reads as a cell reference in the workbook"
    } else if looks_like_r1c1(name) {
        "reads as a row or column reference in the workbook's R1C1 notation"
    } else {
        return None;
    };
    Some(Diagnostic::error(
        pos,
        format!("`{name}` {problem}; choose another name"),
    ))
}

fn sheet_name_error(name: &str, pos: Pos) -> Option<Diagnostic> {
    let problem = if name.is_empty() {
        "a sheet name cannot be empty".to_string()
    } else if name.chars().count() > 31 {
        "a sheet name has at most 31 characters".to_string()
    } else if let Some(c) = name
        .chars()
        .find(|&c| ":\\/?*[]".contains(c) || c.is_control())
    {
        format!("a sheet name cannot contain `{}`", c.escape_default())
    } else if name.starts_with('\'') || name.ends_with('\'') {
        "a sheet name cannot start or end with `'`".to_string()
    } else if name.eq_ignore_ascii_case("history") {
        "the spreadsheet app keeps the sheet name 'History' for itself".to_string()
    } else {
        return None;
    };
    Some(Diagnostic::error(pos, problem))
}

fn text_error(text: &Text) -> Option<Diagnostic> {
    (text.value.chars().count() > MAX_CELL_TEXT)
        .then(|| Diagnostic::error(text.pos, "a cell holds at most 32,767 characters"))
}

/// Why a number format might be rejected by the spreadsheet app, if it might. The check is
/// conservative: literal text belongs in double quotes.
fn format_problem(format: &str) -> Option<String> {
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

    fn model(source: &str) -> Model {
        resolve(&parse(source).expect("parses"), &Inputs::new()).expect("resolves")
    }

    fn errors(source: &str) -> Vec<String> {
        resolve(&parse(source).expect("parses"), &Inputs::new())
            .expect_err("the source is wrong")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn a_label_makes_a_binding_shown() {
        let model =
            model("yup 0.1\nsheet 'S'\nh : 'Horizon' Input\nh = 5\nx : 'X'\nx = h * 2\ny = x\n");
        let roles: Vec<Role> = model.bindings.iter().map(|b| b.role).collect();
        assert_eq!(roles, [Role::Input, Role::Shown, Role::Local]);
        assert_eq!(
            model.sheets[0].items,
            [SheetItem::Binding(0), SheetItem::Binding(1)]
        );
        assert_eq!(model.order, [0, 1, 2]);
    }

    #[test]
    fn definitions_come_after_what_they_use() {
        let model = model("yup 0.1\na = b + d\nb = d\nd = 1\n");
        let names: Vec<&str> = model
            .order
            .iter()
            .map(|&b| model.bindings[b].name.as_str())
            .collect();
        assert_eq!(names, ["d", "b", "a"]);
    }

    #[test]
    fn items_before_any_sheet_go_on_a_default_sheet() {
        let model = model("yup 0.1\nx : 'X'\nx = 1\nsheet 'Two'\n");
        let names: Vec<&str> = model.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Sheet1", "Two"]);
        assert_eq!(model.sheets[0].statement, None);
    }

    #[test]
    fn names_follow_the_rules_for_defined_names() {
        assert_eq!(
            errors("yup 0.1\ntax2026 = 1\nr1c1 = 2\nsum = 3\nrate = 4\nc = 5\n"),
            [
                "2:1: error: `tax2026` reads as a cell reference in the workbook; choose another \
                 name",
                "3:1: error: `r1c1` reads as a row or column reference in the workbook's R1C1 \
                 notation; choose another name",
                "4:1: error: `sum` is the name of a function; choose another name",
                "6:1: error: `c` reads as a row or column reference in the workbook's R1C1 \
                 notation; choose another name",
            ]
        );
    }

    #[test]
    fn names_are_resolved() {
        assert_eq!(
            errors("yup 0.1\nx = y + sum\nz = x 1\nw = lag 1\n"),
            [
                "2:5: error: unknown name `y`",
                "2:9: error: `sum` is a function; give it its arguments: `sum x`",
                "3:5: error: `x` is not a function, so it cannot take arguments",
                "4:5: error: `lag` takes 2 arguments: `lag 1 x, or lag 1 x from first`",
            ]
        );
    }

    #[test]
    fn a_cycle_is_reported_with_its_path() {
        assert_eq!(
            errors("yup 0.1\na = b\nb = d + 1\nd = a\n"),
            [
                "2:1: error: circular definition: a -> b -> d -> a; a value cannot depend on \
                 itself in the same period, but `lag 1 x from first` carries a balance from one \
                 period to the next"
            ]
        );
    }

    #[test]
    fn a_cycle_through_a_lag_with_a_first_value_is_a_recurrence() {
        let model = model(
            "yup 0.1\nopening = lag 1 closing from 100\npaid = min 30 opening\nclosing = opening \
             - paid\nafter = closing * 2\n",
        );
        let names = |bindings: &[usize]| -> Vec<&str> {
            bindings
                .iter()
                .map(|&b| model.bindings[b].name.as_str())
                .collect()
        };
        assert_eq!(names(&model.order), ["opening", "paid", "closing", "after"]);
        assert_eq!(model.recurrences.len(), 1);
        assert_eq!(
            names(&model.recurrences[0].members),
            ["opening", "paid", "closing"]
        );
        assert_eq!(model.bindings[model.recurrences[0].state].name, "closing");
        let counter = self::model("yup 0.1\ncount = lag 1 count from 0 + 1\n");
        assert_eq!(counter.recurrences[0].members, [0]);
    }

    #[test]
    fn a_recurrence_works_one_period_at_a_time() {
        assert_eq!(
            errors("yup 0.1\nopening = lag 1 closing from 100\nclosing = opening - sum opening\n"),
            [
                "3:21: error: `sum` cannot take part in a balance carried from one period to the \
              next, which is worked out one period at a time"
            ]
        );
        assert_eq!(
            errors("yup 0.1\nopening = lag 2 closing from 100\nclosing = opening - 1\n"),
            [
                "2:11: error: inside a cycle, `lag` carries the balance itself one period, with its \
              first value: `lag 1 closing from first`"
            ]
        );
        assert_eq!(
            errors("yup 0.1\na = lag 1 b from 0\nb = lag 1 a from 0 + 1\n"),
            [
                "2:1: error: `a` and `b` are each carried from one period to the next in the same \
              cycle; coupled balances are not supported yet"
            ]
        );
        assert_eq!(
            errors(
                "yup 0.1\nopening = lag 1 closing from 100\nalso = lag 1 closing from 99\nclosing \
                 = opening + also\n"
            ),
            [
                "3:27: error: every `lag 1 closing from …` gives the same first value, and another \
              gives `100`"
            ]
        );
    }

    #[test]
    fn sizes_and_inputs_are_checked() {
        assert_eq!(
            errors(
                "yup 0.1\nn : Size Unknown\nn = 3\ns : Size Fixed\nh : Input\nh = 1 + 1\nx : \
                 'X' Format \"0.0 years\"\nx = 1\n"
            ),
            [
                "7:16: error: `r` cannot appear in a number format outside double quotes",
                "3:1: error: `n` is an unknown size: the workbook decides its value, so it \
                 cannot be given one here",
                "4:1: error: the fixed size `s` needs a value, such as `s = 4`",
                "5:1: error: an input is shown with a label, such as `h : 'Forecast horizon' \
                 Input`",
                "6:1: error: an input's value is written in the file: a number, a string, \
                 `true`, `false`, or a list of these",
            ]
        );
    }

    #[test]
    fn functions_are_called_with_their_arguments() {
        assert_eq!(
            errors(
                "yup 0.1\ndiscount rate t = 1 / (1 + rate) ^ t\na = discount 0.1\nb = \
                 discount\nf x x = x\ng x = x 1\ndiscount : 'D'\n"
            ),
            [
                "7:1: error: `discount` is a function, which has no cells and so no signature; \
                 its size is worked out where it is used",
                "5:5: error: the parameter `x` is given twice",
                "3:5: error: `discount` takes 2 arguments: `discount rate t`",
                "4:5: error: `discount` is a function; give it its arguments: `discount rate t`",
                "6:7: error: `x` is a parameter, and a parameter is a value, not a function",
            ]
        );
    }

    #[test]
    fn sheet_names_follow_the_apps_rules() {
        assert_eq!(
            errors("yup 0.1\nsheet 'a/b'\nsheet 'History'\nsheet 'A/B'\n"),
            [
                "2:7: error: a sheet name cannot contain `/`",
                "3:7: error: the spreadsheet app keeps the sheet name 'History' for itself",
                "4:7: error: a sheet name cannot contain `/`",
                "4:7: error: there is already a sheet called 'a/b' at 2:7; sheet names ignore case",
            ]
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
        ] {
            assert_eq!(format_problem(good), None, "{good}");
        }
        assert!(format_problem("0.0 x").is_some());
        assert!(format_problem("[Orange]0").is_some());
        assert!(format_problem("0;0;0;0;0").is_some());
    }
}
