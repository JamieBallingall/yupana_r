//! Step 1, Parse: from tokens to a syntax tree, one statement per line.
//!
//! A statement that does not parse is reported and skipped, so that one run reports every line
//! that is wrong rather than only the first.
//!
//! Precedence, from loosest to tightest: `if` and `..`; `||`; `&&`; comparisons; `++`; `+` and
//! `-`; `*` and `/`; negation; `^`; application of a function to its arguments.

use crate::cells::{MAX_COLUMNS, MAX_ROWS, parse_cell};
use crate::diagnostic::{Diagnostic, Pos};
use crate::functions::Builtin;
use crate::lexer::{Lexeme, Token, lex};
use crate::syntax::{
    Axis, BinOp, CellType, Direction, Expr, ExprKind, Field, Ident, Item, ItemKind, Move, Offset,
    Placement, Program, Role, ShapeSyntax, Side, Signature, SizeKind, SizeSyntax, StyleName,
    StyleType, Type,
};

/// The language version this reader accepts, as the first line of a file gives it.
pub const VERSION: &str = "0.2";

/// Names that cannot be used for bindings, size variables or functions.
pub const KEYWORDS: &[&str] = &[
    "across", "at", "down", "east", "else", "false", "from", "if", "left", "north", "right",
    "seed", "south", "start", "then", "true", "up", "west", "yup",
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

pub fn is_keyword(name: &str) -> bool {
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
        Token::Number(..)
        | Token::Str(_)
        | Token::Text(_)
        | Token::Word(_)
        | Token::LParen
        | Token::LBracket => true,
        Token::Name(name) => !is_keyword(name),
        _ => false,
    }
}

