# yupana_r

Working name: **yupana_r**. The repository is `yupana_r`; the project and crate names are not
decided. There is an existing **yupana** repo which this will eventually replace.

> **Seed capsule,** drafted 2026-10-14. This is a design record, not documentation of code.
> **No code is written until the range model and the core syntax below are decided.**

## What this is

A small, fully specified language, `.yup`, for writing spreadsheets as text, and a tool written in
Rust that turns a `.yup` file, with any CSV inputs beside it, into an xlsx workbook. The tool runs
natively as a command and in the browser as WebAssembly.

**Core values:**

1. **A concise authoring surface.** The logic of a whole workbook fits in one LLM context window.
   Bulk numbers live in CSV files beside the `.yup`, so the `.yup` holds structure, logic and
   formatting, and little numerical noise.
2. **Files that open cleanly.** Every spreadsheet file the tool writes opens without the
   spreadsheet app warning that it is corrupt or offering to repair it. Any such warning is a
   show-stopping bug.

**Who it is for:**

- **An LLM building or editing a workbook.** Through an API, an LLM sees a workbook a few cells at
  a time. Here it reads and writes the whole `.yup`, and an MCP server lets it check what it wrote.
- **Other model definition languages.** They generate `.yup` and leave every question of workbook
  compatibility to this project.
- **A person** writing a small workbook, or editing a `.yup`, by hand.

**What it is not:** a converter from xlsx. Turning an existing workbook into `.yup` is a job for an
LLM that reads the workbook, decides what matters (with a person where needed) and writes the
`.yup` from scratch. That happens outside this repository. A lossy automatic conversion would drop
things silently or bury the user in warnings.

## Stage 1: scope

Stage 1 is the language and the writer, with no evaluation.

**In scope**

- The `.yup` specification: lexical grammar, syntax, and the checks made on a file.
- A hand-written lexer and parser, and a checker. Every error a user can cause carries its line and
  column.
- An xlsx writer whose output always opens without the spreadsheet app offering to repair it. The
  checker refuses anything the writer cannot write validly.
- Formulas written in `.yup` range by range, not cell by cell.
- LET, LAMBDA, and dynamic array formulas (whose results spill).
- Bindings and defined names. A `.yup` binding names a range or a function so that the author can
  refer to it. A binding may leave no trace in the workbook, or be exported as a workbook-scoped
  defined name: a named range, or a LAMBDA (a defined name whose value is the LAMBDA). See open
  question 13.
- Literal values in formulas (12 months in a year), and input values read from CSV files.
- Formatting: number formats (always a custom format string), column widths, bold, italic, font
  color, fill color, and basic cell borders.
- A command-line tool, a WebAssembly build, a website (upload a `.yup` and its CSVs, download an
  xlsx), and an MCP server (check a `.yup`; later, evaluate it).

**Out of scope**

- **Evaluation,** approximate or exact. It is the next stage. Until then a written workbook carries
  no cached values: the spreadsheet app calculates when it opens the file, and a reader that does
  not calculate sees empty formula cells.
- Reading xlsx (see above).
- Other targets: ods, online spreadsheets.
- Macros and VBA, external data queries, pivot tables, charts, conditional formatting, data
  validation, comments, merged cells, links to other workbooks, and circular references.
- A linter. Warning about a reference across sheets that does not go through a name is wanted
  later, but nothing is enforced.

## Build order

1. **Syntax.** Decide the range model and the core syntax; write the specification.
   **Nothing below starts until this is done.**
2. Lexer, parser, checker.
3. The xlsx writer.
4. Formatting.
5. The website and the MCP server.

## Decided so far

- Rust. A core crate does no input or output and always builds for `wasm32-unknown-unknown`. A
  command-line crate does the reading and writing. Wrappers for the web and for MCP come later.
- `.yup` is a language of its own, with its own lexer and parser, and is designed to be easy to
  lex.
- Formulas are given range by range.
- A `.yup` binding names a range or a function. An exported binding becomes a workbook-scoped
  defined name, LAMBDAs included; any other binding may be eliminated when the workbook is written.
- The project is licensed under Apache-2.0 (see [LICENSE](LICENSE)).
- Inputs come both inline, as literals, and from CSV files.
- A lag operator refers to the previous position along a range.

## The major challenge: what a range is

A `.yup` name is bound to a range, and a range has to be **placed** (where it starts) and given an
**extent** (how many rows and columns it covers). Without evaluation the writer has to know both
when it writes the file. Two things make that hard:

