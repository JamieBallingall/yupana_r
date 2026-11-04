//! Step 2, Resolve: what each name refers to, how each binding looks and where it goes, and the
//! order in which definitions depend on each other.
//!
//! Every binding is a signature and, directly below it, its definition. The signature's role
//! says what the binding becomes; nothing is inferred from what a signature leaves out. Two
//! graphs come out of a file: the formulas' graph from the definitions, and the layout's graph
//! from the placements in the signatures. Each is checked for cycles on its own, and the order
//! of the file means nothing to either.

use std::collections::{BTreeMap, BTreeSet};

use crate::Inputs;
use crate::cells::{looks_like_cell, looks_like_r1c1};
use crate::csv::{self, Table};
use crate::diagnostic::{Diagnostic, Pos};
use crate::functions::Builtin;
use crate::parser::is_keyword;
use crate::printer::expression;
use crate::style::{self, CellStyle, Declared, Evaluator, Font, Rgb, Value};
use crate::syntax::{
    CellType, Expr, ExprKind, Field, Ident, ItemKind, Move, Placement, Program, Role, Signature,
    SizeKind, StyleType, Type,
};

/// The most characters a cell can hold, from the spreadsheet app's published limits.
const MAX_CELL_TEXT: usize = 32_767;

/// What a file's names refer to, how its cells look, and where they go.
#[derive(Clone, Debug)]
pub struct Model {
    pub bindings: Vec<Binding>,
    pub sizes: Vec<SizeDecl>,
    pub symbols: BTreeMap<String, Symbol>,
    /// Every binding that formulas can use, each after the bindings it uses.
    pub order: Vec<usize>,
    /// Every binding with cells of its own, each after the binding it is placed beside.
    pub layout: Vec<usize>,
    /// The table each value defined by `csv "file.csv"` reads, by binding.
    pub tables: BTreeMap<usize, Table>,
    /// The values carried from one position to the next.
    pub recurrences: Vec<Recurrence>,
    /// The sheets, in the order of their tabs.
    pub sheets: Vec<Sheet>,
    /// The workbook's own font: only its name and size are ever set.
    pub font: Font,
}

/// A worksheet and its settings.
#[derive(Clone, Debug)]
pub struct Sheet {
    /// The `Sheet` binding.
    pub binding: usize,
    pub name: String,
    pub color: Option<Rgb>,
    pub hidden: bool,
    /// The sheet's font, which cascades to its cells: only its name and size are ever set.
    pub font: Font,
}

/// Bindings that depend on each other in a cycle, broken by `lag 1 state seed first`: a value
/// carried from one position to the next. Each position depends on the one before.
#[derive(Clone, Debug)]
pub struct Recurrence {
    /// The bindings in the cycle, each after the others it uses at the same position.
    pub members: Vec<usize>,
    /// The binding carried from one position to the next.
    pub state: usize,
}

impl Model {
    /// The recurrence a binding belongs to, if any.
    pub fn recurrence_of(&self, binding: usize) -> Option<&Recurrence> {
        self.recurrences
            .iter()
            .find(|recurrence| recurrence.members.contains(&binding))
    }

    /// A binding's signature.
    pub fn signature<'p>(&self, program: &'p Program, binding: usize) -> &'p Signature {
        match &program.items[self.bindings[binding].signature].kind {
            ItemKind::Signature(sig) => sig,
            _ => unreachable!("a signature index points at a signature"),
        }
    }

    /// A binding's definition.
    pub fn definition<'p>(&self, program: &'p Program, binding: usize) -> &'p Expr {
        match &program.items[self.bindings[binding].definition].kind {
            ItemKind::Definition { expr, .. } => expr,
            _ => unreachable!("a binding's definition index points at a definition"),
        }
    }
}

/// A binding that a definition uses. A use inside `lag 1 x seed first`, in `x`, is delayed: it
/// reads earlier positions only, so it does not stop a cycle being worked out position by
/// position.
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

/// Where a binding's cells go, with the names in its placement resolved.
#[derive(Clone, Debug)]
pub enum Place {
    /// A cell of a sheet, by the sheet's index in tab order.
    At { sheet: usize, row: i64, column: i64 },
    /// Beside another binding.
    Beside { reference: usize, moves: Vec<Move> },
}

/// A name defined by a signature and the definition below it.
#[derive(Clone, Debug)]
pub struct Binding {
    pub name: String,
    /// Where the definition is.
    pub pos: Pos,
    /// The index of the signature item.
    pub signature: usize,
    /// The index of the definition item.
    pub definition: usize,
    pub role: Role,
    /// A lambda's parameters; empty for any other binding.
    pub params: Vec<String>,
    /// What each cell holds, as the signature says, for a formula, a value or a local.
    pub declared: Option<CellType>,
    /// A lambda's parameters' types and its result's.
    pub function: Vec<CellType>,
    /// The sheet the binding's cells are on, by its index in tab order.
    pub sheet: Option<usize>,
    /// Where the binding's cells go.
    pub place: Option<Place>,
    /// How the binding's cells look, before the font's name and size cascade from the sheet and
    /// the workbook.
    pub style: Option<CellStyle>,
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

fn twice(name: &str, pos: Pos, first: Pos) -> Diagnostic {
    Diagnostic::error(pos, format!("`{name}` is already declared at {first}"))
}

/// What a signature is waiting for below it.
enum Pending {
    /// The definition of the binding whose signature is at this item.
    Definition {
        index: usize,
        name: String,
        /// Whether a blank line has come between.
        apart: bool,
    },
    /// Nothing: but a definition of this unknown size below would be wrong.
    Unknown(String),
}

/// A signature with its definition, as the first pass pairs them.
struct Paired {
    signature: usize,
    definition: usize,
}

/// Resolves every name in `program`.
pub fn resolve(program: &Program, inputs: &Inputs) -> Result<Model, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let (pairs, mut sizes) = pair(program, &mut errors);

