# yupana_r

Working name: **yupana_r**. The repository is `yupana_r`; the project and crate names are not
decided. There is an existing **yupana** repo which this will eventually replace.

> **Seed capsule,** drafted 2026-10-14. This is a design record, not documentation of code. A
> first version of the whole pipeline, `.yup` 0.1, was built on 2026-10-14 to try the design end to
> end, and 0.2, which settles much of its syntax, on 2026-11-03. The syntax is still provisional:
> [docs/language.md](docs/language.md) describes it, and [Built for 0.1](#built-for-01) and
> [Built for 0.2](#built-for-02) list what was decided on the way.

## What this is

A small, fully specified language, `.yup`, for writing spreadsheets as text, and a tool written in
Rust that turns a `.yup` file, with any CSV or JSON inputs beside it, into an xlsx workbook. The
tool runs natively as a command and in the browser as WebAssembly.

**Core values:**

1. **A concise authoring surface.** The logic of a whole workbook fits in one LLM context window.
   Bulk numbers live in input files beside the `.yup`, so the `.yup` holds structure, logic and
   formatting, and little numerical noise. The author never writes a size unless they want to,
   and places each range beside another rather than at a coordinate: the tool works out every
   range's size and every cell.
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
prints the cells, and Package the parts of the xlsx file. [examples](examples) has seven files to
start from.

## Stage 1: scope

Stage 1 is the language and the writer, with no evaluation. Its goal is **a concise description
of a workbook's formula logic and styling.** Financial models were its first target and remain its
first examples: ranges of inputs and calculations laid out over periods, segments and scenarios,
with input sheets and summaries. Nothing in the language depends on them. A range runs across or
down, `lag`, `tail` and `init` work along either, and any layout is described by placing each
range at a cell or beside another.

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
- Literal values in formulas (12 months in a year), and values read from CSV or JSON files.
- Formatting, through named styles: number formats (always a custom format string), fonts (name,
  size, bold, italic and color), fills, borders and their colors, alignment and wrapping; tab
  colors and hidden sheets; and column widths and row heights, which the file sets in pixels.
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
  `revenue = base + adjustments`, where `adjustments` is a row read from an input file.
- A linter. Warning about a reference across sheets that does not go through a name is wanted
  later, but nothing is enforced.

**Limits that follow from resizable ranges.** A range whose extent is unknown when the file is
written must spill, and nothing can sit in its path. So nothing is placed after it in the direction
it spills (a total goes before a resizable row, not after it), and a block cannot be repeated once
per member of a set of unknown size (it becomes one two-dimensional range instead).

## Build order

1. **Syntax.** Settle the core syntax and the open parts of the range model; write the
   specification. A provisional 0.1 was built to try the design, and 0.2 settles much of the
   syntax: see [Decided for 0.2](#decided-for-02).
2. Lexer, parser, checker. *Built for 0.1, and rebuilt for 0.2.*
3. The xlsx writer. *Built for 0.1, and extended for 0.2.*
4. Formatting. *Built for 0.2: number formats, fonts, fills, borders, alignment, tab colors,
   hidden sheets, and column widths and row heights.*
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
- The core syntax is S3, like Haskell or ML, decided on 2026-11-03. See [Core syntax](#core-syntax).
- Formulas are given range by range. The tool infers every range's size, and works out every
  cell from the placements.
- Each binding is a pair of lines: a signature (`name : …`), which carries its role, type, style
  and place, and a definition (`name = …`).
- Sizes are tracked through the model with size variables, and positions along them with spans.
  See [The range model](#the-range-model).
- The work is done in eight named steps, and every step up to Check can print its result as a valid
  `.yup`. See [The pipeline](#the-pipeline).
- Sheets are bindings, and formulas never name a sheet.
- A `.yup` binding names a range or a function. An exported binding becomes a workbook-scoped
  defined name, LAMBDAs included; any other binding may be eliminated when the workbook is written.
- Binding names are ASCII. Text and sheet names can be any Unicode.
- Comments start with `//`.
- Operators are preferred to function calls where they read well, such as `1 .. n` for a sequence.
- Values come both inline, as literals, and from CSV or JSON files.

## Built for 0.1

Decided while building the first version, on 2026-10-14, to have something to try. Each is
provisional, and names the open question it answers where there is one. 0.2 replaced much of
this: an entry it replaced says so, in italics, and the sections on 0.2 stand.

- **The first line** is `yup 0.1`, and the extension stays `.yup` for now (question 11). *Replaced
  in 0.2: the first line is `yup 0.2`.*
- **Roles.** A binding is shown when its signature has a label, an input when the signature also
  says `Input`, and local otherwise. A signature without a label still has its size checked.
  *Replaced in 0.2: every signature states its role.*
- **Exporting** (question 13 and observation 4). Every shown binding and input becomes a defined
  name, and formulas refer to them by name. A local binding is written into each formula that
  uses it; a LET would avoid repeating it, and is left for later. *Replaced in 0.2 for locals,
  which are LET variables; every `Formula` and `Value` is still a defined name.*
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
  anchor only. *Replaced in 0.2: see [Built for 0.2](#built-for-02).*
- **Signatures keep their sizes.** When a signature's size agrees with the definition, the range
  takes the size as the signature writes it, so a fixed size named there (`[s, 1]`) carries into
  the layout below it (`@ (s + 3, 4)`). *0.2 keeps this, and writes the place as a move from
  another binding.*
- **Functions.** A definition with parameters, `discount rate years = …`, is a function. Its size
  is worked out at each call from the arguments' sizes, and it becomes a defined name holding a
  LAMBDA, so the workbook can call it too. *0.2 keeps this, with the role `Lambda` and a type.*
- **Recurrences.** A cycle broken by `lag 1 x from first` carries the balance `x` from one period
  to the next. Its size is found in two passes: first with the lagged balance taken as a single
  value, which stretches to fit, then again with the size that gives. The balance is written as a
  SCAN whose step has the rest of the cycle written in place, so the workbook holds no circular
  reference; a row outside the cycle that it uses gives the SCAN its length. *0.2 keeps this,
  writes `seed` for `from`, and lets the balance run down a column too.*
- **`lag`** works along columns. `from` gives the first values of `lag 1` only (question 1).
  *Replaced in 0.2: `lag` works across or down, and `seed` gives the first value.*
- **Names** of bindings and sizes are lower case ASCII and follow the rules for defined names: not
  a cell (`tax2026`), not R1C1 (`r`, `c1`, `rc`), not a function or keyword (question 2).
  *Replaced in 0.2: upper case may follow the first letter, and names that differ only in case
  are an error.*
- **The default sheet** for items before any `sheet` statement is `Sheet1` (question 18).
  *Replaced in 0.2: sheets are bindings, and there is no default sheet.*
- **A possible `#SPILL!`** is a warning, and the workbook is still written (question 19).
- **Pins** are written `@ (row, column)`, numbers as in the elaborated forms (question 17).
  *Replaced in 0.2: a cell is written in A1, `at forecast B2`.*
- **Layout.** Row 1 and column A are margins. Labels and headings go in column B, column C stays
  empty, and position 0 of a range's columns is column D. Headings are bold. *Replaced in 0.2:
  every binding says where it goes, and nothing is styled implicitly.*
- **Built-in functions.** `min` and `max` work position by position, and `minimum` and `maximum`
  reduce, as in Haskell. Arguments that say how much come first (`lag 1 x`, `round 2 x`). Text is
  joined with `++`. Logical `&&`, `||` and `not` work position by position too.
- **Inputs** are literals or lists of literals, written in blue, as financial models usually show
  inputs, or tables read from CSV files (question 4). *Replaced in 0.2: a `Value` holds one
  literal or a CSV table, and is styled only as its signature says.*
- **Tables.** `history = csv "history.csv"` reads a file beside the `.yup` file, which the command
  line passes to the core. Its first row names the columns; unquoted fields are typed, quoted
  ones are text; `column "units" history` picks a column. A table's size is counted when the
  workbook is written, which departs from the range model's rule that a table of variable length
  has an unknown size: rows written as cells cannot grow in the workbook, so a table that grows
  there needs a reference that grows with it, which is left open.
- **Number formats** are checked conservatively before they reach the workbook; literal text in a
  format goes in double quotes. *0.2 puts the format code itself in single quotes.*
- **The zip** stores its parts without compression and with a fixed timestamp, so the same file
  always gives the same bytes (questions 9 and 23).
- **Unnamed unknown sizes** are called after the binding that introduced them, such as `year_n`.

## Decided for 0.2

Decided on 2026-11-03, in discussion, after trying 0.1. Where this section and
[Built for 0.1](#built-for-01) disagree, this section stands. Its aim is that nothing about a
binding is left implicit, so an LLM never has to infer what a line means from what it leaves out,
or from where the line sits in the file.

### Bindings

- **A binding is a signature, then a definition,** as in Haskell. The name is written on both
  lines, and a definition whose name differs from the signature directly above it is an error.
  Every `=` has its signature directly above it, with only comments between. A signature may stand
  alone where there is nothing to define, as `n : Size Unknown` does.

  ```text
  revenue : Formula Number style money revenue_label right 2
  revenue = base * (1 + rate) ^ year
  ```

- **The order of bindings in a file means nothing.** Two graphs are built from a file: the
  formulas' graph from the `=` lines, and the layout's graph from the `:` lines. Each is checked
  for cycles on its own (a balance carried from one period to the next is the formulas' one
  allowed cycle). The two are often alike, but need not be.
- **A signature starts with its role,** one word from a closed set. In 0.1 a binding's role
  follows from whether its signature has a label, so deleting a label silently turns a shown row
  into a local one; a stated role cannot drift like that.

  | Role | In the workbook | Formulas may use it |
  |---|---|---|
  | `Formula` | a formula, and a defined name | yes |
  | `Value` | a value, and a defined name | yes |
  | `Label` | a text value | no |
  | `Empty` | nothing: cells kept empty, whose columns and rows it may size | no |
  | `Local` | nothing of its own: a LET variable in each formula that uses it | yes |
  | `Lambda` | a defined name holding a LAMBDA | yes, by calling it |
  | `Style` | the look of the cells that name it | no |
  | `Size` | nothing: a size the checker tracks | in sizes |
  | `Sheet` | a worksheet | no |
  | `Workbook` | the workbook's own settings; exactly one per file | no |

- **A type follows the role.** The types of a cell are those of the app's TYPE function that
  concern a single value: `Number`, `Text` and `Logical`. An error is not a type: any formula may
  give one, and `na` gives `#N/A`. The type is required for `Lambda` and `Style`, optional but
  checked for `Formula`, `Value` and `Local`, and not written for `Label` (always `Text`),
  `Sheet` and `Workbook`. A type says what each cell holds, not the shape of the range.
- **A `Value` holds one number, string or logical.** Lists are not allowed: bulk values belong in
  input files, and an error value is made by a formula. Provisionally, a `Value` may instead be a
  table read from a CSV file, `csv "history.csv"`: many cells, each column typed from the file,
  and only the values written, not the row of column names.
- **There are no labels in signatures.** A label documenting a range is a `Label` binding of its
  own, a text cell like any other, placed beside the range. Headings are labels too. Generating
  labels from formulas, for concision, may come later.
- **A signature's parts after its role and type each start with a keyword**, `style money`,
  `size [1, n]`, `start [0, 1]`, and a place, `at forecast $B$2` or a binding and its moves, and
  come in any order; the printer writes them in one. A signature without a style takes the
  default. *Changed 2026-11-04: the parts first came in a fixed order, with a style or `_`
  required; see [Built for 0.2](#built-for-02).* `start` is not `from`, which is left to
  placement.
- **A lambda** has a curried type and its parameters on the left of its definition. Every call
  supplies every argument, and the checker counts them. Partial application is written as an
  explicit lambda for now; compiling it into a LAMBDA may come later.

  ```text
  discount : Lambda Number -> Number -> Number
  discount rate years = 1 / (1 + rate) ^ years
  ```

- **`lag 1 x seed v`** gives the first value of a lag, in place of 0.1's `from`.

### Layout

Each `Formula`, `Value` and `Label` says where it goes, in its signature: either at a cell, or
beside another binding. The sheet a binding is on is the sheet of the binding its placements lead
back to. 0.1 placed items in the order of the file; with that order gone, the reason 0.1 gave
against relative placement, recorded in
[Options considered and set aside](#options-considered-and-set-aside), no longer holds.

```text
horizon_label : Label style label at forecast $B$2
horizon       : Value Number horizon_label right 2
base_label    : Label style label horizon_label down
revenue       : Formula Number style money revenue_label right 2
growth        : Formula Number style percent revenue down 1 right 1 from west
```

- **A root** is placed with `at`, a sheet and a cell in A1 notation: `at forecast B2`, or
  `at forecast $B$2`, which means the same. Each sheet has at least one root.
- **A move** is `left`, `right`, `up` or `down`, with a distance that defaults to 1. A placement
  has at most one move along each axis, and their order does not matter.
- **Each move counts in cells from the reference's cell on the side it heads toward:** `right`
  from its east column, `left` from its west, `down` from its south row and `up` from its north.
  `from` after a move names another side: `east`, `west`, `north` or `south`. Along an axis with
  no move, the binding lines up with the reference's west column or north row. So `right 2` from
  a one-cell label skips a column, and `down 1 right 1 from west` is one row below and one column
  in from the reference's first column.
- **Only counting from the east or south side needs the reference's extent,** and that extent must
  be known when the file is written. Counting past an unknown extent is an error that names the
  size.
- **Alignment is checked, not done.** When two ranges placed one from the other share a size
  variable along an axis, position *k* of one should line up with position *k* of the other. A
  placement that does not is a warning: a flag turning warnings into errors may come later.
- Every `Formula`, `Value` and `Label` is placed exactly once, two bindings sharing a cell is an
  error, and a possible `#SPILL!` is a warning, as in 0.1.

### Sheets and the workbook

Sheets and the workbook are bindings, whose definitions are records of settings:

```text
workbook : Workbook
workbook = { sheets = [inputs, forecast], style = base }

forecast : Sheet
forecast = { name = 'Forecast', color = red, hidden = False, style = narrow }
```

- `Workbook` has `sheets`, the sheets in the order of their tabs, and `style`. There is exactly one
  per file, at least one sheet, and at least one sheet that is not hidden. There is no default
  sheet.
- `Sheet` has `name`, which follows the app's rules for sheet names, `color` (its tab's color),
  `hidden` and `style`.
- Both have `column_width` and `row_height`, which cascade like the font: a binding's `width`
  or `height`, then its sheet's, then the workbook's, then the app's default. *Added
  2026-11-04; see [Built for 0.2](#built-for-02).*
- **Styles cascade, for the font's name and size only.** A field a cell's style leaves out comes
  from its sheet's style, then from the workbook's, then from the app's default; a field that is
  set wins. The cascade fills gaps, where an update only adds. Any other field in a sheet's or the
  workbook's style is an error.

### Case

Lower case starts every name the author makes (bindings, parameters, styles, size variables), and
is for record fields and keywords (`if`, `at`, `right`, `from`, `east`). Upper case is for every
word the language defines as a value or a type: roles, types and constructors, such as `Formula`,
`Number`, `Fixed`, `Thin` and `True`. Stage 1 has no types of the author's own, so every upper-case
word comes from a closed list, and a misspelled one can be named in the error. `true` and `false`
become `True` and `False`.

### Styles

Nothing is styled implicitly: a value is blue only if its style says so. A style is a binding
whose role is `Style` and whose type is a record type. Records are strict: a comma between each
pair of fields, whitespace ignored, and a field that the type does not have is an error.

```text
light_grey : Style Color
light_grey = { red = 200, green = 200, blue = 200 }

money : Style Cell
money = { format = '#,##0;(#,##0)' }

one_above_two_below : Style Border
one_above_two_below =
  { top    = { weight = Thin,   color = light_grey }
  , bottom = { weight = Double, color = light_grey }
  }

grand_total : Style Cell
grand_total = money
  { font   = { bold = True }
  , border = one_above_two_below
  }
```

- **One style builds on another** by update, `money { … }`. An update merges field by field at
  every depth, and only adds: setting a field that already has a value, at any depth, is an error.
  So a base style can set a bottom edge and a variation add a top one, but no variation can
  change the base's bottom edge.
- **The record types,** and their fields:
  - `Cell`: `format` (a format code), `font` (a `Font`), `fill` (a `Color`), `border` (a
    `Border`) and `align` (an `Alignment`).
  - `Font`: `name` (`'Arial'`), `size` (in points), `bold` and `italic` (`True` or `False`), and
    `color` (a `Color`).
  - `Border`: `top`, `bottom`, `left` and `right`, each an `Edge`.
  - `Edge`: `weight` (`Thin`, `Medium`, `Thick`, `Double`, `Hair`, `Dotted` or `Dashed`) and
    `color` (a `Color`).
  - `Alignment`: `horizontal` (`Left`, `Center`, `Right` or `Justify`), `vertical` (`Top`,
    `Center` or `Bottom`), `wrap` (`True` or `False`) and `indent` (a whole number).
  - `Color`: `red`, `green` and `blue`, each a whole number from 0 to 255.

  A field left out takes the app's default. The values are checked against ECMA-376 Part 1:
  each weight is one of ST_BorderStyle's and each alignment one of ST_HorizontalAlignment's or
  ST_VerticalAlignment's (§18.18), a font's name has at most 31 characters (§18.8.29), and an
  indent goes only with a horizontal alignment of left or right (§18.8.1). A font's size is from
  1 to 409.55 points: the standard asks only for a positive size (§18.4.11), and [MS-OI29500]
  gives the spreadsheet app's range (Part 1 §18.4.11, note c). The app keeps a size in
  twentieths of a point (note b), so it may round one between them; `.yup` allows any. An indent
  is at most 255: the standard asks only for a whole number (§18.8.1), and [MS-OI29500] gives
  the app's limit (Part 1 §18.8.1, note b).
- **There is no built-in palette.** Colors are styles of type `Color` defined in the file. Sharing
  one palette between files waits for a way to import one file into another.
- **Single quotes hold settings and double quotes hold data.** Sheet names, format codes and font
  names go in single quotes; the text of a `Label`, a `Value` or a string in a formula goes in
  double quotes. The format code for text is `'@'`.
- **Out of scope:** underline, strikethrough, diagonal borders and cell protection. A column's
  width belongs to the column, not to a cell's style. *0.1 and the first 0.2 set widths
  themselves; from 2026-11-04 the file sets them, with `width` and `height` in a signature and
  `column_width` and `row_height` on a sheet or the workbook.*

### Statements over several lines

Inside `(`, `[` or `{` a newline does not end a statement, as in 0.1. In addition, a line whose
first token is `{` continues the statement above it, so a record can start on the line after
`money` or `=`. No statement starts with `{`, so the rule changes the meaning of nothing that
would otherwise be valid. An explicit marker at the end of a line was set aside: `...` is a range
in other languages and looks like `..`, and `\` is easily forgotten and looks like a lambda.

### Beyond financial models

Stage 1 began with financial models, and some of 0.1 assumed them: a timeline is always a row,
and `lag`, `tail`, `init` and carried balances always work along columns. In 0.2 a range made
with `..` and those functions work in either direction, and messages speak of positions, not
periods. How that is written is recorded in [Built for 0.2](#built-for-02).

### The fully elaborated form

After Ground, `yup show` prints a file with nothing left to work out: every binding's type, size
and start, and every placement as `at sheet CELL`. Style names stay names, and fixed sizes are
replaced by their values. Running the pipeline on it gives it back unchanged.

### Still open for 0.2

- The other values the standard allows and `.yup` does not offer yet: more weights of line (such
  as `mediumDashed`), and the alignments `general`, `fill`, `centerContinuous` and `distributed`.
- The syntax of an explicit lambda inside an expression.
- Whether a CSV table gets a role of its own, `Table`, rather than being a kind of `Value`.
- A flag that turns warnings into errors.

## Built for 0.2

Decided while building 0.2, from 2026-11-03, where the decisions above left something open. Each
is provisional. [docs/language.md](docs/language.md) describes 0.2 as it is built.

- **The first line** is `yup 0.2`, and a reader of 0.2 refuses 0.1: no file in 0.1 is kept.
- **The order of a signature's parts** is role, type, style, size, start and place.
  *Replaced 2026-11-04 by keywords, below.*
- **Keywords for a signature's parts** (decided 2026-11-04). After the role and the type, each
  part starts with a keyword: `style money`, `width 160`, `height 28`, `size [1, n]`,
  `start [0, 1]`, and a place, which starts with `at` or with the binding it is beside. Each part
  says what it is, so they may come in any order, and every step prints them in one: role, type,
  style, width, height, size, start, place.
  Liberal in what it reads and strict in what it writes, as in Postel's robustness principle, but
  with no ambiguity to tolerate: every order means the same. The style is optional, so `_` for
  the default style is gone, and `style` and `size` are keywords. Keywords are plain lower-case
  words, as `at` and `from` were already: a sigil, capitals, `=` or commas would each clash with
  something the language or the app already gives a meaning.
- **Directions.** `a .. b` is a row, and a column is `transpose (a .. b)`, which the workbook
  gets as a column of whole numbers rather than a row transposed. `lag`, `tail` and `init` work
  along a range's one dimension; a range of more than one row and column, and anything inside a
  lambda, says which with `across` or `down` straight after the function's name. A value carried
  from one position to the next runs along a row or down a column.
- **Moves left and up** land the binding's own last column or row at the count, so that it sits
  beside its reference rather than over it, and so need the binding's own extent.
- **The alignment check** applies along an axis with no move, or with a move that counts from the
  reference's own near side (`down 1 right 1 from west`). A move that steps off the reference
  along an axis puts the two side by side there, so it is not checked.
- **Place prints as Infer does.** A place that depends on a fixed size, such as row `s + 3`, has
  no A1 form; Ground prints every place as a cell.
- **`$` in a root's cell** (decided 2026-11-04). `at inputs $B$2` means the same as
  `at inputs B2`: a root is the cell everything else is placed from, and `$` marks it as one that
  never moves, as the app marks a reference that does not move when copied. Only the whole form is
  allowed: `$B2` or `B$2` would fix half a cell that never moves at all, so each is refused with
  both whole forms named. Every step keeps the `$` as written, Ground included, where the cells
  worked out from moves are written without it, so the roots still stand out. The examples write
  their roots with `$`. `$` anywhere else is an error.
- **Case in names** (decided 2026-11-04). A name starts with a lower-case letter, and upper case
  may follow, as in `revenue_Q1`: only the first letter tells a name from a word, and a letter
  such as a column's or a quarter's often carries meaning. Bindings become the workbook's defined
  names, which ignore case (ECMA-376 Part 1 §18.17.2.5), so two names that differ only in case are
  an error with both named, and so is a lambda's parameter that differs only in case from a name
  in the file. The rules for defined names are checked ignoring case: `taX2026` reads as a cell,
  and `sUm` is the function `sum`. A name is written one way everywhere; a use in another case is
  an unknown name, and the error gives its spelling. Names never meet the app's reserved names:
  the standard's own begin `_xlnm.` (§18.2.5), and [MS-OI29500] restricts every name that begins
  `_xl` (Part 1 §18.2.5, note m), where a `.yup` name begins with a letter. `History`, the sheet
  name the app keeps for itself, is a rule for sheet names, which are a sheet's `name` and not a
  binding's.
- **Rounding says its direction** (decided 2026-11-04). The app's names disagree about "up":
  ROUNDUP goes away from zero, and CEILING.MATH toward plus infinity, so ROUNDUP(-1.5,0) is -2
  where a ceiling is -1. `.yup` names each direction instead: `round` (the nearest, a half away
  from zero), `round_away` and `round_toward_zero` to digits, `floor` and `ceiling` to whole
  numbers, and `round_to`, `floor_to` and `ceiling_to` to multiples. `round_to` is written
  `SIGN(x)*FLOOR.MATH(ABS(x)+ABS(m)/2,ABS(m))` rather than with MROUND, which gives #NUM! when
  the number and the multiple differ in sign (ECMA-376 Part 1 §18.17.7.221) and, in the app,
  #VALUE! for a range where it takes a number. `mod d x` takes the sign of `d`, as MOD does.
  CEILING.MATH and FLOOR.MATH are written with the `_xlfn.` prefix of [MS-XLSX] §2.2.3, and
  ignore the sign of a multiple.
- **Dates** (decided 2026-11-04) are numbers, the serial days of the 1900 date system, which
  the workbook always uses (ECMA-376 Part 1 §18.2.28, `date1904` left at its default). The
  functions are `date_of`, `year_of`, `month_of` and `end_of_month`, so that `year`, `month`
  and `date`, the most natural names for a timeline's rows, stay free for bindings.
  `end_of_month k x` is written `DATE(YEAR(x),MONTH(x)+k+1,0)`, by DATE's rule that day 0 is the
  last day of the month before (§18.17.7.74), because the app's EOMONTH gives #VALUE! for a
  range of dates.
- **Discounting** (decided 2026-11-04). `npv r x`, `irr x`, `xnpv r dates x` and
  `xirr dates x` are the app's NPV, IRR, XNPV and XIRR, with the rate first as an argument that
  says how much. `npv` keeps the app's convention, which discounts the first cash flow by a whole
  period: a modeller reading the workbook sees NPV mean what it always means there. The docs show
  how to value a project whose first cash flow is today.
- **`first` and `last`** (decided 2026-11-04) give the first or last k positions of a range,
  written with TAKE. Their result starts at position 0 and stands alone, since a span starts at
  a whole number and the last k of a range of unknown length start at a size. `last 1 x` is a
  range's final value. The count is a whole number, and the range must have at least that many
  positions with every unknown size at 1, since TAKE gives fewer where there are fewer.
- **Conditional sums** (decided 2026-11-04). `sum_if c x` and `count_if c` take a condition
  that is a logical at each position, such as `region == "North" && year >= 2026`, lined up with
  `x`. The app's SUMIFS and COUNTIFS take criteria as text instead, where `">5"` is a comparison
  and `*`, `?` and `~` are wildcards in an equality, which is a trap for a value that holds them.
  They are written `SUM(IF(c,x,0))` and `SUM(IF(c,1,0))`. Writing SUMIFS where a condition fits
  its criteria exactly may come later.
- **Types are worked out** for every formula, value and local where its cells hold one type, and
  written into its signature from Infer on. Arithmetic on text is an error. `na` builds `#N/A`,
  whose type goes with any other.
- **A table's empty fields** are written as empty cells, with the table's style.
- **Styles of spilled cells.** Where a formula's extent is known, the cells it spills into are
  written as empty cells with its style. Where it is not, the style goes on the whole row or
  column the range grows along, and the cells before the range on that row or column are
  written with the default style. Past the end of such a spill, the row or column keeps the
  style: a fill or a border runs on to the edge of the sheet.
- **Empty columns.** A column that nothing is in, before the last column that something is in,
  is narrow, as a gap; a range of unknown width counts as being in every column from its first.
  *Replaced 2026-11-04 by widths and heights, below.*
- **Widths and heights** (decided 2026-11-04). Nothing is sized to fit: 0.1's widths from the
  longest text in a column, and 0.2's narrow empty columns, were guesses, and are gone. A
  column or row is as wide or as high as the file says, or else the app's default. `width 160`
  and `height 28` in a signature set every column and row the binding spans; `column_width` and
  `row_height` on a sheet set every other column and row of it, and on the workbook every other
  column and row of each sheet that sets none. A width is refused on a range of unknown width,
  and a height on one of unknown height. Two bindings that set the same column or row must
  agree, and only a disagreement is an error: what matters is the size each column ends up with,
  not which binding says so.
- **`Empty`** (decided 2026-11-04) is a role for cells that stay empty, so that a column or row
  that holds nothing can still be sized: `gap : Empty width 16 horizon_label right 1`. It is a
  signature alone, the only one besides an unknown size, and a definition below it is an error.
  It takes a width, a height, a size and a place, and no type, style or start. Its size is
  never worked out, so it is known when the file is written, and one cell without `size`. It
  claims its cells, as any range does, so Check refuses anything placed over them; formulas
  cannot use it, and other bindings may be placed beside it. The forecast example keeps a
  narrow gap column with one on each sheet.
- **Pixels** (decided 2026-11-04). Widths and heights are whole pixels at 100% scaling, the unit
  the app shows for both beside its own. A height converts to the file's points exactly, at 96
  pixels to 72 points. A width converts to the file's unit, digits of the default font with 5
  pixels of padding, through ECMA-376 Part 1 §18.3.1.13, which gives the default font's digit as
  7 pixels; the standard's way back to pixels gives every width from 1 to 1785 exactly. The
  limits are the app's: a width of at most 255 digits ([MS-OI29500], Part 1 §18.3.1.13, note c),
  so 1785 pixels, and a height of at most 409 points, so 545 pixels. A workbook whose style
  changes the font keeps each column's width in digits, so a column grows or shrinks with the
  font, as the app's own do (question 24).
- **Locals are LET variables** (question 13). A formula binds each local it uses, directly or
  through other locals, with LET under the local's own name, in the order they depend on each
  other, so that each is worked out once and the formula reads as the file does. A member of a
  carried cycle is still written in place, inside the scan, and a lambda with a parameter named
  as one of its locals writes that local in place, so that no LET hides the parameter.
- **The default font** stays as in 0.1, the typeface and size of the standard's own examples, and
  the workbook's style changes it.
- **Error messages** name what to write instead where the mistake is a habit from the
  spreadsheet app: `f(a, b)`, `SUM(x)`, a cell such as `B2` in a formula, `true` for `True`,
  `Input` for `Value`, and the statements of 0.1.

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
  segment, stretched across the periods, is a column of size `[s, 1]`, not a row of size `[1, s]`.
- Expressions over different size variables do not combine, except through a dimension of size 1.
- A lookup's result has the size and span of its key, not of the table. That is how values move
  from an input table of size `m` onto a timeline of size `n`.

### Layout

Spans are positions along a size variable. Layout decides where they land on a sheet. A range's
**anchor** is the top-left cell of its values. Each `Formula`, `Value` and `Label` is placed at a
cell, or beside another binding by moves that count from one of its sides (see
[Layout](#layout) under 0.2). A label is a binding of its own, so there is no label column or
gap built into the layout.

When two ranges placed one from the other share a size variable along an axis, their spans say
whether position *k* of one lines up with position *k* of the other. `growth`, with span
`[1, n)`, placed one column in from the west side of `revenue`, with span `[0, n)`, lines up;
the checker warns when a placement does not.

An anchor may depend on `Fixed` sizes (row `4 + s`) but never on `Unknown` ones. Counting from the
south side of a range of `n` rows, to place something below it, is refused.

### Sheets

A sheet is a binding whose definition is a record of its settings, and the workbook lists the
sheets in the order of their tabs:

```text
workbook : Workbook
workbook = { sheets = [inputs, forecast] }

forecast : Sheet
forecast = { name = 'Forecast' }
```

A binding's sheet is the sheet of the cell its placements lead back to. Bindings are
workbook-wide, so formulas name bindings, never sheets, and renaming a sheet is a one-line edit.
The checker enforces the app's rules for sheet names: the maximum length, the forbidden
characters, and uniqueness ignoring case.

### Fill and spill

A range of `Unknown` size must spill: it is one dynamic array formula at its anchor. Whether a range
of `Fixed` size spills or is filled with one formula per cell is open (question 14). *0.1 and 0.2:
everything spills.*

### Recurrences

A balance carried from one period to the next is natural in a hand-built workbook and awkward
under spill. Because horizons resize, it has to work under spill. In illustrative syntax:

```text
opening = lag 1 closing seed principal
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

*Built for 0.1, and kept in 0.2:* a cycle is allowed when `lag 1 x seed first` breaks it, and
has one balance. The balance is written as one SCAN over the positions, whose step works one
position out with the other members of the cycle written in place; the other members are
ordinary formulas that use the balance. In [the loan example](examples/loan.yup) the closing
balance is `SCAN(amount,SEQUENCE(1,COLUMNS(scheduled)),LAMBDA(_value,_position,_value-…))`, and
the spreadsheet app works it out as intended. In 0.2 a balance may also run down a column,
counted with `ROWS` rather than `COLUMNS`, as in [the stock example](examples/stock.yup).
Coupled balances are refused.

### Elementwise or aggregate

In `.yup`, `min scheduled opening` over two rows has to mean one minimum per period. Under spill,
the app's MIN over two ranges returns a single number, so the writer has to emit `IF(a<b, a, b)` or
a MAP instead. The function table says, for each function, whether it is elementwise or
aggregating.

### Names in written formulas

Exported bindings become defined names. Not decided: whether formulas in the workbook refer to
bindings by name or by A1 reference (question 13). Under spill, names read naturally. *0.1 and
0.2: by name.*

### Two kinds of failure

- **A file the app would offer to repair.** Always refused: the checker rejects any input the
  writer cannot write validly.
- **A possible `#SPILL!`.** The file is valid, but a spill may collide with something when the
  workbook calculates. The checker reports it; whether as an error or a warning is open
  (question 19). *0.1 and 0.2: a warning.*

## The pipeline

Each step takes one form of the model and produces the next.

1. **Parse** produces a syntax tree. It decides structure only. Errors: lexical and syntax.
2. **Resolve** produces the resolved model: what each name refers to, the formulas' dependency
   graph, the layout's graph, and each binding's sheet and style. Errors: an unknown or duplicate
   name, a signature without its definition, a circular reference, placements in a circle, a
   style that does not fit its type.
3. **Infer** produces the typed model: every expression's type, size and span, in size
   variables. Errors: a size or span mismatch, arithmetic on text, the wrong number of
   arguments, an aggregate where an elementwise function is needed.
4. **Place** produces the placed model: every anchor, worked out from the placements. Anchors may
   involve `Fixed` sizes. Errors: a move that counts from the far side of a range of `Unknown`
   extent, or a binding of `Unknown` extent placed left of or above another. A placement that
   does not line up is a warning.
5. **Ground** produces the grounded model by substituting the `Fixed` sizes. Every anchor is now a
   constant, and every extent a size expression in `Unknown` variables only. No errors: this is
   pure substitution.
6. **Check** looks for overlaps, ranges that run off the sheet, and possible `#SPILL!`, for every
   value of the `Unknown` variables, and changes nothing. Errors: a collision or a range off the
   sheet (refused); a possible spill (reported).
7. **Lower** produces cell contents. Range by range becomes cell by cell, and it writes the
   formula text: function names, `_xlfn.` prefixes, parentheses and A1 references. A failure here
   is a bug.
8. **Package** produces the xlsx bytes: the XML parts and the zip. A failure here is a bug.

Steps 1 to 6 are the checker; steps 7 and 8 are the writer.

**Spans are inferred at Infer,** with sizes, from the expressions alone and before any sheet or
coordinate exists. Place is the first step to use them.

**Coordinates are numbers inside the tool.** From Place on, a cell is a pair of numbers
`(row, column)`, counted from 1 as the app counts (A1 is `(1, 1)`). Before Ground an anchor can be
`(4 + s, 4)`, which has no A1 form, so Place prints each placement as it was written. From Ground
on every anchor is a constant, printed in A1 as `at forecast D4`, and Lower writes A1 references
into the formulas.

**Range by range becomes cell by cell at Lower.** A range that spills becomes one formula in its
anchor cell; the cells it spills into are left empty in the file, for the app to fill when it
calculates. A range that is filled (only possible at a `Fixed` size, and not built) would become
one formula per cell, with relative references. Every label becomes a text cell.

**Every step up to Check can print a valid `.yup`.** After Parse and Resolve that is a formatter;
Infer adds each binding's type, size and span; Place prints as Infer does; Ground puts every
binding at a cell and replaces each `Fixed` size by its value; Check adds its warnings as
comments. After Lower the content is cell-level formula text and no longer `.yup`. Two
requirements follow:

- The language has syntax for everything the steps add: sizes (`[1, n]`), spans
  (`start [0, 1]`) and cells (`at forecast D4`).
- Running the pipeline on an elaborated file changes nothing: elaborating twice gives the same
  file as elaborating once. That is a property the tests check.

## A worked example

In 0.2, with the workbook, the sheet and the two styles left out. Each label is a binding, placed
at a cell or below another label, and each range two columns to the right of its label.

```text
n : Size Unknown

horizon_label : Label at forecast $B$2
horizon_label = "Horizon (years)"

horizon : Value Number horizon_label right 2
horizon = 5

year_label : Label horizon_label down 2
year_label = "Year"

year : Formula Number size [1, n] year_label right 2
year = 1 .. horizon                          // n is decided in the workbook

revenue_label : Label year_label down
revenue_label = "Revenue"

revenue : Formula Number style money year down
revenue = 1000 * 1.1 ^ year                  // size [1, n], span [0, n)

growth_label : Label revenue_label down
growth_label = "Revenue growth"

growth : Formula Number style percent revenue down 1 right 1 from west
growth = tail revenue / lag 1 revenue - 1    // size [1, n - 1], span [1, n)
```

After Ground, every binding is at a cell, and each extent is in `Unknown` variables only:

```text
year    : Formula Number size [1, n] at forecast D4
revenue : Formula Number style money size [1, n] at forecast D5
growth  : Formula Number style percent size [1, n - 1] start [0, 1] at forecast E6
```

After Lower, on the sheet:

```text
     B                 C    D                       E                                   …
2    Horizon (years)        5
4    Year                   =SEQUENCE(1,horizon)                                         (spills)
5    Revenue                =1000*1.1^year                                               (spills)
6    Revenue growth                                 =DROP(revenue,,1)/DROP(revenue,,-1)-1  (spills)
```

D6 is empty by construction, and all three ranges end in the same column, D + n − 1, because all
three spans end at n. The file writes the newer functions with their `_xlfn.` prefix, left out
here.

## Core syntax

- **Option S1: like the spreadsheet app.** `EXP(x)`, `IF(a, b, c)`. Familiar to every LLM, which
  is the risk: authors will paste formulas from the app, A1 references included.
- **Option S2: like Python.** `exp(x)`. Familiar, and still close enough to the app to invite the
  same mistake.
- **Option S3: like Haskell or ML.** `exp x`, `if a then b else c`, `let … in …`, `\x y -> …` for
  a LAMBDA, and `discount r t = 1 / (1 + r) ^ t` as a named LAMBDA. It makes clear that `.yup` is
  a language of its own that is compiled to the app's formulas. **Decided on 2026-11-03.** *0.2
  has neither `let … in …` nor `\x y -> …` yet: a `Local` binding does the work of a let, and
  the syntax of a lambda inside an expression is still open.*

**Why S3.** A `.yup` file is written from scratch, chiefly by an LLM. Converting a workbook from
xlsx happens outside this repository, so familiarity with the app's formulas counts for less than
freedom from traps. The trap that matters most is deep nesting: LLMs lose count of closing
parentheses, above all in LAMBDA, LET and nested IF. Most of that nesting is the tool's to write,
not the author's: the range model and Lower turn the three flat lines of a loan balance into a SCAN
of a LAMBDA around a LET. S3 removes much of the rest. A function is defined without LAMBDA, a
call has no argument list and no commas, and `if … then … else` and `let … in …` end at a keyword
or a new line rather than at a counted `)`. Nested calls still need parentheses
(`round 2 (max a b)`). The habits S3 cannot prevent are caught instead: a call written as
`f(a, b)`, a function name in upper case and an A1 reference are refused, with their line and
column and, since 0.2, what to write instead.

Under S3 these need settling:

- **Optional and variadic arguments.** IF without an else, XLOOKUP's three optional arguments, SUM
  over many ranges. Candidates are a list literal for variadics (`sum [a, b, c]`), named arguments
  in a record, and separately named variants. The app also distinguishes an omitted argument from
  a zero.
- **Partial application.** The app's LAMBDA is not curried. The simplest rule is that every call is
  saturated and the checker checks arity; the alternative is to compile partial application into a
  LAMBDA. *0.2: every call is saturated, for now.*
- **Operators.** `=` binds, so equality needs another spelling (`==`, with `/=` or `!=`).
  Concatenation needs a spelling. Precedence is `.yup`'s own: the writer adds parentheses so that
  the app's quirks cannot change the meaning (`-2^2` is 4 in the app). *0.1 and 0.2: `==` and
  `/=`, and `++` joins text; `-2 ^ 2` is -4.*
- **Function names.** `.yup`'s names (lower case) map to the app's through a table, which also
  records each function's arity, optional arguments, whether it is elementwise or aggregating, and
  its encoding in the file (the `_xlfn.` prefix of the newer functions). That table is stage 1's
  function library.

Principles for easy lexing, which 0.1 and 0.2 follow:

- One statement per line. A newline inside brackets is not an end of statement (Python's rule),
  which a lexer handles with a bracket counter. No offside rule.
- Case carries meaning: bindings, functions and local variables are lower case
  (`forecast_horizon`); words at the type level start upper case (`Formula`, `Size`, `Fixed`), as in
  Haskell. A binding may not share a name with a function. Names that differ only in case are an
  error, since defined names in the app ignore case. *0.2: a name may have upper case after its
  first letter (`revenue_Q1`); record fields are lower case, and constructors, `True` and `False`
  among them, upper case.*
- Comments start with `//`, which means nothing in the app's formulas. `--` is Haskell's, but in
  the app it is double negation, used to turn logicals into numbers; `#` is the app's spill
  operator (`A1#`) and starts its error values (`#N/A`).
- Settings in single quotes, as the app quotes sheet names: sheet names, format codes and font
  names. A quote inside is doubled (`'Prior year''s revenue'`). Data in double quotes, with
  defined escapes: the text of a label or a value, and strings in formulas. *0.1 put labels in
  single quotes; 0.2 moved them to double quotes, as data.* Numbers in JSON's grammar, as in
  `.yup` 0.0.2.

## Options considered and set aside

- **Core syntax S1 and S2,** and **S-expressions.** See [Core syntax](#core-syntax) for S1 and S2.
  S-expressions have the simplest grammar of all, but they put every formula in nested
  parentheses, which LLMs miscount, and spreadsheet formulas written that way are unfamiliar.
- **Extent per range** (fill with an explicit extent, spill with the extent inferred, the row
  model, or a choice per range). Replaced by size variables, which infer extents and make the
  choice between fill and spill follow from the size.
- **Placement by the order of the file.** 0.1 put each range below the one before, and pinned a
  cell where that was not enough. Relative placement was set aside then, because between single
  ranges it restated the order and drifted from it. 0.2 sets the order aside instead, so that an
  LLM never has to work out where a range goes from where its line sits in the file: every
  binding is placed at a cell or beside another.
- **Named axes** (a range indexed by `period` rather than sized by `n`). Size variables describe
  the physical rows and columns, which is what layout needs; spans recover the alignment that axes
  would have given.

## Other open questions

Where 0.1 or 0.2 gives an answer, it follows in italics; [Built for 0.1](#built-for-01) and
[Decided for 0.2](#decided-for-02) have the reasons.

1. **The lag operator:** the seed for a lag of more than one, and the direction for a
   two-dimensional range. *0.1: along columns; `from` with `lag 1` only. 0.2: `seed` with `lag 1`
   only, along a range's one dimension, or `across` or `down` where written.*
2. **Name rules:** are sheet-scoped names ever needed? *0.1 and 0.2: every binding follows the
   rules for defined names, so that exporting one never forces a rename.*
3. **An escape hatch for raw formula text:** allowed, and if so how marked?
4. **Input files:** the CSV dialect and the JSON shape, how cells are typed (number, text, logical,
   blank), how a `.yup` binds an input to a range, and how an input table declares its size
   variable. *0.1 and 0.2: CSV in RFC 4180's dialect, `csv "file.csv"` in an input's (in 0.2, a
   `Value`'s) definition, and a size counted when the workbook is written. JSON is not built, and
   neither is a table that grows in the workbook.*
5. **Dates:** a literal syntax, written as serial numbers in the 1900 date system.
6. **Logicals and errors:** whether error literals exist, and how `and`, `or` and `not` are
   written. *0.1: `true` and `false`, and `&&`, `||` and `not`. 0.2: `True` and `False`, and `na`
   for `#N/A`; no other error literals.*
7. **Formatting syntax:** inline on each range, named styles applied to ranges, or both, and the
   defaults. *0.1: inline, `Format "…"`, `Bold` and `Italic` in the signature. 0.2: named styles
   only, as records, with the app's defaults for anything a style leaves out.*
8. **Validity without the app:** schema validation in the tests and reading by independent
   readers. Opening files in the spreadsheet app itself is a check made outside this repository.
   *0.1: the tests check that every part is well formed and listed, and that the zip reads back.*
9. **Dependencies:** `wasm-bindgen` for the web, and whatever the MCP server needs. *0.1 and 0.2:
   none; the zip is stored, with its own CRC-32.*
10. **The MCP server's tools:** check a file, print any step's `.yup`, write the xlsx.
11. **The file:** keep the `.yup` extension? Either way, the first line identifies the version, so
    each reader refuses the other format's files. *0.1: `.yup`, and the first line is `yup 0.1`.
    0.2: `.yup`, and `yup 0.2`; a reader of 0.2 refuses 0.1.*
12. **Names:** of the project and its crates.
13. **Exporting bindings:** whether a shown binding can stay unexported, and whether a local one
    becomes a LET variable rather than being repeated. *0.1: every shown binding is a defined
    name, formulas use those names, and a local binding is written where it is used. 0.2: a
    local is a LET variable in each formula that uses it.*
14. **Fixed sizes:** does a range of `Fixed` size spill, or is it filled with one formula per cell?
    *0.1 and 0.2: everything spills.*
15. **Recurrences under spill:** the rewrite into SCAN or REDUCE, and what to do with coupled
    balances. *0.1 and 0.2: one balance per cycle, written as a SCAN; coupled balances are
    refused, and so is anything in the cycle that does not work position by position. In 0.2 the
    balance may run down a column.*
16. **Signature and layout syntax:** the order and form of size, span, label, anchor and
    attributes in a signature; how blocks, orientation (across or down), two-dimensional blocks
    and pinned cells are written; and how an elaborated file writes spans and numeric anchors.
    *0.2: role and type, then style, width, height, size, start and place, each after its
    keyword and in any order, with no label; a place is a cell, `at forecast B2`, or moves from
    another binding; a range made with `..` is a row, and `transpose` makes it a column; the
    elaborated form writes `size [1, n]`, `start [0, 1]` and `at forecast D4`.*
17. **Pinned cells in the surface:** written in A1, as the author sees the workbook, or as numbers
    like every elaborated form? *0.1: numbers. 0.2: A1, in the surface and the elaborated form
    alike.*
18. **A file with no `sheet` statement:** a default sheet, or an error? *0.1: a sheet called
    `Sheet1`. 0.2: there is no default; every file has one `Workbook`, which lists at least one
    sheet.*
19. **A possible `#SPILL!`:** an error or a warning? *0.1 and 0.2: a warning.*
20. **Recording a spill of unknown extent:** *0.1 and 0.2: the anchor alone as the formula's
    range, with the dynamic array metadata, and no cached values. The spreadsheet app opens and
    spills it.*
21. **Time aggregation:** rolling monthly ranges up to annual ones (`sum revenue by year`), which
    maps one size variable onto another.
22. **The function library:** which of the app's functions stage 1 supports. *0.1: eighteen. 0.2:
    the fifty in [docs/language.md](docs/language.md).*
23. **Deterministic output:** does the same input always give byte-identical xlsx? *0.1 and 0.2:
    yes.*
24. **Widths with another font:** a width in pixels is exact only with the default font, whose
    digit width the standard gives (7 pixels for 11-point Calibri, ECMA-376 Part 1 §18.3.1.13).
    The published sources give no digit widths for other fonts, so with another, a column keeps
    its width in digits instead. Should a width then be refused, or given in digits too?
    *0.2: neither; the docs say so.*

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