- **Placement depends on extent.** `Bar : Below Foo` needs Foo's height.
- **Some extents are not known until the workbook calculates.** A spilled result's shape is
  usually a simple function of its inputs' shapes (elementwise arithmetic, TRANSPOSE, MMULT,
  HSTACK). It is not for FILTER and UNIQUE, nor for SEQUENCE with computed arguments.

### Options for extent and meaning

- **Option A: fill, with an explicit extent.** Every range states its extent, possibly through a
  named constant (`Periods = 12`) so that changing the horizon is one edit. The writer puts a
  relative formula into each cell, so the workbook looks hand-built, works in any spreadsheet app,
  and shows each cell's formula. Spill is used only where the author asks for it.
- **Option B: spill, with the extent inferred.** Each range is one dynamic array formula at its
  anchor. Its extent is inferred where the output shape follows from the input shapes, and declared
  where it does not, either as a reserved maximum or by placing that range last in its direction,
  with nothing after it.
- **Option C: the row model.** This is a special case of B, and the common shape of a financial
  model: a label at the left, then one row whose formula spills to the right across an open-ended
  number of periods. Layout is purely vertical, every height is 1, and nothing is placed to the
  right, so the unknown width never matters.
- **Option D: chosen per range.** Fill by default and spill on request, or the other way round.

### Observations that bear on the choice

1. **CSV shapes are known when the workbook is written.** CSV inputs are read by the tool, not by
   the workbook, so their shapes are known at write time. Under Option B the truly unknown shapes
   are only those that depend on data: FILTER, UNIQUE, and their kind.
2. **Recurrences favor fill.** A balance carried from one period to the next is natural under
   fill and awkward under spill. In illustrative syntax only:

   ```text
   Opening = lag Closing from Principal
   Payment = min Scheduled Opening
   Closing = Opening - Payment
   ```

   Under fill, `lag` becomes the cell to the left. With Scheduled in row 3, Opening in row 4,
   Payment in row 5 and Closing in row 6, column B holds `=Principal`, `=MIN(B3,B4)` and `=B4-B5`;
   column C holds `=B6`, `=MIN(C3,C4)` and `=C4-C5`; and so on across.

   Under spill, if Opening and Closing were each one formula over the other's spill range, that
   would be a circular reference, even though no cell depends on itself. The recurrence has to be
   rewritten as an accumulation:

   ```text
   Closing: =SCAN(Principal, Scheduled, LAMBDA(b, s, b - MIN(s, b)))
   Opening: =HSTACK(Principal, DROP(Closing#, , -1))
   Payment: =Opening# - Closing#
   ```

   That rewrite is a compiler transformation. *To verify:* SCAN's accumulator must be a single
   value, so two or more coupled balances (a revolving facility and a cash balance, say) cannot
   share one SCAN. That case would need REDUCE with stacking, or fill.
3. **Elementwise or aggregate.** In `.yup`, `min Scheduled Opening` over two rows has to mean one
   minimum per period. Under fill, the app's MIN in each cell gives that. Under spill, MIN over two
   ranges returns a single number, so the writer has to emit `IF(a<b, a, b)` or a MAP instead. The
   language has to say, for each function, whether it is elementwise or aggregating.
4. **Names in written formulas.** Decided: exported bindings become defined names. Not decided:
   whether formulas in the workbook refer to them by name or by A1 reference. Under fill, a
   per-cell formula that uses the name of a multi-cell range relies on implicit intersection, and
   the app shows it as `@Revenue`. Under spill, names read naturally.

### Options for placement

Placement is a separate question from extent, though it depends on it.

- **Option P1: absolute.** `Foo_Value : Sheet1!B1`. Simple and explicit; inserting a row means
  renumbering.
- **Option P2: relative.** `Foo_Value : RightOf Foo_Label`, `Bar_Label : Below Foo_Label`. Robust
  to insertion. Placement forms its own dependency graph, which must be acyclic and is resolved
  before anything is written; a self-placement (`Baz : RightOf Baz`) or an unknown name is an error
  at a precise line and column.
- **Option P3: implied by order.** A block (the rows of a sheet, or the columns of a table) lists
  its ranges in order, and position follows from the order, with one anchor per block. Inserting a
  row means inserting a line, which suits an LLM well. It is also the natural syntax for Option C.

P1 and P3 can be combined: an anchored block, plus absolute placement for anything outside a
block. A label usually sits beside its value. Whether a label and value are declared together,
with the label defaulting from the name, is a question for the syntax.

## The second challenge: core syntax

- **Option S1: like the spreadsheet app.** `EXP(x)`, `IF(a, b, c)`. Familiar to every LLM, which
  is the risk: authors will paste formulas from the app, A1 references included.
