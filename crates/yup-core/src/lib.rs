//! The core of the `.yup` tool: it turns the text of a `.yup` file into the bytes of an xlsx
//! workbook.
//!
//! The core does no input or output, so that the same code runs natively and as WebAssembly. The
//! caller reads the files and writes the result.

mod cells;
mod check;
mod csv;
mod diagnostic;
mod elaborate;
mod functions;
mod ground;
mod infer;
mod lexer;
mod lower;
mod parser;
mod place;
mod printer;
mod resolve;
mod size;
mod style;
mod syntax;
mod xlsx;
mod zip;

pub use diagnostic::{Diagnostic, Pos, Severity};

use std::collections::BTreeMap;

use elaborate::{Detail, elaborate};

/// The contents of the input files a source reads, by the name it gives each, such as
/// `history.csv`. The caller reads them; [`input_files`] says which they are.
pub type Inputs = BTreeMap<String, String>;

/// An input file a source reads, and where it names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFile {
    pub name: String,
    pub pos: Pos,
}

/// The input files a source reads with `csv "file.csv"`, in the order they are named.
///
/// ```
/// let source = "yup 0.2\nhistory : Value _ at main B2\nhistory = csv \"history.csv\"\n";
/// let files = yup_core::input_files(source).unwrap();
/// assert_eq!(files[0].name, "history.csv");
/// ```
pub fn input_files(source: &str) -> Result<Vec<InputFile>, Vec<Diagnostic>> {
    let program = parser::parse(source).map_err(sorted)?;
    let mut files: Vec<InputFile> = Vec::new();
    for item in &program.items {
        let syntax::ItemKind::Definition { expr, .. } = &item.kind else {
            continue;
        };
        if let syntax::ExprKind::Apply { func, args, .. } = &expr.kind
            && func.name == "csv"
            && let [arg] = args.as_slice()
            && let syntax::ExprKind::Str(name) = &arg.kind
            && !files.iter().any(|file| file.name == *name)
        {
            files.push(InputFile {
                name: name.clone(),
                pos: arg.pos,
            });
        }
    }
    Ok(files)
}

/// The version of the `.yup` language this crate reads, as a file's first line gives it.
pub const LANGUAGE_VERSION: &str = parser::VERSION;

/// The steps from a `.yup` file to an xlsx workbook, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// Turns the text into a syntax tree.
    Parse,
    /// Works out what each name refers to, how each binding looks, and the order of the
    /// definitions.
    Resolve,
    /// Works out the size, span and cell type of every expression.
    Infer,
    /// Works out where everything goes, in cells that may depend on fixed sizes. A cell that
    /// depends on a fixed size has no A1 form, so Place shows its file as Infer does.
    Place,
    /// Replaces fixed sizes by their values.
    Ground,
    /// Checks that nothing collides, for any value of the unknown sizes.
    Check,
    /// Turns ranges into cells and expressions into the spreadsheet app's formulas.
    Lower,
    /// Writes the cells into the parts of an xlsx file.
    Package,
}