/// The functions that may say which way they work along a range.
fn takes_axis(name: &str) -> bool {
    matches!(name, "lag" | "tail" | "init")
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
            Token::Name(name) if is_keyword(&name) => {
                return Err(Diagnostic::error(
                    pos,
                    format!("`{name}` is a keyword and cannot start a statement"),
                ));
            }
            Token::Name(name) => match self.peek_second() {
                Token::Colon => self.signature()?,
                // A name followed by names is a lambda and its parameters.
                Token::Equals | Token::Name(_) => self.definition()?,
                Token::Text(_) | Token::Newline
                    if matches!(name.as_str(), "sheet" | "heading" | "gap") =>
                {
                    return Err(Diagnostic::error(
                        pos,
                        format!(
                            "`{name}` statements are gone: a sheet is a binding, \
                             `forecast : Sheet`, a heading is a `Label`, and a gap is a \
                             distance, as in `total_label down 2`"
                        ),
                    ));
                }
                _ => {
                    self.advance();
                    return Err(self.unexpected(&format!("`:` or `=` after `{name}`")));
                }
            },
            Token::Word(word) => {
                return Err(Diagnostic::error(
                    pos,
                    format!(
                        "a statement starts with a binding's name, in lower case, not `{word}`"
                    ),
                ));
            }
            _ => return Err(self.unexpected("a statement")),
        };
        let comment = self.end_of_statement()?;
        Ok(Item { pos, kind, comment })
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

    /// The word at the current token, if it is one.
    fn word(&self) -> Option<String> {
        match self.peek() {
            Token::Word(word) => Some(word.clone()),
            _ => None,
        }
    }

    fn signature(&mut self) -> Parsed<ItemKind> {
        let name = self.ident()?;
        self.expect(&Token::Colon, "`:`")?;
        let role_pos = self.pos();
        let Some(word) = self.word() else {
            return Err(self.unexpected(
                "a role: Formula, Value, Label, Local, Lambda, Style, Sheet, Workbook or Size",
            ));
        };
        self.advance();
        if word == "Size" {
            let kind = match self.word().as_deref() {
                Some("Fixed") => SizeKind::Fixed,
                Some("Unknown") => SizeKind::Unknown,
                _ => return Err(self.unexpected("`Fixed` or `Unknown` after `Size`")),
            };
            self.advance();
            return Ok(ItemKind::Size { name, kind });
        }
        let Some(role) = Role::from_word(&word) else {
            let hint = match word.as_str() {
                "Input" => {
                    "; a cell written in the file is a `Value`, such as `rate : Value Number \
                     percent rate_label right 2`"
                }
                _ => "",
            };
            return Err(Diagnostic::error(
                role_pos,
                format!(
                    "`{word}` is not a role; a signature starts with Formula, Value, Label, \
                     Local, Lambda, Style, Sheet, Workbook or Size{hint}"
                ),
            ));
        };
        let mut sig = Signature::new(name, role, role_pos);
        sig.ty = self.signature_type(role)?;
        if role.is_placed() {
            sig.style = Some(match self.peek().clone() {
                Token::Underscore => StyleName::Default(self.advance().pos),
                Token::Name(name) if !is_keyword(&name) => StyleName::Named(self.ident()?),
                _ => return Err(self.unexpected("a style's name, or `_` for the default")),
            });
        }
        if matches!(
            role,
            Role::Formula | Role::Value | Role::Label | Role::Local
        ) {
            if *self.peek() == Token::LBracket {
                sig.shape = Some(self.shape()?);
            }
            if self.at_name("start") {
                let pos = self.advance().pos;
                sig.start = Some(self.offset(pos)?);
            }
        }
        if role.is_placed() {
            sig.place = self.placement()?;
        }
        Ok(ItemKind::Signature(Box::new(sig)))
    }

    /// The type after a role: optional for what holds cells, and required for a lambda and a
    /// style.
    fn signature_type(&mut self, role: Role) -> Parsed<Option<(Type, Pos)>> {
        let pos = self.pos();
        let word = self.word();
        match role {
            Role::Formula | Role::Value | Role::Local => match word {
                Some(word) => match CellType::from_word(&word) {
                    Some(ty) => {
                        self.advance();
                        Ok(Some((Type::Cell(ty), pos)))
                    }
                    None => Err(Diagnostic::error(
                        pos,
                        format!("`{word}` is not a type; a cell holds Number, Text or Logical"),
                    )),
                },
                None => Ok(None),
            },
            Role::Label => match word {
                Some(_) => Err(Diagnostic::error(
                    pos,
                    "a label always holds Text, so its signature names no type",
                )),
                None => Ok(None),
            },
            Role::Lambda => {
                let mut types = vec![self.cell_type()?];
                while *self.peek() == Token::Arrow {
                    self.advance();
                    types.push(self.cell_type()?);
                }
                if types.len() < 2 {
                    return Err(Diagnostic::error(
                        pos,
                        "a lambda's type names its parameters and its result, such as `Number -> \
                         Number`",
                    ));
                }
                Ok(Some((Type::Function(types), pos)))
            }
            Role::Style => match word.as_deref().and_then(StyleType::from_word) {
                Some(ty) => {
                    self.advance();
                    Ok(Some((Type::Style(ty), pos)))
                }
                None => Err(self
                    .unexpected("the kind of style: Cell, Font, Border, Edge, Alignment or Color")),
            },
            Role::Sheet | Role::Workbook => Ok(None),
        }
    }

    fn cell_type(&mut self) -> Parsed<CellType> {
        match self.word().as_deref().and_then(CellType::from_word) {
            Some(ty) => {
                self.advance();
                Ok(ty)
            }
            None => Err(self.unexpected("a type: Number, Text or Logical")),
        }
    }

    /// Where a binding goes: `at sheet B2`, or a binding and up to two moves from it.
    fn placement(&mut self) -> Parsed<Option<Placement>> {
        if self.at_name("at") {
            let pos = self.advance().pos;
            let sheet = self.ident()?;
            let cell_pos = self.pos();
            let Some(word) = self.word() else {
                return Err(self.unexpected("a cell, such as `B2`"));
            };
            let Some((row, column)) = parse_cell(&word) else {
                return Err(Diagnostic::error(
                    cell_pos,
                    format!(
                        "`{word}` is not a cell; a cell is one to three letters and a row, from A1 \
                         to XFD{MAX_ROWS}, of {MAX_COLUMNS} columns"
                    ),
                ));
            };
            self.advance();
            return Ok(Some(Placement::At {
                sheet,
                row,
                column,
                pos,
            }));
        }
        let Token::Name(name) = self.peek().clone() else {
            return Ok(None);
        };
        if is_keyword(&name) {
            return Ok(None);
        }
        let reference = self.ident()?;
        let mut moves: Vec<Move> = Vec::new();
        loop {
            let pos = self.pos();
            let direction = match self.peek() {
                Token::Name(name) => Direction::from_word(name),
                _ => None,
            };
            let Some(direction) = direction else {
                break;
            };
            self.advance();
            let distance = match self.peek() {
                Token::Number(..) => Some(self.integer("a whole number of cells")?),
                _ => None,
            };
            let from = if self.at_name("from") {
                self.advance();
                let side = match self.peek() {
                    Token::Name(name) => Side::from_word(name),
                    _ => None,
                };
                let Some(side) = side else {
                    return Err(self.unexpected("a side: east, west, north or south"));
                };
                self.advance();
                Some(side)
            } else {
                None
            };
            if moves
                .iter()
                .any(|other| other.direction.is_horizontal() == direction.is_horizontal())
            {
                return Err(Diagnostic::error(
                    pos,
                    "a placement moves at most once along each axis: once left or right, and \
                     once up or down",
                ));
            }
            moves.push(Move {
                direction,
                distance,
                from,
                pos,
            });
        }
        if moves.is_empty() {
            return Err(self.unexpected(&format!(
                "a move from `{}`, such as `{} down` or `{} right 2`",
                reference.name, reference.name, reference.name
            )));
        }
        Ok(Some(Placement::Beside { reference, moves }))
    }

    fn shape(&mut self) -> Parsed<ShapeSyntax> {
        let pos = self.expect(&Token::LBracket, "`[`")?;
        let rows = self.size()?;
        self.expect(&Token::Comma, "`,` between rows and columns")?;
        let columns = self.size()?;
        self.expect(&Token::RBracket, "`]`")?;
        Ok(ShapeSyntax { rows, columns, pos })
    }

    fn offset(&mut self, pos: Pos) -> Parsed<Offset> {
        self.expect(&Token::LBracket, "`[` after `start`")?;
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
                "comparisons do not chain; combine them with `&&`",
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
        {
            let axis_next = takes_axis(name)
                && matches!(self.peek_second(), Token::Name(n) if Axis::from_word(n).is_some());
            if axis_next || starts_atom(self.peek_second()) {
                let func = self.ident()?;
                let axis = match self.peek() {
                    Token::Name(word) if takes_axis(&func.name) => Axis::from_word(word),
                    _ => None,
                };
                if axis.is_some() {
                    self.advance();
                }
                let mut args = Vec::new();
                while starts_atom(self.peek()) {
                    args.push(self.atom()?);
                }
                let seed = if func.name == "lag" && self.at_name("seed") {
                    self.advance();
                    Some(Box::new(self.atom()?))
                } else {
                    None
                };
                if self.at_name("from") && func.name == "lag" {
                    return Err(Diagnostic::error(
                        self.pos(),
                        "a lag's first value follows `seed`, as in `lag 1 closing seed opening`",
                    ));
                }
                return Ok(Expr {
                    pos: func.pos,
                    kind: ExprKind::Apply {
                        func,
                        axis,
                        args,
                        seed,
                    },
                });
            }
        }
        let atom = self.atom()?;
        if self.at_name("seed") {
            return Err(Diagnostic::error(
                self.pos(),
                "`seed` gives the first value of a lag, as in `lag 1 closing seed opening`",
            ));
        }
        Ok(atom)
    }

    fn atom(&mut self) -> Parsed<Expr> {
        let pos = self.pos();
        let kind = match self.peek().clone() {
            Token::Number(value, text) => ExprKind::Number { value, text },
            Token::Str(value) => ExprKind::Str(value),
            Token::Text(value) => ExprKind::Text(value),
            Token::Word(word) if word == "True" => ExprKind::Bool(true),
            Token::Word(word) if word == "False" => ExprKind::Bool(false),
            Token::Word(word) => {
                if *self.peek_second() == Token::LParen {
                    return Err(Diagnostic::error(pos, app_function_hint(&word)));
                }
                if parse_cell(&word).is_some() {
                    return Err(Diagnostic::error(
                        pos,
                        format!(
                            "`{word}` reads as a cell; formulas in .yup name bindings, not cells"
                        ),
                    ));
                }
                ExprKind::Constructor(word)
            }
            Token::Name(name) if name == "true" || name == "false" => {
                let word = if name == "true" { "True" } else { "False" };
                return Err(Diagnostic::error(
                    pos,
                    format!("a logical is written `{word}`, in upper case"),
                ));
            }
            Token::Name(name) if !is_keyword(&name) => {
                self.advance();
                let base = Expr {
                    pos,
                    kind: ExprKind::Name(name),
                };
                if *self.peek() == Token::LBrace {
                    let fields = self.fields()?;
                    return Ok(Expr {
                        pos,
                        kind: ExprKind::Update {
                            base: Box::new(base),
                            fields,
                        },
                    });
                }
                return Ok(base);
            }
            Token::LParen => {
                self.advance();
                let inner = self.expr()?;
                if *self.peek() == Token::Comma {
                    return Err(Diagnostic::error(
                        self.pos(),
                        "a function takes its arguments without parentheses or commas, as in \
                         `discount rate years`",
                    ));
                }
                self.expect(&Token::RParen, "`)`")?;
                return Ok(inner);
            }
            Token::LBracket => return self.list(),
            Token::LBrace => {
                let fields = self.fields()?;
                return Ok(Expr {
                    pos,
                    kind: ExprKind::Record(fields),
                });
            }
            _ => return Err(self.unexpected("a value")),
        };
        self.advance();
        Ok(Expr { pos, kind })
    }

    /// The fields of a record, `{ name = value, … }`, from its opening brace.
    fn fields(&mut self) -> Parsed<Vec<Field>> {
        self.expect(&Token::LBrace, "`{`")?;
        let mut fields = Vec::new();
        if *self.peek() == Token::RBrace {
            self.advance();
            return Ok(fields);
        }
        loop {
            // A field may be named by a keyword, such as a border's `left`.
            let name = match self.peek().clone() {
                Token::Name(name) => {
                    let pos = self.advance().pos;
                    Ident { name, pos }
                }
                _ => return Err(self.unexpected("a field's name")),
            };
            self.expect(&Token::Equals, "`=` after the field's name")?;
            let value = self.expr()?;
            fields.push(Field { name, value });
            match self.peek() {
                Token::Comma => {
                    self.advance();
                }
                Token::RBrace => {
                    self.advance();
                    return Ok(fields);
                }
                _ => return Err(self.unexpected("`,` or `}`")),
            }
        }
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

/// What to write instead of one of the spreadsheet app's functions, such as `SUM(…)`.
fn app_function_hint(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    if word == "IF" {
        return "`IF(…)` is the spreadsheet app's; in .yup it is `if a then b else c`".to_string();
    }
    match Builtin::lookup(&lower) {
        Some(builtin) => format!(
            "`{word}(…)` is the spreadsheet app's; in .yup functions are lower case and take \
             their arguments without parentheses: `{}`",
            builtin.usage()
        ),
        None => format!(
            "`{word}(…)` is the spreadsheet app's; in .yup functions are lower case and take \
             their arguments without parentheses, as in `round 2 x`"
        ),
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
        let program = parse(&format!("yup 0.2\nx = {source}\n")).expect("parses");
        match program.items.into_iter().next().map(|item| item.kind) {
            Some(ItemKind::Definition { expr, .. }) => expr,
            other => panic!("expected a definition, found {other:?}"),
        }
    }

    fn signature(source: &str) -> Signature {
        let program = parse(&format!("yup 0.2\n{source}\n")).expect("parses");
        match program.items.into_iter().next().map(|item| item.kind) {
            Some(ItemKind::Signature(sig)) => *sig,
            other => panic!("expected a signature, found {other:?}"),
        }
    }

    #[test]
    fn the_first_line_gives_the_version() {
        assert_eq!(
            errors("x = 1\n"),
            ["1:1: error: a .yup file starts with the line `yup 0.2`"]
        );
        assert_eq!(
            errors("yup 0.1\n"),
            ["1:5: error: this reader reads .yup 0.2, not 0.1"]
        );
    }

    #[test]
    fn a_signature_takes_its_parts_in_order() {
        let sig = signature(
            "g : Formula Number percent [1, n - 1] start [0, 1] revenue down 1 right 1 from west",
        );
        assert_eq!(sig.role, Role::Formula);
        assert!(matches!(sig.ty, Some((Type::Cell(CellType::Number), _))));
        assert!(matches!(&sig.style, Some(StyleName::Named(style)) if style.name == "percent"));
        let shape = sig.shape.as_ref().expect("a shape");
        assert_eq!(shape.columns.constant, -1);
        assert_eq!(sig.start.as_ref().map(|s| (s.row, s.column)), Some((0, 1)));
        let Some(Placement::Beside { reference, moves }) = &sig.place else {
            panic!("expected a placement beside another binding");
        };
        assert_eq!(reference.name, "revenue");
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[1].from, Some(Side::West));
        let at = signature("x : Label _ at forecast B2");
        assert!(matches!(
            at.place,
            Some(Placement::At {
                row: 2,
                column: 2,
                ..
            })
        ));
        let lambda = signature("f : Lambda Number -> Number -> Number");
        assert!(matches!(lambda.ty, Some((Type::Function(ref types), _)) if types.len() == 3));
    }

    #[test]
    fn a_signature_explains_what_it_lacks() {
        assert_eq!(
            errors(
                "yup 0.2\nx : Input\ny : Formula Number money revenue\nz : Formula _ x left 1 left 2\n"
            ),
            [
                "2:5: error: `Input` is not a role; a signature starts with Formula, Value, Label, \
                 Local, Lambda, Style, Sheet, Workbook or Size; a cell written in the file is a \
                 `Value`, such as `rate : Value Number percent rate_label right 2`",
                "3:33: error: expected a move from `revenue`, such as `revenue down` or `revenue \
                 right 2`, found the end of the line",
                "4:24: error: a placement moves at most once along each axis: once left or right, \
                 and once up or down",
            ]
        );
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
    fn a_lag_takes_an_axis_and_a_seed() {
        let ExprKind::Apply {
            func, axis, seed, ..
        } = definition("lag down 1 closing seed opening").kind
        else {
            panic!("expected an application");
        };
        assert_eq!(func.name, "lag");
        assert_eq!(axis, Some(Axis::Down));
        assert!(matches!(seed.map(|s| s.kind), Some(ExprKind::Name(n)) if n == "opening"));
        assert!(matches!(
            definition("tail across x").kind,
            ExprKind::Apply {
                axis: Some(Axis::Across),
                ..
            }
        ));
    }

    #[test]
    fn records_and_updates_parse() {
        let ExprKind::Update { base, fields } =
            definition("money\n  { font = { bold = True }\n  , border = edges\n  }").kind
        else {
            panic!("expected an update");
        };
        assert!(matches!(base.kind, ExprKind::Name(ref n) if n == "money"));
        assert_eq!(fields.len(), 2);
        assert!(matches!(fields[0].value.kind, ExprKind::Record(_)));
        let ExprKind::Record(fields) =
            definition("{ left = { weight = Thin }, format = '0%' }").kind
        else {
            panic!("expected a record");
        };
        assert!(matches!(fields[1].value.kind, ExprKind::Text(ref t) if t == "0%"));
    }

    #[test]
    fn every_wrong_line_is_reported() {
        assert_eq!(
            errors("yup 0.2\nx = \ny = 1 +\nz = (1 2)\n"),
            [
                "2:5: error: expected a value, found the end of the line",
                "3:8: error: expected a value, found the end of the line",
                "4:8: error: expected `)`, found the number `2`",
            ]
        );
    }

    #[test]
    fn the_spreadsheet_apps_habits_get_a_hint() {
        assert_eq!(
            errors("yup 0.2\na = discount(rate, years)\nb = SUM(x)\nc = B2 + 1\nd = true\n"),
            [
                "2:18: error: a function takes its arguments without parentheses or commas, as \
                 in `discount rate years`",
                "3:5: error: `SUM(…)` is the spreadsheet app's; in .yup functions are lower case \
                 and take their arguments without parentheses: `sum x`",
                "4:5: error: `B2` reads as a cell; formulas in .yup name bindings, not cells",
                "5:5: error: a logical is written `True`, in upper case",
            ]
        );
    }

    #[test]
    fn comparisons_do_not_chain() {
        assert_eq!(
            errors("yup 0.2\nx = a < b < c\n"),
            ["2:11: error: comparisons do not chain; combine them with `&&`"]
        );
    }

    #[test]
    fn a_size_subtracts_only_numbers() {
        assert_eq!(
            errors("yup 0.2\nx : Local [1, n - m]\n"),
            ["2:19: error: a size can subtract a number but not a size variable"]
        );
    }
}
