# The `.yup` language, version 0.2

This is a reference for the language as the tool implements it. It is provisional: the README
records why each choice was made and what is still open. Everything here is illustrated in
[the examples](../examples).

## A file

```text
yup 0.2

// A comment runs from // to the end of the line.

workbook : Workbook
workbook = { sheets = [forecast] }

forecast : Sheet
forecast = { name = 'Forecast' }

label : Style Cell
label = { format = '@' }

horizon_label : Label style label at forecast $B$2
horizon_label = "Forecast horizon (years)"

horizon : Value Number horizon_label right 2
horizon = 5

year_label : Label style label horizon_label down 2
year_label = "Year"

year : Formula Number year_label right 2
year = 1 .. horizon
```

- The first line is `yup 0.2`. A reader refuses any other version.
- A file is a list of bindings. Each is a signature, `name : …`, and directly below it, with
  only comments between, its definition, `name = …`. The name is written on both lines.
- The bindings can come in any order: neither what they mean nor where they go depends on it.
- One statement per line. Inside `(`, `[` or `{`, a newline does not end the statement, so a
  long expression can run over several lines. A line whose first token is `{` continues the
  statement above it, so a record can start on the line after its name or its `=`.
- Files are UTF-8. Line endings may be LF or CRLF.

## Roles

A signature starts with the binding's role, which says what the binding becomes.

| Role | In the workbook | Formulas may use it |
|---|---|---|
| `Formula` | a dynamic array formula, and a defined name | yes |
| `Value` | a value, or a table read from a CSV file, and a defined name | yes |
| `Label` | a text cell | no |
| `Empty` | nothing: cells kept empty, whose columns and rows it may size | no |
| `Local` | nothing of its own: a LET variable in each formula that uses it | yes |
| `Lambda` | a defined name holding a LAMBDA | yes, by calling it |
| `Style` | the look of the cells whose signatures name it | no |
| `Size` | nothing: a size the checker tracks | in sizes |
| `Sheet` | a worksheet | no |
| `Workbook` | the workbook's settings: exactly one per file | no |

## Signatures

A signature starts with the binding's role, then its type. Each part after them starts with a
keyword, and they come in any order: each says what it is, and a part given twice is an error.
Every step prints them in the order of this table.

| Part | Example | Which roles |
|---|---|---|
| role | `Formula` | all |
| type | `Number`, `Number -> Number`, `Cell` | see below |
| style | `style money` | `Formula`, `Value`, `Label`: optional; without one, the default |
| width | `width 160` | `Formula`, `Value`, `Label`, `Empty`: optional |
| height | `height 28` | `Formula`, `Value`, `Label`, `Empty`: optional |
| size | `size [1, n - 1]` | `Formula`, `Value`, `Label`, `Local`: optional, checked; `Empty` |
| start | `start [0, 1]` | `Formula`, `Value`, `Label`, `Local`: optional, checked |
| place | `at forecast $B$2`, `revenue down 2` | `Formula`, `Value`, `Label`, `Empty`: required |

The type of a cell is `Number`, `Text` or `Logical`, the kinds of value the spreadsheet app's TYPE
function tells apart. It is optional, and checked, for `Formula`, `Value` and `Local`, and not
written for `Label`, which always holds text, or for `Sheet` and `Workbook`. A `Lambda` has a
type that names its parameters and its result, `Number -> Number -> Number`, and a `Style` names
the kind of record it is: `Cell`, `Font`, `Border`, `Edge`, `Alignment` or `Color`.

A size is `Size Fixed`, with a definition, `s = 4`, or `Size Unknown`, which stands alone: the
workbook decides its value.

```text
segments : Size Fixed
segments = 3

n : Size Unknown
year : Formula Number size [1, n] year_label right 2
year = 1 .. horizon
```

## Values, labels, formulas and locals

- A **value** is one number, string, `True` or `False`, written in the file. Many values belong
  in an input file: `history = csv "history.csv"` reads a CSV file beside the `.yup` file into a
  table of values.