    let mut symbols: BTreeMap<String, Symbol> = BTreeMap::new();
    for (index, size) in sizes.iter().enumerate() {
        symbols.insert(size.name.clone(), Symbol::Size(index));
    }
    let mut bindings: Vec<Binding> = Vec::new();
    for paired in &pairs {
        let ItemKind::Signature(sig) = &program.items[paired.signature].kind else {
            unreachable!("a pair starts with a signature");
        };
        let ItemKind::Definition { params, .. } = &program.items[paired.definition].kind else {
            unreachable!("a pair ends with a definition");
        };
        let name = sig.name.name.clone();
        if let Some(Symbol::Binding(other)) = symbols.get(&name) {
            errors.push(twice(
                &name,
                sig.name.pos,
                program.items[bindings[*other].signature].pos,
            ));
            continue;
        }
        if symbols.contains_key(&name) {
            errors.push(Diagnostic::error(
                sig.name.pos,
                format!("`{name}` is already declared as a size"),
            ));
            continue;
        }
        errors.extend(signature_errors(sig, params));
        let (declared, function) = match &sig.ty {
            Some((Type::Cell(ty), _)) => (Some(*ty), Vec::new()),
            Some((Type::Function(types), _)) => (None, types.clone()),
            _ => (None, Vec::new()),
        };
        symbols.insert(name.clone(), Symbol::Binding(bindings.len()));
        bindings.push(Binding {
            name,
            pos: program.items[paired.definition].pos,
            signature: paired.signature,
            definition: paired.definition,
            role: sig.role,
            params: params.iter().map(|param| param.name.clone()).collect(),
            declared,
            function,
            sheet: None,
            place: None,
            style: None,
        });
    }
    fixed_sizes(program, &pairs, &mut sizes, &mut errors);
    errors.extend(parameter_spellings(program, &bindings, &sizes, &symbols));

    // Styles are worked out first: placed bindings, sheets and the workbook name them.
    let declared: Vec<Declared> = bindings
        .iter()
        .filter(|binding| binding.role == Role::Style)
        .map(|binding| {
            let sig = signature_of(program, binding);
            let ty = match sig.ty {
                Some((Type::Style(ty), _)) => ty,
                _ => unreachable!("the parser gives every style a type"),
            };
            Declared {
                name: &sig.name.name,
                ty,
                expr: definition_of(program, binding),
            }
        })
        .collect();
    let mut styles = Evaluator::new(declared);
    styles.check_all();
    for binding in &mut bindings {
        if !binding.role.is_placed() {
            continue;
        }
        binding.style = match &signature_of(program, binding).style {
            Some(name) => styles
                .named(&name.name, StyleType::Cell, name.pos)
                .map(|value| style::cell(&value))
                .filter(|cell| match style::cell_problem(cell) {
                    Some(problem) => {
                        let message = format!("`{}`: {problem}", name.name);
                        styles.errors.push(Diagnostic::error(name.pos, message));
                        false
                    }
                    None => true,
                }),
            None => Some(CellStyle::default()),
        };
    }

    let (sheets, font) =
        sheets_and_workbook(program, &bindings, &symbols, &mut styles, &mut errors);
    errors.append(&mut styles.errors);
    let layout = places(program, &symbols, &sheets, &mut bindings, &mut errors);

    // A value defined as `csv "file.csv"` is a table, read from the file the caller gave.
    let mut tables = BTreeMap::new();
    let mut table_bindings = BTreeSet::new();
    for (index, binding) in bindings.iter().enumerate() {
        let expr = definition_of(program, binding);
        match binding.role {
            Role::Value => {
                if let Some((file, pos)) = csv_file(expr) {
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
                } else {
                    errors.extend(value_error(expr));
                }
            }
            Role::Label => errors.extend(label_error(expr)),
            _ => {}
        }
    }

    let mut uses: Vec<Vec<Use>> = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        let mut found = Vec::new();
        if matches!(binding.role, Role::Formula | Role::Local | Role::Lambda) {
            let scope = Scope {
                symbols: &symbols,
                sizes: &sizes,
                bindings: &bindings,
                params: &binding.params,
                tables: &tables,
                table_bindings: &table_bindings,
            };
            scope.resolve_expr(
                definition_of(program, binding),
                false,
                &mut found,
                &mut errors,
            );
        }
        // A lambda's uses are never delayed, so a cycle through a lambda stays an error.
        if binding.role == Role::Lambda {
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
    let order: Vec<usize> = order
        .into_iter()
        .filter(|&binding| {
            matches!(
                bindings[binding].role,
                Role::Formula | Role::Value | Role::Local | Role::Lambda
            )
        })
        .collect();
    let mut recurrences = Vec::new();
    for members in cycles {
        match recurrence(program, &symbols, &bindings, &uses, members) {
            Ok(recurrence) => recurrences.push(recurrence),
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Ok(Model {
            bindings,
            sizes,
            symbols,
            order,
            layout,
            tables,
            recurrences,
            sheets,
            font,
        })
    } else {
        Err(errors)
    }
}

/// Refuses a lambda's parameter that differs from one of the file's names only in case. A
/// parameter may have the very name of a binding, which it hides inside the lambda; in another
/// case, the workbook would read the two as one name, which the file does not.
fn parameter_spellings(
    program: &Program,
    bindings: &[Binding],
    sizes: &[SizeDecl],
    symbols: &BTreeMap<String, Symbol>,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    for binding in bindings.iter().filter(|b| b.role == Role::Lambda) {
        let ItemKind::Definition { params, .. } = &program.items[binding.definition].kind else {
            unreachable!("a binding's definition index points at a definition");
        };
        for param in params {
            let other = symbols
                .iter()
                .find(|(name, _)| **name != param.name && name.eq_ignore_ascii_case(&param.name));
            if let Some((name, symbol)) = other {
                let declared = match symbol {
                    Symbol::Binding(b) => program.items[bindings[*b].signature].pos,
                    Symbol::Size(s) => sizes[*s].pos,
                };
                errors.push(case_clash(&param.name, param.pos, name, declared));
            }
        }
    }
    errors
}

/// How a name is written where it is declared, or as a function, if it differs from that only in
/// case. A keyword is left out: none stands for a value, and `true` is written `True`.
fn spelling_of(
    name: &str,
    params: &[String],
    symbols: &BTreeMap<String, Symbol>,
) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    params
        .iter()
        .chain(symbols.keys())
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .cloned()
        .or_else(|| Builtin::lookup(&lower).map(|_| lower))
}

fn signature_of<'p>(program: &'p Program, binding: &Binding) -> &'p Signature {
    match &program.items[binding.signature].kind {
        ItemKind::Signature(sig) => sig,
        _ => unreachable!("a signature index points at a signature"),
    }
}

fn definition_of<'p>(program: &'p Program, binding: &Binding) -> &'p Expr {
    match &program.items[binding.definition].kind {
        ItemKind::Definition { expr, .. } => expr,
        _ => unreachable!("a binding's definition index points at a definition"),
    }
}

