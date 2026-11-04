//! The syntax tree of a `.yup` file, as the parser produces it and the printer prints it.
//!
//! Later steps fill in the optional parts of signatures (types, sizes, spans and places), so an
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
    /// `n : Size Unknown` or `s : Size Fixed`.
    Size { name: Ident, kind: SizeKind },
    /// `name : Role …`: everything about a binding besides its definition.
    Signature(Box<Signature>),
    /// `name = expression`, or `name parameter … = expression` for a lambda.
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

/// Whether a size variable is known when the file is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeKind {
    /// Known when the file is written: `s = 4`.
    Fixed,
    /// Decided in the workbook, such as a horizon its user can change.
    Unknown,
}

/// What a binding is, and so what it becomes in the workbook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// A formula, in its own cells, and a defined name.
    Formula,
    /// A value written into its cells, and a defined name.
    Value,
    /// Text in a cell, which formulas cannot use.
    Label,
    /// A value with no cells of its own, used by the formulas around it.
    Local,
    /// A function of its parameters: a defined name holding a LAMBDA.
    Lambda,
    /// The look of cells.
    Style,
    /// A worksheet's settings.
    Sheet,
    /// The workbook's own settings.
    Workbook,
}

impl Role {
    /// The role as a signature writes it.
    pub fn word(self) -> &'static str {
        match self {
            Role::Formula => "Formula",
            Role::Value => "Value",
            Role::Label => "Label",
            Role::Local => "Local",
            Role::Lambda => "Lambda",
            Role::Style => "Style",
            Role::Sheet => "Sheet",
            Role::Workbook => "Workbook",
        }
    }

    /// The role a signature's word names.
    pub fn from_word(word: &str) -> Option<Role> {
        [
            Role::Formula,
            Role::Value,
            Role::Label,
            Role::Local,
            Role::Lambda,
            Role::Style,
            Role::Sheet,
            Role::Workbook,
        ]
        .into_iter()
        .find(|role| role.word() == word)
    }

    /// Whether the binding has cells of its own, and so a style and a place.
    pub fn is_placed(self) -> bool {
        matches!(self, Role::Formula | Role::Value | Role::Label)
    }
}

/// What a cell holds, as the spreadsheet app's TYPE function tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CellType {
    Number,
    Text,
    Logical,
}

impl CellType {
    pub fn word(self) -> &'static str {
        match self {
            CellType::Number => "Number",
            CellType::Text => "Text",
            CellType::Logical => "Logical",
        }
    }

    pub fn from_word(word: &str) -> Option<CellType> {
        [CellType::Number, CellType::Text, CellType::Logical]
            .into_iter()
            .find(|ty| ty.word() == word)
    }
}

/// The kinds of record a style can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StyleType {
    Cell,
    Font,
    Border,
    Edge,
    Alignment,
    Color,
}

impl StyleType {
    pub fn word(self) -> &'static str {
        match self {
            StyleType::Cell => "Cell",
            StyleType::Font => "Font",
            StyleType::Border => "Border",
            StyleType::Edge => "Edge",
            StyleType::Alignment => "Alignment",
            StyleType::Color => "Color",
        }
    }

    pub fn from_word(word: &str) -> Option<StyleType> {
        [
            StyleType::Cell,
            StyleType::Font,
            StyleType::Border,
            StyleType::Edge,
            StyleType::Alignment,
            StyleType::Color,
        ]
        .into_iter()
        .find(|ty| ty.word() == word)
    }
}

/// A type as a signature writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// What each cell of a range holds: `Number`.
    Cell(CellType),
    /// A lambda's parameters and result: `Number -> Number -> Number`.
    Function(Vec<CellType>),
    /// The kind of record a style is: `Cell`.
    Style(StyleType),
}

/// A size as written: a whole number plus a sum of size variables, each with a positive
/// coefficient, such as `n - 1` or `2 * s + 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeSyntax {
    pub constant: i64,
    pub terms: Vec<(i64, Ident)>,
    pub pos: Pos,
}

/// A size in rows and columns, `[rows, columns]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeSyntax {
    pub rows: SizeSyntax,
    pub columns: SizeSyntax,
    pub pos: Pos,
}

/// Where a range starts along its rows and columns, `start [row, column]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offset {
    pub row: i64,
    pub column: i64,
    pub pos: Pos,
}

/// The way `lag`, `tail` and `init` work along a range: from column to column, or from row to
/// row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    Across,
    Down,
}

impl Axis {
    pub fn word(self) -> &'static str {
        match self {
            Axis::Across => "across",
            Axis::Down => "down",
        }
    }

    pub fn from_word(word: &str) -> Option<Axis> {
        [Axis::Across, Axis::Down]
            .into_iter()
            .find(|axis| axis.word() == word)
    }
}

