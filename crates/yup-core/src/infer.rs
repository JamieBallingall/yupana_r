//! Step 3, Infer: the size of every expression, and its span along each dimension.
//!
//! A size is counted in rows and columns. A span says where a range sits along them: `tail x`
//! and `init x` have the same size but different spans, and only ranges with equal spans can be
//! combined position by position. A dimension of size 1 stretches to match the other operand, as
//! it does in the spreadsheet app.
//!
//! Every `a .. b` whose end is not known when the file is written introduces a new `Unknown`
//! size. A signature can name it, and the name is checked rather than trusted.

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, Pos};
use crate::functions::Builtin;
use crate::resolve::{Model, Recurrence, Role, Symbol};
use crate::size::SizeExpr;
use crate::syntax::{Expr, ExprKind, Ident, Program, Signature, SizeKind, SizeSyntax};

/// Where a range sits along one dimension: `size` positions, starting at position `start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extent {
    pub start: i64,
    pub size: SizeExpr,
}

impl Extent {
    fn one() -> Extent {
        Extent::of(SizeExpr::constant(1))
    }

    fn of(size: SizeExpr) -> Extent {
        Extent { start: 0, size }
    }
}

/// Where an expression sits along its rows and along its columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub rows: Extent,
    pub columns: Extent,
}

impl Shape {
    fn scalar() -> Shape {
        Shape {
            rows: Extent::one(),
            columns: Extent::one(),
        }
    }
}

/// Whether a size variable is known when the file is written, and if so its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Fixed(i64),
    Unknown,
}

/// A size variable, declared in the file or introduced by `a .. b`.
#[derive(Clone, Debug)]
pub struct Var {
    pub name: String,
    pub kind: VarKind,
    /// For a size that no declaration names, the binding whose definition introduced it.
    pub introduced_by: Option<usize>,
}

/// The shape of every binding, in terms of the size variables.
#[derive(Clone, Debug)]
pub struct Typed {
    pub vars: Vec<Var>,
    pub shapes: Vec<Shape>,
    /// The parts of a recurrence that lie outside its cycle and are rows, by position: inside the
    /// scan that works the recurrence out, they are read one period at a time.
    pub rows: BTreeSet<Pos>,
    /// For each balance carried from one period to the next, the part outside its cycle whose
    /// length the scan runs over.
    pub drivers: BTreeMap<usize, Pos>,
}

impl Typed {
    /// The value of a fixed size variable.
    pub fn fixed(&self, var: usize) -> Option<i64> {
        match self.vars[var].kind {
            VarKind::Fixed(value) => Some(value),
            VarKind::Unknown => None,
        }
    }

    /// A size as `.yup` writes it.
    pub fn format(&self, size: &SizeExpr) -> String {
        size.format(|var| self.vars[var].name.clone())
    }
}

type Inferred = Result<Shape, Option<Diagnostic>>;

fn fail(pos: Pos, message: impl Into<String>) -> Option<Diagnostic> {
    Some(Diagnostic::error(pos, message))
}

struct VarState {
    name: Option<String>,
    kind: VarKind,
    declared: bool,
    /// A size introduced by `..` and named by a signature stands for the declared size.
    alias: Option<usize>,
    /// For a declared unknown size, the introduced size that a signature named after it.
    named: Option<usize>,
    pos: Pos,
    binding: Option<usize>,
}

struct Infer<'a> {
    program: &'a Program,
    model: &'a Model,
    vars: Vec<VarState>,
    /// Each binding's shape, or `None` until inferred or when its definition is wrong.
    shapes: Vec<Option<Shape>>,
    errors: Vec<Diagnostic>,
    current: usize,
    /// The shapes of the parameters of the functions being called, innermost last. A function
    /// body sees its own parameters only.
    params: Vec<BTreeMap<String, Shape>>,
    rows: BTreeSet<Pos>,
    drivers: BTreeMap<usize, Pos>,
}

