//! Step 6, Check: whether anything collides, for every value the unknown sizes can take.
//!
//! Every extent grows with its sizes, so two rectangles that ever overlap also overlap when the
//! sizes are large, and two that overlap at the smallest sizes overlap at all of them. That gives
//! two cases. Rectangles that overlap at the smallest sizes collide, and the workbook cannot be
//! written. Rectangles that overlap only once an unknown size grows would show `#SPILL!` in the
//! workbook when it does: the file is still valid, so that is a warning.

use std::collections::BTreeSet;

use crate::diagnostic::{Diagnostic, Pos};
use crate::ground::Grounded;
use crate::infer::Typed;
use crate::resolve::{Model, Role, SheetItem};
use crate::size::SizeExpr;
use crate::syntax::Program;

/// The number of rows on a sheet, from the spreadsheet app's published limits.
pub const MAX_ROWS: i64 = 1_048_576;
/// The number of columns on a sheet, from the spreadsheet app's published limits.
pub const MAX_COLUMNS: i64 = 16_384;

/// A warning about a shown binding, which can still be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub binding: usize,
    pub pos: Pos,
    pub message: String,
}

/// The cells one thing covers.
struct Rect {
    top: i64,
    left: i64,
    height: SizeExpr,
    width: SizeExpr,
    what: String,
    pos: Pos,
    /// The binding the cells belong to; `None` for a heading.
    binding: Option<usize>,
    /// Whether the cells are a formula's spill rather than written values.
    spills: bool,
}

impl Rect {
    fn cell(cell: (i64, i64), what: String, pos: Pos, binding: Option<usize>) -> Rect {
        Rect {
            top: cell.0,
            left: cell.1,
            height: SizeExpr::constant(1),
            width: SizeExpr::constant(1),
            what,
            pos,
            binding,
            spills: false,
        }
    }

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

    fn bounds_error(&self) -> Option<Diagnostic> {
        let on_sheet = (1..=MAX_ROWS).contains(&self.top) && (1..=MAX_COLUMNS).contains(&self.left);
        let (_, bottom) = span(self.top, &self.height, false);
        let (_, right) = span(self.left, &self.width, false);
        if on_sheet && bottom - 1 <= MAX_ROWS && right - 1 <= MAX_COLUMNS {
            return None;
        }
        Some(Diagnostic::error(
            self.pos,
            format!(
                "{} would run off the sheet, whose cells run from (1, 1) to ({MAX_ROWS}, \
                 {MAX_COLUMNS}); it starts at ({}, {})",
                self.what, self.top, self.left
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

/// Checks that nothing collides; returns the warnings about possible spills.
pub fn check(
    program: &Program,
    model: &Model,
    typed: &Typed,
    grounded: &Grounded,
) -> Result<Vec<Warning>, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for sheet in &model.sheets {
        let mut rects = Vec::new();
        for item in &sheet.items {
            match *item {
                SheetItem::Heading(index) => rects.push(Rect::cell(
                    grounded.headings[&index],
                    "a heading".to_string(),
                    program.items[index].pos,
                    None,
                )),
                SheetItem::Gap(_) => {}
                SheetItem::Binding(binding) => {
                    let b = &model.bindings[binding];
                    let pos =
                        program.items[b.signature.expect("a shown binding has a signature")].pos;
                    rects.push(Rect::cell(
                        grounded.labels[&binding],
                        format!("the label of `{}`", b.name),
                        pos,
                        Some(binding),
                    ));
                    let (top, left) = grounded.anchors[&binding];
                    let shape = &grounded.shapes[binding];
                    rects.push(Rect {
                        top,
                        left,
                        height: shape.rows.size.clone(),
                        width: shape.columns.size.clone(),
                        what: format!("`{}`", b.name),
                        pos,
                        binding: Some(binding),
                        spills: b.role == Role::Shown,
                    });
                }
            }
        }
        errors.extend(rects.iter().filter_map(Rect::bounds_error));
        let mut warned = BTreeSet::new();
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                if a.overlaps(b, false) {
                    errors.push(Diagnostic::error(
                        b.pos,
                        format!(
                            "{} and {} would share the cell ({}, {})",
                            a.what,
                            b.what,
                            a.top.max(b.top),
                            a.left.max(b.left)
                        ),
                    ));
                } else if a.overlaps(b, true) {
                    let (spilling, other) = if a.spills && a.grows() {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    let binding = spilling.binding.expect("what grows is a binding's range");
                    // A range that spills into a label usually spills into its values too; one
                    // warning names both.
                    if !warned.insert((binding, other.binding)) {
                        continue;
                    }
                    let mut sizes: Vec<&str> = [&spilling.height, &spilling.width]
                        .iter()
                        .flat_map(|size| size.terms().map(|(var, _)| typed.vars[var].name.as_str()))
                        .collect();
                    sizes.sort_unstable();
                    sizes.dedup();
                    let other = match other.binding {
                        Some(other) => format!("`{}`", model.bindings[other].name),
                        None => "a heading".to_string(),
                    };
                    warnings.push(Warning {
                        binding,
                        pos: spilling.pos,
                        message: format!(
                            "{} spills into {other} once {} grows large enough; the workbook \
                             would show #SPILL!",
                            spilling.what,
                            sizes.join(" or ")
                        ),
                    });
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(errors)
    }
}
