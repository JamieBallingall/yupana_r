//! The built-in functions: their names in `.yup`, what they take and give, and, for most, the
//! app's function that writes them.
//!
//! One entry says everything about a function that the steps share: Resolve counts its
//! arguments, Infer shapes and types its result from its kind, Resolve lets a function that
//! works position by position take part in a carried value, and Lower writes a function that is
//! one of the app's under that name. A function whose sizes, type or writing follow a rule of
//! its own has that rule in the step concerned.
//!
//! Arguments that say how much (`lag 1 x`, `round 2 x`) come first and the data last, so that a
//! call reads from the operation to what it acts on. `lag`, `tail` and `init` may say which way
//! they work, `across` or `down`, straight after their name. A function of no arguments, `na`,
//! is written by its name alone.

use crate::syntax::CellType;

/// A built-in function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Abs,
    Average,
    Ceiling,
    CeilingTo,
    Column,
    Csv,
    Exp,
    Floor,
    FloorTo,
    Init,
    Lag,
    Ln,
    Lookup,
    Max,
    Maximum,
    Min,
    Minimum,
    Mod,
    Na,
    Not,
    Round,
    RoundAway,
    RoundTo,
    RoundTowardZero,
    Sqrt,
    Sum,
    SumColumns,
    SumRows,
    Tail,
    Transpose,
}

/// How a function's result is shaped by its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Position by position. The first `fixed` arguments say how much and are single values;
    /// the rest combine as an operator's operands do, and give the result its shape. Such a
    /// function may take part in a value carried from one position to the next.
    Positionwise { fixed: usize },
    /// One value from all of its argument.
    Reduce,
    /// A rule of its own, in Infer.
    Own,
}

/// What a function's result holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gives {
    /// Always this type.
    Always(CellType),
    /// What the argument at this index holds.
    Like(usize),
    /// A rule of its own, in Infer.
    Own,
}

/// A function written as one of the app's, applied to the function's arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct App {
    /// The app's name, with the `_xlfn.` prefix that the file gives a newer function.
    pub name: &'static str,
    /// The indexes of the `.yup` arguments in the app's order: `round 2 x` is `ROUND(x,2)`.
    pub order: &'static [usize],
}

/// Everything about one function.
struct Entry {
    name: &'static str,
    builtin: Builtin,
    /// How the function is called, for messages.
    usage: &'static str,
    /// What each argument takes: a type, or `None` for anything.
    takes: &'static [Option<CellType>],
    kind: Kind,
    gives: Gives,
    app: Option<App>,
}

const NUMBER: Option<CellType> = Some(CellType::Number);
const TEXT: Option<CellType> = Some(CellType::Text);
const ANY: Option<CellType> = None;

const fn app(name: &'static str, order: &'static [usize]) -> Option<App> {
    Some(App { name, order })
}

/// A function of one number, position by position, that is the app's of the same name.
const fn numeric(
    name: &'static str,
    builtin: Builtin,
    usage: &'static str,
    app_name: &'static str,
) -> Entry {
    Entry {
        name,
        builtin,
        usage,
        takes: &[NUMBER],
        kind: Kind::Positionwise { fixed: 0 },
        gives: Gives::Always(CellType::Number),
        app: app(app_name, &[0]),
    }
}

/// A function that reduces a range of numbers to one, and is the app's of the same name.
const fn reduction(
    name: &'static str,
    builtin: Builtin,
    usage: &'static str,
    app_name: &'static str,
) -> Entry {
    Entry {
        name,
        builtin,
        usage,
        takes: &[NUMBER],
        kind: Kind::Reduce,
        gives: Gives::Always(CellType::Number),
        app: app(app_name, &[0]),
    }
}

/// A function of one number, position by position, after one value that says how much: `round
/// 2 x` is `ROUND(x,2)`.
const fn by(
    name: &'static str,
    builtin: Builtin,
    usage: &'static str,
    app_name: Option<&'static str>,
) -> Entry {
    Entry {
        name,
        builtin,
        usage,
        takes: &[NUMBER, NUMBER],
        kind: Kind::Positionwise { fixed: 1 },
        gives: Gives::Always(CellType::Number),
        app: match app_name {
            Some(app_name) => app(app_name, &[1, 0]),
            None => None,
        },
    }
}

