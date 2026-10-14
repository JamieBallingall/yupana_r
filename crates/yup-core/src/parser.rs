//! Step 1, Parse: from tokens to a syntax tree, one statement per line.
//!
//! A statement that does not parse is reported and skipped, so that one run reports every line
//! that is wrong rather than only the first.
//!
//! Precedence, from loosest to tightest: `if` and `..`; `||`; `&&`; comparisons; `++`; `+` and
//! `-`; `*` and `/`; negation; `^`; application of a function to its arguments.

use crate::diagnostic::{Diagnostic, Pos};
use crate::lexer::{Lexeme, Token, lex};
use crate::syntax::{
    BinOp, Coord, Expr, ExprKind, Ident, Item, ItemKind, Offset, Program, ShapeSyntax, Signature,
    SizeKind, SizeSyntax, Text,
};

/// The language version this reader accepts, as the first line of a file gives it.
pub const VERSION: &str = "0.1";

/// Names that cannot be used for bindings, size variables or functions.
pub const KEYWORDS: &[&str] = &[
    "else", "false", "from", "gap", "heading", "if", "sheet", "then", "true", "yup",
];

/// Parses a whole file.
pub fn parse(source: &str) -> Result<Program, Vec<Diagnostic>> {
    let tokens = lex(source).map_err(|error| vec![error])?;
    let mut parser = Parser {
        tokens,
        next: 0,
        errors: Vec::new(),
    };
    let program = parser.program();
    if parser.errors.is_empty() {
        Ok(program)
    } else {
        Err(parser.errors)
    }
}

type Parsed<T> = Result<T, Diagnostic>;

struct Parser {
    tokens: Vec<Lexeme>,
    next: usize,
    errors: Vec<Diagnostic>,
}

fn is_keyword(name: &str) -> bool {
    KEYWORDS.contains(&name)
}

fn binary(op: BinOp, left: Expr, right: Expr, pos: Pos) -> Expr {
    Expr {
        pos,
        kind: ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
    }
}

fn comparison_op(token: &Token) -> Option<BinOp> {
    match token {
        Token::EqEq => Some(BinOp::Eq),
        Token::NotEq => Some(BinOp::Ne),
        Token::Less => Some(BinOp::Lt),
        Token::LessEq => Some(BinOp::Le),
        Token::Greater => Some(BinOp::Gt),
        Token::GreaterEq => Some(BinOp::Ge),
        _ => None,
    }
}