- A **label** is its text, in double quotes: `year_label = "Year"`. Formulas cannot use it.
- A **formula** is an expression, written into the workbook as one dynamic array formula.
- A **local** is an expression with no cells of its own. A formula that uses it, directly or
  through other locals, binds it with LET under its own name, so that it is worked out once:
  `LET(_xlpm.growth_factor,1+revenue_growth,base_revenue*_xlpm.growth_factor^year)`. In a lambda
  with a parameter of the same name as a local it uses, the local is written in place instead,
  so that the LET cannot hide the parameter.

An error, such as `#N/A`, is not a type: any formula may give one, and `na` gives `#N/A`.

A CSV file's first row names its columns, and every other row is a record with one field for
each column. Fields are separated by commas; a field in double quotes can hold commas, line
breaks and doubled quotes. An unquoted field is typed: empty is blank, a number in JSON's
grammar is a number, `TRUE` and `FALSE` in any case are logicals, and anything else is text. A
quoted field is always text. The table takes as many rows as the file has records, counted when
the workbook is written, and only its values are written: labels name its columns.

`column "units" history` is one column of a table.

## Lambdas

```text
discount : Lambda Number -> Number -> Number
discount rate years = 1 / (1 + rate) ^ years

discount_factor : Formula Number style factor factor_label right 2
discount_factor = discount discount_rate year
```

A lambda has no cells. Each call has the size its arguments give it, worked out where the call
is made, and every call gives every argument. Inside the body, a parameter hides any binding of
the same name. In the workbook a lambda is a defined name holding a LAMBDA, so it can be called
there too, and a call is written as the app writes one: `discount(discount_rate,year)`.

Inside a lambda, `lag`, `tail` and `init` say which way they work, `across` or `down`: the
lambda is written once, however it is called.

## Values carried from one position to the next

```text
opening : Formula Number style money opening_label right 2
opening = lag 1 closing seed amount

repayment : Formula Number style money opening down
repayment = min scheduled opening

closing : Formula Number style total repayment down
closing = opening - repayment
```

Bindings may use each other in a cycle when `lag 1 x seed first` breaks it: each position then
depends only on the position before, and the first on `first`. The binding that is lagged,
`closing` here, is the value the cycle carries. In 0.2:

- a cycle carries one value, in one row or one column;
- inside the cycle, `lag` is always `lag 1 value seed first`, with the same `first` throughout;
- what takes part in the cycle works position by position: operators, `if`, `min`, `max`, `abs`,
  `exp`, `ln`, `sqrt`, `round` and `not`, but not `sum`, `tail` or a lambda;
- the cycle uses a row or a column from outside it, such as `scheduled` above, which gives its
  length;
- a range made with `..` is not inside the cycle.

In the workbook the carried value is a SCAN over the positions. Its step works out one position
from the value a position back, with the rest of the cycle written in place; the other members
of the cycle are ordinary formulas that use it. The workbook holds no circular reference.

## Expressions

| Form | Example |
|---|---|
| numbers, in JSON's grammar | `12`, `0.05`, `1.5e3` |
| strings, in double quotes | `"Retail"`, with escapes `\"`, `\\`, `\/`, `\n`, `\t`, `\uXXXX` |
| logicals | `True`, `False` |
| lists of literals, in formulas | `[1, 2, 3]`, a row; `[1; 2; 3]`, a column; `[1, 2; 3, 4]` |
| whole numbers from a to b | `1 .. horizon`, a row; `transpose (1 .. horizon)`, a column |
| conditions | `if a > b then a else b` |
| function application | `lag 1 revenue`, `round 2 x`, `tail down x` |

A string in a formula has at most 255 characters, the spreadsheet app's limit; a longer text can
be a value.

Operators, from loosest to tightest:

