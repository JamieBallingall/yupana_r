//! The core of the `.yup` tool: it turns the text of a `.yup` file into the bytes of an xlsx
//! workbook.
//!
//! The core does no input or output, so that the same code runs natively and as WebAssembly. The
//! caller reads the files and writes the result.

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