/// Whether a token can start an argument of a function.
fn starts_atom(token: &Token) -> bool {
    match token {
        Token::Number(..) | Token::Str(_) | Token::LParen | Token::LBracket => true,
        Token::Name(name) => name == "true" || name == "false" || !is_keyword(name),
        _ => false,
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, pos: Pos, what: &str) -> Parsed<()> {
    if slot.is_some() {
        return Err(Diagnostic::error(pos, format!("{what} is given twice")));
    }
    *slot = Some(value);
    Ok(())
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.next].token
    }

    fn peek_second(&self) -> &Token {
        let index = (self.next + 1).min(self.tokens.len() - 1);
        &self.tokens[index].token
    }

    fn pos(&self) -> Pos {
        self.tokens[self.next].pos
    }

    /// Moves past the current token and returns it. The final `Eof` is never passed.
    fn advance(&mut self) -> Lexeme {
        let lexeme = self.tokens[self.next].clone();
        if self.next + 1 < self.tokens.len() {
            self.next += 1;
        }
        lexeme
    }

    fn at_name(&self, name: &str) -> bool {
        matches!(self.peek(), Token::Name(n) if n == name)
    }

    fn unexpected(&self, what: &str) -> Diagnostic {
        Diagnostic::error(
            self.pos(),
            format!("expected {what}, found {}", self.peek().describe()),
        )
    }

    fn expect(&mut self, token: &Token, what: &str) -> Parsed<Pos> {
        if self.peek() == token {
            Ok(self.advance().pos)
        } else {
            Err(self.unexpected(what))
        }
    }

    fn skip_line(&mut self) {
        while !matches!(self.peek(), Token::Newline | Token::Eof) {
            self.advance();
        }
        if *self.peek() == Token::Newline {
            self.advance();
        }
    }

    fn program(&mut self) -> Program {
        let version = match self.version() {
            Ok(version) => version,
            Err(error) => {
                self.errors.push(error);
                self.skip_line();
                VERSION.to_string()
            }
        };
        let mut items = Vec::new();
        while *self.peek() != Token::Eof {
            match self.statement() {
                Ok(item) => items.push(item),
                Err(error) => {
                    self.errors.push(error);
                    self.skip_line();
                }
            }
        }
        Program { version, items }
    }

    fn version(&mut self) -> Parsed<String> {
        let missing = Diagnostic::error(
            self.pos(),
            format!("a .yup file starts with the line `yup {VERSION}`"),
        );
        if !self.at_name("yup") {
            return Err(missing);
        }
        self.advance();
        let Token::Number(_, text) = self.peek().clone() else {
            return Err(missing);
        };
        let pos = self.advance().pos;
        if text != VERSION {
            return Err(Diagnostic::error(
                pos,
                format!("this reader reads .yup {VERSION}, not {text}"),
            ));
        }
        self.end_of_line()?;
        Ok(text)
    }

    fn end_of_line(&mut self) -> Parsed<()> {
        match self.peek() {
            Token::Newline => {
                self.advance();
                Ok(())
            }
            Token::Eof => Ok(()),
            _ => Err(self.unexpected("the end of the line")),
        }
    }

    fn end_of_statement(&mut self) -> Parsed<Option<String>> {
        let comment = match self.peek().clone() {
            Token::Comment(text) => {
                self.advance();
                Some(text)
            }
            _ => None,
        };
        self.end_of_line()?;
        Ok(comment)
    }

    fn statement(&mut self) -> Parsed<Item> {
        let pos = self.pos();
        let kind = match self.peek().clone() {
            Token::Newline => {
                self.advance();
                return Ok(Item {
                    pos,
                    kind: ItemKind::Blank,
                    comment: None,
                });
            }
            Token::Comment(text) => {
                self.advance();
                self.end_of_line()?;
                return Ok(Item {
                    pos,
                    kind: ItemKind::Comment(text),
                    comment: None,
                });
            }
            Token::Name(name) => match name.as_str() {
                "sheet" => {
                    self.advance();
                    ItemKind::Sheet(self.text("a sheet name in single quotes, such as 'Forecast'")?)
                }
                "heading" => {
                    self.advance();
                    let text = self.text("a heading in single quotes")?;
                    let at = if *self.peek() == Token::At {
                        self.advance();
                        Some(self.coord()?)
                    } else {
                        None
                    };
                    ItemKind::Heading { text, at }
                }
                "gap" => {
                    self.advance();
                    ItemKind::Gap
                }
                _ if is_keyword(&name) => {
                    return Err(Diagnostic::error(
                        pos,
                        format!("`{name}` is a keyword and cannot start a statement"),
                    ));
                }
                _ => match self.peek_second() {
                    Token::Colon => self.signature()?,
                    // A name followed by names is a function and its parameters.
                    Token::Equals | Token::Name(_) => self.definition()?,
                    _ => {
                        self.advance();
                        return Err(self.unexpected(&format!("`:` or `=` after `{name}`")));
                    }
                },
            },
            _ => return Err(self.unexpected("a statement")),
        };
        let comment = self.end_of_statement()?;
        Ok(Item { pos, kind, comment })
    }

    fn text(&mut self, what: &str) -> Parsed<Text> {
        match self.peek().clone() {
            Token::Text(value) => {
                let pos = self.advance().pos;
                Ok(Text { value, pos })
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn ident(&mut self) -> Parsed<Ident> {
        match self.peek().clone() {
            Token::Name(name) if is_keyword(&name) => Err(Diagnostic::error(
                self.pos(),
                format!("`{name}` is a keyword, not a name"),
            )),
            Token::Name(name) => {
                let pos = self.advance().pos;
                Ok(Ident { name, pos })
            }
            _ => Err(self.unexpected("a name")),
        }
    }

    fn integer(&mut self, what: &str) -> Parsed<i64> {
        match self.peek().clone() {
            Token::Number(_, text) if text.bytes().all(|b| b.is_ascii_digit()) => {
                let pos = self.advance().pos;
                text.parse()
                    .map_err(|_| Diagnostic::error(pos, format!("`{text}` is too large")))
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn signature(&mut self) -> Parsed<ItemKind> {
        let name = self.ident()?;
        self.expect(&Token::Colon, "`:`")?;
        if matches!(self.peek(), Token::Word(w) if w == "Size") {
            self.advance();
            let kind = match self.peek() {
                Token::Word(w) if w == "Fixed" => SizeKind::Fixed,
                Token::Word(w) if w == "Unknown" => SizeKind::Unknown,
                _ => return Err(self.unexpected("`Fixed` or `Unknown` after `Size`")),
            };
            self.advance();
            return Ok(ItemKind::Size { name, kind });
        }
        let mut sig = Signature::new(name);
        loop {
            let pos = self.pos();
            match self.peek().clone() {
                Token::Newline | Token::Comment(_) | Token::Eof => break,
                Token::LBracket => {
                    let shape = self.shape()?;
                    set_once(&mut sig.shape, shape, pos, "a size")?;
                }
                Token::Name(name) if name == "from" => {
                    self.advance();
                    let offset = self.offset(pos)?;
                    set_once(&mut sig.from, offset, pos, "`from`")?;
                }
                Token::Text(_) => {
                    let label = self.text("a label")?;
                    set_once(&mut sig.label, label, pos, "a label")?;
                }
                Token::At => {
                    self.advance();
                    let at = self.coord()?;
                    set_once(&mut sig.at, at, pos, "an anchor")?;
                }
                Token::Word(word) => {
                    self.advance();
                    match word.as_str() {
                        "Input" => set_once(&mut sig.input, pos, pos, "`Input`")?,
                        "Bold" => set_once(&mut sig.bold, pos, pos, "`Bold`")?,
                        "Italic" => set_once(&mut sig.italic, pos, pos, "`Italic`")?,
                        "Format" => {
                            let format = match self.peek().clone() {
                                Token::Str(value) => {
                                    let pos = self.advance().pos;
                                    Text { value, pos }
                                }
                                _ => {
                                    return Err(self.unexpected(
                                        "a number format in double quotes, such as \"0.0%\"",
                                    ));
                                }
                            };
                            set_once(&mut sig.format, format, pos, "`Format`")?;
                        }
                        "Size" => {
                            return Err(Diagnostic::error(
                                pos,
                                "`Size` declares a size variable on a line of its own, such as \
                                 `n : Size Unknown`",
                            ));
                        }
                        _ => {
                            return Err(Diagnostic::error(
                                pos,
                                format!(
                                    "unknown attribute `{word}`; a signature takes Input, \
                                     Format, Bold and Italic"
                                ),
                            ));
                        }
                    }
                }
                _ => {
                    return Err(self.unexpected(
                        "a size such as `[1, n]`, a label in single quotes, `@`, or an \
                         attribute such as `Input`",
                    ));
                }
            }
        }
        Ok(ItemKind::Signature(Box::new(sig)))
    }

    fn shape(&mut self) -> Parsed<ShapeSyntax> {
        let pos = self.expect(&Token::LBracket, "`[`")?;
        let rows = self.size()?;
        self.expect(&Token::Comma, "`,` between rows and columns")?;
        let columns = self.size()?;
        self.expect(&Token::RBracket, "`]`")?;
        Ok(ShapeSyntax { rows, columns, pos })
    }

    fn coord(&mut self) -> Parsed<Coord> {
        let pos = self.expect(&Token::LParen, "`(` to start a cell, such as `(2, 4)`")?;
        let row = self.size()?;
        self.expect(&Token::Comma, "`,` between row and column")?;
        let column = self.size()?;
        self.expect(&Token::RParen, "`)`")?;
        Ok(Coord { row, column, pos })
    }

    fn offset(&mut self, pos: Pos) -> Parsed<Offset> {
        self.expect(&Token::LBracket, "`[` after `from`")?;
        let row = self.integer("a whole number")?;
        self.expect(&Token::Comma, "`,`")?;
        let column = self.integer("a whole number")?;
        self.expect(&Token::RBracket, "`]`")?;
        Ok(Offset { row, column, pos })
    }

    fn size(&mut self) -> Parsed<SizeSyntax> {
        let pos = self.pos();
        let mut constant: i64 = 0;
        let mut terms = Vec::new();
        let mut negative = false;
        loop {
            let term_pos = self.pos();
            match self.peek() {
                Token::Number(..) => {
                    let value = self.integer("a whole number")?;
                    if *self.peek() == Token::Star {
                        self.advance();
                        let name = self.ident()?;
                        if negative {
                            return Err(Diagnostic::error(
                                term_pos,
                                "a size can subtract a number but not a size variable",
                            ));
                        }
                        if value == 0 {
                            return Err(Diagnostic::error(term_pos, "a coefficient of zero"));
                        }
                        terms.push((value, name));
                    } else {
                        let signed = if negative { -value } else { value };
                        constant = constant
                            .checked_add(signed)
                            .ok_or_else(|| Diagnostic::error(term_pos, "this size is too large"))?;
                    }
                }
                Token::Name(_) => {
                    let name = self.ident()?;
                    if negative {
                        return Err(Diagnostic::error(
                            term_pos,
                            "a size can subtract a number but not a size variable",
                        ));
                    }
                    terms.push((1, name));
                }
                _ => return Err(self.unexpected("a size: a whole number or a size variable")),
            }
            match self.peek() {
                Token::Plus => negative = false,
                Token::Minus => negative = true,
                _ => break,
            }
            self.advance();
        }
        Ok(SizeSyntax {
            constant,
            terms,
            pos,
        })
    }

    fn definition(&mut self) -> Parsed<ItemKind> {
        let name = self.ident()?;
        let mut params = Vec::new();
        while matches!(self.peek(), Token::Name(_)) {
            params.push(self.ident()?);
        }
        self.expect(&Token::Equals, "`=`")?;
        let expr = self.expr()?;
        Ok(ItemKind::Definition { name, params, expr })
    }

    fn expr(&mut self) -> Parsed<Expr> {
        let first = self.disjunction()?;
        if *self.peek() != Token::DotDot {
            return Ok(first);
        }
        let pos = self.advance().pos;
        let last = self.disjunction()?;
        if *self.peek() == Token::DotDot {
            return Err(Diagnostic::error(self.pos(), "ranges do not chain"));
        }
        Ok(Expr {
            pos,
            kind: ExprKind::Range {
                first: Box::new(first),
                last: Box::new(last),
            },
        })
    }

    fn disjunction(&mut self) -> Parsed<Expr> {
        let left = self.conjunction()?;
        if *self.peek() != Token::OrOr {
            return Ok(left);
        }
        let pos = self.advance().pos;
        let right = self.disjunction()?;
        Ok(binary(BinOp::Or, left, right, pos))
    }

    fn conjunction(&mut self) -> Parsed<Expr> {
        let left = self.comparison()?;
        if *self.peek() != Token::AndAnd {
            return Ok(left);
        }
        let pos = self.advance().pos;
        let right = self.conjunction()?;
        Ok(binary(BinOp::And, left, right, pos))
    }

    fn comparison(&mut self) -> Parsed<Expr> {
        let left = self.concat()?;
        let Some(op) = comparison_op(self.peek()) else {
            return Ok(left);
        };
        let pos = self.advance().pos;
        let right = self.concat()?;
        if comparison_op(self.peek()).is_some() {
            return Err(Diagnostic::error(
                self.pos(),
                "comparisons do not chain; combine them with `if`",
            ));
        }
        Ok(binary(op, left, right, pos))
    }

    fn concat(&mut self) -> Parsed<Expr> {
        let left = self.additive()?;
        if *self.peek() != Token::PlusPlus {
            return Ok(left);
        }
        let pos = self.advance().pos;
        let right = self.concat()?;
        Ok(binary(BinOp::Concat, left, right, pos))
    }

    fn additive(&mut self) -> Parsed<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => return Ok(left),
            };
            let pos = self.advance().pos;
            let right = self.multiplicative()?;
            left = binary(op, left, right, pos);
        }
    }

    fn multiplicative(&mut self) -> Parsed<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => return Ok(left),
            };
            let pos = self.advance().pos;
            let right = self.unary()?;
            left = binary(op, left, right, pos);
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        if *self.peek() == Token::Minus {
            let pos = self.advance().pos;
            let operand = self.unary()?;
            return Ok(Expr {
                pos,
                kind: ExprKind::Neg(Box::new(operand)),
            });
        }
        if self.at_name("if") {
            return self.conditional();
        }
        self.power()
    }

    fn conditional(&mut self) -> Parsed<Expr> {
        let pos = self.advance().pos;
        let cond = self.expr()?;
        if !self.at_name("then") {
            return Err(self.unexpected("`then`"));
        }
        self.advance();
        let then = self.expr()?;
        if !self.at_name("else") {
            return Err(self.unexpected("`else`; an `if` always has one"));
        }
        self.advance();
        let other = self.expr()?;
        Ok(Expr {
            pos,
            kind: ExprKind::If {
                cond: Box::new(cond),
                then: Box::new(then),
                other: Box::new(other),
            },
        })
    }

    fn power(&mut self) -> Parsed<Expr> {
        let base = self.application()?;
        if *self.peek() != Token::Caret {
            return Ok(base);
        }
        let pos = self.advance().pos;
        let exponent = self.unary()?;
        Ok(binary(BinOp::Pow, base, exponent, pos))
    }

    fn application(&mut self) -> Parsed<Expr> {
        if let Token::Name(name) = self.peek()
            && !is_keyword(name)
            && starts_atom(self.peek_second())
        {
            let func = self.ident()?;
            let mut args = Vec::new();
            while starts_atom(self.peek()) {
                args.push(self.atom()?);
            }
            let seed = if func.name == "lag" && self.at_name("from") {
                self.advance();
                Some(Box::new(self.atom()?))
            } else {
                None
            };
            return Ok(Expr {
                pos: func.pos,
                kind: ExprKind::Apply { func, args, seed },
            });
        }
        let atom = self.atom()?;
        if self.at_name("from") {
            return Err(Diagnostic::error(
                self.pos(),
                "`from` gives the first value of a lag, as in `lag 1 closing from opening`",
            ));
        }
        Ok(atom)
    }

    fn atom(&mut self) -> Parsed<Expr> {
        let pos = self.pos();
        let kind = match self.peek().clone() {
            Token::Number(value, text) => ExprKind::Number { value, text },
            Token::Str(value) => ExprKind::Str(value),
            Token::Name(name) if name == "true" => ExprKind::Bool(true),
            Token::Name(name) if name == "false" => ExprKind::Bool(false),
            Token::Name(name) if !is_keyword(&name) => ExprKind::Name(name),
            Token::LParen => {
                self.advance();
                let inner = self.expr()?;
                self.expect(&Token::RParen, "`)`")?;
                return Ok(inner);
            }
            Token::LBracket => return self.list(),
            Token::Text(_) => {
                return Err(Diagnostic::error(
                    pos,
                    "single quotes hold a label or a sheet name; a string value takes double \
                     quotes",
                ));
            }
            _ => return Err(self.unexpected("a value")),
        };
        self.advance();
        Ok(Expr { pos, kind })
    }

    fn list(&mut self) -> Parsed<Expr> {
        let pos = self.advance().pos;
        if *self.peek() == Token::RBracket {
            return Err(Diagnostic::error(pos, "a list holds at least one value"));
        }
        let mut rows = vec![vec![self.expr()?]];
        loop {
            match self.peek() {
                Token::Comma => {
                    self.advance();
                    let value = self.expr()?;
                    rows.last_mut()
                        .expect("a list starts with a row")
                        .push(value);
                }
                Token::Semicolon => {
                    self.advance();
                    rows.push(vec![self.expr()?]);
                }
                Token::RBracket => {
                    self.advance();
                    break;
                }
                _ => return Err(self.unexpected("`,`, `;` or `]`")),
            }
        }
        Ok(Expr {
            pos,
            kind: ExprKind::List(rows),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(source: &str) -> Vec<String> {
        parse(source)
            .expect_err("the source is wrong")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn definition(source: &str) -> Expr {
        let program = parse(&format!("yup 0.1\nx = {source}\n")).expect("parses");
        match program.items.into_iter().next().map(|item| item.kind) {
            Some(ItemKind::Definition { expr, .. }) => expr,
            other => panic!("expected a definition, found {other:?}"),
        }
    }

    #[test]
    fn the_first_line_gives_the_version() {
        assert_eq!(
            errors("x = 1\n"),
            ["1:1: error: a .yup file starts with the line `yup 0.1`"]
        );
        assert_eq!(
            errors("yup 0.2\n"),
            ["1:5: error: this reader reads .yup 0.1, not 0.2"]
        );
    }

    #[test]
    fn a_signature_takes_its_parts_in_any_order() {
        let program =
            parse("yup 0.1\ng : 'Growth' [1, n - 1] from [0, 1] Format \"0.0%\" @ (4, 5) Bold\n")
                .expect("parses");
        let ItemKind::Signature(sig) = &program.items[0].kind else {
            panic!("expected a signature");
        };
        assert_eq!(sig.label.as_ref().map(|t| t.value.as_str()), Some("Growth"));
        let shape = sig.shape.as_ref().expect("a shape");
        assert_eq!(shape.columns.constant, -1);
        assert_eq!(shape.columns.terms[0].1.name, "n");
        assert_eq!(sig.from.as_ref().map(|f| (f.row, f.column)), Some((0, 1)));
        assert_eq!(sig.format.as_ref().map(|t| t.value.as_str()), Some("0.0%"));
        assert_eq!(sig.at.as_ref().map(|c| c.row.constant), Some(4));
        assert!(sig.bold.is_some());
        assert!(sig.italic.is_none());
    }

    #[test]
    fn application_binds_tighter_than_operators() {
        let ExprKind::Binary { op, left, right } = definition("tail revenue / lag 1 revenue").kind
        else {
            panic!("expected a division");
        };
        assert_eq!(op, BinOp::Div);
        assert!(matches!(left.kind, ExprKind::Apply { ref args, .. } if args.len() == 1));
        assert!(matches!(right.kind, ExprKind::Apply { ref args, .. } if args.len() == 2));
    }

    #[test]
    fn negation_is_looser_than_power() {
        let ExprKind::Neg(operand) = definition("-2 ^ 2").kind else {
            panic!("expected a negation");
        };
        assert!(matches!(
            operand.kind,
            ExprKind::Binary { op: BinOp::Pow, .. }
        ));
    }

    #[test]
    fn a_lag_takes_a_seed() {
        let ExprKind::Apply { func, seed, .. } = definition("lag 1 closing from opening").kind
        else {
            panic!("expected an application");
        };
        assert_eq!(func.name, "lag");
        assert!(matches!(seed.map(|s| s.kind), Some(ExprKind::Name(n)) if n == "opening"));
    }

    #[test]
    fn every_wrong_line_is_reported() {
        assert_eq!(
            errors("yup 0.1\nx = \ny = 1 +\nz = (1 2)\n"),
            [
                "2:5: error: expected a value, found the end of the line",
                "3:8: error: expected a value, found the end of the line",
                "4:8: error: expected `)`, found the number `2`",
            ]
        );
    }

    #[test]
    fn comparisons_do_not_chain() {
        assert_eq!(
            errors("yup 0.1\nx = a < b < c\n"),
            ["2:11: error: comparisons do not chain; combine them with `if`"]
        );
    }

    #[test]
    fn a_size_subtracts_only_numbers() {
        assert_eq!(
            errors("yup 0.1\nx : [1, n - m]\n"),
            ["2:13: error: a size can subtract a number but not a size variable"]
        );
    }
}
