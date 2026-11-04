//! Every example in `examples/` builds without warnings, and what each step up to Check prints
//! reads back to itself.

use std::fs;
use std::path::PathBuf;

use yup_core::{Inputs, Step};

/// Each example's name, its source, and the input files it reads.
fn examples() -> Vec<(String, String, Inputs)> {
    let folder: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "examples"]
        .iter()
        .collect();
    let mut paths: Vec<PathBuf> = fs::read_dir(&folder)
        .expect("the examples folder exists")
        .map(|entry| entry.expect("the folder can be listed").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "yup"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path).expect("an example is UTF-8");
            let mut inputs = Inputs::new();
            for file in yup_core::input_files(&source).expect("an example parses") {
                let text = fs::read_to_string(folder.join(&file.name)).expect("an input is there");
                inputs.insert(file.name, text);
            }
            (path.display().to_string(), source, inputs)
        })
        .collect()
}

#[test]
fn every_example_builds_without_warnings() {
    let examples = examples();
    assert!(examples.len() >= 3, "the examples are found");
    for (name, source, inputs) in &examples {
        let built =
            yup_core::build(source, inputs).unwrap_or_else(|errors| panic!("{name}: {errors:?}"));
        assert!(built.warnings.is_empty(), "{name}: {:?}", built.warnings);
    }
}

#[test]
fn the_fully_elaborated_form_of_every_example_builds_the_same_workbook() {
    for (name, source, inputs) in examples() {
        let grounded = yup_core::show(&source, Step::Ground, &inputs)
            .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
            .text;
        let built = |source: &str| {
            yup_core::build(source, &inputs)
                .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
                .xlsx
        };
        assert!(built(&grounded) == built(&source), "{name}");
    }
}

#[test]
fn every_step_of_every_example_reads_back_to_itself() {
    for (name, source, inputs) in examples() {
        for step in Step::ALL.into_iter().filter(|&step| step <= Step::Check) {
            let once = yup_core::show(&source, step, &inputs)
                .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
                .text;
            let twice = yup_core::show(&once, step, &inputs)
                .unwrap_or_else(|errors| panic!("{name} shown at {}: {errors:?}", step.name()))
                .text;
            assert_eq!(twice, once, "{name} at {}", step.name());
        }
    }
}
