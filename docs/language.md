# The `.yup` language, version 0.1

This is a reference for the language as the tool implements it. It is provisional: the README
records why each choice was made and what is still open. Everything here is illustrated in
[the examples](../examples).

## A file

```text
yup 0.1

// A comment runs from // to the end of the line.

sheet 'Inputs'

horizon : 'Forecast horizon (years)' Input
horizon = 5

sheet 'Forecast'

year : 'Year'
year = 1 .. horizon
```

- The first line is `yup 0.1`. A reader refuses any other version.
- One statement per line. Inside `(` or `[`, a newline does not end the statement, so a long
  expression can run over several lines.
- Files are UTF-8. Line endings may be LF or CRLF.

## Statements

| Statement | Meaning |
|---|---|
| `sheet 'Name'` | What follows goes on this sheet; the tabs come in this order. |
| `heading 'Text'` | A bold line of text in the layout. |
| `gap` | An empty row in the layout. |
| `n : Size Unknown` | Declares a size decided in the workbook. |
| `s : Size Fixed` with `s = 4` | Declares a size known when the file is written. |
| `name : …` | A signature: the label, size, place and look of a binding. |
| `name = expression` | A definition. |

Items written before any `sheet` statement go on a sheet called `Sheet1`.

A sheet name follows the spreadsheet app's rules: 1 to 31 characters, none of `: \ / ? * [ ]`, not
starting or ending with `'`, not `History`, and unique ignoring case.

## Bindings

A binding is a name with a definition. What its signature says decides what it becomes:

| Signature | Role | In the workbook |
|---|---|---|
| has a label and `Input` | input | its values, in cells the reader can change, in blue |
| has a label | shown | one formula in its anchor cell, spilling as far as it needs |
| no label, or none at all | local | nothing: its formula goes into the formulas using it |

Every shown binding and input becomes a workbook-scoped defined name, and formulas in the workbook
refer to them by those names.

Names are lower case ASCII: a letter, then letters, digits and `_`. Every name follows the rules
for defined names, so that showing a binding never forces a rename. A name cannot read as a cell
(`tax2026`) or as R1C1 notation (`r`, `c`, `r1c1`), and cannot be the name of a built-in function
or a keyword (`if`, `then`, `else`, `from`, `sheet`, `heading`, `gap`, `yup`, `true`, `false`).

An input's definition is written out in the file: a number, a string, `true`, `false`, or a list
of these.

## Signatures

A signature takes any of these parts, in any order:

| Part | Example | Meaning |
|---|---|---|
| label | `'Revenue growth'` | The text beside the values. Any Unicode; a quote inside is doubled. |
| size | `[1, n - 1]` | Rows and columns. Checked against what the definition gives. |
| start | `from [0, 1]` | Where the range starts along its rows and columns. Checked. |
| anchor | `@ (4, 5)` | Pins the top-left cell of the values, as `(row, column)` counted from 1. |
| `Input` | | The values are written into cells for the reader to change. |
| `Format "…"` | `Format "0.0%"` | A number format. Literal text goes in double quotes inside it. |
| `Bold`, `Italic` | | The label and the values are bold or italic. |

Sizes and anchors are written as a whole number plus size variables with positive coefficients:
`n`, `n - 1`, `2 * s + 4`.

## Expressions

| Form | Example |
|---|---|
| numbers, in JSON's grammar | `12`, `0.05`, `1.5e3` |
| strings, in double quotes | `"Retail"`, with escapes `\"`, `\\`, `\/`, `\n`, `\t`, `\uXXXX` |
| logicals | `true`, `false` |
| lists of literals | `[1, 2, 3]` is a row; `[1; 2; 3]` is a column; `[1, 2; 3, 4]` a grid |
| whole numbers from a to b | `1 .. horizon` |
| conditions | `if a > b then a else b` |
| function application | `lag 1 revenue`, `round 2 x` |

A string in a formula has at most 255 characters, the spreadsheet app's limit; a longer text can
be an input.

Operators, from loosest to tightest:

| Operators | Grouping |
|---|---|
| `if … then … else …`, `..` | |
| `==`, `/=`, `<`, `<=`, `>`, `>=` | do not chain |
| `++` (joins text) | right |
| `+`, `-` | left |
| `*`, `/` | left |
| `-` (negation) | |
| `^` | right |
| function application | left |