| Operators | Grouping |
|---|---|
| `if … then … else …`, `..` | |
| `\|\|` (logical or) | right |
| `&&` (logical and) | right |
| `==`, `/=`, `<`, `<=`, `>`, `>=` | do not chain |
| `++` (joins text) | right |
| `+`, `-` | left |
| `*`, `/` | left |
| `-` (negation) | |
| `^` | right |
| function application | left |

So `-2 ^ 2` is -4, and `tail revenue / lag 1 revenue - 1` is `((tail revenue) / (lag 1
revenue)) - 1`. Arithmetic on text is an error; text is joined with `++`.

The spreadsheet app's habits are refused with a hint: `discount(rate, years)` takes its
arguments without parentheses or commas, `SUM(x)` is `sum x`, `B2` is a cell where a formula
names bindings, and `true` is `True`.

## Built-in functions

Arguments that say how much come first, and the data last. Each argument is checked against
what the function takes: text goes only where text does, and a number or a logical where
either does, as in arithmetic.

| Function | Result | Written as |
|---|---|---|
| `lag k x` | x moved k positions along, so k shorter | `DROP(x,,-k)` |
| `lag 1 x seed v` | x moved one position along, with v first | `HSTACK(v,DROP(x,,-1))` |
| `tail x` | x without its first position, so starting one later | `DROP(x,,1)` |
| `init x` | x without its last position | `DROP(x,,-1)` |
| `first k x`, `last k x` | the first or last k positions of x | `TAKE(x,,k)`, `TAKE(x,,-k)` |
| `sum x`, `average x` | one value from all of x | `SUM(x)`, `AVERAGE(x)` |
| `minimum x`, `maximum x` | one value from all of x | `MIN(x)`, `MAX(x)` |
| `sum_if c x`, `count_if c` | the sum of x, or the count, where c holds | `SUM(IF(c,x,0))` |
| `sum_columns x` | the sum of each column, as one row | `BYCOL(x,LAMBDA(x,SUM(x)))` |
| `sum_rows x` | the sum of each row, as one column | `BYROW(x,LAMBDA(x,SUM(x)))` |
| `min a b`, `max a b` | the smaller or larger at each position | `IF(a<b,a,b)` |
| `abs x`, `exp x`, `ln x`, `sqrt x` | the function at each position | `ABS(x)` and so on |
| `not x` | logical not at each position | `NOT(x)` |
| `round d x` | x rounded to d digits, the nearest, a half away from zero | `ROUND(x,d)` |
| `round_away d x` | x rounded to d digits, away from zero | `ROUNDUP(x,d)` |
| `round_toward_zero d x` | x rounded to d digits, toward zero | `ROUNDDOWN(x,d)` |
| `floor x`, `ceiling x` | a whole number, toward minus or plus infinity | `INT`, `CEILING.MATH` |
| `round_to m x` | the nearest multiple of m, a half away from zero | `SIGN(x)*FLOOR.MATH(…)` |
| `floor_to m x`, `ceiling_to m x` | a multiple of m, toward minus or plus infinity | `FLOOR.MATH` |
| `mod d x` | the remainder of x divided by d, with the sign of d | `MOD(x,d)` |
| `transpose x` | rows and columns swapped | `TRANSPOSE(x)` |
| `all x`, `any x`, `xor x` | one logical from all of x | `AND(x)`, `OR(x)`, `XOR(x)` |
| `if_error v x` | x, or v at each position where x is an error | `IFERROR(x,v)` |
| `trim x` | text without spaces at either end, and one between words | `TRIM(x)` |
| `upper x`, `lower x` | text in upper or lower case | `UPPER(x)`, `LOWER(x)` |
| `exact a b` | whether two texts are the same, case and all; `==` ignores case | `EXACT(a,b)` |
| `date_of y m d` | the date of a year, month and day | `DATE(y,m,d)` |
| `year_of x`, `month_of x` | the year or month of a date | `YEAR(x)`, `MONTH(x)` |
| `end_of_month k x` | the last day of the month k months after x | `DATE(YEAR(x),MONTH(x)+k+1,0)` |
| `lookup k ks vs` | the value beside each key, shaped as `k` | `XLOOKUP(k,ks,vs)` |
| `column "c" t` | the column called `c` of the table `t` | `INDEX(t,0,j)` |
| `na` | the error `#N/A` | `NA()` |
| `csv "file.csv"` | a table read from a file, as the whole of a value's definition | its cells |

