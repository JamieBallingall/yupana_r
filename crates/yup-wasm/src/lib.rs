//! The core as a WebAssembly module with no imports, for JavaScript to call.
//!
//! JavaScript asks [`request`] for a buffer of the length it needs, writes its request there,
//! calls [`run`], and reads the response, [`run`]'s length of bytes from [`response`]. The module
//! never reads through a pointer that JavaScript gives it, so it needs no unsafe code beyond the
//! attributes that export these three functions.
//!
//! Both are byte layouts, written and read here and in `js/template.mjs`. A number is a
//! little-endian `u32`, a text is its length in bytes and then its UTF-8, bytes are their length
//! and then themselves, and a list is its length and then its items.
//!
//! A request is the command, one byte: 0 for the input files, 1 for show, 2 for build; the
//! step, one byte, its index in [`Step::ALL`], which only show reads; the source, a text; and
//! the input files, a list of each one's name and contents, both texts.
//!
//! A response is a status, one byte, and what follows it:
//!
//! - 0, done: for the input files, a list of each one's name, a text, and the line and column
//!   that name it; for show, the text shown and the warnings; for build, the xlsx file, as
//!   bytes, and the warnings.
//! - 1, problems: the diagnostics.
//! - 2, a request that cannot be read: a text that says why.
//!
//! The warnings and diagnostics are each a list of diagnostics. A diagnostic is its severity,
//! one byte, 0 for an error and 1 for a warning; whether it names an input file, one byte, and
//! then the file's name, a text, if it does; its line; its column; and its message, a text.

use std::cell::RefCell;

use yup_core::{Diagnostic, Inputs, Severity, Step};

thread_local! {
    static REQUEST: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static RESPONSE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// A buffer of `len` bytes for the next request, which JavaScript writes into.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn request(len: u32) -> *mut u8 {
    REQUEST.with_borrow_mut(|request| {
        *request = vec![0; usize::try_from(len).expect("a length fits in memory")];
        request.as_mut_ptr()
    })
}

/// Runs the request, keeps the response for [`response`], and returns its length.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn run() -> u32 {
    let response = REQUEST.with_borrow(|request| respond(request));
    let len = number(response.len());
    RESPONSE.set(response);
    len
}

/// Where the last response is.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn response() -> *const u8 {
    RESPONSE.with_borrow(|response| response.as_ptr())
}

/// The response to a request, both as the module documentation lays them out.
pub fn respond(request: &[u8]) -> Vec<u8> {
    let mut out = Writer::default();
    match read_request(request) {
        Err(message) => {
            out.byte(2);
            out.text(&message);
        }
        Ok((command, step, source, inputs)) => match command {
            0 => match yup_core::input_files(&source) {
                Ok(files) => {
                    out.byte(0);
                    out.number(files.len());
                    for file in &files {
                        out.text(&file.name);
                        out.number(file.pos.line);
                        out.number(file.pos.column);
                    }
                }
                Err(errors) => out.problems(&errors),
            },
            1 => match yup_core::show(&source, step, &inputs) {
                Ok(shown) => {
                    out.byte(0);
                    out.text(&shown.text);
                    out.diagnostics(&shown.warnings);
                }
                Err(errors) => out.problems(&errors),
            },
            2 => match yup_core::build(&source, &inputs) {
                Ok(built) => {
                    out.byte(0);
                    out.bytes(&built.xlsx);
                    out.diagnostics(&built.warnings);
                }
                Err(errors) => out.problems(&errors),
            },
            other => {
                out.byte(2);
                out.text(&format!("unknown command {other}"));
            }
        },
    }
    out.0
}

/// The command, the step, the source and the input files of a request.
fn read_request(request: &[u8]) -> Result<(u8, Step, String, Inputs), String> {
    let mut reader = Reader { bytes: request };
    let command = reader.byte()?;
    let index = reader.byte()?;
    let step = *Step::ALL
        .get(usize::from(index))
        .ok_or_else(|| format!("there is no step {index}"))?;
    let source = reader.text()?;
    let mut inputs = Inputs::new();
    for _ in 0..reader.number()? {
        let name = reader.text()?;
        let contents = reader.text()?;
        inputs.insert(name, contents);
    }
    if !reader.bytes.is_empty() {
        return Err("the request runs on past its end".to_string());
    }
    Ok((command, step, source, inputs))
}

/// A number as the layout writes it. Everything the module holds fits in 32 bits, since its
/// memory does.
fn number(value: usize) -> u32 {
    u32::try_from(value).expect("a length fits in 32 bits")
}

/// Reads a request from its start.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        if self.bytes.len() < count {
            return Err("the request ends early".to_string());
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn number(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn text(&mut self) -> Result<String, String> {
        let len = usize::try_from(self.number()?).expect("a length fits in memory");
        let bytes = self.take(len)?.to_vec();
        String::from_utf8(bytes).map_err(|_| "a text is not valid UTF-8".to_string())
    }
}