/// Pairs each signature with the definition directly below it, and collects the sizes.
fn pair(program: &Program, errors: &mut Vec<Diagnostic>) -> (Vec<Paired>, Vec<SizeDecl>) {
    let mut pairs = Vec::new();
    let mut sizes: Vec<SizeDecl> = Vec::new();
    let mut spellings: BTreeMap<String, Ident> = BTreeMap::new();
    let mut pending: Option<Pending> = None;
    let unpaired = |pending: &Option<Pending>, errors: &mut Vec<Diagnostic>| {
        if let Some(Pending::Definition { index, name, .. }) = pending {
            errors.push(Diagnostic::error(
                program.items[*index].pos,
                format!(
                    "`{name}` has a signature but no definition below it, such as `{name} = …`"
                ),
            ));
        }
    };
    for (index, item) in program.items.iter().enumerate() {
        match &item.kind {
            ItemKind::Blank => {
                if let Some(Pending::Definition { apart, .. }) = &mut pending {
                    *apart = true;
                }
            }
            ItemKind::Comment(_) => {}
            ItemKind::Size { name, kind } => {
                unpaired(&pending, errors);
                errors.extend(name_error(&name.name, name.pos));
                errors.extend(spelling_error(&mut spellings, name));
                if let Some(other) = sizes.iter().find(|size| size.name == name.name) {
                    errors.push(twice(&name.name, name.pos, other.pos));
                }
                sizes.push(SizeDecl {
                    name: name.name.clone(),
                    pos: name.pos,
                    kind: *kind,
                    value: None,
                });
                pending = Some(match kind {
                    SizeKind::Fixed => Pending::Definition {
                        index,
                        name: name.name.clone(),
                        apart: false,
                    },
                    SizeKind::Unknown => Pending::Unknown(name.name.clone()),
                });
            }
            ItemKind::Signature(sig) => {
                unpaired(&pending, errors);
                errors.extend(name_error(&sig.name.name, sig.name.pos));
                errors.extend(spelling_error(&mut spellings, &sig.name));
                pending = Some(Pending::Definition {
                    index,
                    name: sig.name.name.clone(),
                    apart: false,
                });
            }
            ItemKind::Definition { name, .. } => {
                match pending.take() {
                    Some(Pending::Definition {
                        index: signature,
                        name: expected,
                        apart,
                    }) if expected == name.name => {
                        if apart {
                            errors.push(Diagnostic::error(
                                item.pos,
                                format!(
                                    "the definition of `{expected}` follows its signature \
                                     directly, with only comments between"
                                ),
                            ));
                        }
                        // A fixed size's value is read by `fixed_sizes`.
                        if matches!(program.items[signature].kind, ItemKind::Signature(_)) {
                            pairs.push(Paired {
                                signature,
                                definition: index,
                            });
                        }
                    }
                    Some(Pending::Definition { name: expected, .. }) => {
                        errors.push(Diagnostic::error(
                            item.pos,
                            format!(
                                "this defines `{}`, but the signature above it is of `{expected}`; \
                                 a definition follows its own signature",
                                name.name
                            ),
                        ));
                    }
                    Some(Pending::Unknown(size)) if size == name.name => {
                        errors.push(Diagnostic::error(
                            item.pos,
                            format!(
                                "`{size}` is an unknown size: the workbook decides its value, so \
                                 it cannot be given one here"
                            ),
                        ));
                    }
                    _ => errors.push(Diagnostic::error(
                        item.pos,
                        format!(
                            "`{0}` has no signature above it; a binding is a signature, such as \
                             `{0} : Formula Number …`, and then its definition",
                            name.name
                        ),
                    )),
                }
            }
        }
    }
    unpaired(&pending, errors);
    (pairs, sizes)
}