So `-2 ^ 2` is -4, and `tail revenue / lag 1 revenue - 1` is `((tail revenue) / (lag 1
revenue)) - 1`.

## Built-in functions

Arguments that say how much come first, and the data last.

| Function | Result | Written as |
|---|---|---|
| `lag k x` | x moved k columns along, so k columns shorter | `DROP(x,,-k)` |
| `lag 1 x from v` | x moved one column along, with v first | `HSTACK(v,DROP(x,,-1))` |
| `tail x` | x without its first column, so starting one later | `DROP(x,,1)` |
| `init x` | x without its last column | `DROP(x,,-1)` |
| `sum x`, `average x` | one value from all of x | `SUM(x)`, `AVERAGE(x)` |
| `minimum x`, `maximum x` | one value from all of x | `MIN(x)`, `MAX(x)` |
| `sum_columns x` | the sum of each column, as one row | `BYCOL(x,LAMBDA(x,SUM(x)))` |
| `sum_rows x` | the sum of each row, as one column | `BYROW(x,LAMBDA(x,SUM(x)))` |
| `min a b`, `max a b` | the smaller or larger at each position | `IF(a<b,a,b)` |
| `abs x`, `exp x`, `ln x`, `sqrt x` | the function at each position | `ABS(x)` and so on |
| `round d x` | x rounded to d digits | `ROUND(x,d)` |
| `transpose x` | rows and columns swapped | `TRANSPOSE(x)` |
| `lookup k ks vs` | the value beside each key, shaped as `k` | `XLOOKUP(k,ks,vs)` |

`min` and `max` work position by position, like Haskell's; `minimum` and `maximum` reduce a range
to one value.

## Sizes and spans

Every expression has a size in rows and columns, and a span: where it sits along them.

- A number, string or logical is one cell.
- `a .. b` is one row. If `b` is a whole number or a fixed size, its length is known. Otherwise
  it introduces a new unknown size, decided in the workbook. Two ranges made by two `..` have
  different sizes, even if they look the same: make every range on a timeline from one
  `..`.
- A signature can name an unknown size: `year : [1, n] 'Year'` with `n : Size Unknown`. The name
  is checked, and each declared size names exactly one `..`.
- Operators and functions that work position by position need operands of the same size and
  span. A dimension of size 1 stretches to match the other operand, as in the spreadsheet app, so
  a column of segments times a row of years is a grid.
- `tail revenue` and `init revenue` have the same size but different spans, so `tail revenue /
  init revenue` is refused as misaligned; `tail revenue / lag 1 revenue` is accepted.

## Layout

Each sheet is laid out in the order of the file. Row 1 is left empty, the first item goes in
row 2, and each item goes below the one before:

- a heading takes a row, its text in column B;
- a gap takes a row;
- a shown binding or input takes as many rows as it has, its label in column B and position 0 of
  its columns in column D, leaving column C empty. A range that starts later (`from [0, 1]`)
  starts that many columns further right, so the years line up.

An anchor may depend on fixed sizes but not on unknown ones, so nothing can be placed below a
range of unknown height unless it is pinned with `@`. Anything pinned is placed where it says,
and the items after it follow below it.

Check refuses two things that would share a cell, and warns when a range would spill into
something once an unknown size grows: the workbook would show `#SPILL!` there.

## The steps, and what `yup show` prints

| Step | `yup show <step>` prints |
|---|---|
| parse | the file, formatted |
| resolve | the same, with the default sheet written out |
| infer | the same, with each binding's size and span, and the unknown sizes declared |
| place | the same, with every anchor, in `(row, column)` that may depend on fixed sizes |
| ground | the same, with fixed sizes replaced by their values |
| check | the same, with any warnings as comments |
| lower | the cells and defined names, in A1 notation |
| package | the parts of the xlsx file |

Every step up to Check prints a valid `.yup` file, and running the same step on what it printed
changes nothing.

## The workbook

- Each shown binding is one dynamic array formula in its anchor cell; the cells it spills into
  are filled by the spreadsheet app when it calculates. The file caches no values and asks to be
  calculated in full when it opens.
- Each input is written cell by cell, in blue.
- Each shown binding and input is a defined name. A spilled range's name covers the whole spill,
  which the app shows as `D4#`.
- A number format applies to the anchor and to the rows the range spills along.
- The same file always gives the same bytes.