/// Writes a response.
#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn byte(&mut self, value: u8) {
        self.0.push(value);
    }

    fn number(&mut self, value: impl TryInto<u32>) {
        let value: u32 = value
            .try_into()
            .unwrap_or_else(|_| panic!("a number fits in 32 bits"));
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.number(value.len());
        self.0.extend_from_slice(value);
    }

    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn diagnostics(&mut self, diagnostics: &[Diagnostic]) {
        self.number(diagnostics.len());
        for diagnostic in diagnostics {
            self.byte(match diagnostic.severity {
                Severity::Error => 0,
                Severity::Warning => 1,
            });
            match &diagnostic.file {
                Some(file) => {
                    self.byte(1);
                    self.text(file);
                }
                None => self.byte(0),
            }
            self.number(diagnostic.pos.line);
            self.number(diagnostic.pos.column);
            self.text(&diagnostic.message);
        }
    }

    fn problems(&mut self, diagnostics: &[Diagnostic]) {
        self.byte(1);
        self.diagnostics(diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request as JavaScript writes one.
    fn request_for(command: u8, step: Step, source: &str, inputs: &[(&str, &str)]) -> Vec<u8> {
        let index = Step::ALL.iter().position(|&s| s == step).expect("a step");
        let mut out = Writer::default();
        out.byte(command);
        out.byte(u8::try_from(index).expect("a step's index fits in a byte"));
        out.text(source);
        out.number(inputs.len());
        for (name, contents) in inputs {
            out.text(name);
            out.text(contents);
        }
        out.0
    }

    fn file(body: &str) -> String {
        format!(
            "yup 0.2\nworkbook : Workbook\nworkbook = {{ sheets = [main] }}\nmain : Sheet\nmain = \
             {{ name = 'Main' }}\n{body}"
        )
    }

    /// Reads a response back with the request reader, which reads the same layout.
    fn reader(response: &[u8]) -> Reader<'_> {
        Reader { bytes: response }
    }

    #[test]
    fn the_input_files_are_named_with_where() {
        let source = file("history : Value at main $B$2\nhistory = csv \"history.csv\"\n");
        let response = respond(&request_for(0, Step::Parse, &source, &[]));
        let mut out = reader(&response);
        assert_eq!(out.byte(), Ok(0));
        assert_eq!(out.number(), Ok(1));
        assert_eq!(out.text().as_deref(), Ok("history.csv"));
        assert_eq!((out.number(), out.number()), (Ok(7), Ok(15)));
        assert!(out.bytes.is_empty());
    }

    #[test]
    fn show_gives_the_text_and_build_the_workbook() {
        let source = file("rate : Value at main $B$2\nrate = 0.05\n");
        let response = respond(&request_for(1, Step::Ground, &source, &[]));
        let mut out = reader(&response);
        assert_eq!(out.byte(), Ok(0));
        let text = out.text().expect("a text");
        assert!(
            text.contains("rate : Value Number size [1, 1] at main $B$2"),
            "{text}"
        );
        assert_eq!(out.number(), Ok(0));
        let response = respond(&request_for(2, Step::Parse, &source, &[]));
        let mut out = reader(&response);
        assert_eq!(out.byte(), Ok(0));
        let len = usize::try_from(out.number().expect("a length")).expect("a length");
        let xlsx = out.take(len).expect("the workbook").to_vec();
        assert_eq!(
            xlsx,
            yup_core::build(&source, &Inputs::new())
                .expect("builds")
                .xlsx
        );
        assert_eq!(out.number(), Ok(0));
    }

    #[test]
    fn problems_come_with_their_place_and_inputs_are_read() {
        let source = file("t : Value at main $B$2\nt = csv \"t.csv\"\n");
        let response = respond(&request_for(
            2,
            Step::Parse,
            &source,
            &[("t.csv", "a\n\"x\n")],
        ));
        let mut out = reader(&response);
        assert_eq!(out.byte(), Ok(1));
        assert_eq!(out.number(), Ok(1));
        assert_eq!(out.byte(), Ok(0), "an error");
        assert_eq!(out.byte(), Ok(1), "in an input file");
        assert_eq!(out.text().as_deref(), Ok("t.csv"));
        assert_eq!((out.number(), out.number()), (Ok(2), Ok(1)));
        assert!(out.text().is_ok_and(|message| !message.is_empty()));
    }

    #[test]
    fn a_request_that_cannot_be_read_says_why() {
        let mut request = request_for(1, Step::Parse, "yup 0.2\n", &[]);
        request.pop();
        let response = respond(&request);
        let mut out = reader(&response);
        assert_eq!(out.byte(), Ok(2));
        assert_eq!(out.text().as_deref(), Ok("the request ends early"));
        let response = respond(&[1, 99]);
        let mut out = reader(&response);
        assert_eq!(out.byte(), Ok(2));
        assert_eq!(out.text().as_deref(), Ok("there is no step 99"));
    }

    #[test]
    fn the_exports_keep_the_request_and_the_response() {
        let source = "yup 0.2\n".as_bytes();
        let mut request_bytes = vec![0, 0];
        request_bytes.extend_from_slice(&number(source.len()).to_le_bytes());
        request_bytes.extend_from_slice(source);
        request_bytes.extend_from_slice(&0_u32.to_le_bytes());
        request(number(request_bytes.len()));
        REQUEST.with_borrow_mut(|buffer| buffer.copy_from_slice(&request_bytes));
        let len = run();
        let expected = respond(&request_bytes);
        assert_eq!(len, number(expected.len()));
        assert!(!response().is_null());
        RESPONSE.with_borrow(|response| assert_eq!(*response, expected));
    }
}