`first` and `last` give k positions that stand alone, starting at position 0, rather than lining
up with the range: `last 1 revenue` is the last revenue, one value. The range must have at
least k positions however its unknown sizes turn out.

`lag`, `tail`, `init`, `first` and `last` work along a range's one dimension when it has only
one: across a row, from column to column, or down a column, from row to row, where they are
written `DROP(x,-k)` and so on, and a seed is stacked with `VSTACK`. A range of more than one row
and column needs the direction written straight after the function's name: `tail across grid`,
`lag down 1 grid`.

A date is a number: the serial day of the 1900 date system, in which the workbook is always
written, so 1 January 2026 is 46023. `date_of` lets its month and day run over, so
`date_of 2026 13 0` is 31 December 2026, and reads a year from 0 to 99 as 1900 and that year, as
the app does. `end_of_month` is written with DATE because the app's EOMONTH gives `#VALUE!` for a
range of dates.

`min` and `max` work position by position, like Haskell's; `minimum` and `maximum` reduce a range
to one value. In the same way, `&&` and `||` work position by position, and `all`, `any` and
`xor` reduce a range of logicals to one, as the app's AND, OR and XOR do. So `&&` and `||` are
written with arithmetic on the logicals: `a && b` as `(a)*(b)<>0`, and `a || b` as
`(a)+(b)<>0`.

## Sizes and spans

Every expression has a size in rows and columns, and a span: where it sits along them.

- A number, string or logical is one cell.
- `a .. b` is one row. If `b` is a whole number or a fixed size, its length is known. Otherwise
  it introduces a new unknown size, decided in the workbook. Two ranges made by two `..` have
  different sizes, even if they look the same: make every range on a timeline from one `..`.
- A signature can name an unknown size: `year : Formula Number size [1, n] …` with
  `n : Size Unknown`. The name is checked, and each declared size names exactly one `..`. An
  unknown size no signature names is called after the binding that made it, such as `year_n`.
- Operators and functions that work position by position need operands of the same size and
  span. A dimension of size 1 stretches to match the other operand, as in the spreadsheet app, so
  a column of segments times a row of years is a grid.
- `tail revenue` and `init revenue` have the same size but different spans, so `tail revenue /
  init revenue` is refused as misaligned; `tail revenue / lag 1 revenue` is accepted.

## Layout

Each formula, value and label says where its cells go, in its signature. The top-left cell of
its values is its anchor.

- `at forecast B2` puts the anchor at a cell of a sheet, in A1 notation. Each sheet has at least
  one binding placed this way. `at forecast $B$2` means the same, and marks the cell as one that
  never moves; `$` goes before both the column and the row or before neither, and nowhere else
  in a file. Every step prints it as it was written.
- `revenue_label right 2` puts it beside another binding. A move is `left`, `right`, `up` or
  `down`, with a distance that defaults to 1, and a placement has at most one move along each
  axis, in either order.
- Each move counts in cells from the reference's cell on the side it heads toward: `right` from
  its east column, `left` from its west, `down` from its south row and `up` from its north.
  `from east`, `from west`, `from north` or `from south` after a move counts from that side
  instead. A move right or down lands the binding's own first column or row there; a move left
  or up lands its last.
- Along an axis with no move, the binding lines up with the reference's west column or north
  row.

So, with `revenue` a row at D8, `revenue_label right 2` from a one-cell label at B8 is D8, and
`revenue down 1 right 1 from west` is E9: one row below `revenue` and one column in from its
first.

Counting from the east or south side of a range needs its extent when the file is written, so
counting past a range of unknown width or height is refused. A binding placed left of or above
another needs its own extent known too.

