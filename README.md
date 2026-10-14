# yupana_r

Working name: **yupana_r**. The repository is `yupana_r`; the project and crate names are not
decided. There is an existing **yupana** repo which this will eventually replace.

> **Seed capsule,** drafted 2026-10-14. This is a design record, not documentation of code. A
> first version of the whole pipeline, `.yup` 0.1, was built on 2026-10-14 to try the design end to
> end. Its syntax is provisional: [docs/language.md](docs/language.md) describes it, and
> [Built for 0.1](#built-for-01) lists what was decided on the way.

## What this is

A small, fully specified language, `.yup`, for writing spreadsheets as text, and a tool written in
Rust that turns a `.yup` file, with any CSV or JSON inputs beside it, into an xlsx workbook. The
tool runs natively as a command and in the browser as WebAssembly.

**Core values:**

1. **A concise authoring surface.** The logic of a whole workbook fits in one LLM context window.
   Bulk numbers live in input files beside the `.yup`, so the `.yup` holds structure, logic and
   formatting, and little numerical noise. The author never writes a coordinate or a size unless
   they want to: the tool works out every range's size and lays the workbook out.
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

## Trying it

    cargo run -p yup-cli -- build examples/forecast.yup -o target/forecast.xlsx
    cargo run -p yup-cli -- check examples/segments.yup
    cargo run -p yup-cli -- show place examples/segments.yup

`yup build` writes the workbook, by default beside the `.yup` file. `yup show <step>` prints what
any step makes of a file: the steps up to Check print a `.yup` file with more worked out, Lower
prints the cells, and Package the parts of the xlsx file. [examples](examples) has five files to
start from.

## Stage 1: scope

Stage 1 is the language and the writer, with no evaluation. It targets **financial models**: ranges
of inputs and calculations laid out over time periods, segments, scenarios and the like, with
input sheets and summaries. The default layout is the common one (a label at the left and one row
per line item, spilling right across the periods), but other layouts are possible at the cost of a
more explicit `.yup`.

The target is the spreadsheet app's current versions: dynamic arrays, LET, LAMBDA and its helper
functions (SCAN, REDUCE, MAP and the like), and the newer array functions (SEQUENCE, DROP, TAKE,
HSTACK, VSTACK). Earlier versions are not supported.

**In scope**

- The `.yup` specification: lexical grammar, syntax, and the checks made on a file.
- A hand-written lexer and parser, and a checker. Every error a user can cause carries its line and
  column.
- An xlsx writer whose output always opens without the spreadsheet app offering to repair it. The
  checker refuses anything the writer cannot write validly.
- Formulas written in `.yup` range by range, not cell by cell.
- Ranges whose size is a workbook input, such as a forecast horizon, and which resize when that
  input is changed in the workbook.
- LET, LAMBDA, and dynamic array formulas (whose results spill).
- Bindings and defined names. A `.yup` binding names a range or a function so that the author can
  refer to it. A binding may leave no trace in the workbook, or be exported as a workbook-scoped
  defined name: a named range, or a LAMBDA (a defined name whose value is the LAMBDA). See open
  question 13.
- Literal values in formulas (12 months in a year), and input values read from CSV or JSON files.
- Formatting: number formats (always a custom format string), column widths, bold, italic, font
  color, fill color, and basic cell borders.
- A command-line tool, a WebAssembly build, a website (upload a `.yup` and its inputs, download an
  xlsx), and an MCP server (check a `.yup`; later, evaluate it).

**Out of scope**

- **Evaluation,** approximate or exact. It is the next stage. Until then a written workbook carries
  no cached values: the spreadsheet app calculates when it opens the file, and a reader that does
  not calculate sees empty formula cells.
- Reading xlsx (see above).
- Other targets: ods, online spreadsheets, and versions of the app without dynamic arrays.
- Macros and VBA, external data queries, pivot tables, charts, conditional formatting, data
  validation, comments, merged cells, links to other workbooks, what-if data tables, and circular
  references.
- Overriding one cell inside a range. An exception is expressed as data instead:
  `revenue = base + adjustments`, where `adjustments` is an input row.
- A linter. Warning about a reference across sheets that does not go through a name is wanted
  later, but nothing is enforced.

**Limits that follow from resizable ranges.** A range whose extent is unknown when the file is
written must spill, and nothing can sit in its path. So nothing is placed after it in the direction
it spills (a total goes before a resizable row, not after it), and a block cannot be repeated once
per member of a set of unknown size (it becomes one two-dimensional range instead).

## Build order

1. **Syntax.** Settle the core syntax and the open parts of the range model; write the
   specification. A provisional 0.1 is built, to try the design; the steps below were begun for
   it, and settling the syntax may change them.
2. Lexer, parser, checker. *Built for 0.1.*
3. The xlsx writer. *Built for 0.1.*
4. Formatting. *Begun: number formats, bold, italic, blue inputs and column widths.*
5. The website and the MCP server.

## Decided so far

- Rust. A core crate does no input or output and always builds for `wasm32-unknown-unknown`. A
  command-line crate does the reading and writing. Wrappers for the web and for MCP come later.
- The crates are `yup-core` and `yup-cli`, and the command is `yup`: provisional names until open
  question 12 is settled. The command has `build`, `check` and `show <step>`; how it finds input
  files is part of open question 4.
- The project is licensed under Apache-2.0 (see [LICENSE](LICENSE)).
- `.yup` is a language of its own, with its own lexer and parser, and is designed to be easy to
  lex.
- Formulas are given range by range. The tool infers every range's size and does the layout.
- Each binding is a pair of lines: a signature (`name : …`), which carries the label and, later,
  formatting and type information, and a definition (`name = …`).
- Sizes are tracked through the model with size variables, and positions along them with spans.
  See [The range model](#the-range-model).
- The work is done in eight named steps, and every step up to Check can print its result as a valid
  `.yup`. See [The pipeline](#the-pipeline).
- Sheets are opened by `sheet` statements, and formulas never name a sheet.
- A `.yup` binding names a range or a function. An exported binding becomes a workbook-scoped
  defined name, LAMBDAs included; any other binding may be eliminated when the workbook is written.
- Binding names are ASCII. Labels and sheet names can be any Unicode, in single quotes.
- Comments start with `//`.
- Operators are preferred to function calls where they read well, such as `1 .. n` for a sequence.
- Inputs come both inline, as literals, and from CSV or JSON files.

## Built for 0.1

Decided while building the first version, on 2026-10-14, to have something to try. Each is
provisional, and names the open question it answers where there is one.

- **The first line** is `yup 0.1`, and the extension stays `.yup` for now (question 11).
- **Roles.** A binding is shown when its signature has a label, an input when the signature also
  says `Input`, and local otherwise. A signature without a label still has its size checked.
- **Exporting** (question 13 and observation 4). Every shown binding and input becomes a defined
  name, and formulas refer to them by name. A local binding is written into each formula that
  uses it; a LET would avoid repeating it, and is left for later.
- **Everything spills** (question 14). Every shown formula is a dynamic array formula, whatever its
  size; writing a formula into each cell of a range of fixed size is left for later.
- **Recording a spill** (question 20). The formula sits in its anchor cell as an array formula
  whose range is the anchor alone, with the cell metadata that marks it dynamic ([MS-XLSX]). No
  values are cached, and the workbook asks to be calculated in full when it opens. A spilled
  range's defined name is `_xlfn.ANCHORARRAY` of its anchor, which the app shows as `D4#`. The
  examples were opened in the spreadsheet app, outside this repository: they opened without an
  offer to repair, calculated, and spilled as intended.
- **Formats of spilled cells.** The cells a formula spills into are not in the file, so every row a
  styled range spills along carries the range's style. A range of unknown height is styled in its
  anchor only.
- **Signatures keep their sizes.** When a signature's size agrees with the definition, the range
  takes the size as the signature writes it, so a fixed size named there (`[s, 1]`) carries into
  the layout below it (`@ (s + 3, 4)`).
- **Functions.** A definition with parameters, `discount rate years = …`, is a function. Its size
  is worked out at each call from the arguments' sizes, and it becomes a defined name holding a
  LAMBDA, so the workbook can call it too.
- **Recurrences.** A cycle broken by `lag 1 x from first` carries the balance `x` from one period
  to the next. Its size is found in two passes: first with the lagged balance taken as a single
  value, which stretches to fit, then again with the size that gives. The balance is written as a
  SCAN whose step has the rest of the cycle written in place, so the workbook holds no circular
  reference; a row outside the cycle that it uses gives the SCAN its length.
- **`lag`** works along columns. `from` gives the first values of `lag 1` only (question 1).
- **Names** of bindings and sizes are lower case ASCII and follow the rules for defined names: not
  a cell (`tax2026`), not R1C1 (`r`, `c1`, `rc`), not a function or keyword (question 2).
- **The default sheet** for items before any `sheet` statement is `Sheet1` (question 18).
- **A possible `#SPILL!`** is a warning, and the workbook is still written (question 19).
- **Pins** are written `@ (row, column)`, numbers as in the elaborated forms (question 17).
- **Layout.** Row 1 and column A are margins. Labels and headings go in column B, column C stays
  empty, and position 0 of a range's columns is column D. Headings are bold.
- **Functions.** `min` and `max` work position by position, and `minimum` and `maximum` reduce, as
  in Haskell. Arguments that say how much come first (`lag 1 x`, `round 2 x`). Text is joined
  with `++`. Logical `&&`, `||` and `not` work position by position too.
- **Inputs** are literals or lists of literals, written in blue, as financial models usually show
  inputs, or tables read from CSV files (question 4).
- **Tables.** `history = csv "history.csv"` reads a file beside the `.yup` file, which the command
  line passes to the core. Its first row names the columns; unquoted fields are typed, quoted
  ones are text; `column "units" history` picks a column. A table's size is counted when the
  workbook is written, which departs from the range model's rule that a table of variable length
  has an unknown size: rows written as cells cannot grow in the workbook, so a table that grows
  there needs a reference that grows with it, which is left open.
- **Number formats** are checked conservatively before they reach the workbook; literal text in a
  format goes in double quotes.
- **The zip** stores its parts without compression and with a fixed timestamp, so the same file
  always gives the same bytes (questions 9 and 23).
- **Unnamed unknown sizes** are called after the binding that introduced them, such as `year_n`.

## The range model

### Sizes

Every range has a size, rows by columns, and each of the two is a **size expression**: an integer
constant, a size variable, or a sum of these with positive coefficients (`1`, `n`, `n - 1`,
`4 + s`, `n + m`). A product of two variables is not allowed. That keeps every extent growing with
its variables, so checking for collisions stays simple (see [Check](#the-pipeline)).

There are two kinds of size variable:

- **`Fixed`:** known when the file is written, such as the number of revenue segments
  (`s : Size Fixed`, `s = 4`).
- **`Unknown`:** decided in the workbook, such as a forecast horizon that the workbook's user can
  change. A range of `Unknown` size spills.

A third kind, **`Maximum`**, is planned for later: unknown, but with an upper bound inferred from
the inputs (FILTER over `m` rows gives at most `m`).

**Where `Unknown` sizes come from.** Every `1 .. x`, where `x` is not a constant, introduces a new
`Unknown` variable. Writing `1 .. forecast_horizon` twice gives two different sizes, and the
checker will not combine them. Standard practice is to generate each time series once and derive
everything else from it. An input table of variable length also introduces one, shared by all its
columns. A signature may name a size variable; the checker checks it rather than trusting it.

### Spans

Sizes alone are not enough. `init revenue` and `tail revenue` both have size `n - 1`, but they sit
on different periods. So every expression also carries a **span**: the positions it covers along
each size variable, written half-open. A span says nothing about the sheet.

| Expression | Values | Span |
|---|---|---|
| `revenue` | items 0 to n−1 | `[0, n)` |
| `tail revenue` | items 1 to n−1 | `[1, n)` |
| `init revenue` | items 0 to n−2 | `[0, n−1)` |
| `lag 1 revenue` | items 0 to n−2 | `[1, n)`, shifted one position along |

`tail revenue / lag 1 revenue` combines two expressions with span `[1, n)`, so it checks and gives
period-on-period growth. `tail revenue / init revenue` has matching sizes but different spans, and
is refused: it is the off-by-one-period error that sizes alone would let through.

### How shapes combine

- Elementwise operations need equal sizes and equal spans.
- A dimension of size 1 is stretched to match the other operand, as the app does. One value per
  segment, stretched across the periods, is a column of size `s 1`, not a row of size `1 s`.
- Expressions over different size variables do not combine, except through a dimension of size 1.
- A lookup's result has the size and span of its key, not of the table. That is how values move
  from an input table of size `m` onto a timeline of size `n`.

### Layout

Spans are positions along a size variable. Layout decides where position 0 of each lands on a
sheet. The label column, the gap between label and values, and the column where position 0 lands
are constants of the layout, not sizes. A range's **anchor** is the top-left cell of its values,
not its label, and its anchor column is the column of position 0 plus the start of its span.

By default, layout follows the order of the definitions: each range goes below the one before.
More explicit placement, at the cost of a longer `.yup`, is possible: blocks on several sheets,
blocks that run down rather than across, two-dimensional blocks, and ranges pinned to a cell. The
syntax for these is open (question 16).

An anchor may depend on `Fixed` sizes (row `4 + s`) but never on `Unknown` ones. Placing something
below a range of `n` rows is refused.

### Sheets

A `sheet` statement opens a section, and everything after it belongs to that sheet until the next
one. The order of the `sheet` statements is the order of the tabs.

```text
sheet 'Inputs'
...
sheet 'Forecast'
...
```

Bindings are workbook-wide, so formulas name bindings, never sheets, and renaming a sheet is a
one-line edit. The checker enforces the app's rules for sheet names: the maximum length, the
forbidden characters, and uniqueness ignoring case.

### Fill and spill

A range of `Unknown` size must spill: it is one dynamic array formula at its anchor. Whether a range
of `Fixed` size spills or is filled with one formula per cell is open (question 14).

### Recurrences

A balance carried from one period to the next is natural in a hand-built workbook and awkward
under spill. Because horizons resize, it has to work under spill. In illustrative syntax:

```text
opening = lag 1 closing from principal
payment = min scheduled opening
closing = opening - payment
```

With one formula per cell, `lag` is the cell to the left. With scheduled in row 3, opening in row
4, payment in row 5 and closing in row 6, column B holds `=principal`, `=MIN(B3,B4)` and `=B4-B5`;
column C holds `=B6`, `=MIN(C3,C4)` and `=C4-C5`; and so on across.

Under spill, if opening and closing were each one formula over the other's spill range, that would
be a circular reference, even though no cell depends on itself. The recurrence has to be rewritten
as an accumulation:

```text
closing: =SCAN(principal, scheduled, LAMBDA(b, s, b - MIN(s, b)))
opening: =HSTACK(principal, DROP(closing#, , -1))
payment: =opening# - closing#
```

That rewrite is a compiler transformation (question 15). *To verify:* SCAN's accumulator must be a
single value, so two or more coupled balances (a revolving facility and a cash balance, say) cannot
share one SCAN. That case would need REDUCE with stacking.

*Built for 0.1:* a cycle is allowed when `lag 1 x from first` breaks it, and has one balance. The
balance is written as one SCAN over the periods, whose step works one period out with the other
members of the cycle written in place; the other members are ordinary formulas that use the
balance. In [the loan example](examples/loan.yup) the closing balance is
`SCAN(amount,SEQUENCE(1,COLUMNS(scheduled)),LAMBDA(_balance,_period,_balance-…))`, and the
spreadsheet app works it out as intended. Coupled balances are refused.

### Elementwise or aggregate

In `.yup`, `min scheduled opening` over two rows has to mean one minimum per period. Under spill,
the app's MIN over two ranges returns a single number, so the writer has to emit `IF(a<b, a, b)` or
a MAP instead. The function table says, for each function, whether it is elementwise or
aggregating.

### Names in written formulas

Exported bindings become defined names. Not decided: whether formulas in the workbook refer to
bindings by name or by A1 reference (question 13). Under spill, names read naturally.

### Two kinds of failure

- **A file the app would offer to repair.** Always refused: the checker rejects any input the
  writer cannot write validly.
- **A possible `#SPILL!`.** The file is valid, but a spill may collide with something when the
  workbook calculates. The checker reports it; whether as an error or a warning is open
  (question 19).

## The pipeline

Each step takes one form of the model and produces the next.

1. **Parse** produces a syntax tree. It decides structure only. Errors: lexical and syntax.
2. **Resolve** produces the resolved model: what each name refers to, and the dependency graph.
   Errors: an unknown or duplicate name, a circular reference.
3. **Infer** produces the typed model: every expression's size and span, in size variables.
   Errors: a size or span mismatch, the wrong number of arguments, an aggregate where an
   elementwise function is needed.
4. **Place** produces the placed model: each block's sheet, every anchor and label cell, and the
   gaps. Anchors may involve `Fixed` sizes. Errors: an anchor that depends on an `Unknown` size.
5. **Ground** produces the grounded model by substituting the `Fixed` sizes. Every anchor is now a
   constant, and every extent a size expression in `Unknown` variables only. No errors: this is
   pure substitution.
6. **Check** looks for overlaps and possible `#SPILL!`, for every value of the `Unknown`
   variables, and changes nothing. Errors: a collision (refused); a possible spill (reported).
7. **Lower** produces cell contents. Range by range becomes cell by cell, and it writes the
   formula text: function names, `_xlfn.` prefixes, parentheses and A1 references. A failure here
   is a bug.
8. **Package** produces the xlsx bytes: the XML parts and the zip. A failure here is a bug.

Steps 1 to 6 are the checker; steps 7 and 8 are the writer.

**Spans are inferred at Infer,** with sizes, from the expressions alone and before any sheet or
coordinate exists. Place is the first step to use them.

**Coordinates are numbers until Lower.** Through steps 3 to 6 a cell is a pair of numbers
`(row, column)`, counted from 1 as the app counts (A1 is `(1, 1)`). Before Ground an anchor can be
`(4 + s, 4)`, which has no A1 form. Lower is the only step that writes A1.

**Range by range becomes cell by cell at Lower.** A range that spills becomes one formula in its
anchor cell; the cells it spills into are left empty in the file, for the app to fill when it
calculates. A range that is filled (only possible at a `Fixed` size) becomes one formula per cell,
with relative references. Every label becomes a text cell.

**Every step up to Check can print a valid `.yup`.** After Parse that is a formatter; after Resolve
the definitions are regenerated; Infer, Place and Ground add sizes and spans, anchors, and
substituted values; Check adds its findings as comments. After Lower the content is cell-level
formula text and no longer `.yup`. Two requirements follow:

- The language has syntax for everything the steps add: size variables, spans and numeric anchors.
- Running the pipeline on an elaborated file changes nothing: elaborating twice gives the same
  file as elaborating once. That is a property the tests check.

## A worked example

Illustrative syntax only. Labels are in column B, column C is a gap, and position 0 of the period
timeline lands in column D.

```text
sheet 'Forecast'

n : Size Unknown

period : 1 n 'Period'
period = 1 .. forecast_horizon               // introduces n

revenue : 'Revenue'
revenue = <lookup by period>                 // size 1 n, span [0, n)

growth : 'Revenue growth'
growth = tail revenue / lag 1 revenue        // size 1 (n - 1), span [1, n)
```

After Ground (numeric anchors, `Unknown` extents):

```text
period   [1, n]       @ (2, 4)    label @ (2, 2)
revenue  [1, n]       @ (3, 4)    label @ (3, 2)
growth   [1, n - 1]   @ (4, 5)    label @ (4, 2)
```

After Lower, on the sheet:

```text
     B                C    D          E          F    …
2    Period                =SEQUENCE(1, forecast_horizon)   (spills)
3    Revenue               =…                               (spills)
4    Revenue growth                   =DROP(revenue,,1)/DROP(revenue,,-1)   (spills)
```

D4 is empty by construction, and all three ranges end in the same column, D + n − 1, because all
three spans end at n.

## Core syntax

- **Option S1: like the spreadsheet app.** `EXP(x)`, `IF(a, b, c)`. Familiar to every LLM, which
  is the risk: authors will paste formulas from the app, A1 references included.
- **Option S2: like Python.** `exp(x)`. Familiar, and still close enough to the app to invite the
  same mistake.
- **Option S3: like Haskell or ML.** `exp x`, `if a then b else c`, `let … in …`, `\x y -> …` for
  a LAMBDA, and `discount r t = 1 / (1 + r) ^ t` as a named LAMBDA. It makes clear that `.yup` is
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
- Case carries meaning: bindings, functions and local variables are lower case
  (`forecast_horizon`); words at the type level start upper case (`Input`, `Size`, `Fixed`), as in
  Haskell. A binding may not share a name with a function. Names that differ only in case are an
  error, since defined names in the app ignore case.
- Comments start with `//`, which means nothing in the app's formulas. `--` is Haskell's, but in
  the app it is double negation, used to turn logicals into numbers; `#` is the app's spill
  operator (`A1#`) and starts its error values (`#N/A`).
- Labels and sheet names in single quotes, as the app quotes sheet names; a quote inside is
  doubled (`'Prior year''s revenue'`). String values in double quotes, with defined escapes.
  Numbers in JSON's grammar, as in `.yup` 0.0.2.

## Options considered and set aside

- **Extent per range** (fill with an explicit extent, spill with the extent inferred, the row
  model, or a choice per range). Replaced by size variables, which infer extents and make the
  choice between fill and spill follow from the size.
- **Placement by absolute cell, by relative position (`Below foo`), or by order.** Order is the
  default; pinning a cell is kept for the explicit end of the layout. Relative placement between
  single ranges restates the order and drifts from it.
- **Named axes** (a range indexed by `period` rather than sized by `n`). Size variables describe
  the physical rows and columns, which is what layout needs; spans recover the alignment that axes
  would have given.

## Other open questions

Where 0.1 gives a provisional answer, it follows in italics; [Built for 0.1](#built-for-01) has the
reasons.

1. **The lag operator:** the seed for a lag of more than one, and the direction for a
   two-dimensional range. *0.1: along columns; `from` with `lag 1` only.*
2. **Name rules:** are sheet-scoped names ever needed? *0.1: every binding follows the rules for
   defined names, so that exporting one never forces a rename.*
3. **An escape hatch for raw formula text:** allowed, and if so how marked?
4. **Input files:** the CSV dialect and the JSON shape, how cells are typed (number, text, logical,
   blank), how a `.yup` binds an input to a range, and how an input table declares its size
   variable. *0.1: CSV in RFC 4180's dialect, `csv "file.csv"` in an input's definition, and a
   size counted when the workbook is written. JSON is not built, and neither is a table that grows
   in the workbook.*
5. **Dates:** a literal syntax, written as serial numbers in the 1900 date system.
6. **Logicals and errors:** whether error literals exist, and how `and`, `or` and `not` are
   written. *0.1: `true` and `false`, and `&&`, `||` and `not`.*
7. **Formatting syntax:** inline on each range, named styles applied to ranges, or both, and the
   defaults. *0.1: inline, `Format "…"`, `Bold` and `Italic` in the signature.*
8. **Validity without the app:** schema validation in the tests and reading by independent
   readers. Opening files in the spreadsheet app itself is a check made outside this repository.
   *0.1: the tests check that every part is well formed and listed, and that the zip reads back.*
9. **Dependencies:** `wasm-bindgen` for the web, and whatever the MCP server needs. *0.1: none; the
   zip is stored, with its own CRC-32.*
10. **The MCP server's tools:** check a file, print any step's `.yup`, write the xlsx.
11. **The file:** keep the `.yup` extension? Either way, the first line identifies the version, so
    each reader refuses the other format's files. *0.1: `.yup`, and the first line is `yup 0.1`.*
12. **Names:** of the project and its crates.
13. **Exporting bindings:** whether a shown binding can stay unexported, and whether a local one
    becomes a LET variable rather than being repeated. *0.1: every shown binding is a defined
    name, formulas use those names, and a local binding is written where it is used.*
14. **Fixed sizes:** does a range of `Fixed` size spill, or is it filled with one formula per cell?
    *0.1: everything spills.*
15. **Recurrences under spill:** the rewrite into SCAN or REDUCE, and what to do with coupled
    balances. *0.1: one balance per cycle, written as a SCAN; coupled balances are refused, and
    so is anything in the cycle that does not work position by position.*
16. **Signature and layout syntax:** the order and form of size, span, label, anchor and
    attributes in a signature; how blocks, orientation (across or down), two-dimensional blocks
    and pinned cells are written; and how an elaborated file writes spans and numeric anchors.
    *0.1: see [docs/language.md](docs/language.md).*
17. **Pinned cells in the surface:** written in A1, as the author sees the workbook, or as numbers
    like every elaborated form? *0.1: numbers.*
18. **A file with no `sheet` statement:** a default sheet, or an error? *0.1: a sheet called
    `Sheet1`.*
19. **A possible `#SPILL!`:** an error or a warning? *0.1: a warning.*
20. **Recording a spill of unknown extent:** *0.1: the anchor alone as the formula's range, with
    the dynamic array metadata, and no cached values. The spreadsheet app opens and spills it.*
21. **Time aggregation:** rolling monthly ranges up to annual ones (`sum revenue by year`), which
    maps one size variable onto another.
22. **The function library:** which of the app's functions stage 1 supports. *0.1: the eighteen in
    [docs/language.md](docs/language.md).*
23. **Deterministic output:** does the same input always give byte-identical xlsx? *0.1: yes.*

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