/// A direction a move goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn word(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }

    pub fn from_word(word: &str) -> Option<Direction> {
        [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ]
        .into_iter()
        .find(|direction| direction.word() == word)
    }

    /// Whether the direction runs along a row, from column to column.
    pub fn is_horizontal(self) -> bool {
        matches!(self, Direction::Left | Direction::Right)
    }
}

/// A side of a range, as `from` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    East,
    West,
    North,
    South,
}

impl Side {
    pub fn word(self) -> &'static str {
        match self {
            Side::East => "east",
            Side::West => "west",
            Side::North => "north",
            Side::South => "south",
        }
    }

    pub fn from_word(word: &str) -> Option<Side> {
        [Side::East, Side::West, Side::North, Side::South]
            .into_iter()
            .find(|side| side.word() == word)
    }

    /// Whether the side is one of a range's columns rather than one of its rows.
    pub fn is_column(self) -> bool {
        matches!(self, Side::East | Side::West)
    }
}

/// One move of a placement: `right 2`, or `right 1 from west`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub direction: Direction,
    /// The distance as written; a move without one goes 1.
    pub distance: Option<i64>,
    pub from: Option<Side>,
    pub pos: Pos,
}

impl Move {
    pub fn distance(&self) -> i64 {
        self.distance.unwrap_or(1)
    }
}

/// Where a binding's cells go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// `at forecast B2`: a cell of a sheet, counted from 1.
    At {
        sheet: Ident,
        row: i64,
        column: i64,
        /// Whether the cell was written `$B$2`. It means the same as `B2`, and is kept so that a
        /// cell everything else is placed from can be marked as one that never moves.
        absolute: bool,
        pos: Pos,
    },
    /// `revenue down 1 right 1 from west`: beside another binding.
    Beside { reference: Ident, moves: Vec<Move> },
}

/// A binding's style as its signature names it: a style's name, or `_` for the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StyleName {
    Default(Pos),
    Named(Ident),
}

/// Everything a signature line says about a binding.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub name: Ident,
    pub role: Role,
    pub role_pos: Pos,
    pub ty: Option<(Type, Pos)>,
    pub style: Option<StyleName>,
    pub shape: Option<ShapeSyntax>,
    pub start: Option<Offset>,
    pub place: Option<Placement>,
}

impl Signature {
    /// A signature that says nothing beyond the binding's role.
    pub fn new(name: Ident, role: Role, role_pos: Pos) -> Signature {
        Signature {
            name,
            role,
            role_pos,
            ty: None,
            style: None,
            shape: None,
            start: None,
            place: None,
        }
    }
}

/// An expression and where it was written. For an operator, the position is the operator's.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub pos: Pos,
    pub kind: ExprKind,
}

/// A field of a record: `bold = True`.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Ident,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Number {
        value: f64,
        text: String,
    },
    /// A string value, in double quotes.
    Str(String),
    /// A setting in single quotes: a sheet name, a format code or a font name.
    Text(String),
    Bool(bool),
    Name(String),
    /// A word the language defines, such as `Thin` or `Center`.
    Constructor(String),
    /// A function applied to its arguments, `f a b`. `lag`, `tail` and `init` may say the
    /// direction they work along, and only `lag` takes a first value, `seed x`.
    Apply {
        func: Ident,
        axis: Option<Axis>,
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
    /// A list of values: rows separated by `;`, values by `,`.
    List(Vec<Vec<Expr>>),
    /// `first .. last`: the whole numbers from `first` to `last`, inclusive.
    Range {
        first: Box<Expr>,
        last: Box<Expr>,
    },
    /// `{ name = value, … }`.
    Record(Vec<Field>),
    /// `base { name = value, … }`: a record built on another.
    Update {
        base: Box<Expr>,
        fields: Vec<Field>,
    },
}

impl Expr {
    /// The expressions directly inside this one.
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::Number { .. }
            | ExprKind::Str(_)
            | ExprKind::Text(_)
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Constructor(_) => Vec::new(),
            ExprKind::Apply { args, seed, .. } => args.iter().chain(seed.as_deref()).collect(),
            ExprKind::Neg(operand) => vec![operand],
            ExprKind::Binary { left, right, .. } => vec![left, right],
            ExprKind::If { cond, then, other } => vec![cond, then, other],
            ExprKind::List(rows) => rows.iter().flatten().collect(),
            ExprKind::Range { first, last } => vec![first, last],
            ExprKind::Record(fields) => fields.iter().map(|field| &field.value).collect(),
            ExprKind::Update { base, fields } => std::iter::once(base.as_ref())
                .chain(fields.iter().map(|field| &field.value))
                .collect(),
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