The sheet a binding is on is the sheet of the cell its placements lead back to. Placements that
go round in a circle are refused.

Check refuses two bindings that would share a cell, and warns when a range would spill into
something once an unknown size grows: the workbook would show `#SPILL!` there. It also warns
when two ranges placed one from the other share a size along an axis but do not line up along
it, position by position, unless the placement steps off the reference along that axis.

## Widths and heights

Nothing is sized to fit its contents: a column is as wide, and a row as high, as the file says,
or else the app's default.

```text
workbook = { sheets = [inputs, forecast], column_width = 80, row_height = 20 }
forecast = { name = 'Forecast', column_width = 96 }
horizon_label : Label style label width 160 at forecast $B$2
```

- Widths and heights are whole numbers of pixels at 100% display scaling, as the app shows them
  beside its own units when a column or row is resized.
- `width` in a signature sets every column the binding spans, and `height` every row. A width is
  refused on a range whose columns are unknown when the file is written, and a height on one
  whose rows are.
- Two bindings may set the same column, or the same row, if they agree; if they differ, Check
  refuses them and names both.
- A sheet's `column_width` and `row_height` size every column and row that no binding sets. The
  workbook's apply to each sheet that does not set its own, and the app's defaults to the rest.
- A column is from 1 to 1785 pixels wide and a row from 1 to 545 pixels high: the app's limits of
  255 characters and 409 points.
- An `Empty` sizes columns and rows that hold nothing, such as a narrow gap between labels and
  numbers: `gap : Empty width 16 size [1, 2] horizon_label right 1`. It is a signature alone,
  with no definition and no type or style. Its size is one cell without `size`, and is known
  when the file is written: it may name a fixed size, but not an unknown one. Nothing else may
  be placed over its cells, formulas cannot use it, and other bindings may be placed beside it.
- The file counts a column's width in digits of the default font, so a width in pixels is
  exact with the default font, 11-point Calibri, whose digits are 7 pixels wide. With another, a
  column keeps its width in digits, and grows or shrinks with the font, as the app's own
  columns do. A row's height is in points in the file, and is always exact.

## Styles

A style is a binding whose role is `Style` and whose definition is a record of its type:

```text
light_grey : Style Color
light_grey = { red = 200, green = 200, blue = 200 }

money : Style Cell
money = { format = '#,##0' }

grand_total : Style Cell
grand_total = money
  { font   = { bold = True }
  , border = { top = { weight = Thin }, bottom = { weight = Double, color = light_grey } }
  }
```

The types of style, and their fields:

- `Cell`: `format` (a format code), `font` (a `Font`), `fill` (a `Color`), `border` (a `Border`)
  and `align` (an `Alignment`).
- `Font`: `name` (`'Arial'`), `size` (in points), `bold` and `italic` (`True` or `False`), and
  `color` (a `Color`).
- `Border`: `top`, `bottom`, `left` and `right`, each an `Edge`.
- `Edge`: `weight` (`Thin`, `Medium`, `Thick`, `Double`, `Hair`, `Dotted` or `Dashed`) and
  `color` (a `Color`).
- `Alignment`: `horizontal` (`Left`, `Center`, `Right` or `Justify`), `vertical` (`Top`,
  `Center` or `Bottom`), `wrap` (`True` or `False`) and `indent` (a whole number).
- `Color`: `red`, `green` and `blue`, each a whole number from 0 to 255.

- Every field is optional; a field left out takes the app's default, and a color component
  left out is 0.
- A record has a comma between each pair of fields, a field at most once, and only the fields
  of its type. A field's value is a record, or a style of the field's type by name.
- `base { … }` builds a style on another. It merges field by field at every depth and only
  adds: setting a field that the base has already set, at any depth, is an error.
- A formula, value or label may name a style of type `Cell`, as in `style money`; without one,
  its cells take the default.
