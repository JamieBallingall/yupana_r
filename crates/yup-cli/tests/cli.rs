//! Runs the built `yup` binary as a user would, through its standard streams.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_yup(input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yup"))
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

#[test]
fn any_input_gives_the_placeholder() {
    for input in [&b""[..], b"Revenue = 100\n", &[0xff, 0x00, 0xfe]] {
        let output = run_yup(input);
        assert!(output.status.success());
        assert_eq!(output.stdout, yup_core::NOT_YET_IMPLEMENTED.as_bytes());
        assert!(output.stderr.is_empty());
    }
}
