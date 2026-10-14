//! The `yup` command: reads a `.yup` source on standard input and writes the result to standard
//! output.

use std::io::{self, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("yup: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let mut source = Vec::new();
    io::stdin().read_to_end(&mut source)?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(&yup_core::compile(&source))?;
    stdout.flush()
}
