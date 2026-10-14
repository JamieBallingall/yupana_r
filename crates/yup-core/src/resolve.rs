//! Step 2, Resolve: what each name refers to, which sheet each item belongs to, and the order
//! in which definitions depend on each other.
//!
//! A binding is shown in the workbook when its signature has a label. Without a label it is
//! local: it has no cells of its own, and its formula is written into the formulas that use it.

use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Pos};
use crate::functions::Builtin;
use crate::syntax::{Expr, ExprKind, ItemKind, Program, Signature, SizeKind, Text};

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
pub fn resolve(program: &Program) -> Result<Model, Vec<Diagnostic>> {
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
        let signature = signatures.get(&name).copied();
        let sig = signature.map(|index| signature_at(program, index));
        let role = match sig {
            Some(sig) if sig.input.is_some() => Role::Input,
            Some(sig) if sig.label.is_some() => Role::Shown,
            _ => Role::Local,
        };
        if let Some(sig) = sig {
            errors.extend(role_errors(sig, role));
        }
        symbols.insert(name.clone(), Symbol::Binding(bindings.len()));
        bindings.push(Binding {
            name,
            pos: program.items[index].pos,
            signature,
            definition: index,
            role,
            sheet: None,
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

    let mut uses: Vec<Vec<usize>> = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        let ItemKind::Definition { expr, .. } = &program.items[binding.definition].kind else {
            unreachable!("a binding's definition index points at a definition");
        };
        let mut found = Vec::new();
        resolve_expr(expr, &symbols, &sizes, &mut found, &mut errors);
        if binding.role == Role::Input
            && let Err(error) = check_input(expr, binding.pos)
        {
            errors.push(error);
        }
        uses.push(found);
    }
    let order = match dependency_order(&bindings, &uses) {
        Ok(order) => order,
        Err(error) => {
            errors.push(error);
            Vec::new()
        }
    };

    if errors.is_empty() {
        Ok(Model {
            sheets,
            bindings,
            sizes,
            symbols,
            order,
        })
    } else {
        Err(errors)
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

fn resolve_expr(
    expr: &Expr,
    symbols: &BTreeMap<String, Symbol>,
    sizes: &[SizeDecl],
    uses: &mut Vec<usize>,
    errors: &mut Vec<Diagnostic>,
) {
    match &expr.kind {
        ExprKind::Name(name) => match symbols.get(name) {
            Some(Symbol::Binding(b)) => uses.push(*b),
            Some(Symbol::Size(s)) => {
                if sizes[*s].kind == SizeKind::Unknown {
                    errors.push(Diagnostic::error(
                        expr.pos,
                        format!(
                            "`{name}` is an unknown size; a formula cannot use its value, but can \
                             use the range it is the size of"
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
        ExprKind::Apply { func, args, .. } => match Builtin::lookup(&func.name) {
            Some(builtin) if args.len() != builtin.arity() => {
                let plural = if builtin.arity() == 1 { "" } else { "s" };
                errors.push(Diagnostic::error(
                    func.pos,
                    format!(
                        "`{}` takes {} argument{plural}: `{}`",
                        func.name,
                        builtin.arity(),
                        builtin.usage()
                    ),
                ));
            }
            Some(_) => {}
            None if symbols.contains_key(&func.name) => errors.push(Diagnostic::error(
                func.pos,
                format!(
                    "`{}` is not a function, so it cannot take arguments",
                    func.name
                ),
            )),
            None => errors.push(Diagnostic::error(
                func.pos,
                format!("unknown function `{}`", func.name),
            )),
        },
        _ => {}
    }
    for child in expr.children() {
        resolve_expr(child, symbols, sizes, uses, errors);
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

/// Orders the bindings so that each comes after the bindings it uses.
fn dependency_order(bindings: &[Binding], uses: &[Vec<usize>]) -> Result<Vec<usize>, Diagnostic> {
    const UNSEEN: u8 = 0;
    const OPEN: u8 = 1;
    const DONE: u8 = 2;
    let mut state = vec![UNSEEN; bindings.len()];
    let mut order = Vec::with_capacity(bindings.len());
    for start in 0..bindings.len() {
        if state[start] != UNSEEN {
            continue;
        }
        state[start] = OPEN;
        // Each entry is a binding and how many of its uses have been followed.
        let mut stack = vec![(start, 0)];
        while let Some(top) = stack.last_mut() {
            let (binding, followed) = *top;
            let Some(&next) = uses[binding].get(followed) else {
                state[binding] = DONE;
                order.push(binding);
                stack.pop();
                continue;
            };
            top.1 += 1;
            match state[next] {
                UNSEEN => {
                    state[next] = OPEN;
                    stack.push((next, 0));
                }
                OPEN => {
                    let from = stack
                        .iter()
                        .position(|&(b, _)| b == next)
                        .expect("an open binding is on the stack");
                    let mut path: Vec<&str> = stack[from..]
                        .iter()
                        .map(|&(b, _)| bindings[b].name.as_str())
                        .collect();
                    path.push(&bindings[next].name);
                    return Err(Diagnostic::error(
                        bindings[next].pos,
                        format!(
                            "circular definition: {}; a balance carried from one period to the \
                             next is not supported yet",
                            path.join(" -> ")
                        ),
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(order)
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

/// Whether a name reads as an A1 cell reference: one to three letters, then digits.
fn looks_like_cell(name: &str) -> bool {
    let letters = name.bytes().take_while(u8::is_ascii_lowercase).count();
    let digits = &name[letters..];
    (1..=3).contains(&letters) && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Whether a name reads as an R1C1 reference: `r`, `c`, `rc`, or either with digits.
fn looks_like_r1c1(name: &str) -> bool {
    let bytes = name.as_bytes();
    let digits_from = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        i
    };
    let mut i = 0;
    let row = bytes.first() == Some(&b'r');
    if row {
        i = digits_from(1);
    }
    if bytes.get(i) == Some(&b'c') {
        i = digits_from(i + 1);
    } else if !row {
        return false;
    }
    i == bytes.len()
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
        resolve(&parse(source).expect("parses")).expect("resolves")
    }

    fn errors(source: &str) -> Vec<String> {
        resolve(&parse(source).expect("parses"))
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
                "2:1: error: circular definition: a -> b -> d -> a; a balance carried from one \
                 period to the next is not supported yet"
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
