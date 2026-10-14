//! Step 4, Place: where each item goes on its sheet, as `(row, column)`, both counted from 1.
//!
//! Layout follows the order of the file: each item goes below the one before. Position 0 of a
//! range's columns lands in column 4, and the range's label two columns to the left of that,
//! leaving an empty column between label and values. A range's anchor is the top-left cell of
//! its values, not its label.
//!
//! An anchor may depend on fixed sizes but never on unknown ones, so nothing goes below a range
//! whose height is unknown unless it is pinned with `@`.

use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Pos};
use crate::infer::{Typed, VarKind};
use crate::resolve::{Model, SheetItem, Symbol};
use crate::size::SizeExpr;
use crate::syntax::{Coord, ItemKind, Program, SizeSyntax};

/// The row of the first item on a sheet. Row 1 is left empty as a margin.
pub const FIRST_ROW: i64 = 2;
/// The column where position 0 of a range's columns lands: column D.
pub const VALUE_COLUMN: i64 = 4;
/// How many columns to the left of position 0 a label goes: column B, leaving column C empty.
pub const LABEL_OFFSET: i64 = 2;

/// A cell whose row and column may depend on fixed sizes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
    pub row: SizeExpr,
    pub column: SizeExpr,
}

/// Where every heading, label and range goes.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    /// Each heading's cell, by the heading's item index.
    pub headings: BTreeMap<usize, At>,
    /// Each shown binding's anchor: the top-left cell of its values.
    pub anchors: BTreeMap<usize, At>,
    /// Each shown binding's label cell.
    pub labels: BTreeMap<usize, At>,
    /// Each table's first column name, in the row above its values.
    pub headers: BTreeMap<usize, At>,
}

/// The next free row, or the binding of unknown height that nothing can go below.
enum Cursor {
    Row(SizeExpr),
    Below(usize),
}

/// Places every item of every sheet.
pub fn place(program: &Program, model: &Model, typed: &Typed) -> Result<Placed, Vec<Diagnostic>> {
    let mut placed = Placed::default();
    let mut errors = Vec::new();
    for sheet in &model.sheets {
        let mut cursor = Cursor::Row(SizeExpr::constant(FIRST_ROW));
        for item in &sheet.items {
            match *item {
                SheetItem::Heading(index) => {
                    let ItemKind::Heading { at, .. } = &program.items[index].kind else {
                        unreachable!("a heading index points at a heading");
                    };
                    let pos = program.items[index].pos;
                    let cell = match at {
                        Some(at) => pin(at, model, typed),
                        None => {
                            next_row(&cursor, model, typed, "this heading", pos).map(|row| At {
                                row,
                                column: SizeExpr::constant(VALUE_COLUMN - LABEL_OFFSET),
                            })
                        }
                    };
                    match cell {
                        Ok(cell) => {
                            cursor = Cursor::Row(cell.row.offset(1));
                            placed.headings.insert(index, cell);
                        }
                        Err(error) => errors.push(error),
                    }
                }
                SheetItem::Gap(_) => {
                    if let Cursor::Row(row) = &cursor {
                        cursor = Cursor::Row(row.offset(1));
                    }
                }
                SheetItem::Binding(binding) => {
                    let sig = model
                        .signature(program, binding)
                        .expect("a shown binding has a signature");
                    let shape = &typed.shapes[binding];
                    // A table takes a row of column names above its values.
                    let header = i64::from(model.tables.contains_key(&binding));
                    let anchor = match &sig.at {
                        Some(at) => pin(at, model, typed),
                        None => {
                            let what = format!("`{}`", sig.name.name);
                            next_row(&cursor, model, typed, &what, sig.name.pos).map(|row| At {
                                row: row.offset(shape.rows.start + header),
                                column: SizeExpr::constant(VALUE_COLUMN + shape.columns.start),
                            })
                        }
                    };
                    let anchor = match anchor {
                        Ok(anchor) => anchor,
                        Err(error) => {
                            errors.push(error);
                            continue;
                        }
                    };
                    // A table's label goes beside its column names, in the row above its values.
                    let label = At {
                        row: anchor.row.offset(-header),
                        column: anchor.column.offset(-(shape.columns.start + LABEL_OFFSET)),
                    };
                    if header == 1 {
                        placed.headers.insert(
                            binding,
                            At {
                                row: anchor.row.offset(-1),
                                column: anchor.column.clone(),
                            },
                        );
                    }
                    let unknown_height = shape
                        .rows
                        .size
                        .any_var(|var| typed.vars[var].kind == VarKind::Unknown);
                    cursor = if unknown_height {
                        Cursor::Below(binding)
                    } else {
                        Cursor::Row(anchor.row.plus(&shape.rows.size))
                    };
                    placed.anchors.insert(binding, anchor);
                    placed.labels.insert(binding, label);
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(placed)
    } else {
        Err(errors)
    }
}

fn next_row(
    cursor: &Cursor,
    model: &Model,
    typed: &Typed,
    what: &str,
    pos: Pos,
) -> Result<SizeExpr, Diagnostic> {
    match cursor {
        Cursor::Row(row) => Ok(row.clone()),
        Cursor::Below(binding) => {
            let name = &model.bindings[*binding].name;
            Err(Diagnostic::error(
                pos,
                format!(
                    "{what} would go below `{name}`, whose height, {}, is unknown when the file \
                     is written; move it above `{name}`, onto another sheet, or pin it with `@`",
                    typed.format(&typed.shapes[*binding].rows.size)
                ),
            ))
        }
    }
}

/// The cell a `@ (row, column)` names.
fn pin(at: &Coord, model: &Model, typed: &Typed) -> Result<At, Diagnostic> {
    Ok(At {
        row: fixed_size(&at.row, model, typed)?,
        column: fixed_size(&at.column, model, typed)?,
    })
}

fn fixed_size(syntax: &SizeSyntax, model: &Model, typed: &Typed) -> Result<SizeExpr, Diagnostic> {
    let mut terms = Vec::new();
    for (coefficient, ident) in &syntax.terms {
        match model.symbols.get(&ident.name) {
            Some(Symbol::Size(var)) if typed.fixed(*var).is_some() => {
                terms.push((*var, *coefficient));
            }
            Some(Symbol::Size(_)) => {
                return Err(Diagnostic::error(
                    ident.pos,
                    format!(
                        "a cell cannot depend on `{}`, which is unknown until the workbook \
                         calculates",
                        ident.name
                    ),
                ));
            }
            _ => {
                return Err(Diagnostic::error(
                    ident.pos,
                    format!("`{}` is not a fixed size", ident.name),
                ));
            }
        }
    }
    Ok(SizeExpr::from_parts(syntax.constant, terms))
}
