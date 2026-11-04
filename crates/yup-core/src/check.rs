//! Step 6, Check: whether anything collides, for every value the unknown sizes can take.
//!
//! Every extent grows with its sizes, so two rectangles that ever overlap also overlap when the
//! sizes are large, and two that overlap at the smallest sizes overlap at all of them. That gives
//! two cases. Rectangles that overlap at the smallest sizes collide, and the workbook cannot be
//! written. Rectangles that overlap only once an unknown size grows would show `#SPILL!` in the
//! workbook when it does: the file is still valid, so that is a warning.
//!
//! Check also makes sure that every width and height a binding sets can be written: on a range
//! whose columns, or rows, are known, and agreeing with any other binding that sets the same one.

use std::collections::{BTreeMap, BTreeSet};

use crate::cells::{MAX_COLUMNS, MAX_ROWS, cell_name, column_name};
use crate::diagnostic::{Diagnostic, Pos};
use crate::ground::Grounded;
use crate::infer::Typed;
use crate::resolve::Model;
use crate::size::SizeExpr;
use crate::syntax::{Program, Role};

/// A warning about a binding, which can still be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub binding: usize,
    pub pos: Pos,
    pub message: String,
}

/// The cells one binding covers.
struct Rect {
    top: i64,
    left: i64,
    height: SizeExpr,
    width: SizeExpr,
    binding: usize,
    pos: Pos,
    /// Whether the cells are a formula's spill rather than written values.
    spills: bool,
}

impl Rect {
    fn grows(&self) -> bool {
        self.height.any_var(|_| true) || self.width.any_var(|_| true)
    }

    fn overlaps(&self, other: &Rect, large: bool) -> bool {
        meets(
            span(self.top, &self.height, large),
            span(other.top, &other.height, large),
        ) && meets(
            span(self.left, &self.width, large),
            span(other.left, &other.width, large),
        )
    }

    fn bounds_error(&self, name: &str) -> Option<Diagnostic> {
        let on_sheet = (1..=MAX_ROWS).contains(&self.top) && (1..=MAX_COLUMNS).contains(&self.left);
        let (_, bottom) = span(self.top, &self.height, false);
        let (_, right) = span(self.left, &self.width, false);
        if on_sheet && bottom - 1 <= MAX_ROWS && right - 1 <= MAX_COLUMNS {
            return None;
        }
        Some(Diagnostic::error(
            self.pos,
            format!(
                "`{name}` would run off the sheet, whose cells run from A1 to XFD{MAX_ROWS}; its \
                 first cell is at row {} and column {}",
                self.top, self.left
            ),
        ))
    }
}

/// The positions covered along one dimension, end exclusive: at the smallest sizes, or with
/// every size that has a variable taken as without end.
fn span(start: i64, size: &SizeExpr, large: bool) -> (i64, i64) {
    if large && size.any_var(|_| true) {
        (start, i64::MAX)
    } else {
        (start, start.saturating_add(size.evaluate(|_| 1)))
    }
}

/// Whether two spans share a position. An empty span shares none.
fn meets(a: (i64, i64), b: (i64, i64)) -> bool {
    a.0 < a.1 && b.0 < b.1 && a.0 < b.1 && b.0 < a.1
}

fn cell(row: i64, column: i64) -> String {
    match (u32::try_from(row), u32::try_from(column)) {
        (Ok(row), Ok(column)) => cell_name(row, column),
        _ => format!("(row {row}, column {column})"),
    }
}