- **Option S2: like Python.** `exp(x)`. Familiar, and still close enough to the app to invite the
  same mistake.
- **Option S3: like Haskell or ML.** `exp x`, `if a then b else c`, `let … in …`, `\x y -> …` for
  a LAMBDA, and `Discount r t = 1 / (1 + r) ^ t` as a named LAMBDA. It makes clear that `.yup` is
  a language of its own that is compiled to the app's formulas. **This is the current leaning.**

Under S3 these need settling:

- **Optional and variadic arguments.** IF without an else, XLOOKUP's three optional arguments, SUM
  over many ranges. Candidates are a list literal for variadics (`sum [a, b, c]`), named arguments
  in a record, and separately named variants. The app also distinguishes an omitted argument from
  a zero.
- **Partial application.** The app's LAMBDA is not curried. The simplest rule is that every call is
  saturated and the checker checks arity; the alternative is to compile partial application into a
  LAMBDA.
- **Operators.** `=` binds, so equality needs another spelling (`==`, with `/=` or `!=`).
  Concatenation needs a spelling. Precedence is `.yup`'s own: the writer adds parentheses so that
  the app's quirks cannot change the meaning (`-2^2` is 4 in the app).
- **Function names.** `.yup`'s names (lower case) map to the app's through a table, which also
  records each function's arity, optional arguments, whether it is elementwise or aggregating, and
  its encoding in the file (the `_xlfn.` prefix of the newer functions). That table is stage 1's
  function library.

Principles for easy lexing, proposed:

- One statement per line. A newline inside brackets is not an end of statement (Python's rule),
  which a lexer handles with a bracket counter. No offside rule.
- Case carries meaning: bindings start upper case (`Foo_Value`, `Discount`); functions, keywords
  and local variables start lower case. Names that differ only in case are an error, since defined
  names in the app ignore case.
- A comment marker that clashes with nothing: `--` is Haskell's, but it is also double negation,
  which the app uses to turn logicals into numbers. `#` would collide with a Markdown-style
  `# Sheet1` heading if headings were used for sheets.
- Strings in double quotes, with defined escapes. Numbers in JSON's grammar, as in `.yup` 0.0.2.

## Other open questions

1. **The lag operator:** the seed for the first position, a lag of more than one, and the axis for
   a two-dimensional range.
2. **Name rules:** an exported binding must be a legal defined name, and must not read as a cell
   reference (`TAX2026` is a cell) or as `R1C1`, `TRUE` and the like. Do the same rules apply to
   every binding, so that exporting one never forces a rename? Are sheet-scoped names ever needed?
3. **An escape hatch for raw formula text:** allowed, and if so how marked?
4. **CSV inputs:** the dialect, how cells are typed (number, text, logical, blank), how a `.yup`
   binds a CSV to a range, and whether a CSV fixes the range's extent.
5. **Dates:** a literal syntax, written as serial numbers in the 1900 date system.
6. **Logicals and errors:** literal syntax for TRUE and FALSE, and whether error literals exist.
7. **Formatting syntax:** inline on each range, named styles applied to ranges, or both, and the
   defaults.
8. **Validity without the app:** schema validation in the tests and reading by independent
   readers. Opening files in the spreadsheet app itself is a check made outside this repository.
9. **Dependencies:** zip (a store-only zip with its own CRC-32 is small; deflate is a dependency or
   a real chunk of code), `wasm-bindgen` for the web, and whatever the MCP server needs.
10. **The MCP server's tools:** check a file, show where everything was placed, write the xlsx.
11. **The file:** keep the `.yup` extension? Either way, the first line identifies the version, so
    each reader refuses the other format's files.
12. **Names:** of the project and its crates.
13. **Exporting bindings:** how a binding is marked for export (or whether export is the default),
    and what a binding that is not exported becomes in the workbook: A1 references, a LET variable,
    or an inlined expression.

## References

- **ECMA-376 / ISO/IEC 29500 (Office Open XML).** Part 1 §18 is SpreadsheetML; §18.17 covers
  formulas, including the grammar and the predefined functions. Part 2 is the packaging (zip,
  content types, relationships).
- **[MS-XLSX],** Microsoft's published extensions to SpreadsheetML: the file encoding of the newer
  functions (`_xlfn.`, `_xlpm.` for LAMBDA and LET parameters), and of dynamic array formulas.
- **[MS-OI29500],** Microsoft's published notes on where its implementation departs from
  ISO/IEC 29500.
- **The spreadsheet app's public function documentation,** for the semantics of functions newer
  than the standard.
