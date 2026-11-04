//! Writes what the steps have worked out back into the syntax tree, so that the result of each
//! step up to Check reads as a `.yup` file.
//!
//! Reading an elaborated file and running the same steps again changes nothing: the types, sizes
//! and places written into it are checked, and agree. After Ground the file is fully elaborated:
//! every binding with cells is placed at a cell, and nothing is left to work out but what the
//! workbook decides.

use std::collections::BTreeMap;

use crate::check::Warning;
use crate::diagnostic::Pos;
use crate::ground::Grounded;
use crate::infer::{Shape, Typed};
use crate::resolve::Model;
use crate::size::SizeExpr;
use crate::syntax::{
    Ident, Item, ItemKind, Offset, Placement, Program, Role, ShapeSyntax, Signature, SizeKind,
    SizeSyntax, Type,
};

/// How much has been worked out.
pub enum Detail<'a> {
    /// Names: nothing is added.
    Resolved,
    /// Types, sizes and spans, written into every signature.
    Typed(&'a Typed),
    /// As `Typed`, with every place a cell, sizes with fixed sizes replaced by their values, and
    /// any warnings as comments.
    Grounded(&'a Typed, &'a Grounded, &'a [Warning]),
}

/// The program with what `detail` says written into it.
pub fn elaborate(program: &Program, model: &Model, detail: &Detail) -> Program {
    let mut edits = Edits::default();
    match *detail {
        Detail::Resolved => {}
        Detail::Typed(typed) => sizes(program, model, typed, &typed.shapes, &mut edits),
        Detail::Grounded(typed, grounded, warnings) => {
            sizes(program, model, typed, &grounded.shapes, &mut edits);
            for (&binding, &(row, column)) in &grounded.anchors {
                let item = model.bindings[binding].signature;
                let sheet = model.bindings[binding]
                    .sheet
                    .expect("a placed binding is on a sheet");
                let sig = edits
                    .signatures
                    .get_mut(&item)
                    .expect("sizes are written before places");
                let pos = sig.name.pos;
                // A cell the author wrote as `$B$2` stays so, and only those, so that the cells
                // the rest are placed from still stand out once every place is a cell.
                let absolute = matches!(sig.place, Some(Placement::At { absolute: true, .. }));
                sig.place = Some(Placement::At {
                    sheet: Ident {
                        name: model.bindings[model.sheets[sheet].binding].name.clone(),
                        pos,
                    },
                    row,
                    column,
                    absolute,
                    pos,
                });
            }
            for warning in warnings {
                let item = model.bindings[warning.binding].signature;
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

/// Writes each binding's type and size into its signature, and declares each size that a `..`
/// introduced and no signature named.
fn sizes(program: &Program, model: &Model, typed: &Typed, shapes: &[Shape], edits: &mut Edits) {
    for var in &typed.vars {
        if let Some(binding) = var.introduced_by {
            let pos = model.bindings[binding].pos;
            edits.insert(
                model.bindings[binding].signature,
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
        if !matches!(
            binding.role,
            Role::Formula | Role::Value | Role::Label | Role::Empty | Role::Local
        ) {
            continue;
        }
        let mut sig = model.signature(program, index).clone();
        let pos = sig.name.pos;
        if sig.ty.is_none()
            && !matches!(binding.role, Role::Label | Role::Empty)
            && let Some(ty) = typed.types[index]
        {
            sig.ty = Some((Type::Cell(ty), pos));
        }
        let shape = &shapes[index];
        sig.shape = Some(ShapeSyntax {
            rows: size_syntax(&shape.rows.size, typed, pos),
            columns: size_syntax(&shape.columns.size, typed, pos),
            pos,
        });
        sig.start = (shape.rows.start != 0 || shape.columns.start != 0).then_some(Offset {
            row: shape.rows.start,
            column: shape.columns.start,
            pos,
        });
        edits.signatures.insert(binding.signature, sig);
    }
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
