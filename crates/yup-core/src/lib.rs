//! The core of the `.yup` tool: it turns the text of a `.yup` file into the bytes of an xlsx
//! workbook.
//!
//! The core does no input or output, so that the same code runs natively and as WebAssembly. The
//! caller reads the files and writes the result.

mod check;
mod diagnostic;
mod elaborate;
mod functions;
mod ground;
mod infer;
mod lexer;
mod parser;
mod place;
mod printer;
mod resolve;
mod size;
mod syntax;

pub use diagnostic::{Diagnostic, Pos, Severity};

use elaborate::{Detail, elaborate};

/// The output of [`compile`] until the language is specified.
pub const NOT_YET_IMPLEMENTED: &str = "Not yet implemented\n";

/// Turns a `.yup` source into the bytes of an xlsx workbook.
///
/// Nothing is implemented yet: the input is ignored, and the output is always
/// [`NOT_YET_IMPLEMENTED`]. The input is bytes rather than text because the
/// caller should not have to decide what a valid source looks like.
///
/// ```
/// let output = yup_core::compile(b"anything at all");
/// assert_eq!(output, b"Not yet implemented\n");
/// ```
pub fn compile(_source: &[u8]) -> Vec<u8> {
    NOT_YET_IMPLEMENTED.as_bytes().to_vec()
}

/// The steps from a `.yup` file to an xlsx workbook, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// Turns the text into a syntax tree.
    Parse,
    /// Works out what each name refers to, and the order of the definitions.
    Resolve,
    /// Works out the size and span of every expression.
    Infer,
    /// Works out where everything goes, in cells that may depend on fixed sizes.
    Place,
    /// Replaces fixed sizes by their values.
    Ground,
    /// Checks that nothing collides, for any value of the unknown sizes.
    Check,
}

impl Step {
    /// Every step, in order.
    pub const ALL: [Step; 6] = [
        Step::Parse,
        Step::Resolve,
        Step::Infer,
        Step::Place,
        Step::Ground,
        Step::Check,
    ];

    /// The step's name, in lower case, as the command line takes it.
    pub fn name(self) -> &'static str {
        match self {
            Step::Parse => "parse",
            Step::Resolve => "resolve",
            Step::Infer => "infer",
            Step::Place => "place",
            Step::Ground => "ground",
            Step::Check => "check",
        }
    }

    /// The step with the given name.
    pub fn from_name(name: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|step| step.name() == name)
    }
}

/// What one step produced, as text, and any warnings found on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub warnings: Vec<Diagnostic>,
}

/// Puts problems in the order of the file.
fn sorted(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diagnostics.sort_by(|a, b| (&a.file, a.pos).cmp(&(&b.file, b.pos)));
    diagnostics
}

