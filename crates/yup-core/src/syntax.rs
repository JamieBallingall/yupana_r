//! The syntax tree of a `.yup` file, as the parser produces it and the printer prints it.
//!
//! Later steps fill in the optional parts of signatures (sizes, spans and anchors), so an
//! elaborated file is the same tree with more of it present.

use crate::diagnostic::Pos;

/// A whole `.yup` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// The language version, from the first line.
    pub version: String,
    pub items: Vec<Item>,
}

/// One line of a file, or several when brackets are left open.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub pos: Pos,
    pub kind: ItemKind,
    /// A comment at the end of the line, without its `//`.
    pub comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// An empty line, kept so that a printed file keeps its paragraphs.
    Blank,
    /// A line that holds only a comment, without its `//`.
    Comment(String),
    /// `sheet 'Name'`: the following items go on this sheet.
    Sheet(Text),
    /// `heading 'Text'`: a line of bold text in the layout.
    Heading { text: Text, at: Option<Coord> },
    /// `gap`: an empty row in the layout.
    Gap,
    /// `n : Size Unknown` or `s : Size Fixed`.
    Size { name: Ident, kind: SizeKind },
    /// `name : …`: what is known about a binding besides its definition.
    Signature(Box<Signature>),
    /// `name = expression`, or `name parameter … = expression` for a function.
    Definition {
        name: Ident,
        params: Vec<Ident>,
        expr: Expr,
    },
}

/// A name and where it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub pos: Pos,
}

/// Quoted text and where it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub value: String,
    pub pos: Pos,
}

/// Whether a size variable is known when the file is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeKind {
    /// Known when the file is written: `s = 4`.
    Fixed,
    /// Decided in the workbook, such as a horizon its user can change.
    Unknown,
}

/// A size as written: a whole number plus a sum of size variables, each with a positive
/// coefficient, such as `n - 1` or `2 * s + 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeSyntax {
    pub constant: i64,
    pub terms: Vec<(i64, Ident)>,
    pub pos: Pos,
}

/// A cell as written, `(row, column)`, both counted from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coord {
    pub row: SizeSyntax,
    pub column: SizeSyntax,
    pub pos: Pos,
}

/// A size in rows and columns, `[rows, columns]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeSyntax {
    pub rows: SizeSyntax,
    pub columns: SizeSyntax,
    pub pos: Pos,
}

/// Where a range starts along its rows and columns, `from [row, column]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offset {
    pub row: i64,
    pub column: i64,
    pub pos: Pos,
}

/// Everything a signature line can say about a binding.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub name: Ident,
    pub shape: Option<ShapeSyntax>,
    pub from: Option<Offset>,
    pub label: Option<Text>,
    pub at: Option<Coord>,
    pub input: Option<Pos>,
    /// A number format, such as `0.0%`.
    pub format: Option<Text>,
    pub bold: Option<Pos>,
    pub italic: Option<Pos>,
}

impl Signature {
    /// A signature that says nothing yet.
    pub fn new(name: Ident) -> Signature {
        Signature {
            name,
            shape: None,
            from: None,
            label: None,
            at: None,
            input: None,
            format: None,
            bold: None,
            italic: None,
        }
    }
}

/// An expression and where it was written. For an operator, the position is the operator's.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub pos: Pos,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Number {
        value: f64,
        text: String,
    },
    Str(String),
    Bool(bool),
    Name(String),
    /// A function applied to its arguments, `f a b`. Only `lag` takes a seed, `from x`.
    Apply {
        func: Ident,
        args: Vec<Expr>,
        seed: Option<Box<Expr>>,
    },
    Neg(Box<Expr>),
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then: Box<Expr>,
        other: Box<Expr>,
    },
    /// A list of literal values: rows separated by `;`, values by `,`.
    List(Vec<Vec<Expr>>),
    /// `first .. last`: the whole numbers from `first` to `last`, inclusive.
    Range {
        first: Box<Expr>,
        last: Box<Expr>,
    },
}

impl Expr {
    /// The expressions directly inside this one.
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::Number { .. } | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Name(_) => {
                Vec::new()
            }
            ExprKind::Apply { args, seed, .. } => args.iter().chain(seed.as_deref()).collect(),
            ExprKind::Neg(operand) => vec![operand],
            ExprKind::Binary { left, right, .. } => vec![left, right],
            ExprKind::If { cond, then, other } => vec![cond, then, other],
            ExprKind::List(rows) => rows.iter().flatten().collect(),
            ExprKind::Range { first, last } => vec![first, last],
        }
    }

    /// The value of a whole number written as a literal, such as `3` or `-1`.
    pub fn integer(&self) -> Option<i64> {
        match &self.kind {
            ExprKind::Number { text, .. } if text.bytes().all(|b| b.is_ascii_digit()) => {
                text.parse().ok()
            }
            ExprKind::Neg(operand) => operand.integer().map(|value| -value),
            _ => None,
        }
    }

    /// Whether this is a literal value: a number (perhaps negated), a string or a logical.
    pub fn is_literal(&self) -> bool {
        match &self.kind {
            ExprKind::Number { .. } | ExprKind::Str(_) | ExprKind::Bool(_) => true,
            ExprKind::Neg(operand) => matches!(operand.kind, ExprKind::Number { .. }),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOp {
    /// How the operator is written in `.yup`.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
            BinOp::Concat => "++",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Eq => "==",
            BinOp::Ne => "/=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
        }
    }
}
