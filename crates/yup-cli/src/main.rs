//! The `yup` command: builds an xlsx workbook from a `.yup` file, checks a file, or shows what
//! any one step makes of it.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yup_core::{Diagnostic, Inputs, Pos, Step};

const USAGE: &str = "\
usage: yup build <file.yup> [-o <file.xlsx>]
       yup check <file.yup>
       yup show <step> <file.yup>

build   runs every step and writes the workbook, by default beside the .yup file
check   runs the steps up to Check and reports any problems
show    prints what one step makes of the file: parse, resolve, infer, place, ground
        and check print a .yup file, lower prints the cells, package the file's parts

A file named - is standard input, and -o - writes the workbook to standard output.";

enum Command {
    Build {
        input: String,
        output: Option<String>,
    },
    Check {
        input: String,
    },
    Show {
        step: Step,
        input: String,
    },
    Help,
    Version,
}

/// Why the command stopped early.
enum Failure {
    /// The command line was wrong.
    Usage(String),
    /// Something outside the `.yup` file went wrong, such as a file that cannot be read.
    Io(String),
    /// The problems have been reported already.
    Reported,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("yup: {message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Io(message)) => {
            eprintln!("yup: {message}");
            ExitCode::FAILURE
        }
        Err(Failure::Reported) => ExitCode::FAILURE,
    }
}

fn parse_args(args: &[String]) -> Result<Command, Failure> {
    let usage = |message: &str| Failure::Usage(message.to_string());
    let Some((command, rest)) = args.split_first() else {
        return Err(usage("no command given"));
    };
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    match command.as_str() {
        "build" => {
            let mut input = None;
            let mut output = None;
            let mut words = rest.into_iter();
            while let Some(word) = words.next() {
                match word {
                    "-o" | "--output" => {
                        let file = words.next().ok_or_else(|| usage("-o needs a file name"))?;
                        output = Some(file.to_string());
                    }
                    _ if input.is_none() => input = Some(word.to_string()),
                    _ => return Err(Failure::Usage(format!("unexpected argument `{word}`"))),
                }
            }
            let input = input.ok_or_else(|| usage("build needs a .yup file"))?;
            Ok(Command::Build { input, output })
        }
        "check" => match rest.as_slice() {
            [input] => Ok(Command::Check {
                input: input.to_string(),
            }),
            _ => Err(usage("check takes one .yup file")),
        },
        "show" => match rest.as_slice() {
            [step, input] => {
                let step = Step::from_name(step).ok_or_else(|| {
                    let names: Vec<&str> = Step::ALL.iter().map(|step| step.name()).collect();
                    Failure::Usage(format!(
                        "unknown step `{step}`; the steps are {}",
                        names.join(", ")
                    ))
                })?;
                Ok(Command::Show {
                    step,
                    input: input.to_string(),
                })
            }
            _ => Err(usage("show takes a step and a .yup file")),
        },
        "help" | "-h" | "--help" => Ok(Command::Help),
        "version" | "-V" | "--version" => Ok(Command::Version),
        other => Err(Failure::Usage(format!("unknown command `{other}`"))),
    }
}

fn run(command: Command) -> Result<(), Failure> {
    match command {
        Command::Help => println!("{USAGE}"),
        Command::Version => println!(
            "yup {}, reading .yup {}",
            env!("CARGO_PKG_VERSION"),
            yup_core::LANGUAGE_VERSION
        ),
        Command::Build { input, output } => {
            let output = match output {
                Some(output) => output,
                None if input == "-" => {
                    return Err(Failure::Usage(
                        "reading standard input, build needs -o".to_string(),
                    ));
                }
                None => Path::new(&input)
                    .with_extension("xlsx")
                    .to_string_lossy()
                    .into_owned(),
            };
            let source = read_source(&input)?;
            let inputs = read_inputs(&input, &source)?;
            let built =
                yup_core::build(&source, &inputs).map_err(|errors| report(&input, &errors))?;
            report(&input, &built.warnings);
            write_output(&output, &built.xlsx)?;
            if output != "-" {
                eprintln!("yup: wrote {output}");
            }
        }
        Command::Check { input } => show(&input, Step::Check, false)?,
        Command::Show { step, input } => show(&input, step, true)?,
    }
    Ok(())
}

/// Runs the steps up to `step`, reports any warnings, and prints the result if asked to.
fn show(input: &str, step: Step, print: bool) -> Result<(), Failure> {
    let source = read_source(input)?;
    let inputs = read_inputs(input, &source)?;
    let shown = yup_core::show(&source, step, &inputs).map_err(|errors| report(input, &errors))?;
    report(input, &shown.warnings);
    if print {
        print!("{}", shown.text);
    }
    Ok(())
}

/// Reads the input files the source names, from the folder the source is in.
fn read_inputs(input: &str, source: &str) -> Result<Inputs, Failure> {
    // A source that does not parse is reported by the steps that follow.
    let Ok(files) = yup_core::input_files(source) else {
        return Ok(Inputs::new());
    };
    let folder = match Path::new(input).parent() {
        Some(folder) if input != "-" => folder.to_path_buf(),
        _ => PathBuf::new(),
    };
    let mut inputs = Inputs::new();
    let mut problems = Vec::new();
    for file in files {
        let path = folder.join(&file.name);
        let read = fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
            .and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| format!("{} is not valid UTF-8", path.display()))
            });
        match read {
            Ok(text) => {
                inputs.insert(file.name, text);
            }
            Err(message) => problems.push(Diagnostic::error(file.pos, message)),
        }
    }
    if problems.is_empty() {
        Ok(inputs)
    } else {
        Err(report(input, &problems))
    }
}

/// Prints each problem after the name of the file it is in, and returns the failure that says
/// so.
fn report(input: &str, diagnostics: &[Diagnostic]) -> Failure {
    let name = display_name(input);
    for diagnostic in diagnostics {
        if diagnostic.file.is_some() {
            eprintln!("{diagnostic}");
        } else {
            eprintln!("{name}:{diagnostic}");
        }
    }
    Failure::Reported
}

fn display_name(input: &str) -> &str {
    if input == "-" { "<stdin>" } else { input }
}

fn read_source(input: &str) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    let read = if input == "-" {
        io::stdin().read_to_end(&mut bytes).map(|_| ())
    } else {
        fs::read(input).map(|contents| bytes = contents)
    };
    read.map_err(|error| Failure::Io(format!("cannot read {}: {error}", display_name(input))))?;
    String::from_utf8(bytes).map_err(|error| {
        let valid = &error.as_bytes()[..error.utf8_error().valid_up_to()];
        let text = String::from_utf8_lossy(valid);
        let line = text.matches('\n').count() + 1;
        let column = text
            .rsplit('\n')
            .next()
            .map_or(0, |last| last.chars().count())
            + 1;
        let pos = Pos::new(
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(column).unwrap_or(u32::MAX),
        );
        report(
            input,
            &[Diagnostic::error(pos, "the file is not valid UTF-8 here")],
        )
    })
}

fn write_output(output: &str, bytes: &[u8]) -> Result<(), Failure> {
    let written = if output == "-" {
        let mut stdout = io::stdout().lock();
        stdout.write_all(bytes).and_then(|()| stdout.flush())
    } else {
        fs::write(output, bytes)
    };
    written.map_err(|error| Failure::Io(format!("cannot write {output}: {error}")))
}