/// Reads each fixed size's value from the definition below its declaration.
fn fixed_sizes(
    program: &Program,
    pairs: &[Paired],
    sizes: &mut [SizeDecl],
    errors: &mut Vec<Diagnostic>,
) {
    let paired_definitions: BTreeSet<usize> = pairs.iter().map(|pair| pair.definition).collect();
    for (index, item) in program.items.iter().enumerate() {
        let ItemKind::Definition { name, expr, .. } = &item.kind else {
            continue;
        };
        if paired_definitions.contains(&index) {
            continue;
        }
        let Some(size) = sizes
            .iter_mut()
            .find(|size| size.name == name.name && size.kind == SizeKind::Fixed)
        else {
            continue;
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
    for size in sizes.iter() {
        if size.kind == SizeKind::Fixed && size.value.is_none() {
            let reported = errors.iter().any(|error| error.pos == size.pos);
            if !reported {
                errors.push(Diagnostic::error(
                    size.pos,
                    format!(
                        "the fixed size `{0}` needs a value, such as `{0} = 4`",
                        size.name
                    ),
                ));
            }
        }
    }
}

/// What is wrong with a signature, given its definition's parameters.
fn signature_errors(sig: &Signature, params: &[Ident]) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let name = &sig.name.name;
    match sig.role {
        Role::Lambda => {
            let types = match &sig.ty {
                Some((Type::Function(types), _)) => types.len(),
                _ => unreachable!("the parser gives every lambda a type"),
            };
            if params.len() + 1 != types {
                let wanted = types - 1;
                let plural = if wanted == 1 { "" } else { "s" };
                errors.push(Diagnostic::error(
                    sig.name.pos,
                    format!(
                        "`{name}`'s type names {wanted} parameter{plural}, and its definition \
                         gives {}",
                        params.len()
                    ),
                ));
            }
            for (i, param) in params.iter().enumerate() {
                errors.extend(name_error(&param.name, param.pos));
                let earlier = params[..i]
                    .iter()
                    .find(|other| other.name.eq_ignore_ascii_case(&param.name));
                match earlier {
                    Some(other) if other.name == param.name => errors.push(Diagnostic::error(
                        param.pos,
                        format!("the parameter `{}` is given twice", param.name),
                    )),
                    Some(other) => {
                        errors.push(case_clash(&param.name, param.pos, &other.name, other.pos));
                    }
                    None => {}
                }
            }
        }
        _ if !params.is_empty() => errors.push(Diagnostic::error(
            params[0].pos,
            format!(
                "`{name}` is a {}, and only a lambda takes parameters",
                sig.role.word()
            ),
        )),
        _ => {}
    }
    if sig.role.is_placed() && sig.place.is_none() {
        errors.push(Diagnostic::error(
            sig.name.pos,
            format!(
                "`{name}` needs a place: a cell, as in `at forecast B2`, or beside another \
                 binding, as in `… {name}_label right 2`"
            ),
        ));
    }
    errors
}

/// The file a value reads, and where its name is written, when the definition is
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

/// Checks that a value's definition is one literal written in the file.
fn value_error(expr: &Expr) -> Option<Diagnostic> {
    if let ExprKind::Str(text) = &expr.kind {
        return text_error(text, expr.pos);
    }
    if expr.is_literal() {
        return None;
    }
    let message = match &expr.kind {
        ExprKind::List(_) => {
            "a value holds one number, string or logical; many values belong in an input file, \
             read with `csv \"file.csv\"`"
        }
        ExprKind::Text(_) => "a value's text is data, in double quotes, such as \"Retail\"",
        _ => {
            "a value is written in the file: a number, a string, `True` or `False`, or `csv \
             \"file.csv\"`; a value worked out from others is a `Formula`"
        }
    };
    Some(Diagnostic::error(expr.pos, message))
}

/// Checks that a label's definition is its text, in double quotes.
fn label_error(expr: &Expr) -> Option<Diagnostic> {
    match &expr.kind {
        ExprKind::Str(text) => text_error(text, expr.pos),
        ExprKind::Text(_) => Some(Diagnostic::error(
            expr.pos,
            "a label's text is data, in double quotes, such as \"Revenue\"",
        )),
        _ => Some(Diagnostic::error(
            expr.pos,
            "a label is its text, in double quotes, such as \"Revenue\"",
        )),
    }
}

fn text_error(text: &str, pos: Pos) -> Option<Diagnostic> {
    (text.chars().count() > MAX_CELL_TEXT)
        .then(|| Diagnostic::error(pos, "a cell holds at most 32,767 characters"))
}

/// The binding a name refers to, if it is a binding of one of `roles`.
fn binding_of(
    symbols: &BTreeMap<String, Symbol>,
    bindings: &[Binding],
    name: &str,
    roles: &[Role],
) -> Option<usize> {
    match symbols.get(name) {
        Some(&Symbol::Binding(binding)) if roles.contains(&bindings[binding].role) => Some(binding),
        _ => None,
    }
}

/// Reads the sheets and the workbook: the sheets come in the order the workbook lists them.
fn sheets_and_workbook(
    program: &Program,
    bindings: &[Binding],
    symbols: &BTreeMap<String, Symbol>,
    styles: &mut Evaluator,
    errors: &mut Vec<Diagnostic>,
) -> (Vec<Sheet>, Font) {
    let definition = |binding: usize| definition_of(program, &bindings[binding]);

    let workbooks: Vec<usize> = (0..bindings.len())
        .filter(|&binding| bindings[binding].role == Role::Workbook)
        .collect();
    let Some(&workbook) = workbooks.first() else {
        errors.push(Diagnostic::error(
            Pos::START,
            "a file has one `Workbook` binding, which lists the sheets: `workbook : Workbook` and \
             `workbook = { sheets = [forecast] }`",
        ));
        return (Vec::new(), Font::default());
    };
    for &other in &workbooks[1..] {
        errors.push(Diagnostic::error(
            bindings[other].pos,
            format!(
                "a file has one `Workbook` binding, and `{}` is another",
                bindings[other].name
            ),
        ));
    }

    let mut font = Font::default();
    let mut listed: Vec<(usize, Pos)> = Vec::new();
    for field in record_fields(definition(workbook), "workbook", errors) {
        match field.name.name.as_str() {
            "sheets" => {
                let ExprKind::List(rows) = &field.value.kind else {
                    errors.push(Diagnostic::error(
                        field.value.pos,
                        "the sheets are a list of their names, in the order of their tabs, such as \
                         `[inputs, forecast]`",
                    ));
                    continue;
                };
                for value in rows.iter().flatten() {
                    match &value.kind {
                        ExprKind::Name(name) => {
                            match binding_of(symbols, bindings, name, &[Role::Sheet]) {
                                Some(sheet) if listed.iter().any(|(s, _)| *s == sheet) => {
                                    errors.push(Diagnostic::error(
                                        value.pos,
                                        format!("the sheet `{name}` is listed twice"),
                                    ));
                                }
                                Some(sheet) => listed.push((sheet, value.pos)),
                                None => errors.push(Diagnostic::error(
                                    value.pos,
                                    format!("`{name}` is not a sheet, such as `{name} : Sheet`"),
                                )),
                            }
                        }
                        _ => errors.push(Diagnostic::error(
                            value.pos,
                            "the sheets are listed by their names",
                        )),
                    }
                }
            }
            "style" => font = cascade(field, styles, errors).unwrap_or_default(),
            other => errors.push(unknown_field("workbook", "sheets and style", other, field)),
        }
    }

    let mut sheets: Vec<Sheet> = Vec::new();
    for &(binding, _) in &listed {
        let mut sheet = Sheet {
            binding,
            name: String::new(),
            color: None,
            hidden: false,
            font: Font::default(),
        };
        let mut named = false;
        for field in record_fields(definition(binding), "sheet", errors) {
            match field.name.name.as_str() {
                "name" => match &field.value.kind {
                    ExprKind::Text(name) => {
                        errors.extend(sheet_name_error(name, field.value.pos));
                        if let Some(other) = sheets
                            .iter()
                            .find(|other| other.name.to_lowercase() == name.to_lowercase())
                        {
                            errors.push(Diagnostic::error(
                                field.value.pos,
                                format!(
                                    "there is already a sheet called '{}'; sheet names ignore case",
                                    other.name
                                ),
                            ));
                        }
                        sheet.name = name.clone();
                        named = true;
                    }
                    _ => errors.push(Diagnostic::error(
                        field.value.pos,
                        "a sheet's name is a setting, in single quotes, such as 'Forecast'",
                    )),
                },
                "color" => {
                    sheet.color = styles
                        .record(&field.value, StyleType::Color)
                        .map(|value| style::color(&value));
                }
                "hidden" => match field.value.kind {
                    ExprKind::Bool(hidden) => sheet.hidden = hidden,
                    _ => errors.push(Diagnostic::error(
                        field.value.pos,
                        "whether a sheet is hidden is `True` or `False`",
                    )),
                },
                "style" => sheet.font = cascade(field, styles, errors).unwrap_or_default(),
                other => errors.push(unknown_field(
                    "sheet",
                    "name, color, hidden and style",
                    other,
                    field,
                )),
            }
        }
        if !named {
            errors.push(Diagnostic::error(
                bindings[binding].pos,
                format!(
                    "the sheet `{}` needs a name, such as `{{ name = 'Forecast' }}`",
                    bindings[binding].name
                ),
            ));
        }
        sheets.push(sheet);
    }
    for (binding, b) in bindings.iter().enumerate() {
        if b.role == Role::Sheet && !listed.iter().any(|(sheet, _)| *sheet == binding) {
            errors.push(Diagnostic::error(
                b.pos,
                format!(
                    "the workbook's `sheets` does not list `{}`; every sheet is listed, in the \
                     order of the tabs",
                    b.name
                ),
            ));
        }
    }
    if listed.is_empty() {
        errors.push(Diagnostic::error(
            bindings[workbook].pos,
            "the workbook lists at least one sheet, such as `{ sheets = [forecast] }`",
        ));
    } else if sheets.iter().all(|sheet| sheet.hidden) {
        errors.push(Diagnostic::error(
            bindings[workbook].pos,
            "every sheet is hidden; at least one must be shown",
        ));
    }
    (sheets, font)
}

/// The font a sheet's or the workbook's style gives, which cascades to the cells. Nothing else
/// may be set in such a style.
fn cascade(field: &Field, styles: &mut Evaluator, errors: &mut Vec<Diagnostic>) -> Option<Font> {
    let value = styles.record(&field.value, StyleType::Cell)?;
    for (part, pos) in style::beyond_font_face(&value) {
        errors.push(Diagnostic::error(
            pos,
            format!(
                "only the font's name and size cascade from a sheet or the workbook, and `{part}` \
                 is neither; set it in the style of the cells"
            ),
        ));
    }
    let Value::Record(_, fields) = &value else {
        unreachable!("a cell's style is a record");
    };
    Some(style::font(fields.get("font").map(|(font, _)| font)))
}

/// The fields of a record that must be one, as a sheet's or the workbook's definition is.
fn record_fields<'e>(expr: &'e Expr, what: &str, errors: &mut Vec<Diagnostic>) -> &'e [Field] {
    match &expr.kind {
        ExprKind::Record(fields) => {
            for (i, field) in fields.iter().enumerate() {
                if fields[..i]
                    .iter()
                    .any(|other| other.name.name == field.name.name)
                {
                    errors.push(Diagnostic::error(
                        field.name.pos,
                        format!("the field `{}` is set twice", field.name.name),
                    ));
                }
            }
            fields
        }
        _ => {
            errors.push(Diagnostic::error(
                expr.pos,
                format!("a {what} is a record of settings, such as `{{ name = 'Forecast' }}`"),
            ));
            &[]
        }
    }
}

