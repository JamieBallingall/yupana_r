//! Runs the built `yup` binary as a user would: through its arguments and standard streams.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn yup(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yup"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the yup binary is built by cargo test");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("the child reads all of stdin");
    child
        .wait_with_output()
        .expect("the child runs to completion")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

const SOURCE: &str = "yup 0.1\nrate : 'Rate' Input\nrate = 0.05\nyears = 1 .. 3\ngrowth : \
                      'Growth'\ngrowth = (1 + rate) ^ years\n";

#[test]
fn show_prints_what_a_step_makes_of_the_file() {
    let output = yup(&["show", "place", "-"], SOURCE.as_bytes());
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "yup 0.1\nsheet 'Sheet1'\nrate : [1, 1] 'Rate' @ (2, 4) Input\nrate = 0.05\nyears : [1, \
         3]\nyears = 1 .. 3\ngrowth : [1, 3] 'Growth' @ (3, 4)\ngrowth = (1 + rate) ^ years\n"
    );
}

#[test]
fn build_writes_a_workbook_to_standard_output() {
    let output = yup(&["build", "-", "-o", "-"], SOURCE.as_bytes());
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(output.stdout.starts_with(b"PK\x03\x04"));
}

#[test]
fn build_writes_a_workbook_file() {
    let path: PathBuf = [
        env!("CARGO_TARGET_TMPDIR"),
        "build_writes_a_workbook_file.xlsx",
    ]
    .iter()
    .collect();
    let path = path.to_string_lossy().into_owned();
    let output = yup(&["build", "-", "-o", &path], SOURCE.as_bytes());
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), format!("yup: wrote {path}\n"));
    let bytes = std::fs::read(&path).expect("the workbook is written");
    assert!(bytes.starts_with(b"PK\x03\x04"));
}

#[test]
fn problems_are_reported_with_file_line_and_column() {
    let output = yup(&["check", "-"], b"yup 0.1\nx = y + 1\nz = 1 ++\n");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "<stdin>:3:9: error: expected a value, found the end of the line\n"
    );
    let output = yup(&["check", "-"], b"yup 0.1\nx = y + 1\n");
    assert_eq!(
        text(&output.stderr),
        "<stdin>:2:5: error: unknown name `y`\n"
    );
}

#[test]
fn a_file_that_is_not_utf8_is_refused_where_it_goes_wrong() {
    let output = yup(&["check", "-"], b"yup 0.1\nx = '\xff'\n");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "<stdin>:2:6: error: the file is not valid UTF-8 here\n"
    );
}

#[test]
fn a_wrong_command_line_prints_the_usage() {
    let output = yup(&["show", "nowhere", "-"], b"");
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).starts_with(
        "yup: unknown step `nowhere`; the steps are parse, resolve, infer, place, ground, check, \
         lower, package\n\nusage:"
    ));
}

#[test]
fn build_reads_input_files_beside_the_source() {
    let source: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "examples",
        "history.yup",
    ]
    .iter()
    .collect();
    let path: PathBuf = [env!("CARGO_TARGET_TMPDIR"), "history.xlsx"]
        .iter()
        .collect();
    let source = source.to_string_lossy().into_owned();
    let path = path.to_string_lossy().into_owned();
    let output = yup(&["build", &source, "-o", &path], b"");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let output = yup(
        &["check", "-"],
        b"yup 0.1\nt : 'T' Input\nt = csv \"nowhere.csv\"\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).starts_with("<stdin>:3:9: error: cannot read nowhere.csv: "),
        "{}",
        text(&output.stderr)
    );
}
