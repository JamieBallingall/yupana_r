//! Step 4, Place: where each binding's cells go on its sheet, as `(row, column)`, both counted
//! from 1.
//!
//! A binding is placed at a cell, or beside another binding. Each move counts in cells from the
//! reference's cell on the side it heads toward: `right` from its east column, `left` from its
//! west, `down` from its south row and `up` from its north; `from` names another side. A move
//! right or down lands the binding's own first column or row there, and a move left or up its
//! last, so that the binding sits beside the reference rather than over it. Along an axis with no
//! move, the binding lines up with the reference's first column or row.
//!
//! A cell may depend on fixed sizes but never on unknown ones, so counting from the far side of
//! a range of unknown extent is refused.

use std::collections::BTreeMap;

use crate::cells::column_name;
use crate::check::Warning;
use crate::diagnostic::{Diagnostic, Pos};
use crate::infer::{Extent, Typed, VarKind};
use crate::resolve::{Model, Place};
use crate::size::SizeExpr;
use crate::syntax::{Direction, Move, Program, Side};

/// A cell whose row and column may depend on fixed sizes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
    pub row: SizeExpr,
    pub column: SizeExpr,
}

/// Where every binding with cells goes: the top-left cell of its values.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    pub anchors: BTreeMap<usize, At>,
    /// Placements that do not line up ranges that share a size.
    pub warnings: Vec<Warning>,
}

/// One axis of a sheet, as a placement sees it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Line {
    Rows,
    Columns,
}

impl Line {
    fn of(direction: Direction) -> Line {
        if direction.is_horizontal() {
            Line::Columns
        } else {
            Line::Rows
        }
    }

    /// The side a move counts from when it says none: the side it heads toward.
    fn default_side(direction: Direction) -> Side {
        match direction {
            Direction::Right => Side::East,
            Direction::Left => Side::West,
            Direction::Down => Side::South,
            Direction::Up => Side::North,
        }
    }

    fn extent(self, shape: &crate::infer::Shape) -> &Extent {
        match self {
            Line::Rows => &shape.rows,
            Line::Columns => &shape.columns,
        }
    }

    fn of_at(self, at: &At) -> &SizeExpr {
        match self {
            Line::Rows => &at.row,
            Line::Columns => &at.column,
        }
    }

    fn noun(self) -> (&'static str, &'static str) {
        match self {
            Line::Rows => ("height", "rows"),
            Line::Columns => ("width", "columns"),
        }
    }
}

/// Places every binding with cells.
pub fn place(program: &Program, model: &Model, typed: &Typed) -> Result<Placed, Vec<Diagnostic>> {
    let mut placed = Placed::default();
    let mut errors = Vec::new();
    for &binding in &model.layout {
        let pos = model.signature(program, binding).name.pos;
        let at = match &model.bindings[binding].place {
            Some(Place::At { row, column, .. }) => At {
                row: SizeExpr::constant(*row),
                column: SizeExpr::constant(*column),
            },
            Some(Place::Beside { reference, moves }) => {
                let Some(base) = placed.anchors.get(reference).cloned() else {
                    // The reference could not be placed, and has been reported.
                    continue;
                };
                let mut at = base.clone();
                let mut failed = false;
                for line in [Line::Rows, Line::Columns] {
                    let step = moves.iter().find(|step| Line::of(step.direction) == line);
                    match beside(model, typed, binding, *reference, &base, line, step) {
                        Ok(value) => match line {
                            Line::Rows => at.row = value,
                            Line::Columns => at.column = value,
                        },
                        Err(error) => {
                            errors.push(error);
                            failed = true;
                        }
                    }
                }
                if failed {
                    continue;
                }
                placed.warnings.extend(misaligned(
                    model, typed, binding, *reference, &at, &base, moves, pos,
                ));
                at
            }
            None => unreachable!("Resolve gives every binding with cells a place"),
        };
        placed.anchors.insert(binding, at);
    }
    if errors.is_empty() {
        Ok(placed)
    } else {
        Err(errors)
    }
}

/// Whether a size is known when the file is written.
fn known(typed: &Typed, size: &SizeExpr) -> bool {
    !size.any_var(|var| typed.vars[var].kind == VarKind::Unknown)
}

