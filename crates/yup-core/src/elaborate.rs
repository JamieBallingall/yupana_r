//! Writes what the steps have worked out back into the syntax tree, so that the result of each
//! step up to Check reads as a `.yup` file.
//!
//! Reading an elaborated file and running the same steps again changes nothing: the sizes and
//! anchors written into it are checked, and agree.

use std::collections::BTreeMap;

use crate::check::Warning;
use crate::diagnostic::Pos;
use crate::ground::Grounded;
use crate::infer::{Shape, Typed};
use crate::place::{At, Placed};
use crate::resolve::{Model, SheetItem};
use crate::size::SizeExpr;
use crate::syntax::{
    Coord, Ident, Item, ItemKind, Offset, Program, ShapeSyntax, Signature, SizeKind, SizeSyntax,
    Text,
};

/// How much has been worked out.
pub enum Detail<'a> {
    /// Names: the sheet for items before any `sheet` statement is written out.
    Resolved,
    /// Sizes and spans: every binding gets a signature with its size.
    Typed(&'a Typed),
    /// Anchors, which may depend on fixed sizes.
    Placed(&'a Typed, &'a Placed),
    /// Anchors as cells, sizes with fixed sizes replaced by their values, and any warnings from
    /// Check as comments.
    Grounded(&'a Typed, &'a Grounded, &'a [Warning]),
}

/// The program with what `detail` says written into it.
pub fn elaborate(program: &Program, model: &Model, detail: &Detail) -> Program {
    let mut edits = Edits::default();
    default_sheet(model, &mut edits);
    match *detail {
        Detail::Resolved => {}
        Detail::Typed(typed) => sizes(program, model, typed, &typed.shapes, &mut edits),
        Detail::Placed(typed, placed) => {
            sizes(program, model, typed, &typed.shapes, &mut edits);
            let coord = |at: &At, pos| Coord {
                row: size_syntax(&at.row, typed, pos),
                column: size_syntax(&at.column, typed, pos),
                pos,
            };
            anchors(
                model,
                placed.anchors.iter().map(|(&b, at)| (b, at)),
                placed.headings.iter().map(|(&i, at)| (i, at)),
                coord,
                &mut edits,
            );
        }
        Detail::Grounded(typed, grounded, warnings) => {
            sizes(program, model, typed, &grounded.shapes, &mut edits);
            let coord = |&(row, column): &(i64, i64), pos| Coord {
                row: constant_syntax(row, pos),
                column: constant_syntax(column, pos),
                pos,
            };
            anchors(
                model,
                grounded.anchors.iter().map(|(&b, cell)| (b, cell)),
                grounded.headings.iter().map(|(&i, cell)| (i, cell)),
                coord,
                &mut edits,
            );
            for warning in warnings {
                let item = model.bindings[warning.binding]
                    .signature
                    .expect("a shown binding has a signature");
                edits
                    .comments
                    .entry(item)
                    .or_default()
                    .push(format!(" warning: {}", warning.message));
            }
        }
    }
    edits.apply(program)
}

/// Changes to a program, by the index of the item they belong to.
#[derive(Default)]
struct Edits {
    /// Items to insert before an item.
    before: BTreeMap<usize, Vec<Item>>,
    /// Signatures that replace a signature item.
    signatures: BTreeMap<usize, Signature>,
    /// Cells for headings.
    headings: BTreeMap<usize, Coord>,
    /// Comments to add at the end of an item's line.
    comments: BTreeMap<usize, Vec<String>>,
}

impl Edits {
    fn insert(&mut self, index: usize, pos: Pos, kind: ItemKind) {
        self.before.entry(index).or_default().push(Item {
            pos,
            kind,
            comment: None,
        });
    }

    fn apply(mut self, program: &Program) -> Program {
        let mut items = Vec::with_capacity(program.items.len());
        for (index, item) in program.items.iter().enumerate() {
            items.extend(self.before.remove(&index).unwrap_or_default());
            let mut item = item.clone();
            if let Some(sig) = self.signatures.remove(&index) {
                item.kind = ItemKind::Signature(Box::new(sig));
            }
            if let (Some(coord), ItemKind::Heading { at, .. }) =
                (self.headings.remove(&index), &mut item.kind)
            {
                *at = Some(coord);
            }
            for comment in self.comments.remove(&index).unwrap_or_default() {
                // A comment already there, from an earlier run, is not repeated.
                item.comment = match item.comment {
                    Some(old) if old.contains(comment.trim()) => Some(old),
                    Some(old) => Some(format!("{old} //{comment}")),
                    None => Some(comment),
                };
            }
            items.push(item);
        }
        Program {
            version: program.version.clone(),
            items,
        }
    }
}

/// The first item of a binding: its signature or its definition, whichever comes first.
fn first_item(model: &Model, binding: usize) -> usize {
    let binding = &model.bindings[binding];
    binding
        .signature
        .map_or(binding.definition, |sig| sig.min(binding.definition))
}

/// Writes out the `sheet` statement of the sheet that holds items written before any.
fn default_sheet(model: &Model, edits: &mut Edits) {
    let Some(sheet) = model.sheets.iter().find(|sheet| sheet.statement.is_none()) else {
        return;
    };
    let first = sheet.items.iter().map(|item| match *item {
        SheetItem::Heading(index) | SheetItem::Gap(index) => index,
        SheetItem::Binding(binding) => model.bindings[binding]
            .signature
            .expect("a shown binding has a signature"),
    });
    if let Some(first) = first.min() {
        edits.insert(
            first,
            sheet.pos,
            ItemKind::Sheet(Text {
                value: sheet.name.clone(),
                pos: sheet.pos,
            }),
        );
    }
}

/// Writes each binding's size into its signature, giving a signature to bindings without one
/// and declaring each size that a `..` introduced and no signature named.
fn sizes(program: &Program, model: &Model, typed: &Typed, shapes: &[Shape], edits: &mut Edits) {
    for var in &typed.vars {
        if let Some(binding) = var.introduced_by {
            let pos = model.bindings[binding].pos;
            edits.insert(
                first_item(model, binding),
                pos,
                ItemKind::Size {
                    name: Ident {
                        name: var.name.clone(),
                        pos,
                    },
                    kind: SizeKind::Unknown,
                },
            );
        }
    }
    for (index, binding) in model.bindings.iter().enumerate() {
        let shape = &shapes[index];
        match binding.signature {
            Some(item) => {
                let mut sig = model
                    .signature(program, index)
                    .expect("the binding has a signature")
                    .clone();
                write_shape(&mut sig, shape, typed);
                edits.signatures.insert(item, sig);
            }
            None => {
                let mut sig = Signature::new(Ident {
                    name: binding.name.clone(),
                    pos: binding.pos,
                });
                write_shape(&mut sig, shape, typed);
                edits.insert(
                    binding.definition,
                    binding.pos,
                    ItemKind::Signature(Box::new(sig)),
                );
            }
        }
    }
}

/// Writes anchors into the signatures `sizes` has written, and cells into headings.
fn anchors<'a, T: 'a>(
    model: &Model,
    bindings: impl Iterator<Item = (usize, &'a T)>,
    headings: impl Iterator<Item = (usize, &'a T)>,
    coord: impl Fn(&T, Pos) -> Coord,
    edits: &mut Edits,
) {
    for (binding, at) in bindings {
        let item = model.bindings[binding]
            .signature
            .expect("a shown binding has a signature");
        let sig = edits
            .signatures
            .get_mut(&item)
            .expect("sizes are written before anchors");
        sig.at = Some(coord(at, sig.name.pos));
    }
    for (index, at) in headings {
        edits.headings.insert(index, coord(at, Pos::START));
    }
}

fn write_shape(sig: &mut Signature, shape: &Shape, typed: &Typed) {
    let pos = sig.name.pos;
    sig.shape = Some(ShapeSyntax {
        rows: size_syntax(&shape.rows.size, typed, pos),
        columns: size_syntax(&shape.columns.size, typed, pos),
        pos,
    });
    sig.from = (shape.rows.start != 0 || shape.columns.start != 0).then_some(Offset {
        row: shape.rows.start,
        column: shape.columns.start,
        pos,
    });
}

/// A size as syntax, its variables in alphabetical order.
fn size_syntax(size: &SizeExpr, typed: &Typed, pos: Pos) -> SizeSyntax {
    let mut terms: Vec<(i64, Ident)> = size
        .terms()
        .map(|(var, coefficient)| {
            let name = typed.vars[var].name.clone();
            (coefficient, Ident { name, pos })
        })
        .collect();
    terms.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    SizeSyntax {
        constant: size.constant_part(),
        terms,
        pos,
    }
}

fn constant_syntax(value: i64, pos: Pos) -> SizeSyntax {
    SizeSyntax {
        constant: value,
        terms: Vec::new(),
        pos,
    }
}