impl Step {
    /// Every step, in order.
    pub const ALL: [Step; 8] = [
        Step::Parse,
        Step::Resolve,
        Step::Infer,
        Step::Place,
        Step::Ground,
        Step::Check,
        Step::Lower,
        Step::Package,
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
            Step::Lower => "lower",
            Step::Package => "package",
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

/// A workbook, and any warnings found while making it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The bytes of the xlsx file.
    pub xlsx: Vec<u8>,
    pub warnings: Vec<Diagnostic>,
}

/// Runs every step and returns the workbook.
///
/// ```
/// let source = "yup 0.2\nworkbook : Workbook\nworkbook = { sheets = [main] }\n\
///               main : Sheet\nmain = { name = 'Main' }\n\
///               rate : Value Number _ at main D2\nrate = 0.05\n\
///               growth : Formula Number _ rate down\ngrowth = (1 + rate) ^ (1 .. 3)\n";
/// let built = yup_core::build(source, &yup_core::Inputs::new()).unwrap();
/// assert!(built.xlsx.starts_with(b"PK"));
/// assert!(built.warnings.is_empty());
/// ```
pub fn build(source: &str, inputs: &Inputs) -> Result<Built, Vec<Diagnostic>> {
    match run(source, inputs, Step::Package, true)? {
        Outcome::Bytes(xlsx, warnings) => Ok(Built { xlsx, warnings }),
        Outcome::Text(..) => unreachable!("Package gives bytes when asked for them"),
    }
}

/// Runs the steps up to and including `step`, and shows its result.
///
/// The steps up to Check show a valid `.yup` file, with more worked out at each step. Lower
/// shows the cells and defined names, and Package the parts of the file.
///
/// ```
/// use yup_core::{Inputs, Step, show};
///
/// let source = "yup 0.2\nworkbook : Workbook\nworkbook = { sheets = [main] }\n\
///               main : Sheet\nmain = { name = 'Main' }\n\
///               years : Local\nyears = 1 .. 5\ngrowth : Local\ngrowth = tail years / lag 1 years\n";
/// let shown = show(source, Step::Infer, &Inputs::new()).unwrap();
/// assert!(shown.text.ends_with(
///     "years : Local Number [1, 5]\nyears = 1 .. 5\n\
///      growth : Local Number [1, 4] start [0, 1]\ngrowth = tail years / lag 1 years\n"
/// ));
/// ```
pub fn show(source: &str, step: Step, inputs: &Inputs) -> Result<Shown, Vec<Diagnostic>> {
    match run(source, inputs, step, false)? {
        Outcome::Text(text, warnings) => Ok(Shown { text, warnings }),
        Outcome::Bytes(..) => unreachable!("bytes are given only when asked for"),
    }
}

/// What running the steps up to one of them gives.
enum Outcome {
    Text(String, Vec<Diagnostic>),
    Bytes(Vec<u8>, Vec<Diagnostic>),
}

/// Runs the steps up to `last`. Package gives the file's bytes when `bytes` is set, and a
/// listing of its parts otherwise.
fn run(source: &str, inputs: &Inputs, last: Step, bytes: bool) -> Result<Outcome, Vec<Diagnostic>> {
    let program = parser::parse(source).map_err(sorted)?;
    let text = |text: String| Ok(Outcome::Text(text, Vec::new()));
    if last == Step::Parse {
        return text(printer::print(&program));
    }
    let model = resolve::resolve(&program, inputs).map_err(sorted)?;
    let print = |detail: Detail| printer::print(&elaborate(&program, &model, &detail));
    if last == Step::Resolve {
        return text(print(Detail::Resolved));
    }
    let typed = infer::infer(&program, &model).map_err(sorted)?;
    if last == Step::Infer {
        return text(print(Detail::Typed(&typed)));
    }
    let placed = place::place(&program, &model, &typed).map_err(sorted)?;
    let as_diagnostics = |found: &[check::Warning]| -> Vec<Diagnostic> {
        sorted(
            found
                .iter()
                .map(|warning| Diagnostic::warning(warning.pos, warning.message.clone()))
                .collect(),
        )
    };
    if last == Step::Place {
        let text = print(Detail::Typed(&typed));
        return Ok(Outcome::Text(text, as_diagnostics(&placed.warnings)));
    }
    let grounded = ground::ground(&typed, &placed);
    if last == Step::Ground {
        let text = print(Detail::Grounded(&typed, &grounded, &[]));
        return Ok(Outcome::Text(text, as_diagnostics(&placed.warnings)));
    }
    let mut found = placed.warnings.clone();
    found.extend(check::check(&program, &model, &typed, &grounded).map_err(sorted)?);
    let warnings = as_diagnostics(&found);
    if last == Step::Check {
        let text = print(Detail::Grounded(&typed, &grounded, &found));
        return Ok(Outcome::Text(text, warnings));
    }
    let workbook = lower::lower(&program, &model, &typed, &grounded).map_err(sorted)?;
    Ok(match (last, bytes) {
        (Step::Lower, _) => Outcome::Text(lower::listing(&workbook), warnings),
        (_, false) => Outcome::Text(xlsx::listing(&workbook), warnings),
        (_, true) => Outcome::Bytes(xlsx::package(&workbook), warnings),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workbook of one sheet, `main`, for the bindings in `body`, which start on line 6.
    fn file(body: &str) -> String {
        format!(
            "yup 0.2\nworkbook : Workbook\nworkbook = {{ sheets = [main] }}\nmain : Sheet\nmain = \
             {{ name = 'Main' }}\n{body}"
        )
    }

    fn text(body: &str, step: Step) -> String {
        show(&file(body), step, &Inputs::new())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .text
    }

    fn errors(body: &str, step: Step) -> Vec<String> {
        show(&file(body), step, &Inputs::new())
            .expect_err("the source is wrong")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// What `text` shows after the five lines that `file` adds.
    fn body(text: &str) -> String {
        text.lines()
            .skip(5)
            .map(|line| format!("{line}\n"))
            .collect()
    }

    const FORECAST: &str = "\
horizon_label : Label _ at main B2
horizon_label = \"Horizon\"
horizon : Value Number _ horizon_label right 2
horizon = 5
period : Formula _ horizon down 2
period = 1 .. horizon
revenue : Formula _ period down
revenue = 100 * 1.1 ^ period
growth : Formula _ revenue down 1 right 1 from west
growth = tail revenue / lag 1 revenue - 1
";

    #[test]
    fn infer_writes_types_sizes_and_spans_into_the_signatures() {
        assert_eq!(
            body(&text(FORECAST, Step::Infer)),
            "\
horizon_label : Label _ [1, 1] at main B2
horizon_label = \"Horizon\"
horizon : Value Number _ [1, 1] horizon_label right 2
horizon = 5
period_n : Size Unknown
period : Formula Number _ [1, period_n] horizon down 2
period = 1 .. horizon
revenue : Formula Number _ [1, period_n] period down
revenue = 100 * 1.1 ^ period
growth : Formula Number _ [1, period_n - 1] start [0, 1] revenue down 1 right 1 from west
growth = tail revenue / lag 1 revenue - 1
"
        );
    }

    #[test]
    fn ground_writes_every_place_as_a_cell() {
        let grounded = body(&text(FORECAST, Step::Ground));
        for line in [
            "horizon : Value Number _ [1, 1] at main D2",
            "period : Formula Number _ [1, period_n] at main D4",
            "revenue : Formula Number _ [1, period_n] at main D5",
            "growth : Formula Number _ [1, period_n - 1] start [0, 1] at main E6",
        ] {
            assert!(grounded.contains(line), "{line}\n{grounded}");
        }
    }

    #[test]
    fn a_cell_written_with_dollars_keeps_them_and_means_the_same() {
        let source = "a : Value Number _ at main $B$2\na = 1\nb : Value Number _ a down\nb = 2\n";
        assert_eq!(body(&text(source, Step::Parse)), source);
        assert_eq!(
            body(&text(source, Step::Ground)),
            "a : Value Number _ [1, 1] at main $B$2\na = 1\nb : Value Number _ [1, 1] at main \
             B3\nb = 2\n"
        );
        let plain = source.replace("$B$2", "B2");
        let built = |source: &str| {
            build(&file(source), &Inputs::new())
                .unwrap_or_else(|errors| panic!("{errors:?}"))
                .xlsx
        };
        assert!(built(source) == built(&plain));
    }

    #[test]
    fn a_name_keeps_its_case_in_the_workbook() {
        let source = "revenue_Q1 : Value _ at main $B$2\nrevenue_Q1 = 100\ntotal : Formula _ \
                      revenue_Q1 down 1\ntotal = revenue_Q1 * 2\n";
        let listing = text(source, Step::Lower);
        for line in ["  B3     =revenue_Q1*2", "  revenue_Q1 = Main!$B$2"] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn ground_replaces_fixed_sizes_by_their_values() {
        let source = "s : Size Fixed\ns = 3\nbase : Formula _ [s, 1] at main D2\nbase = [100; \
                      200; 300]\nyear : Formula _ base down 1\nyear = 1 .. 5\ntotal : Formula _ \
                      year down 1\ntotal = sum_columns (base * 1.1 ^ year)\n";
        let grounded = text(source, Step::Ground);
        assert!(
            grounded.contains("year : Formula Number _ [1, 5] at main D5"),
            "{grounded}"
        );
        assert!(
            grounded.contains("base : Formula Number _ [3, 1] at main D2"),
            "{grounded}"
        );
    }

    #[test]
    fn lower_writes_formulas_values_labels_and_names() {
        assert_eq!(
            text(FORECAST, Step::Lower),
            "\
sheet 'Main'
  B2     \"Horizon\"
  D2     5
  D4     =_xlfn.SEQUENCE(1,horizon)
  D5     =100*1.1^period
  E6     =_xlfn.DROP(revenue,,1)/_xlfn.DROP(revenue,,-1)-1
names
  growth = _xlfn.ANCHORARRAY(Main!$E$6)
  horizon = Main!$D$2
  period = _xlfn.ANCHORARRAY(Main!$D$4)
  revenue = _xlfn.ANCHORARRAY(Main!$D$5)
"
        );
    }

    #[test]
    fn styles_cascade_their_font_from_the_sheet_and_the_workbook() {
        let source = "\
yup 0.2
workbook : Workbook
workbook = { sheets = [main], style = narrow }
main : Sheet
main = { name = 'Main', style = big, color = red }
red : Style Color
red = { red = 255 }
narrow : Style Cell
narrow = { font = { name = 'Arial Narrow' } }
big : Style Cell
big = { font = { size = 14 } }
plain : Style Cell
plain = { fill = red, border = { bottom = { weight = Double, color = red } } }
a : Label plain at main B2
a = \"A\"
b : Formula _ a right 2
b = 1 .. 3
";
        let listing = show(source, Step::Lower, &Inputs::new())
            .expect("lowers")
            .text;
        for line in [
            "sheet 'Main'  [tab FF0000]",
            "  B2     \"A\"  [font 'Arial Narrow', size 14, fill FF0000, bottom double FF0000]",
            "  D2     =_xlfn.SEQUENCE(1,3)  [font 'Arial Narrow', size 14]",
            "  E2     _  [font 'Arial Narrow', size 14]",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn a_lambda_becomes_a_lambda_and_takes_the_shape_of_its_arguments() {
        let source = "rate : Value Number _ at main D2\nrate = 0.1\ndiscount : Lambda Number -> \
                      Number -> Number\ndiscount rate t = 1 / (1 + rate) ^ t\nfactors : Formula \
                      _ rate down\nfactors = discount rate (1 .. 3)\none : Formula _ factors \
                      down\none = discount 0.2 1\n";
        let typed = text(source, Step::Infer);
        assert!(
            typed.contains("factors : Formula Number _ [1, 3] rate down"),
            "{typed}"
        );
        let listing = text(source, Step::Lower);
        for line in [
            "  D3     =discount(rate,_xlfn.SEQUENCE(1,3))",
            "  D4     =discount(0.2,1)",
            "  discount = _xlfn.LAMBDA(_xlpm.rate,_xlpm.t,1/(1+_xlpm.rate)^_xlpm.t)",
            "  one = Main!$D$4",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn an_error_in_a_lambda_names_the_call() {
        let source = "shift : Lambda Number -> Number\nshift x = tail across x / init across \
                      x\ny : Local\ny = shift (1 .. 3)\n";
        assert_eq!(
            errors(source, Step::Infer),
            [
                "7:25: error: the two sides of `/` are misaligned along the columns: one covers \
                 positions [1, 3) and the other [0, 2); `lag` shifts a range along, and `tail` \
                 and `init` drop its ends, in `shift` as it is called at 9:5"
            ]
        );
    }

    const LOAN: &str = "\
due : Formula _ at main D2
due = [30, 30, 30, 30]
opening : Formula _ due down
opening = lag 1 closing seed 100
paid : Formula _ opening down
paid = min due opening
closing : Formula _ paid down
closing = opening - paid
";

    #[test]
    fn a_recurrence_becomes_a_scan_with_the_cycle_written_in_place() {
        let typed = text(LOAN, Step::Infer);
        assert!(
            typed.contains("closing : Formula Number _ [1, 4]"),
            "{typed}"
        );
        let listing = text(LOAN, Step::Lower);
        for line in [
            "  D3     =_xlfn.HSTACK(100,_xlfn.DROP(closing,,-1))",
            "  D4     =IF(due<opening,due,opening)",
            "  D5     =_xlfn.SCAN(100,_xlfn.SEQUENCE(1,COLUMNS(due)),_xlfn.LAMBDA(_xlpm._value,\
             _xlpm._position,_xlpm._value-_xlfn.LET(_xlpm._a,INDEX(due,1,_xlpm._position),\
             _xlpm._b,_xlpm._value,IF(_xlpm._a<_xlpm._b,_xlpm._a,_xlpm._b))))",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn a_recurrence_can_run_down_a_column() {
        let source = "due : Formula _ at main D2\ndue = [30; 30; 30]\nopening : Formula _ due \
                      right\nopening = lag 1 closing seed 100\nclosing : Formula _ opening \
                      right\nclosing = opening - due\n";
        let listing = text(source, Step::Lower);
        for line in [
            "  E2     =_xlfn.VSTACK(100,_xlfn.DROP(closing,-1))",
            "  F2     =_xlfn.SCAN(100,_xlfn.SEQUENCE(ROWS(due)),_xlfn.LAMBDA(_xlpm._value,\
             _xlpm._position,_xlpm._value-INDEX(due,_xlpm._position,1)))",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn a_recurrence_needs_a_line_outside_it_to_give_its_length() {
        assert_eq!(
            errors(
                "opening : Local\nopening = lag 1 closing seed 100\nclosing : Local\nclosing = \
                 opening - 1\n",
                Step::Infer
            ),
            [
                "9:1: error: nothing outside the cycle gives `closing` its length: it needs a row \
                 or a column it uses position by position, such as a timeline"
            ]
        );
    }

    const SALES: &str = "\
sales : Value _ at main D2
sales = csv \"sales.csv\"
latest : Formula _ sales down 3
latest = lookup (maximum (column \"year\" sales)) (column \"year\" sales) (column \"units\" sales)
";

    fn sales_inputs(csv: &str) -> Inputs {
        Inputs::from([("sales.csv".to_string(), csv.to_string())])
    }

    #[test]
    fn a_table_is_read_from_its_file_and_written_as_its_values() {
        let inputs = sales_inputs("year,units\n2024,10\n2025,12\n");
        assert_eq!(
            input_files(&file(SALES)).expect("parses"),
            [InputFile {
                name: "sales.csv".to_string(),
                pos: Pos::new(7, 13)
            }]
        );
        let listing = show(&file(SALES), Step::Lower, &inputs)
            .expect("lowers")
            .text;
        for line in [
            "  D2     2024",
            "  E3     12",
            "  D6     =_xlfn.XLOOKUP(MAX(INDEX(sales,0,1)),INDEX(sales,0,1),INDEX(sales,0,2))",
            "  sales = Main!$D$2:$E$3",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
        let typed = show(&file(SALES), Step::Infer, &inputs)
            .expect("infers")
            .text;
        assert!(
            typed.contains("latest : Formula Number _ [1, 1] sales down 3"),
            "{typed}"
        );
    }

    #[test]
    fn mistakes_with_tables_are_reported_where_they_are() {
        let missing = show(&file(SALES), Step::Resolve, &Inputs::new()).expect_err("no file");
        assert_eq!(
            missing[0].to_string(),
            "7:13: error: the input file `sales.csv` was not given"
        );
        let short = show(
            &file(SALES),
            Step::Resolve,
            &sales_inputs("year,units\n2024\n"),
        )
        .expect_err("a short row");
        assert_eq!(
            short[0].to_string(),
            "sales.csv:2:1: error: this row has 1 field, and the header has 2 fields"
        );
    }

    #[test]
    fn logical_operators_work_position_by_position() {
        let source = "years : Local\nyears = 1 .. 4\nmiddle : Formula _ at main D2\nmiddle = \
                      years > 1 && years < 4 || not (years /= 2)\n";
        let listing = text(source, Step::Lower);
        assert!(
            listing.contains(
                "=_xlfn.LET(_xlpm.years,_xlfn.SEQUENCE(1,4),((_xlpm.years>1)*(_xlpm.years<4)<>0)\
                 +NOT(_xlpm.years<>2)<>0)"
            ),
            "{listing}"
        );
    }

    #[test]
    fn a_formula_binds_the_locals_it_uses_with_let() {
        let source = "rate : Value Number _ at main D2\nrate = 0.1\nfactor : Local\nfactor = 1 + \
                      rate\nsquared : Local\nsquared = factor * factor\nout : Formula _ rate \
                      down\nout = squared - factor\nf : Lambda Number -> Number\nf x = x * \
                      factor\ng : Lambda Number -> Number\ng factor = factor + squared\nuse : \
                      Formula _ out down\nuse = f 1 + g 2\n";
        let listing = text(source, Step::Lower);
        for line in [
            "  D3     =_xlfn.LET(_xlpm.factor,1+rate,_xlpm.squared,_xlpm.factor*_xlpm.factor,\
             _xlpm.squared-_xlpm.factor)",
            "  f = _xlfn.LAMBDA(_xlpm.x,_xlfn.LET(_xlpm.factor,1+rate,_xlpm.x*_xlpm.factor))",
            // A local named as a parameter is written in place, so that no LET hides the parameter.
            "  g = _xlfn.LAMBDA(_xlpm.factor,_xlpm.factor+(1+rate)*(1+rate))",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn a_column_of_whole_numbers_is_written_as_one() {
        let source = "col : Formula _ at main D2\ncol = transpose (2 .. 4)\nfirst : Formula _ \
                      col right\nfirst = init col\n";
        let listing = text(source, Step::Lower);
        for line in [
            "  D2     =_xlfn.SEQUENCE(3,1,2)",
            "  E2     =_xlfn.DROP(col,-1)",
        ] {
            assert!(listing.contains(line), "{line}\n{listing}");
        }
    }

    #[test]
    fn build_gives_an_xlsx_with_every_part() {
        let built = build(&file(FORECAST), &Inputs::new()).expect("builds");
        let names: Vec<String> = zip::read(&built.xlsx)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
                "xl/styles.xml",
                "xl/sharedStrings.xml",
                "xl/metadata.xml",
                "xl/worksheets/sheet1.xml",
            ]
        );
        assert_eq!(
            build(&file(FORECAST), &Inputs::new()).expect("builds again"),
            built,
            "the same bytes"
        );
    }

    #[test]
    fn showing_a_shown_file_changes_nothing() {
        for source in [file(FORECAST), file(LOAN)] {
            for step in Step::ALL.into_iter().filter(|&step| step <= Step::Check) {
                let once = show(&source, step, &Inputs::new()).expect("shows").text;
                let twice = show(&once, step, &Inputs::new()).expect("shows again").text;
                assert_eq!(twice, once, "{}", step.name());
            }
        }
    }

    #[test]
    fn nothing_is_counted_from_the_far_side_of_an_unknown_extent() {
        let source = "h : Value Number _ at main D2\nh = 3\nrow : Formula _ h down\nrow = 1 .. \
                      h\nafter : Formula _ row right\nafter = 1\n";
        assert_eq!(
            errors(source, Step::Place),
            [
                "10:23: error: `after` counts from the east side of `row`, whose width, row_n, \
                 is unknown when the file is written; count from its west side, with `from \
                 west`, or place `after` elsewhere"
            ]
        );
    }

    #[test]
    fn a_placement_that_does_not_line_up_is_a_warning() {
        let source =
            format!("{FORECAST}late : Formula _ growth down\nlate = tail (tail revenue)\n");
        let shown = show(&file(&source), Step::Check, &Inputs::new()).expect("only a warning");
        assert_eq!(
            shown
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "16:1: warning: `late` does not line up with `growth`: the two share period_n \
                 along their columns, but position 2 of `late` is in column E and of `growth` in \
                 column F"
            ]
        );
    }

    #[test]
    fn check_finds_collisions_and_possible_spills() {
        let clash = "a : Value _ at main D2\na = 1\nb : Value _ a right 0\nb = 2\n";
        assert_eq!(
            errors(clash, Step::Check),
            ["8:1: error: `a` and `b` would share the cell D2"]
        );
        let spill = "h : Value Number _ at main D2\nh = 3\nrow : Formula _ h down\nrow = 1 .. \
                     h\nside : Value _ at main I3\nside = 1\n";
        let shown = show(&file(spill), Step::Check, &Inputs::new()).expect("only a warning");
        assert_eq!(
            shown.warnings[0].to_string(),
            "8:1: warning: `row` spills into `side` once row_n grows large enough; the workbook \
             would show #SPILL!"
        );
        assert!(shown.text.contains(
            "row : Formula Number _ [1, row_n] at main D3 // warning: `row` spills into `side` \
             once row_n grows large enough; the workbook would show #SPILL!"
        ));
        assert_eq!(
            show(&shown.text, Step::Check, &Inputs::new())
                .expect("shows again")
                .text,
            shown.text
        );
    }

    #[test]
    fn problems_come_in_the_order_of_the_file() {
        assert_eq!(
            errors("b : Local\nb = c\na : Local\na = d\n", Step::Resolve),
            [
                "7:5: error: unknown name `c`",
                "9:5: error: unknown name `d`"
            ]
        );
    }
}
