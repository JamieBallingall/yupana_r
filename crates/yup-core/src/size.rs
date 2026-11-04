//! Sizes: a whole number plus a sum of size variables, each with a positive coefficient.
//!
//! Positive coefficients keep every extent growing with its variables. That is what makes the
//! check for collisions simple: two ranges that ever overlap also overlap when the variables are
//! large.

use std::collections::BTreeMap;

/// A size, such as `n - 1` or `2 * s + 4`, with variables numbered.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SizeExpr {
    constant: i64,
    /// Each variable's coefficient. A variable with coefficient zero is left out, so that equal
    /// sizes compare equal.
    terms: BTreeMap<usize, i64>,
}

impl SizeExpr {
    /// A size that is a plain number.
    pub fn constant(value: i64) -> SizeExpr {
        SizeExpr {
            constant: value,
            terms: BTreeMap::new(),
        }
    }

    /// A size that is exactly the variable `var`.
    pub fn var(var: usize) -> SizeExpr {
        SizeExpr::from_parts(0, [(var, 1)])
    }

    /// A size from its constant and its terms. Terms for the same variable add up.
    pub fn from_parts(constant: i64, terms: impl IntoIterator<Item = (usize, i64)>) -> SizeExpr {
        let mut size = SizeExpr::constant(constant);
        for (var, coefficient) in terms {
            size.add_term(var, coefficient);
        }
        size
    }

    fn add_term(&mut self, var: usize, coefficient: i64) {
        let entry = self.terms.entry(var).or_insert(0);
        *entry = entry.saturating_add(coefficient);
        if *entry == 0 {
            self.terms.remove(&var);
        }
    }

    /// The value, if the size has no variables.
    pub fn as_constant(&self) -> Option<i64> {
        self.terms.is_empty().then_some(self.constant)
    }

    /// The whole-number part.
    pub fn constant_part(&self) -> i64 {
        self.constant
    }

    /// Each variable with its coefficient, in the order of the variables' numbers.
    pub fn terms(&self) -> impl Iterator<Item = (usize, i64)> + '_ {
        self.terms
            .iter()
            .map(|(&var, &coefficient)| (var, coefficient))
    }

    /// Whether any term satisfies `test`.
    pub fn any_var(&self, test: impl Fn(usize) -> bool) -> bool {
        self.terms.keys().any(|&var| test(var))
    }

    /// This size plus a number.
    pub fn offset(&self, amount: i64) -> SizeExpr {
        SizeExpr {
            constant: self.constant.saturating_add(amount),
            terms: self.terms.clone(),
        }
    }

    /// This size plus another.
    pub fn plus(&self, other: &SizeExpr) -> SizeExpr {
        let mut sum = self.offset(other.constant);
        for (var, coefficient) in other.terms() {
            sum.add_term(var, coefficient);
        }
        sum
    }

    /// This size less another. Only a cell is worked out this way, never an extent, so the
    /// result may have a negative coefficient until its fixed sizes are replaced by their values.
    pub fn minus(&self, other: &SizeExpr) -> SizeExpr {
        let mut difference = self.offset(other.constant.saturating_neg());
        for (var, coefficient) in other.terms() {
            difference.add_term(var, coefficient.saturating_neg());
        }
        difference
    }

    /// The size with each variable that `value` knows replaced by its value.
    pub fn substitute(&self, value: impl Fn(usize) -> Option<i64>) -> SizeExpr {
        let mut result = SizeExpr::constant(self.constant);
        for (var, coefficient) in self.terms() {
            match value(var) {
                Some(v) => {
                    result.constant = result
                        .constant
                        .saturating_add(coefficient.saturating_mul(v));
                }
                None => result.add_term(var, coefficient),
            }
        }
        result
    }

    /// The size with each variable renumbered by `rename`.
    pub fn rename(&self, rename: impl Fn(usize) -> usize) -> SizeExpr {
        SizeExpr::from_parts(
            self.constant,
            self.terms()
                .map(|(var, coefficient)| (rename(var), coefficient)),
        )
    }

    /// The value when every variable has the value `value` gives it.
    pub fn evaluate(&self, value: impl Fn(usize) -> i64) -> i64 {
        self.terms()
            .fold(self.constant, |total, (var, coefficient)| {
                total.saturating_add(coefficient.saturating_mul(value(var)))
            })
    }

    /// The size as `.yup` writes it, with variables named by `name` and in alphabetical order.
    pub fn format(&self, name: impl Fn(usize) -> String) -> String {
        let mut terms: Vec<(String, i64)> = self
            .terms()
            .map(|(var, coefficient)| (name(var), coefficient))
            .collect();
        terms.sort();
        let mut text = String::new();
        for (name, coefficient) in terms {
            if !text.is_empty() {
                text.push_str(" + ");
            }
            if coefficient == 1 {
                text.push_str(&name);
            } else {
                text.push_str(&format!("{coefficient} * {name}"));
            }
        }
        if text.is_empty() {
            return self.constant.to_string();
        }
        if self.constant > 0 {
            text.push_str(&format!(" + {}", self.constant));
        } else if self.constant < 0 {
            text.push_str(&format!(" - {}", self.constant.unsigned_abs()));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(var: usize) -> String {
        ["n", "s"][var].to_string()
    }

    #[test]
    fn sizes_add_and_cancel() {
        let n = SizeExpr::var(0);
        let s = SizeExpr::var(1);
        let size = n.offset(-1).plus(&s).plus(&s).offset(4);
        assert_eq!(size.format(name), "n + 2 * s + 3");
        assert_eq!(
            size.substitute(|v| (v == 1).then_some(4)).format(name),
            "n + 11"
        );
        assert_eq!(size.evaluate(|_| 1), 6);
        assert_eq!(
            SizeExpr::from_parts(2, [(0, 1), (0, -1)]),
            SizeExpr::constant(2)
        );
        assert_eq!(SizeExpr::constant(3).as_constant(), Some(3));
        assert_eq!(n.as_constant(), None);
    }
}