// Rounding says its direction in its name. The app's ROUNDUP goes away from zero, while its
// CEILING.MATH goes toward plus infinity, so neither "up" nor "down" appears in a name here.
const TABLE: &[Entry] = &[
    numeric("abs", Builtin::Abs, "abs x", "ABS"),
    reduction("average", Builtin::Average, "average x", "AVERAGE"),
    numeric(
        "ceiling",
        Builtin::Ceiling,
        "ceiling x",
        "_xlfn.CEILING.MATH",
    ),
    by(
        "ceiling_to",
        Builtin::CeilingTo,
        "ceiling_to multiple x",
        Some("_xlfn.CEILING.MATH"),
    ),
    Entry {
        name: "column",
        builtin: Builtin::Column,
        usage: "column \"name\" table",
        takes: &[TEXT, ANY],
        kind: Kind::Own,
        gives: Gives::Own,
        app: None,
    },
    Entry {
        name: "csv",
        builtin: Builtin::Csv,
        usage: "csv \"file.csv\"",
        takes: &[TEXT],
        kind: Kind::Own,
        gives: Gives::Own,
        app: None,
    },
    numeric("exp", Builtin::Exp, "exp x", "EXP"),
    numeric("floor", Builtin::Floor, "floor x", "INT"),
    by(
        "floor_to",
        Builtin::FloorTo,
        "floor_to multiple x",
        Some("_xlfn.FLOOR.MATH"),
    ),
    Entry {
        name: "init",
        builtin: Builtin::Init,
        usage: "init x",
        takes: &[ANY],
        kind: Kind::Own,
        gives: Gives::Like(0),
        app: None,
    },
    Entry {
        name: "lag",
        builtin: Builtin::Lag,
        usage: "lag 1 x, or lag 1 x seed first",
        takes: &[NUMBER, ANY],
        kind: Kind::Own,
        gives: Gives::Own,
        app: None,
    },
    numeric("ln", Builtin::Ln, "ln x", "LN"),
    Entry {
        name: "lookup",
        builtin: Builtin::Lookup,
        usage: "lookup key keys values",
        takes: &[ANY, ANY, ANY],
        kind: Kind::Own,
        gives: Gives::Like(2),
        app: app("_xlfn.XLOOKUP", &[0, 1, 2]),
    },
    // The app's MAX and MIN reduce whole ranges, so `max` and `min`, which work position by
    // position, are written by Lower as a choice between the two.
    Entry {
        name: "max",
        builtin: Builtin::Max,
        usage: "max a b",
        takes: &[NUMBER, NUMBER],
        kind: Kind::Positionwise { fixed: 0 },
        gives: Gives::Always(CellType::Number),
        app: None,
    },
    reduction("maximum", Builtin::Maximum, "maximum x", "MAX"),
    Entry {
        name: "min",
        builtin: Builtin::Min,
        usage: "min a b",
        takes: &[NUMBER, NUMBER],
        kind: Kind::Positionwise { fixed: 0 },
        gives: Gives::Always(CellType::Number),
        app: None,
    },
    reduction("minimum", Builtin::Minimum, "minimum x", "MIN"),
    // The remainder takes the sign of the divisor (ECMA-376 Part 1 §18.17.7.218).
    by("mod", Builtin::Mod, "mod divisor x", Some("MOD")),
    Entry {
        name: "na",
        builtin: Builtin::Na,
        usage: "na",
        takes: &[],
        kind: Kind::Own,
        gives: Gives::Own,
        app: None,
    },
    Entry {
        name: "not",
        builtin: Builtin::Not,
        usage: "not x",
        takes: &[ANY],
        kind: Kind::Positionwise { fixed: 0 },
        gives: Gives::Always(CellType::Logical),
        app: app("NOT", &[0]),
    },
    Entry {
        name: "round",
        builtin: Builtin::Round,
        usage: "round digits x",
        takes: &[NUMBER, NUMBER],
        kind: Kind::Positionwise { fixed: 1 },
        gives: Gives::Always(CellType::Number),
        app: app("ROUND", &[1, 0]),
    },
    by(
        "round_away",
        Builtin::RoundAway,
        "round_away digits x",
        Some("ROUNDUP"),
    ),
    // Written by Lower with FLOOR.MATH: the app's MROUND gives #NUM! for a number and a multiple
    // of different signs (§18.17.7.221), and #VALUE! for a range where it takes a number.
    by("round_to", Builtin::RoundTo, "round_to multiple x", None),
    by(
        "round_toward_zero",
        Builtin::RoundTowardZero,
        "round_toward_zero digits x",
        Some("ROUNDDOWN"),
    ),
    numeric("sqrt", Builtin::Sqrt, "sqrt x", "SQRT"),
    reduction("sum", Builtin::Sum, "sum x", "SUM"),
    Entry {
        name: "sum_columns",
        builtin: Builtin::SumColumns,
        usage: "sum_columns x",
        takes: &[NUMBER],
        kind: Kind::Own,
        gives: Gives::Always(CellType::Number),
        app: None,
    },
    Entry {
        name: "sum_rows",
        builtin: Builtin::SumRows,
        usage: "sum_rows x",
        takes: &[NUMBER],
        kind: Kind::Own,
        gives: Gives::Always(CellType::Number),
        app: None,
    },
    Entry {
        name: "tail",
        builtin: Builtin::Tail,
        usage: "tail x",
        takes: &[ANY],
        kind: Kind::Own,
        gives: Gives::Like(0),
        app: None,
    },
    Entry {
        name: "transpose",
        builtin: Builtin::Transpose,
        usage: "transpose x",
        takes: &[ANY],
        kind: Kind::Own,
        gives: Gives::Like(0),
        app: None,
    },
];

