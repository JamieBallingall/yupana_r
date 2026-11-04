//! The built-in functions: their names in `.yup` and how many arguments they take.
//!
//! What each does to sizes is in Infer, and how it is written in the workbook is in Lower.
//! Arguments that say how much (`lag 1 x`, `round 2 x`) come first and the data last, so that a
//! call reads from the operation to what it acts on. `lag`, `tail` and `init` may say which way
//! they work, `across` or `down`, straight after their name. A function of no arguments, `na`,
//! is written by its name alone.

/// A built-in function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Abs,
    Average,
    Column,
    Csv,
    Exp,
    Init,
    Lag,
    Ln,
    Lookup,
    Max,
    Maximum,
    Min,
    Minimum,
    Na,
    Not,
    Round,
    Sqrt,
    Sum,
    SumColumns,
    SumRows,
    Tail,
    Transpose,
}

/// Each function's name, the function, how many arguments it takes, and how it is called.
const TABLE: &[(&str, Builtin, usize, &str)] = &[
    ("abs", Builtin::Abs, 1, "abs x"),
    ("average", Builtin::Average, 1, "average x"),
    ("column", Builtin::Column, 2, "column \"name\" table"),
    ("csv", Builtin::Csv, 1, "csv \"file.csv\""),
    ("exp", Builtin::Exp, 1, "exp x"),
    ("init", Builtin::Init, 1, "init x"),
    ("lag", Builtin::Lag, 2, "lag 1 x, or lag 1 x seed first"),
    ("ln", Builtin::Ln, 1, "ln x"),
    ("lookup", Builtin::Lookup, 3, "lookup key keys values"),
    ("max", Builtin::Max, 2, "max a b"),
    ("maximum", Builtin::Maximum, 1, "maximum x"),
    ("min", Builtin::Min, 2, "min a b"),
    ("minimum", Builtin::Minimum, 1, "minimum x"),
    ("na", Builtin::Na, 0, "na"),
    ("not", Builtin::Not, 1, "not x"),
    ("round", Builtin::Round, 2, "round digits x"),
    ("sqrt", Builtin::Sqrt, 1, "sqrt x"),
    ("sum", Builtin::Sum, 1, "sum x"),
    ("sum_columns", Builtin::SumColumns, 1, "sum_columns x"),
    ("sum_rows", Builtin::SumRows, 1, "sum_rows x"),
    ("tail", Builtin::Tail, 1, "tail x"),
    ("transpose", Builtin::Transpose, 1, "transpose x"),
];

impl Builtin {
    /// The function with the given `.yup` name.
    pub fn lookup(name: &str) -> Option<Builtin> {
        TABLE
            .iter()
            .find(|entry| entry.0 == name)
            .map(|entry| entry.1)
    }

    fn entry(self) -> &'static (&'static str, Builtin, usize, &'static str) {
        TABLE
            .iter()
            .find(|entry| entry.1 == self)
            .expect("every built-in function is in the table")
    }

    /// How many arguments the function takes.
    pub fn arity(self) -> usize {
        self.entry().2
    }

    /// How the function is called, for messages.
    pub fn usage(self) -> &'static str {
        self.entry().3
    }
}
