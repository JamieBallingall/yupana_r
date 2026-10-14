# AGENTS.md

Instructions for coding agents working in this repository.

## Read first

[README.md](README.md) is the design record: what the project is, the scope of stage 1, what is
decided, and what is still open. Until the range model and the core syntax are decided there,
the work is design, not code: the crates are a stub that keeps the commands below clean.

Where the code and the README disagree, fix whichever is wrong in the same commit. A decision
made while coding is recorded in the README when it is made.

## Human

See HUMAN.md, if it exists, for details of who you are working with. If it does not exist and you
wish to record information about working style, offer to create it. Never commit it: it is listed
in `.gitignore`.

## What this is

`.yup` is a small, fully specified language for writing spreadsheets as text. This repository
holds its specification and a tool, written in Rust, that turns a `.yup` file and its CSV inputs
into an xlsx workbook, natively and as WebAssembly.

## Commands

    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo build -p yup-core --target wasm32-unknown-unknown

All four are clean before every commit.

## Style

- Rust, edition 2024, on the toolchain pinned in `rust-toolchain.toml`.
- Functional in flavor, Rust in form: data is plain structs and enums, taken apart with `match`.
  Never push it past what reads as natural Rust.
- Expected failures are values: `Result`, with an error type that says what went wrong and
  where. A bug panics, and nothing catches it. `unwrap` and `expect` are for broken invariants
  only.
- Every error a user can cause carries the line and column it came from.
- Comments and doc comments say why, in the present tense. Doc tests are the examples.
- US spelling.
- Text files have LF line endings on every platform.
- Markdown is wrapped at 100 columns.
- Dates are absolute, such as 2026-10-14.

## Sources

Work only from open, published sources:

- ECMA-376 / ISO/IEC 29500 (Office Open XML);
- Microsoft's published Open Specifications for the format, chiefly [MS-XLSX] and
  [MS-OI29500];
- the spreadsheet app's public documentation.

Cite them by clause or section, and put things in your own words rather than copying their text.
Where they are silent or ambiguous, say so and record an open question in the README rather than
guessing.

## Legal

The project is licensed under Apache-2.0 ([LICENSE](LICENSE)). It interoperates with a file format
owned by a large vendor, so it is written clean-room and stays easy to defend.

- Everything committed is original work under Apache-2.0, or a dependency under a license
  compatible with it (MIT, BSD, Apache-2.0 and the like). Ask before relying on any other license.
- No code, tests or text copied from other projects, whatever their license.
- Knowledge of the format comes only from the published sources above. Never from disassembling,
  decompiling or tracing the spreadsheet app, from inspecting its private files, or from leaked or
  confidential material.
- No third-party material: no downloaded workbooks, no data from anyone else. Examples use
  made-up numbers for invented companies.
- Call the file format "xlsx" and the program "the spreadsheet app". No product names or
  trademarks in any identifier, module, file or command, and nothing that suggests endorsement by
  or affiliation with the vendor.

## Rules

- The core crate does no input or output: no files, no environment, no clock, no threads. It
  always builds for `wasm32-unknown-unknown`. It forbids `unsafe`.
- Few dependencies. A library is used only if it is unlikely to move underneath us and solves a
  problem that would be a real chunk of code to write ourselves. The lexer and parser are written
  by hand. Ask before adding any dependency.
- The writer never emits a file the spreadsheet app would offer to repair. If the checker cannot
  guarantee that, it refuses the input.
- No workbook is ever committed. Generated files go to `target/`.
- Nothing personal: no email addresses, no paths on anyone's machine, and no other personal
  details.
- Describe techniques generically, never as if from inside an organization.
- Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`), one concern
  per commit.
