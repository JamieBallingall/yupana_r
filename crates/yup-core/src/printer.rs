//! Prints a syntax tree as `.yup` text.
//!
//! Every step up to Check shows its result this way. The spacing and the parentheses are the
//! printer's own, so reading a printed file and printing it again gives the same text.

use crate::cells::{self, cell_name};
use crate::syntax::{
    BinOp, Expr, ExprKind, Field, Item, ItemKind, Placement, Program, Signature, SizeKind,
    SizeSyntax, Type,
};

/// The widest line the printer writes a record on before it gives each field a line of its own.
const WIDTH: usize = 100;

/// Prints a whole file.
pub fn print(program: &Program) -> String {
    let mut out = format!("yup {}\n", program.version);
    let mut previous_blank = true;
    for item in &program.items {
        if item.kind == ItemKind::Blank {
            // Runs of blank lines print as one, and a file never starts with one.
            if !previous_blank {
                out.push('\n');
            }
            previous_blank = true;
            continue;
        }
        previous_blank = false;
        out.push_str(&item_text(item));
        if let Some(comment) = &item.comment {
            out.push_str(" //");
            out.push_str(comment);
        }
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn item_text(item: &Item) -> String {
    match &item.kind {
        ItemKind::Blank => String::new(),
        ItemKind::Comment(text) => format!("//{text}"),
        ItemKind::Size { name, kind } => {
            let kind = match kind {
                SizeKind::Fixed => "Fixed",
                SizeKind::Unknown => "Unknown",
            };
            format!("{} : Size {kind}", name.name)
        }
        ItemKind::Signature(sig) => signature(sig),
        ItemKind::Definition { name, params, expr } => {
            let mut head = name.name.clone();
            for param in params {
                head.push(' ');
                head.push_str(&param.name);
            }
            let line = format!("{head} = {}", expression(expr));
            if line.chars().count() <= WIDTH {
                return line;
            }
            // A long record gives each field a line of its own, commas first.
            match &expr.kind {
                ExprKind::Record(fields) => format!("{head} =\n{}", field_lines(fields)),
                ExprKind::Update { base, fields } => {
                    let mut base_text = String::new();
                    write_expr(&mut base_text, base, ATOM);
                    format!("{head} = {base_text}\n{}", field_lines(fields))
                }
                _ => line,
            }
        }
    }
}

fn field_lines(fields: &[Field]) -> String {
    let mut out = String::new();
    for (index, field) in fields.iter().enumerate() {
        out.push_str(if index == 0 { "  { " } else { "  , " });
        out.push_str(&field.name.name);
        out.push_str(" = ");
        write_expr(&mut out, &field.value, LOOSE);
        out.push('\n');
    }
    out.push_str("  }");
    out
}

/// A signature, with its parts in the canonical order whatever order they were written in: the
/// role and type, then how the cells look, then the range's shape, then its place.
fn signature(sig: &Signature) -> String {
    let mut parts = vec![format!("{} :", sig.name.name), sig.role.word().to_string()];
    if let Some((ty, _)) = &sig.ty {
        parts.push(type_text(ty));
    }
    if let Some(style) = &sig.style {
        parts.push(format!("style {}", style.name));
    }
    if let Some(shape) = &sig.shape {
        parts.push(format!(
            "size [{}, {}]",
            size(&shape.rows),
            size(&shape.columns)
        ));
    }
    if let Some(start) = &sig.start {
        parts.push(format!("start [{}, {}]", start.row, start.column));
    }
    match &sig.place {
        Some(Placement::At {
            sheet,
            row,
            column,
            absolute,
            ..
        }) => {
            let row = u32::try_from(*row).expect("the parser keeps a cell on the sheet");
            let column = u32::try_from(*column).expect("the parser keeps a cell on the sheet");
            let cell = if *absolute {
                cells::absolute(row, column)
            } else {
                cell_name(row, column)
            };
            parts.push(format!("at {} {cell}", sheet.name));
        }
        Some(Placement::Beside { reference, moves }) => {
            parts.push(reference.name.clone());
            for step in moves {
                parts.push(step.direction.word().to_string());
                if let Some(distance) = step.distance {
                    parts.push(distance.to_string());
                }
                if let Some(side) = step.from {
                    parts.push(format!("from {}", side.word()));
                }
            }
        }
        None => {}
    }
    parts.join(" ")
}

fn type_text(ty: &Type) -> String {
    match ty {
        Type::Cell(ty) => ty.word().to_string(),
        Type::Function(types) => types
            .iter()
            .map(|ty| ty.word())
            .collect::<Vec<_>>()
            .join(" -> "),
        Type::Style(ty) => ty.word().to_string(),
    }
}

/// Prints a size: its variables first, then its constant, as in `n - 1`.
pub fn size(size: &SizeSyntax) -> String {
    let mut text = String::new();
    for (coefficient, name) in &size.terms {
        if !text.is_empty() {
            text.push_str(" + ");
        }
        if *coefficient == 1 {
            text.push_str(&name.name);
        } else {
            text.push_str(&format!("{coefficient} * {}", name.name));
        }
    }
    if text.is_empty() {
        text = size.constant.to_string();
    } else if size.constant > 0 {
        text.push_str(&format!(" + {}", size.constant));
    } else if size.constant < 0 {
        text.push_str(&format!(" - {}", size.constant.unsigned_abs()));
    }
    text
}

/// Quotes a setting, such as a sheet name, doubling any quote inside.
pub fn quote_text(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Quotes a string value, escaping what needs it.
pub fn quote_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Prints an expression with as few parentheses as `.yup`'s precedence allows.
pub fn expression(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr, LOOSE);
    out
}

// How tightly each form binds, loosest first. A form needs parentheses where its context asks
// for a tighter one.
const LOOSE: u8 = 0;
const OR: u8 = 1;
const AND: u8 = 2;
const COMPARE: u8 = 3;
const CONCAT: u8 = 4;
const ADDITIVE: u8 = 5;
const MULTIPLICATIVE: u8 = 6;
const NEGATION: u8 = 7;
const POWER: u8 = 8;
const APPLICATION: u8 = 9;
const ATOM: u8 = 10;

fn precedence(expr: &Expr) -> u8 {
    match &expr.kind {
        ExprKind::If { .. } | ExprKind::Range { .. } => LOOSE,
        ExprKind::Binary { op, .. } => match op {
            BinOp::Or => OR,
            BinOp::And => AND,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => COMPARE,
            BinOp::Concat => CONCAT,
            BinOp::Add | BinOp::Sub => ADDITIVE,
            BinOp::Mul | BinOp::Div => MULTIPLICATIVE,
            BinOp::Pow => POWER,
        },
        ExprKind::Neg(_) => NEGATION,
        ExprKind::Apply { .. } => APPLICATION,
        ExprKind::Number { .. }
        | ExprKind::Str(_)
        | ExprKind::Text(_)
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Constructor(_)
        | ExprKind::List(_)
        | ExprKind::Record(_)
        | ExprKind::Update { .. } => ATOM,
    }
}

fn write_fields(out: &mut String, fields: &[Field]) {
    if fields.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{ ");
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&field.name.name);
        out.push_str(" = ");
        write_expr(out, &field.value, LOOSE);
    }
    out.push_str(" }");
}

fn write_expr(out: &mut String, expr: &Expr, context: u8) {
    let parens = precedence(expr) < context;
    if parens {
        out.push('(');
    }
    match &expr.kind {
        ExprKind::Number { text, .. } => out.push_str(text),
        ExprKind::Str(value) => out.push_str(&quote_string(value)),
        ExprKind::Text(value) => out.push_str(&quote_text(value)),
        ExprKind::Bool(value) => out.push_str(if *value { "True" } else { "False" }),
        ExprKind::Name(name) | ExprKind::Constructor(name) => out.push_str(name),
        ExprKind::Apply {
            func,
            axis,
            args,
            seed,
        } => {
            out.push_str(&func.name);
            if let Some(axis) = axis {
                out.push(' ');
                out.push_str(axis.word());
            }
            for arg in args {
                out.push(' ');
                write_expr(out, arg, ATOM);
            }
            if let Some(seed) = seed {
                out.push_str(" seed ");
                write_expr(out, seed, ATOM);
            }
        }
        ExprKind::Neg(operand) => {
            out.push('-');
            write_expr(out, operand, POWER);
        }
        ExprKind::Binary { op, left, right } => {
            // Comparisons do not chain; `||`, `&&`, `++` and `^` group to the right, and the
            // rest to the left.
            let (left_context, right_context) = match op {
                BinOp::Or => (AND, OR),
                BinOp::And => (COMPARE, AND),
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    (CONCAT, CONCAT)
                }
                BinOp::Concat => (ADDITIVE, CONCAT),
                BinOp::Add | BinOp::Sub => (ADDITIVE, MULTIPLICATIVE),
                BinOp::Mul | BinOp::Div => (MULTIPLICATIVE, NEGATION),
                BinOp::Pow => (APPLICATION, NEGATION),
            };
            write_expr(out, left, left_context);
            out.push(' ');
            out.push_str(op.symbol());
            out.push(' ');
            write_expr(out, right, right_context);
        }
        ExprKind::If { cond, then, other } => {
            out.push_str("if ");
            write_expr(out, cond, LOOSE);
            out.push_str(" then ");
            write_expr(out, then, LOOSE);
            out.push_str(" else ");
            write_expr(out, other, LOOSE);
        }
        ExprKind::List(rows) => {
            out.push('[');
            for (r, row) in rows.iter().enumerate() {
                if r > 0 {
                    out.push_str("; ");
                }
                for (v, value) in row.iter().enumerate() {
                    if v > 0 {
                        out.push_str(", ");
                    }
                    write_expr(out, value, LOOSE);
                }
            }
            out.push(']');
        }
        ExprKind::Range { first, last } => {
            write_expr(out, first, OR);
            out.push_str(" .. ");
            write_expr(out, last, OR);
        }
        ExprKind::Record(fields) => write_fields(out, fields),
        ExprKind::Update { base, fields } => {
            write_expr(out, base, ATOM);
            out.push(' ');
            write_fields(out, fields);
        }
    }
    if parens {
        out.push(')');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn reprint(source: &str) -> String {
        print(&parse(source).expect("parses"))
    }

    #[test]
    fn printing_is_canonical() {
        let source = "yup 0.2\n\n\n// Values\nh : Value   Number at  sheet  B2  style  money\nh = \
                      5 // years\n\nx=(1+2)*3-(4-5)\ny = - (2 ^ 2)  ++ \"a\\\"b\"\nz=[1,2;3,4]\n\n";
        assert_eq!(
            reprint(source),
            "yup 0.2\n// Values\nh : Value Number style money at sheet B2\nh = 5 // \
             years\n\nx = (1 + 2) * 3 - (4 - 5)\ny = -2 ^ 2 ++ \"a\\\"b\"\nz = [1, 2; 3, 4]\n"
        );
    }

    #[test]
    fn a_long_record_gives_each_field_a_line() {
        let source = "yup 0.2\ngrand_total = money { font = { bold = True, name = 'Arial' }, border \
                      = { top = { weight = Thin }, bottom = { weight = Double } } }\n";
        assert_eq!(
            reprint(source),
            "yup 0.2\ngrand_total = money\n  { font = { bold = True, name = 'Arial' }\n  , border \
             = { top = { weight = Thin }, bottom = { weight = Double } }\n  }\n"
        );
    }

    #[test]
    fn printing_twice_changes_nothing() {
        let sources = [
            "yup 0.2\nx = (a - b) - (c - d)\n",
            "yup 0.2\nx = 2 ^ 3 ^ 2 + (2 ^ 3) ^ 2\n",
            "yup 0.2\nx = (-a) ^ 2 + -(a * b) - -c\n",
            "yup 0.2\nx = if a > b then 1 .. n else f (g x) y\n",
            "yup 0.2\nx = 1 + (if c then 2 else 3)\n",
            "yup 0.2\nx = (a ++ b) ++ c ++ d\n",
            "yup 0.2\nx = a || b && c || not d && (e || f)\n",
            "yup 0.2\nx = (a || b) .. c\n",
            "yup 0.2\nx = lag down 1 (a + b) seed (c * 2)\n",
            "yup 0.2\nx : Formula Number style s size [s + 4, 2 * n - 1] start [0, 1] y down 2 right 1 from west\n",
            "yup 0.2\nx : Lambda Number -> Text -> Logical\n",
            "yup 0.2\nx = base { a = { b = Thin }, c = 'x''y' }\n",
            "yup 0.2\nx = { a = 1 }\n",
        ];
        for source in sources {
            let once = reprint(source);
            assert_eq!(reprint(&once), once, "{source}");
        }
        assert_eq!(
            reprint(sources[0]),
            "yup 0.2\nx = a - b - (c - d)\n",
            "left-grouped operators drop parentheses on the left only"
        );
    }
}