/// Checks that nothing collides; returns the warnings about possible spills.
pub fn check(
    program: &Program,
    model: &Model,
    typed: &Typed,
    grounded: &Grounded,
) -> Result<Vec<Warning>, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for sheet in 0..model.sheets.len() {
        let rects: Vec<Rect> = model
            .layout
            .iter()
            .filter(|&&binding| model.bindings[binding].sheet == Some(sheet))
            .map(|&binding| {
                let (top, left) = grounded.anchors[&binding];
                let shape = &grounded.shapes[binding];
                Rect {
                    top,
                    left,
                    height: shape.rows.size.clone(),
                    width: shape.columns.size.clone(),
                    binding,
                    pos: model.signature(program, binding).name.pos,
                    spills: model.bindings[binding].role == Role::Formula,
                }
            })
            .collect();
        let name = |rect: &Rect| model.bindings[rect.binding].name.as_str();
        errors.extend(
            rects
                .iter()
                .filter_map(|rect| rect.bounds_error(name(rect))),
        );
        let mut warned = BTreeSet::new();
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                if a.overlaps(b, false) {
                    let (first, second) = if a.pos <= b.pos { (a, b) } else { (b, a) };
                    errors.push(Diagnostic::error(
                        second.pos,
                        format!(
                            "`{}` and `{}` would share the cell {}",
                            name(first),
                            name(second),
                            cell(a.top.max(b.top), a.left.max(b.left))
                        ),
                    ));
                } else if a.overlaps(b, true) {
                    let (spilling, other) = if a.spills && a.grows() {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    if !warned.insert((spilling.binding, other.binding)) {
                        continue;
                    }
                    let mut sizes: Vec<&str> = [&spilling.height, &spilling.width]
                        .iter()
                        .flat_map(|size| size.terms().map(|(var, _)| typed.vars[var].name.as_str()))
                        .collect();
                    sizes.sort_unstable();
                    sizes.dedup();
                    warnings.push(Warning {
                        binding: spilling.binding,
                        pos: spilling.pos,
                        message: format!(
                            "`{}` spills into `{}` once {} grows large enough; the workbook would \
                             show #SPILL!",
                            name(spilling),
                            name(other),
                            sizes.join(" or ")
                        ),
                    });
                }
            }
        }
        errors.extend(widths_and_heights(program, model, typed, &rects));
    }
    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(errors)
    }
}

/// Refuses a width on a range of unknown width, a height on a range of unknown height, and two
/// bindings that set the same column, or row, to different sizes. Two that agree are fine: what
/// matters is the size each column ends up with, not which binding says so.
fn widths_and_heights(
    program: &Program,
    model: &Model,
    typed: &Typed,
    rects: &[Rect],
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    for columns in [true, false] {
        let (part, noun, line, adjective) = if columns {
            ("width", "columns", "column", "wide")
        } else {
            ("height", "rows", "row", "high")
        };
        // The size each line is set to, by the first binding to set it, and where.
        let mut set: BTreeMap<i64, (u32, usize, Pos)> = BTreeMap::new();
        for rect in rects {
            let sig = model.signature(program, rect.binding);
            let Some((pixels, pos)) = (if columns { sig.width } else { sig.height }) else {
                continue;
            };
            let name = &model.bindings[rect.binding].name;
            let (first, extent) = if columns {
                (rect.left, &rect.width)
            } else {
                (rect.top, &rect.height)
            };
            let Some(count) = extent.as_constant() else {
                errors.push(Diagnostic::error(
                    pos,
                    format!(
                        "`{name}` cannot set a {part}: how many {noun} it has, {}, is unknown \
                         when the file is written",
                        typed.format(extent)
                    ),
                ));
                continue;
            };
            for at in first..first + count {
                match set.get(&at) {
                    None => {
                        set.insert(at, (pixels, rect.binding, pos));
                    }
                    Some(&(other_pixels, other, other_pos)) if other_pixels != pixels => {
                        let label = match u32::try_from(at) {
                            Ok(at) if columns => column_name(at),
                            _ => at.to_string(),
                        };
                        errors.push(Diagnostic::error(
                            pos,
                            format!(
                                "`{name}` makes {line} {label} {pixels} pixels {adjective}, and \
                                 `{}`, at {other_pos}, makes it {other_pixels}",
                                model.bindings[other].name
                            ),
                        ));
                        break;
                    }
                    Some(_) => {}
                }
            }
        }
    }
    errors
}
