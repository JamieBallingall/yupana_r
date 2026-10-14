//! Prints a syntax tree as `.yup` text.
//!
//! Every step up to Check shows its result this way. The spacing and the parentheses are the
//! printer's own, so reading a printed file and printing it again gives the same text.

use crate::syntax::{
    BinOp, Coord, Expr, ExprKind, Item, ItemKind, Program, Signature, SizeKind, SizeSyntax,
};

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
        ItemKind::Sheet(name) => format!("sheet {}", quote_text(&name.value)),
        ItemKind::Heading { text, at } => match at {
            Some(at) => format!("heading {} @ {}", quote_text(&text.value), coord(at)),
            None => format!("heading {}", quote_text(&text.value)),
        },
        ItemKind::Gap => "gap".to_string(),
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
            format!("{head} = {}", expression(expr))
        }
    }
}

fn signature(sig: &Signature) -> String {
    let mut parts = vec![format!("{} :", sig.name.name)];
    if let Some(shape) = &sig.shape {
        parts.push(format!("[{}, {}]", size(&shape.rows), size(&shape.columns)));
    }
    if let Some(from) = &sig.from {
        parts.push(format!("from [{}, {}]", from.row, from.column));
    }
    if let Some(label) = &sig.label {
        parts.push(quote_text(&label.value));
    }
    if let Some(at) = &sig.at {
        parts.push(format!("@ {}", coord(at)));
    }
    if sig.input.is_some() {
        parts.push("Input".to_string());
    }
    if let Some(format) = &sig.format {
        parts.push(format!("Format {}", quote_string(&format.value)));
    }
    if sig.bold.is_some() {
        parts.push("Bold".to_string());
    }
    if sig.italic.is_some() {
        parts.push("Italic".to_string());
    }
    parts.join(" ")
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

fn coord(coord: &Coord) -> String {
    format!("({}, {})", size(&coord.row), size(&coord.column))
}

/// Quotes a label or a sheet name, doubling any quote inside.
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
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::List(_) => ATOM,
    }
}

fn write_expr(out: &mut String, expr: &Expr, context: u8) {
    let parens = precedence(expr) < context;
    if parens {
        out.push('(');
    }
    match &expr.kind {
        ExprKind::Number { text, .. } => out.push_str(text),
        ExprKind::Str(value) => out.push_str(&quote_string(value)),
        ExprKind::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        ExprKind::Name(name) => out.push_str(name),
        ExprKind::Apply { func, args, seed } => {
            out.push_str(&func.name);
            for arg in args {
                out.push(' ');
                write_expr(out, arg, ATOM);
            }
            if let Some(seed) = seed {
                out.push_str(" from ");
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
        let source = "yup 0.1\n\n\n// Inputs\nsheet 'Inputs'\nh : 'Horizon''s end'   Input\nh = \
                      5 // years\n\nx=(1+2)*3-(4-5)\ny = - (2 ^ 2)  ++ \"a\\\"b\"\nz=[1,2;3,4]\n\n";
        assert_eq!(
            reprint(source),
            "yup 0.1\n// Inputs\nsheet 'Inputs'\nh : 'Horizon''s end' Input\nh = 5 // \
             years\n\nx = (1 + 2) * 3 - (4 - 5)\ny = -2 ^ 2 ++ \"a\\\"b\"\nz = [1, 2; 3, 4]\n"
        );
    }

    #[test]
    fn printing_twice_changes_nothing() {
        let sources = [
            "yup 0.1\nx = (a - b) - (c - d)\n",
            "yup 0.1\nx = 2 ^ 3 ^ 2 + (2 ^ 3) ^ 2\n",
            "yup 0.1\nx = (-a) ^ 2 + -(a * b) - -c\n",
            "yup 0.1\nx = if a > b then 1 .. n else f (g x) y\n",
            "yup 0.1\nx = 1 + (if c then 2 else 3)\n",
            "yup 0.1\nx = (a ++ b) ++ c ++ d\n",
            "yup 0.1\nx = a || b && c || not d && (e || f)\n",
            "yup 0.1\nx = (a || b) .. c\n",
            "yup 0.1\nx = lag 1 (a + b) from (c * 2)\n",
            "yup 0.1\nx : [s + 4, 2 * n - 1] from [0, 1] @ (4 + s, 5) 'L' Format \"0%\" Bold\n",
        ];
        for source in sources {
            let once = reprint(source);
            assert_eq!(reprint(&once), once, "{source}");
        }
        assert_eq!(
            reprint(sources[0]),
            "yup 0.1\nx = a - b - (c - d)\n",
            "left-grouped operators drop parentheses on the left only"
        );
    }
}