/// Runs the steps up to and including `step`, and shows its result.
///
/// The steps up to Check show a valid `.yup` file, with more worked out at each step.
///
/// ```
/// use yup_core::{Step, show};
///
/// let source = "yup 0.1\nyears = 1 .. 5\ngrowth = tail years / lag 1 years\n";
/// let shown = show(source, Step::Infer).unwrap();
/// assert_eq!(
///     shown.text,
///     "yup 0.1\nyears : [1, 5]\nyears = 1 .. 5\ngrowth : [1, 4] from [0, 1]\n\
///      growth = tail years / lag 1 years\n"
/// );
/// ```
pub fn show(source: &str, step: Step) -> Result<Shown, Vec<Diagnostic>> {
    let program = parser::parse(source).map_err(sorted)?;
    let shown = |text: String| Shown {
        text,
        warnings: Vec::new(),
    };
    if step == Step::Parse {
        return Ok(shown(printer::print(&program)));
    }
    let model = resolve::resolve(&program).map_err(sorted)?;
    let print = |detail: Detail| printer::print(&elaborate(&program, &model, &detail));
    if step == Step::Resolve {
        return Ok(shown(print(Detail::Resolved)));
    }
    let typed = infer::infer(&program, &model).map_err(sorted)?;
    if step == Step::Infer {
        return Ok(shown(print(Detail::Typed(&typed))));
    }
    let placed = place::place(&program, &model, &typed).map_err(sorted)?;
    if step == Step::Place {
        return Ok(shown(print(Detail::Placed(&typed, &placed))));
    }
    let grounded = ground::ground(&typed, &placed);
    if step == Step::Ground {
        return Ok(shown(print(Detail::Grounded(&typed, &grounded, &[]))));
    }
    let warnings = check::check(&program, &model, &typed, &grounded).map_err(sorted)?;
    Ok(Shown {
        text: print(Detail::Grounded(&typed, &grounded, &warnings)),
        warnings: warnings
            .iter()
            .map(|warning| Diagnostic::warning(warning.pos, warning.message.clone()))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &str = "\
yup 0.1
sheet 'Inputs'
horizon : 'Horizon' Input
horizon = 5
sheet 'Forecast'
heading 'Revenue'
period : 'Period'
period = 1 .. horizon
revenue : 'Revenue'
revenue = 100 * 1.1 ^ period
growth : 'Growth'
growth = tail revenue / lag 1 revenue - 1
";

    const SEGMENTS: &str = "\
yup 0.1
s : Size Fixed
s = 3
base : [s, 1] 'Base' Input
base = [100; 200; 300]
year : 'Year'
year = 1 .. 5
revenue : 'Revenue'
revenue = base * 1.1 ^ year
total : 'Total'
total = sum_columns revenue
";

    fn text(source: &str, step: Step) -> String {
        show(source, step).expect("runs").text
    }

    #[test]
    fn infer_writes_sizes_and_spans_into_the_signatures() {
        assert_eq!(
            text(FORECAST, Step::Infer),
            "\
yup 0.1
sheet 'Inputs'
horizon : [1, 1] 'Horizon' Input
horizon = 5
sheet 'Forecast'
heading 'Revenue'
period_n : Size Unknown
period : [1, period_n] 'Period'
period = 1 .. horizon
revenue : [1, period_n] 'Revenue'
revenue = 100 * 1.1 ^ period
growth : [1, period_n - 1] from [0, 1] 'Growth'
growth = tail revenue / lag 1 revenue - 1
"
        );
    }

    #[test]
    fn place_follows_the_order_of_the_file() {
        assert_eq!(
            text(FORECAST, Step::Place),
            "\
yup 0.1
sheet 'Inputs'
horizon : [1, 1] 'Horizon' @ (2, 4) Input
horizon = 5
sheet 'Forecast'
heading 'Revenue' @ (2, 2)
period_n : Size Unknown
period : [1, period_n] 'Period' @ (3, 4)
period = 1 .. horizon
revenue : [1, period_n] 'Revenue' @ (4, 4)
revenue = 100 * 1.1 ^ period
growth : [1, period_n - 1] from [0, 1] 'Growth' @ (5, 5)
growth = tail revenue / lag 1 revenue - 1
"
        );
    }

    #[test]
    fn ground_replaces_fixed_sizes_by_their_values() {
        let placed = text(SEGMENTS, Step::Place);
        assert!(
            placed.contains("revenue : [s, 5] 'Revenue' @ (s + 3, 4)"),
            "{placed}"
        );
        assert!(
            placed.contains("total : [1, 5] 'Total' @ (2 * s + 3, 4)"),
            "{placed}"
        );
        let grounded = text(SEGMENTS, Step::Ground);
        assert!(
            grounded.contains("revenue : [3, 5] 'Revenue' @ (6, 4)"),
            "{grounded}"
        );
        assert!(
            grounded.contains("total : [1, 5] 'Total' @ (9, 4)"),
            "{grounded}"
        );
    }

    #[test]
    fn showing_a_shown_file_changes_nothing() {
        for source in [FORECAST, SEGMENTS] {
            for step in Step::ALL {
                let once = text(source, step);
                assert_eq!(text(&once, step), once, "{}", step.name());
            }
        }
    }

    #[test]
    fn nothing_goes_below_a_range_of_unknown_height() {
        let source = "yup 0.1\nh : 'H' Input\nh = 3\ndown : 'Down'\ndown = transpose (1 .. \
                      h)\nafter : 'After'\nafter = 1\n";
        let errors = show(source, Step::Place).expect_err("blocked");
        assert_eq!(
            errors[0].to_string(),
            "6:1: error: `after` would go below `down`, whose height, down_n, is unknown when \
             the file is written; move it above `down`, onto another sheet, or pin it with `@`"
        );
    }

    #[test]
    fn check_finds_collisions_and_possible_spills() {
        let clash = "yup 0.1\na : 'A' @ (2, 4)\na = 1\nb : 'B' @ (2, 4)\nb = 2\n";
        let errors: Vec<String> = show(clash, Step::Check)
            .expect_err("collides")
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            errors,
            [
                "4:1: error: the label of `a` and the label of `b` would share the cell (2, 2)",
                "4:1: error: `a` and `b` would share the cell (2, 4)",
            ]
        );
        let spill = "yup 0.1\nh : 'H' Input\nh = 3\nrow : 'Row'\nrow = 1 .. h\nside : 'Side' @ \
                     (3, 9)\nside = 1\n";
        let shown = show(spill, Step::Check).expect("only a warning");
        assert_eq!(
            shown.warnings[0].to_string(),
            "4:1: warning: `row` spills into `side` once row_n grows large enough; the workbook \
             would show #SPILL!"
        );
        assert!(shown.text.contains(
            "row : [1, row_n] 'Row' @ (3, 4) // warning: `row` spills into `side` once row_n \
             grows large enough; the workbook would show #SPILL!"
        ));
        assert_eq!(text(&shown.text, Step::Check), shown.text);
    }

    #[test]
    fn resolve_writes_out_the_default_sheet() {
        assert_eq!(
            text("yup 0.1\nx : 'X'\nx = 1\n", Step::Resolve),
            "yup 0.1\nsheet 'Sheet1'\nx : 'X'\nx = 1\n"
        );
    }

    #[test]
    fn problems_come_in_the_order_of_the_file() {
        let errors = show("yup 0.1\nb = c\na = d\n", Step::Resolve).expect_err("unknown names");
        let lines: Vec<u32> = errors.iter().map(|e| e.pos.line).collect();
        assert_eq!(lines, [2, 3]);
    }
}
