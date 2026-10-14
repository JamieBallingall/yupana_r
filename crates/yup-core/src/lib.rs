//! The core of the `.yup` tool: it turns the text of a `.yup` file into the bytes of an xlsx
//! workbook.
//!
//! The core does no input or output, so that the same code runs natively and as WebAssembly. The
//! caller reads the files and writes the result.

mod diagnostic;
mod lexer;
mod parser;
mod printer;
mod syntax;

pub use diagnostic::{Diagnostic, Pos, Severity};

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
}

impl Step {
    /// Every step, in order.
    pub const ALL: [Step; 1] = [Step::Parse];

    /// The step's name, in lower case, as the command line takes it.
    pub fn name(self) -> &'static str {
        match self {
            Step::Parse => "parse",
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

/// Runs the steps up to and including `step`, and shows its result.
///
/// The steps up to Check show a valid `.yup` file, with more worked out at each step.
///
/// ```
/// use yup_core::{Step, show};
///
/// let shown = show("yup 0.1\nx=(1+2)*3\n", Step::Parse).unwrap();
/// assert_eq!(shown.text, "yup 0.1\nx = (1 + 2) * 3\n");
/// ```
pub fn show(source: &str, step: Step) -> Result<Shown, Vec<Diagnostic>> {
    let program = parser::parse(source)?;
    let text = match step {
        Step::Parse => printer::print(&program),
    };
    Ok(Shown {
        text,
        warnings: Vec::new(),
    })
}