fn unknown_field(what: &str, fields: &str, name: &str, field: &Field) -> Diagnostic {
    Diagnostic::error(
        field.name.pos,
        format!("a {what} has no field `{name}`; its fields are {fields}"),
    )
}

/// Resolves each placed binding's placement, and finds the sheet it is on by following its
/// placements back to a cell. Returns the placed bindings, each after the one it is placed
/// beside.
fn places(
    program: &Program,
    symbols: &BTreeMap<String, Symbol>,
    sheets: &[Sheet],
    bindings: &mut [Binding],
    errors: &mut Vec<Diagnostic>,
) -> Vec<usize> {
    let placed: Vec<usize> = (0..bindings.len())
        .filter(|&binding| bindings[binding].role.is_placed())
        .collect();
    for &binding in &placed {
        let ItemKind::Signature(sig) = &program.items[bindings[binding].signature].kind else {
            unreachable!("a signature index points at a signature");
        };
        let place = match &sig.place {
            Some(Placement::At {
                sheet, row, column, ..
            }) => {
                let found = binding_of(symbols, bindings, &sheet.name, &[Role::Sheet])
                    .and_then(|sheet| sheets.iter().position(|s| s.binding == sheet));
                match found {
                    Some(index) => Some(Place::At {
                        sheet: index,
                        row: *row,
                        column: *column,
                    }),
                    None => {
                        // A sheet the workbook does not list has been reported already.
                        if binding_of(symbols, bindings, &sheet.name, &[Role::Sheet]).is_none() {
                            errors.push(Diagnostic::error(
                                sheet.pos,
                                format!(
                                    "`{0}` is not a sheet; `at` takes a sheet and a cell, as in \
                                     `at {0} B2`, where `{0} : Sheet`",
                                    sheet.name
                                ),
                            ));
                        }
                        None
                    }
                }
            }
            Some(Placement::Beside { reference, moves }) => {
                match binding_of(
                    symbols,
                    bindings,
                    &reference.name,
                    &[Role::Formula, Role::Value, Role::Label],
                ) {
                    Some(other) if other == binding => {
                        errors.push(Diagnostic::error(
                            reference.pos,
                            format!("`{}` cannot be placed beside itself", reference.name),
                        ));
                        None
                    }
                    Some(other) => Some(Place::Beside {
                        reference: other,
                        moves: moves.clone(),
                    }),
                    None => {
                        errors.push(Diagnostic::error(
                            reference.pos,
                            format!(
                                "`{}` has no cells to be placed beside; a binding is placed \
                                 beside a formula, a value or a label",
                                reference.name
                            ),
                        ));
                        None
                    }
                }
            }
            // A missing place has been reported with the signature.
            None => None,
        };
        bindings[binding].place = place;
    }

    // The layout's graph: each placed binding points at the one it is placed beside.
    let mut order = Vec::new();
    let mut done: BTreeSet<usize> = BTreeSet::new();
    for &start in &placed {
        let mut path: Vec<usize> = Vec::new();
        let mut at = start;
        while !done.contains(&at) {
            if let Some(from) = path.iter().position(|&p| p == at) {
                let mut cycle: Vec<&str> = path[from..]
                    .iter()
                    .map(|&p| bindings[p].name.as_str())
                    .collect();
                cycle.push(&bindings[at].name);
                errors.push(Diagnostic::error(
                    program.items[bindings[at].signature].pos,
                    format!(
                        "these bindings are placed beside each other in a circle: {}; one of \
                         them goes at a cell, with `at`",
                        cycle.join(" -> ")
                    ),
                ));
                done.extend(path.drain(..));
                break;
            }
            path.push(at);
            match &bindings[at].place {
                Some(Place::Beside { reference, .. }) => at = *reference,
                _ => break,
            }
        }
        // The path, from its far end back, is in layout order.
        for &p in path.iter().rev() {
            if done.insert(p) {
                order.push(p);
            }
        }
    }
    for &binding in &order {
        bindings[binding].sheet = match &bindings[binding].place {
            Some(Place::At { sheet, .. }) => Some(*sheet),
            Some(Place::Beside { reference, .. }) => bindings[*reference].sheet,
            None => None,
        };
    }
    order
}

