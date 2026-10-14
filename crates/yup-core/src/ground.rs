//! Step 5, Ground: fixed sizes replaced by their values. Every anchor becomes a cell, and every
//! extent depends on unknown sizes only.

use std::collections::BTreeMap;

use crate::infer::{Extent, Shape, Typed};
use crate::place::{At, Placed};
use crate::size::SizeExpr;

/// A cell, `(row, column)`, both counted from 1.
pub type Cell = (i64, i64);

/// Where everything goes, as cells, and every binding's shape in unknown sizes only.
#[derive(Clone, Debug, Default)]
pub struct Grounded {
    pub headings: BTreeMap<usize, Cell>,
    pub anchors: BTreeMap<usize, Cell>,
    pub labels: BTreeMap<usize, Cell>,
    pub shapes: Vec<Shape>,
}

/// Substitutes the fixed sizes in `placed` and in the shapes of `typed`.
pub fn ground(typed: &Typed, placed: &Placed) -> Grounded {
    let size = |size: &SizeExpr| size.substitute(|var| typed.fixed(var));
    let cell = |at: &At| (constant(&size(&at.row)), constant(&size(&at.column)));
    let cells = |ats: &BTreeMap<usize, At>| -> BTreeMap<usize, Cell> {
        ats.iter().map(|(&key, at)| (key, cell(at))).collect()
    };
    let extent = |extent: &Extent| Extent {
        start: extent.start,
        size: size(&extent.size),
    };
    Grounded {
        headings: cells(&placed.headings),
        anchors: cells(&placed.anchors),
        labels: cells(&placed.labels),
        shapes: typed
            .shapes
            .iter()
            .map(|shape| Shape {
                rows: extent(&shape.rows),
                columns: extent(&shape.columns),
            })
            .collect(),
    }
}

fn constant(size: &SizeExpr) -> i64 {
    size.as_constant()
        .expect("Place keeps unknown sizes out of anchors")
}