/// Infers the shape of every binding in `model`.
pub fn infer(program: &Program, model: &Model) -> Result<Typed, Vec<Diagnostic>> {
    let vars = model
        .sizes
        .iter()
        .map(|size| VarState {
            name: Some(size.name.clone()),
            kind: match size.kind {
                SizeKind::Fixed => {
                    VarKind::Fixed(size.value.expect("Resolve gives every fixed size a value"))
                }
                SizeKind::Unknown => VarKind::Unknown,
            },
            declared: true,
            alias: None,
            named: None,
            pos: size.pos,
            binding: None,
        })
        .collect();
    let mut infer = Infer {
        program,
        model,
        vars,
        shapes: vec![None; model.bindings.len()],
        errors: Vec::new(),
        current: 0,
        params: Vec::new(),
        rows: BTreeSet::new(),
        drivers: BTreeMap::new(),
    };
    for &binding in &model.order {
        match model.recurrence_of(binding) {
            // A recurrence's members come together in the order, and are inferred together.
            Some(recurrence) if recurrence.members[0] == binding => infer.recurrence(recurrence),
            Some(_) => {}
            None => infer.binding(binding),
        }
    }
    infer.finish()
}

impl Infer<'_> {
    fn canonical(&self, var: usize) -> usize {
        self.vars[var].alias.unwrap_or(var)
    }

    /// A size with aliases followed and fixed sizes replaced by their values, so that equal
    /// sizes compare equal.
    fn normal(&self, size: &SizeExpr) -> SizeExpr {
        size.rename(|var| self.canonical(var))
            .substitute(|var| match self.vars[var].kind {
                VarKind::Fixed(value) => Some(value),
                VarKind::Unknown => None,
            })
    }

    fn same(&self, a: &SizeExpr, b: &SizeExpr) -> bool {
        self.normal(a) == self.normal(b)
    }

    fn is_one(&self, size: &SizeExpr) -> bool {
        self.normal(size).as_constant() == Some(1)
    }

    fn is_scalar(&self, shape: &Shape) -> bool {
        self.is_one(&shape.rows.size) && self.is_one(&shape.columns.size)
    }

    fn var_name(&self, var: usize) -> String {
        let var = self.canonical(var);
        match (&self.vars[var].name, self.vars[var].binding) {
            (Some(name), _) => name.clone(),
            (None, Some(binding)) => format!("{}_n", self.model.bindings[binding].name),
            (None, None) => unreachable!("an introduced size has a binding"),
        }
    }

    fn show(&self, size: &SizeExpr) -> String {
        size.rename(|var| self.canonical(var))
            .format(|var| self.var_name(var))
    }

    fn positions(&self, extent: &Extent) -> String {
        format!(
            "[{}, {})",
            extent.start,
            self.show(&extent.size.offset(extent.start))
        )
    }

    fn fresh(&mut self, pos: Pos) -> SizeExpr {
        self.vars.push(VarState {
            name: None,
            kind: VarKind::Unknown,
            declared: false,
            alias: None,
            named: None,
            pos,
            binding: Some(self.current),
        });
        SizeExpr::var(self.vars.len() - 1)
    }

    fn binding(&mut self, binding: usize) {
        self.current = binding;
        // A function has no shape of its own; each call is inferred where it is made.
        if self.model.bindings[binding].role == Role::Function {
            self.shapes[binding] = Some(Shape::scalar());
            return;
        }
        let expr = self.model.definition(self.program, binding);
        let shape = match self.shape(expr) {
            Ok(shape) => shape,
            Err(error) => {
                self.errors.extend(error);
                return;
            }
        };
        let shape = match self.model.signature(self.program, binding) {
            Some(sig) => match self.check_signature(sig, shape) {
                Ok(shape) => shape,
                Err(error) => {
                    self.errors.push(error);
                    return;
                }
            },
            None => shape,
        };
        self.shapes[binding] = Some(shape);
    }

    fn shape(&mut self, expr: &Expr) -> Inferred {
        match &expr.kind {
            ExprKind::Number { .. } | ExprKind::Str(_) | ExprKind::Bool(_) => Ok(Shape::scalar()),
            ExprKind::Name(name)
                if self
                    .params
                    .last()
                    .is_some_and(|frame| frame.contains_key(name)) =>
            {
                Ok(self.params.last().expect("checked just now")[name].clone())
            }
            ExprKind::Name(name) => match self.model.symbols.get(name) {
                // A binding whose definition is wrong has been reported; what uses it is not.
                Some(Symbol::Binding(binding)) => self.shapes[*binding].clone().ok_or(None),
                Some(Symbol::Size(_)) => Ok(Shape::scalar()),
                None => unreachable!("Resolve reports unknown names"),
            },
            ExprKind::Neg(operand) => self.shape(operand),
            ExprKind::Binary { op, left, right } => {
                let left = self.shape(left)?;
                let right = self.shape(right)?;
                self.combine(&left, &right).map_err(|problem| {
                    fail(
                        expr.pos,
                        format!("the two sides of `{}` {problem}", op.symbol()),
                    )
                })
            }
            ExprKind::If { cond, then, other } => {
                let cond = self.shape(cond)?;
                let then = self.shape(then)?;
                let other = self.shape(other)?;
                let branches = self.combine(&then, &other).map_err(|problem| {
                    fail(expr.pos, format!("the two branches of `if` {problem}"))
                })?;
                self.combine(&cond, &branches).map_err(|problem| {
                    fail(
                        expr.pos,
                        format!("the condition and the branches of `if` {problem}"),
                    )
                })
            }
            ExprKind::List(rows) => {
                let width = rows[0].len();
                if let Some(row) = rows.iter().find(|row| row.len() != width) {
                    return Err(fail(
                        row[0].pos,
                        format!(
                            "every row of a list has the same number of values; the first has \
                             {width}"
                        ),
                    ));
                }
                if let Some(value) = rows.iter().flatten().find(|value| !value.is_literal()) {
                    return Err(fail(
                        value.pos,
                        "a list holds literal values: numbers, strings, `true` and `false`",
                    ));
                }
                Ok(Shape {
                    rows: Extent::of(SizeExpr::constant(count(rows.len()))),
                    columns: Extent::of(SizeExpr::constant(count(width))),
                })
            }
            ExprKind::Range { first, last } => self.range(expr, first, last),
            ExprKind::Apply { func, args, seed } => match Builtin::lookup(&func.name) {
                Some(builtin) => self.apply(expr, builtin, args, seed.as_deref()),
                None => self.call(func, args),
            },
        }
    }

    /// A call of a function of the file's own. Its body is inferred with its parameters taking
    /// the shapes of the arguments, so each call has the shape its arguments give it.
    fn call(&mut self, func: &Ident, args: &[Expr]) -> Inferred {
        let Some(&Symbol::Binding(binding)) = self.model.symbols.get(&func.name) else {
            unreachable!("Resolve checks function names");
        };
        let mut frame = BTreeMap::new();
        for (param, arg) in self.model.bindings[binding].params.iter().zip(args) {
            frame.insert(param.clone(), self.shape(arg)?);
        }
        self.params.push(frame);
        let body = self.model.definition(self.program, binding);
        let shape = self.shape(body);
        self.params.pop();
        shape.map_err(|error| {
            error.map(|mut error| {
                error.message.push_str(&format!(
                    ", in `{}` as it is called at {}",
                    func.name, func.pos
                ));
                error
            })
        })
    }

    fn range(&mut self, expr: &Expr, first: &Expr, last: &Expr) -> Inferred {
        for end in [first, last] {
            let shape = self.shape(end)?;
            if !self.is_scalar(&shape) {
                return Err(fail(end.pos, "each end of `..` is a single value"));
            }
        }
        let Some(start) = first.integer() else {
            return Err(fail(
                first.pos,
                "a range starts at a whole number written in the file, such as `1 .. n`",
            ));
        };
        let end = match (&last.kind, last.integer()) {
            (_, Some(end)) => Some(SizeExpr::constant(end)),
            (ExprKind::Name(name), None) => match self.model.symbols.get(name) {
                Some(Symbol::Size(var)) => Some(SizeExpr::var(*var)),
                _ => None,
            },
            _ => None,
        };
        let size = match end {
            Some(end) => {
                let size = end.offset(1 - start);
                if self.normal(&size).as_constant().is_some_and(|n| n < 1) {
                    return Err(fail(expr.pos, "this range has no numbers in it"));
                }
                size
            }
            // The end is decided in the workbook, so the length is too.
            None => self.fresh(expr.pos),
        };
        Ok(Shape {
            rows: Extent::one(),
            columns: Extent::of(size),
        })
    }

    fn apply(
        &mut self,
        expr: &Expr,
        builtin: Builtin,
        args: &[Expr],
        seed: Option<&Expr>,
    ) -> Inferred {
        match builtin {
            Builtin::Lag => {
                let periods = periods(&args[0], "lag")?;
                let x = self.shape(&args[1])?;
                let Some(seed) = seed else {
                    return self.shift(&x, periods, periods, expr.pos, "lag");
                };
                if periods != 1 {
                    return Err(fail(
                        seed.pos,
                        "`from` gives one column of first values, so it goes with `lag 1`",
                    ));
                }
                let first = self.shape(seed)?;
                if !self.is_one(&first.columns.size) {
                    return Err(fail(
                        seed.pos,
                        "the first values after `from` are one column",
                    ));
                }
                if !self.same(&first.rows.size, &x.rows.size) {
                    return Err(fail(
                        seed.pos,
                        format!(
                            "the first values after `from` have {} rows, and the range has {}",
                            self.show(&first.rows.size),
                            self.show(&x.rows.size)
                        ),
                    ));
                }
                Ok(x)
            }
            Builtin::Tail => {
                let x = self.shape(&args[0])?;
                self.shift(&x, 1, 1, expr.pos, "tail")
            }
            Builtin::Init => {
                let x = self.shape(&args[0])?;
                self.shift(&x, 0, 1, expr.pos, "init")
            }
            Builtin::Sum | Builtin::Average | Builtin::Minimum | Builtin::Maximum => {
                self.shape(&args[0])?;
                Ok(Shape::scalar())
            }
            Builtin::SumColumns => {
                let x = self.shape(&args[0])?;
                Ok(Shape {
                    rows: Extent::one(),
                    columns: x.columns,
                })
            }
            Builtin::SumRows => {
                let x = self.shape(&args[0])?;
                Ok(Shape {
                    rows: x.rows,
                    columns: Extent::one(),
                })
            }
            Builtin::Min | Builtin::Max => {
                let a = self.shape(&args[0])?;
                let b = self.shape(&args[1])?;
                self.combine(&a, &b).map_err(|problem| {
                    fail(
                        expr.pos,
                        format!("the two arguments of `{}` {problem}", function_name(expr)),
                    )
                })
            }
            Builtin::Csv => {
                let table = &self.model.tables[&self.current];
                if table.rows.is_empty() {
                    return Err(fail(args[0].pos, "the file has column names but no rows"));
                }
                Ok(Shape {
                    rows: Extent::of(SizeExpr::constant(count(table.rows.len()))),
                    columns: Extent::of(SizeExpr::constant(count(table.header.len()))),
                })
            }
            Builtin::Column => {
                let table = self.shape(&args[1])?;
                Ok(Shape {
                    rows: table.rows,
                    columns: Extent::one(),
                })
            }
            Builtin::Abs | Builtin::Exp | Builtin::Ln | Builtin::Sqrt | Builtin::Not => {
                self.shape(&args[0])
            }
            Builtin::Round => {
                let digits = self.shape(&args[0])?;
                if !self.is_scalar(&digits) {
                    return Err(fail(args[0].pos, "`round` takes one number of digits"));
                }
                self.shape(&args[1])
            }
            Builtin::Transpose => {
                let x = self.shape(&args[0])?;
                Ok(Shape {
                    rows: x.columns,
                    columns: x.rows,
                })
            }
            Builtin::Lookup => {
                let key = self.shape(&args[0])?;
                let keys = self.shape(&args[1])?;
                let values = self.shape(&args[2])?;
                if !self.is_one(&keys.rows.size) && !self.is_one(&keys.columns.size) {
                    return Err(fail(
                        args[1].pos,
                        "the keys of `lookup` are one row or one column",
                    ));
                }
                if !self.same_shape(&keys, &values) {
                    return Err(fail(
                        args[2].pos,
                        "the values of `lookup` have the same size and span as its keys",
                    ));
                }
                // One result for each key looked up, where the key is.
                Ok(key)
            }
        }
    }

    /// Infers a recurrence, in two passes. First the balance, where it is lagged, is taken as a
    /// single value, which stretches to fit anything: that gives the balance its shape from what
    /// lies outside the cycle. Then every member is inferred again, with that shape for the
    /// balance and with its signature checked, and the balance must come out the same.
    fn recurrence(&mut self, recurrence: &Recurrence) {
        let state = recurrence.state;
        let name = &self.model.bindings[state].name;
        let pos = self.model.bindings[state].pos;
        let vars = self.vars.len();
        let errors = self.errors.len();
        let fail = |infer: &mut Self, message: String| {
            infer.errors.push(Diagnostic::error(pos, message));
            for &member in &recurrence.members {
                infer.shapes[member] = None;
            }
        };
        self.shapes[state] = Some(Shape::scalar());
        for &member in &recurrence.members {
            self.current = member;
            match self.shape(self.model.definition(self.program, member)) {
                Ok(shape) => self.shapes[member] = Some(shape),
                Err(error) => {
                    self.errors.extend(error);
                    return fail(self, format!("`{name}` cannot be worked out"));
                }
            }
        }
        if self.vars.len() != vars {
            self.vars.truncate(vars);
            return fail(
                self,
                format!(
                    "a range made with `..` cannot be inside the cycle that carries `{name}`; \
                     make it a binding of its own, outside"
                ),
            );
        }
        let assumed = self.shapes[state].clone().expect("the balance has a shape");
        for &member in &recurrence.members {
            self.binding(member);
        }
        if self.errors.len() != errors {
            return fail(self, format!("`{name}` cannot be worked out"));
        }
        let settled = self.shapes[state].clone().expect("the balance has a shape");
        if !self.same_shape(&assumed, &settled) {
            return fail(self, format!("the size of `{name}` does not settle"));
        }
        if !self.is_one(&settled.rows.size) {
            return fail(
                self,
                format!(
                    "a balance carried from one period to the next is one row, and `{name}` has \
                     {} rows",
                    self.show(&settled.rows.size)
                ),
            );
        }
        let members: BTreeSet<usize> = recurrence.members.iter().copied().collect();
        let mut driver = None;
        for &member in &recurrence.members {
            let definition = self.model.definition(self.program, member);
            self.outside_rows(definition, &members, &mut driver);
        }
        match driver {
            Some(driver) => {
                self.drivers.insert(state, driver);
            }
            None => fail(
                self,
                format!(
                    "nothing outside the cycle gives `{name}` its length: it needs a row it uses \
                     period by period, such as a timeline"
                ),
            ),
        }
    }

    /// Whether an expression uses a binding in `members`.
    fn mentions(&self, expr: &Expr, members: &BTreeSet<usize>) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => matches!(
                self.model.symbols.get(name),
                Some(Symbol::Binding(binding)) if members.contains(binding)
            ),
            _ => expr
                .children()
                .into_iter()
                .any(|child| self.mentions(child, members)),
        }
    }

    /// Finds the largest parts of a member's definition that lie outside the cycle, and records
    /// those that are rows. The first is the one whose length the scan runs over.
    fn outside_rows(&mut self, expr: &Expr, members: &BTreeSet<usize>, driver: &mut Option<Pos>) {
        if !self.mentions(expr, members) {
            if let Ok(shape) = self.shape(expr)
                && !self.is_one(&shape.columns.size)
            {
                self.rows.insert(expr.pos);
                driver.get_or_insert(expr.pos);
            }
            return;
        }
        match &expr.kind {
            // The balance a period back is the scan's own; its first value starts the scan.
            ExprKind::Apply { func, .. } if func.name == "lag" => {}
            _ => {
                for child in expr.children() {
                    self.outside_rows(child, members, driver);
                }
            }
        }
    }

    fn same_shape(&self, a: &Shape, b: &Shape) -> bool {
        let same = |a: &Extent, b: &Extent| a.start == b.start && self.same(&a.size, &b.size);
        same(&a.rows, &b.rows) && same(&a.columns, &b.columns)
    }

    /// Drops `dropped` columns and moves the start along by `start`: `lag k` moves by `k`,
    /// `tail` by 1, and `init` stays put.
    fn shift(&self, x: &Shape, start: i64, dropped: i64, pos: Pos, what: &str) -> Inferred {
        let size = x.columns.size.offset(-dropped);
        if self.normal(&size).as_constant().is_some_and(|n| n < 1) {
            return Err(fail(
                pos,
                format!(
                    "`{what}` leaves no columns of a range that has {}",
                    self.show(&x.columns.size)
                ),
            ));
        }
        Ok(Shape {
            rows: x.rows.clone(),
            columns: Extent {
                start: x.columns.start + start,
                size,
            },
        })
    }

    fn combine(&self, a: &Shape, b: &Shape) -> Result<Shape, String> {
        Ok(Shape {
            rows: self.combine_extent(&a.rows, &b.rows, "rows")?,
            columns: self.combine_extent(&a.columns, &b.columns, "columns")?,
        })
    }

    /// Combines two operands along one dimension. The result reads after "the two sides of
    /// `+`" when it is a problem.
    fn combine_extent(&self, a: &Extent, b: &Extent, axis: &str) -> Result<Extent, String> {
        match (self.is_one(&a.size), self.is_one(&b.size)) {
            (true, true) => Ok(if a.start == 0 { b.clone() } else { a.clone() }),
            (true, false) => Ok(b.clone()),
            (false, true) => Ok(a.clone()),
            (false, false) if self.same(&a.size, &b.size) && a.start == b.start => Ok(a.clone()),
            (false, false) if self.same(&a.size, &b.size) => Err(format!(
                "are misaligned along the {axis}: one covers positions {} and the other {}; \
                 `lag` shifts a range along, and `tail` and `init` drop its ends",
                self.positions(a),
                self.positions(b)
            )),
            (false, false) => Err(format!(
                "have different numbers of {axis}: {} and {}",
                self.show(&a.size),
                self.show(&b.size)
            )),
        }
    }

    /// Checks a signature against the inferred shape. Where they agree, the shape takes the
    /// sizes as the signature writes them, so that a fixed size named there (`[s, 1]`) is
    /// carried through to what is laid out below it.
    fn check_signature(&mut self, sig: &Signature, mut shape: Shape) -> Result<Shape, Diagnostic> {
        let name = &sig.name.name;
        if let Some(written) = &sig.shape {
            shape.rows.size = self.match_size(&written.rows, &shape.rows.size, name, "rows")?;
            shape.columns.size =
                self.match_size(&written.columns, &shape.columns.size, name, "columns")?;
        }
        if let Some(from) = &sig.from
            && (from.row, from.column) != (shape.rows.start, shape.columns.start)
        {
            return Err(Diagnostic::error(
                from.pos,
                format!(
                    "`{name}` starts at [{}, {}], not [{}, {}]",
                    shape.rows.start, shape.columns.start, from.row, from.column
                ),
            ));
        }
        Ok(shape)
    }

    fn size_of_syntax(&self, syntax: &SizeSyntax) -> Result<SizeExpr, Diagnostic> {
        let mut terms = Vec::new();
        for (coefficient, ident) in &syntax.terms {
            match self.model.symbols.get(&ident.name) {
                Some(Symbol::Size(var)) => terms.push((*var, *coefficient)),
                Some(Symbol::Binding(_)) => {
                    return Err(Diagnostic::error(
                        ident.pos,
                        format!("`{}` is a binding, not a size", ident.name),
                    ));
                }
                None => {
                    return Err(Diagnostic::error(
                        ident.pos,
                        format!(
                            "unknown size `{0}`; declare it with `{0} : Size Unknown`",
                            ident.name
                        ),
                    ));
                }
            }
        }
        Ok(SizeExpr::from_parts(syntax.constant, terms))
    }

    fn match_size(
        &mut self,
        written: &SizeSyntax,
        inferred: &SizeExpr,
        name: &str,
        axis: &str,
    ) -> Result<SizeExpr, Diagnostic> {
        let declared = self.size_of_syntax(written)?;
        if self.same(&declared, inferred) {
            return Ok(declared);
        }
        if let Some((var, introduced)) = self.naming(&declared, inferred) {
            self.vars[introduced].alias = Some(var);
            self.vars[var].named = Some(introduced);
            return Ok(declared);
        }
        let taken = declared
            .terms()
            .find_map(|(var, _)| self.vars[var].named.map(|introduced| (var, introduced)));
        if let Some((var, introduced)) = taken
            && inferred.any_var(|v| !self.vars[self.canonical(v)].declared)
        {
            return Err(Diagnostic::error(
                written.pos,
                format!(
                    "`{}` already names the size introduced at {}; every `..` introduces a new \
                     size, so make `{name}` from the range that has that size",
                    self.var_name(var),
                    self.vars[introduced].pos
                ),
            ));
        }
        Err(Diagnostic::error(
            written.pos,
            format!(
                "`{name}` has {} {axis}, not {}",
                self.show(inferred),
                self.show(&declared)
            ),
        ))
    }

    /// When a signature names a size that a `..` introduced, the declared variable and the
    /// introduced one: both stand alone, with coefficient 1 and the same constant beside them.
    fn naming(&self, declared: &SizeExpr, inferred: &SizeExpr) -> Option<(usize, usize)> {
        let inferred = inferred.rename(|var| self.canonical(var));
        let declared_terms: Vec<(usize, i64)> = declared.terms().collect();
        let inferred_terms: Vec<(usize, i64)> = inferred.terms().collect();
        let (&[(var, 1)], &[(introduced, 1)]) =
            (declared_terms.as_slice(), inferred_terms.as_slice())
        else {
            return None;
        };
        let declared_var = &self.vars[var];
        let ok = declared_var.declared
            && declared_var.kind == VarKind::Unknown
            && declared_var.named.is_none()
            && !self.vars[introduced].declared
            && declared.constant_part() == inferred.constant_part();
        ok.then_some((var, introduced))
    }

    fn finish(mut self) -> Result<Typed, Vec<Diagnostic>> {
        if self.errors.is_empty() {
            for var in &self.vars {
                if var.declared && var.kind == VarKind::Unknown && var.named.is_none() {
                    let name = var.name.as_deref().unwrap_or_default();
                    self.errors.push(Diagnostic::error(
                        var.pos,
                        format!(
                            "nothing has the size `{name}`: name it in the signature of a range \
                             made with `..`, such as `period : [1, {name}]`"
                        ),
                    ));
                }
            }
        }
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // A size that no signature named takes its name from the binding that introduced it.
        let mut taken: BTreeSet<String> = self.model.symbols.keys().cloned().collect();
        for var in &mut self.vars {
            if var.name.is_some() || var.alias.is_some() {
                continue;
            }
            let binding = var.binding.expect("an introduced size has a binding");
            let base = format!("{}_n", self.model.bindings[binding].name);
            let mut name = base.clone();
            let mut suffix = 2;
            while taken.contains(&name) || Builtin::lookup(&name).is_some() {
                name = format!("{base}{suffix}");
                suffix += 1;
            }
            taken.insert(name.clone());
            var.name = Some(name);
        }

        let canonical: Vec<usize> = (0..self.vars.len()).map(|v| self.canonical(v)).collect();
        let extent = |extent: &Extent| Extent {
            start: extent.start,
            size: extent.size.rename(|var| canonical[var]),
        };
        let shapes = self
            .shapes
            .iter()
            .map(|shape| {
                let shape = shape
                    .as_ref()
                    .expect("with no errors, every binding has a shape");
                Shape {
                    rows: extent(&shape.rows),
                    columns: extent(&shape.columns),
                }
            })
            .collect();
        let vars = self
            .vars
            .into_iter()
            .map(|var| Var {
                introduced_by: if var.declared || var.alias.is_some() {
                    None
                } else {
                    var.binding
                },
                // A size that stands for a declared one appears in no shape, so needs no name.
                name: var.name.unwrap_or_default(),
                kind: var.kind,
            })
            .collect();
        Ok(Typed {
            vars,
            shapes,
            rows: self.rows,
            drivers: self.drivers,
        })
    }
}