/// What the names in a definition can refer to: the file's names, and, inside a lambda, its
/// parameters, which come first.
struct Scope<'a> {
    symbols: &'a BTreeMap<String, Symbol>,
    sizes: &'a [SizeDecl],
    bindings: &'a [Binding],
    params: &'a [String],
    /// The tables that were read.
    tables: &'a BTreeMap<usize, Table>,
    /// Every value defined by `csv`, whether or not its file could be read.
    table_bindings: &'a BTreeSet<usize>,
}

/// How a lambda is called, for messages: `name a b`.
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
                Some(Symbol::Binding(b)) => {
                    let binding = &self.bindings[*b];
                    let problem = match binding.role {
                        Role::Formula | Role::Value | Role::Local => {
                            uses.push(Use {
                                binding: *b,
                                delayed,
                            });
                            None
                        }
                        Role::Lambda => Some(format!(
                            "`{name}` is a lambda; give it its arguments: `{}`",
                            call_usage(binding)
                        )),
                        Role::Label => Some(format!(
                            "`{name}` is a label, which formulas cannot use; a text a formula \
                             uses is a `Value`"
                        )),
                        role => Some(format!(
                            "`{name}` is a {}, not something a formula can use",
                            role.word().to_lowercase()
                        )),
                    };
                    errors.extend(problem.map(|message| Diagnostic::error(expr.pos, message)));
                }
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
                        Some(builtin) if builtin.arity() == 0 => return,
                        Some(builtin) => format!(
                            "`{name}` is a function; give it its arguments: `{}`",
                            builtin.usage()
                        ),
                        None => match spelling_of(name, self.params, self.symbols) {
                            Some(spelling) => {
                                format!("unknown name `{name}`; it is written `{spelling}`")
                            }
                            None => format!("unknown name `{name}`"),
                        },
                    };
                    errors.push(Diagnostic::error(expr.pos, message));
                }
            },
            ExprKind::Apply {
                func,
                axis,
                args,
                seed,
            } => {
                self.resolve_call(func, args.len(), uses, errors);
                let shifts = matches!(func.name.as_str(), "lag" | "tail" | "init");
                if axis.is_some() && !shifts {
                    errors.push(Diagnostic::error(
                        func.pos,
                        "only `lag`, `tail` and `init` say which way they work",
                    ));
                }
                // A lambda's body is written once into the workbook, however it is called.
                if shifts && axis.is_none() && !self.params.is_empty() {
                    errors.push(Diagnostic::error(
                        func.pos,
                        format!(
                            "inside a lambda, `{0}` says which way it works, `across` or `down`, \
                             as in `{0} across x`: the lambda may be given rows or columns",
                            func.name
                        ),
                    ));
                }
                match (func.name.as_str(), args.as_slice()) {
                    ("csv", _) => errors.push(Diagnostic::error(
                        func.pos,
                        "`csv` reads a file into a value, as the whole of its definition: \
                         `history = csv \"history.csv\"`, with `Value` in its signature",
                    )),
                    ("column", [name, table]) => errors.extend(self.column_error(name, table)),
                    // What `lag 1 x seed first` lags is read only at earlier positions.
                    ("lag", [steps, lagged]) if seed.is_some() => {
                        self.resolve_expr(steps, delayed, uses, errors);
                        self.resolve_expr(lagged, true, uses, errors);
                        if let Some(seed) = seed {
                            self.resolve_expr(seed, delayed, uses, errors);
                        }
                        return;
                    }
                    _ => {}
                }
            }
            ExprKind::Text(_) => errors.push(Diagnostic::error(
                expr.pos,
                "single quotes hold a setting, such as a format code; a string in a formula takes \
                 double quotes",
            )),
            ExprKind::Constructor(word) => errors.push(Diagnostic::error(
                expr.pos,
                format!("`{word}` is a setting of a style, not a value a formula can use"),
            )),
            ExprKind::Record(_) | ExprKind::Update { .. } => errors.push(Diagnostic::error(
                expr.pos,
                "a record belongs to a style, a sheet or the workbook, not to a formula",
            )),
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
                                "`column` takes a table, a value defined by `csv`, and \
                                 `{table_name}` is not one"
                            ),
                        ));
                    }
                }
            }
            _ => {
                return Some(Diagnostic::error(
                    table.pos,
                    "`column` takes a table by its name: a value defined by `csv`",
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
                Some(Symbol::Binding(b)) if self.bindings[*b].role == Role::Lambda => {
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
                            "`{}` is not a lambda, so it cannot take arguments",
                            func.name
                        ),
                    ));
                    return;
                }
                None => {
                    let message = match spelling_of(&func.name, self.params, self.symbols) {
                        Some(spelling) => {
                            format!(
                                "unknown function `{}`; it is written `{spelling}`",
                                func.name
                            )
                        }
                        None => format!("unknown function `{}`", func.name),
                    };
                    errors.push(Diagnostic::error(func.pos, message));
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
            "circular definition: {}; a value cannot depend on itself, but `lag 1 x seed \
             first` carries a value from one position to the next",
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
/// Each position of a recurrence is worked out from the one before, so inside it the carried
/// value and the bindings it depends on are taken one position at a time. Only what works
/// position by position can take them; `lag` takes the carried value itself, one position back,
/// always with the same first value.
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
                "{} are each carried from one position to the next in the same cycle; coupled \
                 values are not supported yet",
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
    /// The first value, as written, of the first `lag 1 state seed first` seen.
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
            ExprKind::Apply {
                func, args, seed, ..
            } if func.name == "lag" => {
                let carries_state = matches!(&args[1].kind, ExprKind::Name(name) if name == state);
                let (Some(seed), true, Some(1)) = (seed, carries_state, args[0].integer()) else {
                    return Err(Diagnostic::error(
                        func.pos,
                        format!(
                            "inside a cycle, `lag` carries the value itself one position, \
                             with its first value: `lag 1 {state} seed first`"
                        ),
                    ));
                };
                if self.mentions(seed) {
                    return Err(Diagnostic::error(
                        seed.pos,
                        format!("the first value of `{state}` cannot depend on `{state}`"),
                    ));
                }
                let text = expression(seed);
                match &self.seed {
                    Some(first) if *first != text => Err(Diagnostic::error(
                        seed.pos,
                        format!(
                            "every `lag 1 {state} seed …` gives the same first value, and \
                             another gives `{first}`"
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
                            "`{}` cannot take part in a value carried from one position to \
                             the next, which is worked out one position at a time",
                            func.name
                        ),
                    ));
                }
                args.iter().try_for_each(|arg| self.check(arg))
            }
            ExprKind::List(_) | ExprKind::Range { .. } => Err(Diagnostic::error(
                expr.pos,
                "a list or a range cannot hold a value carried from one position to the next",
            )),
            _ => expr
                .children()
                .into_iter()
                .try_for_each(|child| self.check(child)),
        }
    }
}