/// Where a binding placed beside `reference` goes along one axis.
fn beside(
    model: &Model,
    typed: &Typed,
    binding: usize,
    reference: usize,
    base: &At,
    line: Line,
    step: Option<&Move>,
) -> Result<SizeExpr, Diagnostic> {
    let start = line.of_at(base).clone();
    let Some(step) = step else {
        return Ok(start);
    };
    let name = &model.bindings[binding].name;
    let other = &model.bindings[reference].name;
    let side = step.from.unwrap_or(Line::default_side(step.direction));
    if side.is_column() != (line == Line::Columns) {
        let sides = match line {
            Line::Rows => "north or south",
            Line::Columns => "east or west",
        };
        return Err(Diagnostic::error(
            step.pos,
            format!(
                "a move {} counts from the {sides} side",
                step.direction.word()
            ),
        ));
    }
    let (noun, _) = line.noun();
    let extent = &line.extent(&typed.shapes[reference]).size;
    let from = if matches!(side, Side::East | Side::South) {
        if !known(typed, extent) {
            let near = match line {
                Line::Rows => "north",
                Line::Columns => "west",
            };
            return Err(Diagnostic::error(
                step.pos,
                format!(
                    "`{name}` counts from the {} side of `{other}`, whose {noun}, {}, is unknown \
                     when the file is written; count from its {near} side, with `from {near}`, or \
                     place `{name}` elsewhere",
                    side.word(),
                    typed.format(extent)
                ),
            ));
        }
        start.plus(extent).offset(-1)
    } else {
        start
    };
    let distance = step.distance();
    match step.direction {
        Direction::Right | Direction::Down => Ok(from.offset(distance)),
        Direction::Left | Direction::Up => {
            // The binding's own last cell lands at the count, so its own extent must be known.
            let own = &line.extent(&typed.shapes[binding]).size;
            if !known(typed, own) {
                return Err(Diagnostic::error(
                    step.pos,
                    format!(
                        "`{name}` is placed {} of `{other}`, so its own {noun}, {}, must be known \
                         when the file is written",
                        step.direction.word(),
                        typed.format(own)
                    ),
                ));
            }
            Ok(from.offset(1 - distance).minus(own))
        }
    }
}

/// A warning for each axis along which `binding` and `reference` share a size but are placed so
/// that their positions do not line up. A move that steps off the reference along an axis puts
/// the two side by side there, so only an axis with no move, or with a move that counts from the
/// reference's own first column or row, is checked.
#[allow(clippy::too_many_arguments)]
fn misaligned(
    model: &Model,
    typed: &Typed,
    binding: usize,
    reference: usize,
    at: &At,
    base: &At,
    moves: &[Move],
    pos: Pos,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for line in [Line::Rows, Line::Columns] {
        let stepped_off = moves.iter().any(|step| {
            Line::of(step.direction) == line
                && step
                    .from
                    .is_none_or(|side| side == Line::default_side(step.direction))
        });
        if stepped_off {
            continue;
        }
        let own = line.extent(&typed.shapes[binding]);
        let theirs = line.extent(&typed.shapes[reference]);
        let shared: Vec<usize> = own
            .size
            .terms()
            .map(|(var, _)| var)
            .filter(|&var| theirs.size.any_var(|other| other == var))
            .collect();
        let Some(&var) = shared.first() else {
            continue;
        };
        let value = |size: &SizeExpr| size.substitute(|var| typed.fixed(var)).as_constant();
        // Position 0 of each range, where it would be.
        let (Some(own_zero), Some(their_zero)) = (
            value(&line.of_at(at).offset(-own.start)),
            value(&line.of_at(base).offset(-theirs.start)),
        ) else {
            continue;
        };
        if own_zero == their_zero {
            continue;
        }
        let first = own.start.max(theirs.start);
        let (_, dimension) = line.noun();
        let show = |n: i64| match line {
            Line::Rows => format!("row {n}"),
            Line::Columns => {
                format!(
                    "column {}",
                    column_name(u32::try_from(n.max(1)).unwrap_or(1))
                )
            }
        };
        warnings.push(Warning {
            binding,
            pos,
            message: format!(
                "`{}` does not line up with `{}`: the two share {} along their {dimension}, but \
                 position {first} of `{}` is in {} and of `{}` in {}",
                model.bindings[binding].name,
                model.bindings[reference].name,
                typed.vars[var].name,
                model.bindings[binding].name,
                show(own_zero + first),
                model.bindings[reference].name,
                show(their_zero + first),
            ),
        });
    }
    warnings
}