fn count(n: usize) -> i64 {
    i64::try_from(n).expect("a list in a file has fewer than 2^63 values")
}

/// How many periods a `lag` moves: a whole number, at least 1, written in the file.
fn periods(arg: &Expr, what: &str) -> Result<i64, Option<Diagnostic>> {
    match arg.integer() {
        Some(periods) if periods >= 1 => Ok(periods),
        _ => Err(fail(
            arg.pos,
            format!(
                "`{what}` takes a whole number of periods written in the file, such as `{what} 1 \
                 revenue`"
            ),
        )),
    }
}

fn function_name(expr: &Expr) -> &str {
    match &expr.kind {
        ExprKind::Apply { func, .. } => &func.name,
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::resolve::resolve;

    fn typed(source: &str) -> Result<(Model, Typed), Vec<String>> {
        let program = parse(source).expect("parses");
        let model = resolve(&program, &crate::Inputs::new()).expect("resolves");
        infer(&program, &model)
            .map(|typed| (model, typed))
            .map_err(|errors| errors.iter().map(ToString::to_string).collect())
    }

    fn shapes(source: &str) -> Vec<String> {
        let (model, typed) = typed(source).expect("infers");
        model
            .bindings
            .iter()
            .zip(&typed.shapes)
            .map(|(binding, shape)| {
                format!(
                    "{} [{}, {}] from [{}, {}]",
                    binding.name,
                    typed.format(&shape.rows.size),
                    typed.format(&shape.columns.size),
                    shape.rows.start,
                    shape.columns.start
                )
            })
            .collect()
    }

    const FORECAST: &str = "yup 0.1\nhorizon : 'Horizon' Input\nhorizon = 5\nperiod : \
                            'Period'\nperiod = 1 .. horizon\nrevenue = 100 * 1.1 ^ period\n";

    #[test]
    fn spans_follow_lag_tail_and_init() {
        let source =
            format!("{FORECAST}growth = tail revenue / lag 1 revenue - 1\nfirst = init revenue\n");
        assert_eq!(
            shapes(&source),
            [
                "horizon [1, 1] from [0, 0]",
                "period [1, period_n] from [0, 0]",
                "revenue [1, period_n] from [0, 0]",
                "growth [1, period_n - 1] from [0, 1]",
                "first [1, period_n - 1] from [0, 0]",
            ]
        );
    }

    #[test]
    fn misaligned_ranges_do_not_combine() {
        let source = format!("{FORECAST}growth = tail revenue / init revenue\n");
        assert_eq!(
            typed(&source).expect_err("misaligned"),
            [
                "7:23: error: the two sides of `/` are misaligned along the columns: one covers \
              positions [1, period_n) and the other [0, period_n - 1); `lag` shifts a range \
              along, and `tail` and `init` drop its ends"
            ]
        );
    }

    #[test]
    fn every_range_made_with_dots_has_its_own_size() {
        let source = format!("{FORECAST}other = 1 .. horizon\ntotal = period + other\n");
        assert_eq!(
            typed(&source).expect_err("two sizes"),
            [
                "8:16: error: the two sides of `+` have different numbers of columns: period_n and \
              other_n"
            ]
        );
    }

    #[test]
    fn a_signature_names_a_size_and_is_checked() {
        let named = "yup 0.1\nn : Size Unknown\nh = 5\np : [1, n] 'P'\np = 1 .. h\nq : [1, n - \
                     1]\nq = tail p\n";
        assert_eq!(
            shapes(named),
            [
                "h [1, 1] from [0, 0]",
                "p [1, n] from [0, 0]",
                "q [1, n - 1] from [0, 1]"
            ]
        );
        let wrong = "yup 0.1\nn : Size Unknown\nh = 5\np : [1, n] 'P'\np = 1 .. h\nq : [1, n]\nq \
                     = tail p\n";
        assert_eq!(
            typed(wrong).expect_err("wrong size"),
            ["6:9: error: `q` has n - 1 columns, not n"]
        );
        let again = "yup 0.1\nn : Size Unknown\nh = 5\np : [1, n] 'P'\np = 1 .. h\nq : [1, n]\nq \
                     = 1 .. h\n";
        assert_eq!(
            typed(again).expect_err("named twice"),
            [
                "6:9: error: `n` already names the size introduced at 5:7; every `..` introduces a \
              new size, so make `q` from the range that has that size"
            ]
        );
    }

    #[test]
    fn fixed_sizes_compare_by_value() {
        let source = "yup 0.1\ns : Size Fixed\ns = 3\nsegment : [1, 3]\nsegment = 1 .. s\nbase = \
                      [100; 200; 300]\nyear = 1 .. 5\ngrid = base * 1.1 ^ year\n";
        assert_eq!(
            shapes(source),
            [
                "segment [1, 3] from [0, 0]",
                "base [3, 1] from [0, 0]",
                "year [1, 5] from [0, 0]",
                "grid [3, 5] from [0, 0]",
            ]
        );
    }

    #[test]
    fn a_seed_fills_the_first_position() {
        let source = format!("{FORECAST}opening = lag 1 revenue from 100\n");
        assert_eq!(shapes(&source)[3], "opening [1, period_n] from [0, 0]");
    }
}