- A font's name has at most 31 characters, and its size is from 1 to 409.55 points; the app keeps
  a size in twentieths of a point, so it may round one between them. An indent is at most 255,
  and goes only with a horizontal alignment of `Left` or `Right`.
- A format code is checked conservatively: literal text in it goes in double quotes, and the
  format for text is `'@'`.

## Sheets and the workbook

```text
workbook : Workbook
workbook = { sheets = [inputs, forecast], style = narrow }

forecast : Sheet
forecast = { name = 'Forecast', color = red, hidden = False, style = big }
```

- The workbook lists every sheet once, in the order of the tabs, and at least one sheet is not
  hidden. There is no default sheet.
- A sheet's name follows the spreadsheet app's rules: 1 to 31 characters, none of
  `: \ / ? * [ ]`, not starting or ending with `'`, not `History`, and unique ignoring case.
- `color` is the color of the sheet's tab.
- `column_width` and `row_height`, on a sheet or the workbook, are the width of every column
  and the height of every row that no binding sets, in pixels; a sheet's win over the
  workbook's. See [Widths and heights](#widths-and-heights).
- The style of a sheet or of the workbook sets only the font's name and size. A cell's style
  that leaves either out takes it from its sheet's style, then from the workbook's, then from
  the app's default.

## Case and quotes

- A name the author makes (a binding, parameter, style or size) starts with a lower-case letter,
  then has letters in either case, digits and `_`, as in `revenue_Q1`. Names are ASCII. Record
  fields and keywords are lower case.
- Upper case is for the language's own words: roles, types and constructors, such as `Formula`,
  `Number`, `Thin` and `True`. Only the first letter tells a name from a word.
- Every name follows the rules for defined names, so that exporting a binding never forces a
  rename. A name cannot read as a cell (`tax2026`) or as R1C1 notation (`r`, `c`, `r1c1`), and
  cannot be the name of a built-in function or a keyword: `across`, `at`, `down`, `east`,
  `else`, `false`, `from`, `height`, `if`, `left`, `north`, `right`, `seed`, `size`, `south`,
  `start`, `style`, `then`, `true`, `up`, `west`, `width` or `yup`.
- Defined names ignore case, so these rules do too: `taX2026` reads as a cell, and `sUm` is the
  function `sum`. Two names that differ only in case are an error, as is a lambda's parameter
  that differs only in case from a name in the file. A name is written the same way everywhere:
  a use in another case is an unknown name, and the error gives its spelling.
- Single quotes hold settings: sheet names, format codes and font names. A quote inside is
  doubled: `'Year''s end'`. Double quotes hold data: the text of a label or a value, and strings
  in formulas.

## The steps, and what `yup show` prints

| Step | `yup show <step>` prints |
|---|---|
| parse | the file, formatted |
| resolve | the same |
| infer | the same, with each binding's type, size and span, and the unknown sizes declared |
| place | the same as infer: a place that depends on a fixed size has no A1 form |
| ground | the same, with every place a cell and fixed sizes replaced by their values |
| check | the same, with any warnings as comments |
| lower | the cells and defined names, in A1 notation |
| package | the parts of the xlsx file |

Every step up to Check prints a valid `.yup` file, and running the same step on what it printed
changes nothing. What Ground prints is fully elaborated: nothing is left to work out but what the
workbook decides.

## The workbook

- Each formula is one dynamic array formula in its anchor cell; the cells it spills into are
  filled by the spreadsheet app when it calculates. The file caches no values and asks to be
  calculated in full when it opens.
- Each value is written cell by cell.
- Each formula and value is a defined name. A spilled range's name covers the whole spill, which
  the app shows as `D4#`.
- The cells a formula spills into take its style. Where the extent is known, they are written as
  empty cells with that style. Where it is not, the style goes on the whole row or column the
  range grows along, and the cells before the range on it keep the default.
- A column or row has the width or height the file gives it, or else the app's default:
  nothing is sized to fit what is in it.
- The same file always gives the same bytes.