/// Why a binding, size or parameter cannot have this name, if it cannot. The rules are those for
/// defined names, which ignore case (ECMA-376 Part 1 §18.17.2.5), so that exporting a binding
/// never forces a rename: `taX2026` reads as a cell just as `tax2026` does.
fn name_error(name: &str, pos: Pos) -> Option<Diagnostic> {
    let lower = name.to_ascii_lowercase();
    let reserved = if Builtin::lookup(&lower).is_some() {
        Some("function")
    } else if is_keyword(&lower) {
        Some("keyword")
    } else {
        None
    };
    let problem = if let Some(kind) = reserved {
        if lower == name {
            format!("is the name of a {kind}")
        } else {
            format!("is the {kind} `{lower}` in another case, and the workbook ignores case")
        }
    } else if name.len() > 255 {
        "is longer than the 255 characters a defined name can have".to_string()
    } else if looks_like_cell(&lower) {
        "reads as a cell reference in the workbook".to_string()
    } else if looks_like_r1c1(&lower) {
        "reads as a row or column reference in the workbook's R1C1 notation".to_string()
    } else {
        return None;
    };
    Some(Diagnostic::error(
        pos,
        format!("`{name}` {problem}; choose another name"),
    ))
}

/// Two names that differ only in case, which the workbook would read as one name.
fn case_clash(name: &str, pos: Pos, other: &str, other_pos: Pos) -> Diagnostic {
    Diagnostic::error(
        pos,
        format!(
            "`{name}` differs from `{other}`, declared at {other_pos}, only in case; the \
             workbook ignores case in names, so choose another name"
        ),
    )
}