impl Builtin {
    /// The function with the given `.yup` name.
    pub fn lookup(name: &str) -> Option<Builtin> {
        TABLE
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.builtin)
    }

    fn entry(self) -> &'static Entry {
        TABLE
            .iter()
            .find(|entry| entry.builtin == self)
            .expect("every built-in function is in the table")
    }

    /// The function's `.yup` name.
    pub fn name(self) -> &'static str {
        self.entry().name
    }

    /// How many arguments the function takes.
    pub fn arity(self) -> usize {
        self.entry().takes.len()
    }

    /// How the function is called, for messages.
    pub fn usage(self) -> &'static str {
        self.entry().usage
    }

    /// What each argument takes: a type, or `None` for anything.
    pub fn takes(self) -> &'static [Option<CellType>] {
        self.entry().takes
    }

    /// How the result is shaped by the arguments.
    pub fn kind(self) -> Kind {
        self.entry().kind
    }

    /// What the result holds.
    pub fn gives(self) -> Gives {
        self.entry().gives
    }

    /// The app's function that writes this one, if it is written as one.
    pub fn app(self) -> Option<App> {
        self.entry().app
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_entry_is_consistent() {
        for entry in TABLE {
            let builtin = entry.builtin;
            assert_eq!(Builtin::lookup(entry.name), Some(builtin), "{}", entry.name);
            assert!(entry.usage.starts_with(entry.name), "{}", entry.name);
            if let Kind::Positionwise { fixed } = entry.kind {
                assert!(fixed < entry.takes.len(), "{}", entry.name);
            }
            if let Gives::Like(index) = entry.gives {
                assert!(index < entry.takes.len(), "{}", entry.name);
            }
            if let Some(app) = entry.app {
                let mut order = app.order.to_vec();
                order.sort_unstable();
                assert!(
                    order.iter().copied().eq(0..entry.takes.len()),
                    "{} writes each argument once",
                    entry.name
                );
            }
        }
    }
}