/// Records the spelling of a name declared in the file, and refuses one that differs from an
/// earlier name only in case. The same spelling twice is reported where each kind of name is
/// collected.
fn spelling_error(spellings: &mut BTreeMap<String, Ident>, name: &Ident) -> Option<Diagnostic> {
    let first = spellings
        .entry(name.name.to_ascii_lowercase())
        .or_insert_with(|| name.clone());
    (first.name != name.name).then(|| case_clash(&name.name, name.pos, &first.name, first.pos))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    /// A workbook of one sheet, `s`, for the bindings in `body`.
    fn file(body: &str) -> String {
        format!(
            "yup 0.2\nworkbook : Workbook\nworkbook = {{ sheets = [s] }}\ns : Sheet\ns = {{ name = \
             'S' }}\n{body}"
        )
    }

    fn model(body: &str) -> Model {
        resolve(&parse(&file(body)).expect("parses"), &Inputs::new()).expect("resolves")
    }

    fn errors(body: &str) -> Vec<String> {
        resolve(&parse(&file(body)).expect("parses"), &Inputs::new())
            .expect_err("the source is wrong")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn a_signature_gives_the_role() {
        let model = model(
            "h_label : Label at s B2\nh_label = \"Horizon\"\nh : Value Number h_label right \
             2\nh = 5\nx : Formula Number h down\nx = h * 2\ny : Local\ny = x\n",
        );
        let roles: Vec<Role> = model.bindings.iter().map(|b| b.role).collect();
        assert_eq!(
            roles,
            [
                Role::Workbook,
                Role::Sheet,
                Role::Label,
                Role::Value,
                Role::Formula,
                Role::Local
            ]
        );
        let names = |order: &[usize]| -> Vec<&str> {
            order
                .iter()
                .map(|&b| model.bindings[b].name.as_str())
                .collect()
        };
        assert_eq!(names(&model.order), ["h", "x", "y"]);
        assert_eq!(names(&model.layout), ["h_label", "h", "x"]);
        assert_eq!(model.bindings[4].sheet, Some(0));
        assert_eq!(model.sheets[0].name, "S");
    }

    #[test]
    fn definitions_come_after_what_they_use() {
        let model = model("a : Local\na = b + d\nb : Local\nb = d\nd : Local\nd = 1\n");
        let names: Vec<&str> = model
            .order
            .iter()
            .map(|&b| model.bindings[b].name.as_str())
            .collect();
        assert_eq!(names, ["d", "b", "a"]);
    }

    #[test]
    fn a_definition_follows_its_own_signature() {
        assert_eq!(
            errors("a : Local\n\na = 1\nb : Local\nc = 2\nd = 3\ne : Local\n"),
            [
                "8:1: error: the definition of `a` follows its signature directly, with only \
                 comments between",
                "10:1: error: this defines `c`, but the signature above it is of `b`; a \
                 definition follows its own signature",
                "11:1: error: `d` has no signature above it; a binding is a signature, such as \
                 `d : Formula Number …`, and then its definition",
                "12:1: error: `e` has a signature but no definition below it, such as `e = …`",
            ]
        );
    }

    #[test]
    fn names_follow_the_rules_for_defined_names() {
        assert_eq!(
            errors("tax2026 : Local\ntax2026 = 1\nr1c1 : Local\nr1c1 = 2\nsum : Local\nsum = 3\n"),
            [
                "6:1: error: `tax2026` reads as a cell reference in the workbook; choose another \
                 name",
                "8:1: error: `r1c1` reads as a row or column reference in the workbook's R1C1 \
                 notation; choose another name",
                "10:1: error: `sum` is the name of a function; choose another name",
            ]
        );
    }

    #[test]
    fn names_differ_in_more_than_case() {
        assert_eq!(
            errors(
                "revenue_Q1 : Local\nrevenue_Q1 = 1\nrevenue_q1 : Local\nrevenue_q1 = 2\nrate : \
                 Local\nrate = 3\nf : Lambda Number -> Number -> Number\nf x_a x_A = x_a\ng : \
                 Lambda Number -> Number\ng rAte = rAte\ny : Local\ny = rATE + mAx 1 2\ntaX2026 : \
                 Local\ntaX2026 = 4\nsUm : Local\nsUm = 5\ntRue : Local\ntRue = 6\n"
            ),
            [
                "8:1: error: `revenue_q1` differs from `revenue_Q1`, declared at 6:1, only in \
                 case; the workbook ignores case in names, so choose another name",
                "18:1: error: `taX2026` reads as a cell reference in the workbook; choose another \
                 name",
                "20:1: error: `sUm` is the function `sum` in another case, and the workbook \
                 ignores case; choose another name",
                "22:1: error: `tRue` is the keyword `true` in another case, and the workbook \
                 ignores case; choose another name",
                "13:7: error: `x_A` differs from `x_a`, declared at 13:3, only in case; the \
                 workbook ignores case in names, so choose another name",
                "15:3: error: `rAte` differs from `rate`, declared at 10:1, only in case; the \
                 workbook ignores case in names, so choose another name",
                "17:5: error: unknown name `rATE`; it is written `rate`",
                "17:12: error: unknown function `mAx`; it is written `max`",
            ]
        );
    }

    #[test]
    fn names_are_resolved() {
        assert_eq!(
            errors(
                "x : Local\nx = y + sum\nz : Local\nz = x 1\nw : Local\nw = lag 1\nl : Label at s \
                 B2\nl = \"L\"\nv : Local\nv = l + s\n"
            ),
            [
                "7:5: error: unknown name `y`",
                "7:9: error: `sum` is a function; give it its arguments: `sum x`",
                "9:5: error: `x` is not a lambda, so it cannot take arguments",
                "11:5: error: `lag` takes 2 arguments: `lag 1 x, or lag 1 x seed first`",
                "15:5: error: `l` is a label, which formulas cannot use; a text a formula uses is \
                 a `Value`",
                "15:9: error: `s` is a sheet, not something a formula can use",
            ]
        );
    }

    #[test]
    fn a_cycle_is_reported_with_its_path() {
        assert_eq!(
            errors("a : Local\na = b\nb : Local\nb = d + 1\nd : Local\nd = a\n"),
            [
                "7:1: error: circular definition: a -> b -> d -> a; a value cannot depend on \
                 itself, but `lag 1 x seed first` carries a value from one position to the next"
            ]
        );
    }

    #[test]
    fn a_cycle_through_a_seeded_lag_is_a_recurrence() {
        let model = model(
            "opening : Local\nopening = lag 1 closing seed 100\npaid : Local\npaid = min 30 \
             opening\nclosing : Local\nclosing = opening - paid\n",
        );
        assert_eq!(model.recurrences.len(), 1);
        assert_eq!(model.bindings[model.recurrences[0].state].name, "closing");
    }

    #[test]
    fn a_recurrence_works_one_position_at_a_time() {
        assert_eq!(
            errors(
                "opening : Local\nopening = lag 1 closing seed 100\nclosing : Local\nclosing = opening - sum opening\n"
            ),
            [
                "9:21: error: `sum` cannot take part in a value carried from one position to the \
                 next, which is worked out one position at a time"
            ]
        );
    }

    #[test]
    fn values_and_labels_are_written_in_the_file() {
        assert_eq!(
            errors(
                "a : Value at s B2\na = [1, 2]\nb : Value a down\nb = 1 + 1\nk : Label b \
                 down\nk = 'K'\nn : Size Unknown\nn = 3\n"
            ),
            [
                "13:1: error: `n` is an unknown size: the workbook decides its value, so it \
                 cannot be given one here",
                "7:5: error: a value holds one number, string or logical; many values belong in \
                 an input file, read with `csv \"file.csv\"`",
                "9:7: error: a value is written in the file: a number, a string, `True` or \
                 `False`, or `csv \"file.csv\"`; a value worked out from others is a `Formula`",
                "11:5: error: a label's text is data, in double quotes, such as \"Revenue\"",
            ]
        );
    }

    #[test]
    fn lambdas_are_called_with_their_arguments() {
        assert_eq!(
            errors(
                "discount : Lambda Number -> Number -> Number\ndiscount rate t = 1 / (1 + rate) ^ \
                 t\na : Local\na = discount 0.1\nb : Local\nb = discount\nf : Lambda Number -> \
                 Number\nf x y = x\nx : Local\nx y = 1\n"
            ),
            [
                "12:1: error: `f`'s type names 1 parameter, and its definition gives 2",
                "15:3: error: `x` is a Local, and only a lambda takes parameters",
                "9:5: error: `discount` takes 2 arguments: `discount rate t`",
                "11:5: error: `discount` is a lambda; give it its arguments: `discount rate t`",
            ]
        );
        assert_eq!(
            errors("g : Lambda Number -> Number\ng x = tail x\n"),
            [
                "7:7: error: inside a lambda, `tail` says which way it works, `across` or \
                 `down`, as in `tail across x`: the lambda may be given rows or columns"
            ]
        );
    }

    #[test]
    fn sheets_and_the_workbook_are_checked() {
        let source = "yup 0.2\nworkbook : Workbook\nworkbook = { sheets = [a, a, b], colour = 1 \
                      }\na : Sheet\na = { name = 'a/b', hidden = True }\nb : Sheet\nb = { name = \
                      'A/B', hidden = True, style = big }\nk : Sheet\nk = { name = 'K' }\nbig : \
                      Style Cell\nbig = { font = { size = 20, bold = True } }\n";
        let errors: Vec<String> = resolve(&parse(source).expect("parses"), &Inputs::new())
            .expect_err("wrong")
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            errors,
            [
                "3:27: error: the sheet `a` is listed twice",
                "3:34: error: a workbook has no field `colour`; its fields are sheets and style",
                "5:14: error: a sheet name cannot contain `/`",
                "7:14: error: a sheet name cannot contain `/`",
                "7:14: error: there is already a sheet called 'a/b'; sheet names ignore case",
                "11:29: error: only the font's name and size cascade from a sheet or the \
                 workbook, and `font.bold` is neither; set it in the style of the cells",
                "9:1: error: the workbook's `sheets` does not list `k`; every sheet is listed, in \
                 the order of the tabs",
                "3:1: error: every sheet is hidden; at least one must be shown",
            ]
        );
    }

    #[test]
    fn placements_are_resolved_and_go_round_in_no_circle() {
        assert_eq!(
            errors(
                "a : Label b down\na = \"A\"\nb : Label a down\nb = \"B\"\nk : Label k \
                 right\nk = \"K\"\nd : Label at nowhere B2\nd = \"D\"\ne : Label\ne = \"E\"\n"
            ),
            [
                "14:1: error: `e` needs a place: a cell, as in `at forecast B2`, or beside \
                 another binding, as in `… e_label right 2`",
                "10:11: error: `k` cannot be placed beside itself",
                "12:14: error: `nowhere` is not a sheet; `at` takes a sheet and a cell, as in `at \
                 nowhere B2`, where `nowhere : Sheet`",
                "6:1: error: these bindings are placed beside each other in a circle: a -> b -> \
                 a; one of them goes at a cell, with `at`",
            ]
        );
    }
}
