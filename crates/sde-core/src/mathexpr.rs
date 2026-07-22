//! Math channels: simple arithmetic expressions over existing session
//! channels (e.g. `Ground Speed * 3.6`, `abs([Steering Angle])`),
//! evaluated sample-by-sample onto a new derived [`Channel`]. Lives in
//! `sde-core` (not `sde-app`) so it stays UI-free and independently
//! testable, per the workspace's modularity principles (see
//! PROJECT_PLAN.md).
//!
//! Grammar (standard precedence, `^` right-associative and binding
//! tighter than unary minus so `-x^2` parses as `-(x^2)`):
//! ```text
//! expr   := term (('+' | '-') term)*
//! term   := unary (('*' | '/') unary)*
//! unary  := '-' unary | power
//! power  := primary ('^' unary)?
//! primary := number | channel | call | '(' expr ')'
//! channel := IDENT | '[' any characters except ']' ']'
//! call   := IDENT '(' expr (',' expr)* ')'
//! ```
//! Bracket syntax (`[Ground Speed]`) is required for channel names
//! containing spaces or other characters that aren't valid identifiers;
//! bare identifiers (`RPM`) work for simple names. Supported functions:
//! `abs`, `sqrt` (1 argument), `min`, `max` (2 arguments).

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean), applying to this module:
// - too_long_first_doc_paragraph fires on this module's and MathError's
//   doc comments, which deliberately front-load full context in one
//   paragraph rather than splitting off a one-line summary.
#![allow(clippy::too_long_first_doc_paragraph)]

use std::fmt;

use crate::{Channel, Session};

/// Everything that can go wrong turning a math-channel expression string
/// into a [`Channel`]: a syntax error, a reference to a channel that
/// doesn't exist in the session, an expression with no channel reference
/// at all (nothing to derive a sample-time base from), or a function
/// call with the wrong arity or an unknown name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MathError {
    Parse(String),
    UnknownChannel(String),
    NoChannelReferenced,
    UnknownFunction(String),
    WrongArgCount {
        function: String,
        expected: usize,
        got: usize,
    },
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::UnknownChannel(name) => write!(f, "unknown channel: {name}"),
            Self::NoChannelReferenced => {
                write!(f, "expression must reference at least one channel")
            }
            Self::UnknownFunction(name) => write!(f, "unknown function: {name}"),
            Self::WrongArgCount {
                function,
                expected,
                got,
            } => write!(f, "{function}() expects {expected} argument(s), got {got}"),
        }
    }
}

impl std::error::Error for MathError {}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Number(f64),
    ChannelRef(String),
    Neg(Box<Self>),
    BinOp(Op, Box<Self>, Box<Self>),
    Call(String, Vec<Self>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Comma,
}

fn tokenize(input: &str) -> Result<Vec<Token>, MathError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '[' {
            let start = i + 1;
            let Some(end_offset) = chars[start..].iter().position(|&c| c == ']') else {
                return Err(MathError::Parse("unterminated '[' channel reference".into()));
            };
            let name: String = chars[start..start + end_offset].iter().collect();
            tokens.push(Token::Ident(name));
            i = start + end_offset + 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let n: f64 = text
                .parse()
                .map_err(|_| MathError::Parse(format!("invalid number: {text}")))?;
            tokens.push(Token::Number(n));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            let token = match c {
                '+' => Token::Plus,
                '-' => Token::Minus,
                '*' => Token::Star,
                '/' => Token::Slash,
                '^' => Token::Caret,
                '(' => Token::LParen,
                ')' => Token::RParen,
                ',' => Token::Comma,
                other => return Err(MathError::Parse(format!("unexpected character: {other}"))),
            };
            tokens.push(token);
            i += 1;
        }
    }

    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse_expr(&mut self) -> Result<Expr, MathError> {
        let mut lhs = self.parse_term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.advance();
                    let rhs = self.parse_term()?;
                    lhs = Expr::BinOp(Op::Add, Box::new(lhs), Box::new(rhs));
                }
                Some(Token::Minus) => {
                    self.advance();
                    let rhs = self.parse_term()?;
                    lhs = Expr::BinOp(Op::Sub, Box::new(lhs), Box::new(rhs));
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> Result<Expr, MathError> {
        let mut lhs = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    lhs = Expr::BinOp(Op::Mul, Box::new(lhs), Box::new(rhs));
                }
                Some(Token::Slash) => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    lhs = Expr::BinOp(Op::Div, Box::new(lhs), Box::new(rhs));
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, MathError> {
        if matches!(self.peek(), Some(Token::Minus)) {
            self.advance();
            Ok(Expr::Neg(Box::new(self.parse_unary()?)))
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> Result<Expr, MathError> {
        let base = self.parse_primary()?;
        if matches!(self.peek(), Some(Token::Caret)) {
            self.advance();
            // right-associative, and binds tighter than unary minus on
            // either side: 2^3^2 == 2^(3^2), -2^2 == -(2^2), 2^-2 == 2^(-2)
            let exp = self.parse_unary()?;
            Ok(Expr::BinOp(Op::Pow, Box::new(base), Box::new(exp)))
        } else {
            Ok(base)
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, MathError> {
        match self.advance() {
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::Ident(name)) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.advance(); // consume '('
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        args.push(self.parse_expr()?);
                        while matches!(self.peek(), Some(Token::Comma)) {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    if !matches!(self.advance(), Some(Token::RParen)) {
                        return Err(MathError::Parse(format!("expected ')' after {name}(...)")));
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::ChannelRef(name))
                }
            }
            Some(Token::LParen) => {
                let inner = self.parse_expr()?;
                if matches!(self.advance(), Some(Token::RParen)) {
                    Ok(inner)
                } else {
                    Err(MathError::Parse("expected ')'".into()))
                }
            }
            other => Err(MathError::Parse(format!("unexpected token: {other:?}"))),
        }
    }
}

fn parse(expr: &str) -> Result<Expr, MathError> {
    let tokens = tokenize(expr)?;
    if tokens.is_empty() {
        return Err(MathError::Parse("empty expression".into()));
    }
    let mut parser = Parser { tokens, pos: 0 };
    let ast = parser.parse_expr()?;
    if parser.pos != parser.tokens.len() {
        return Err(MathError::Parse("unexpected trailing tokens".into()));
    }
    Ok(ast)
}

/// Names of every channel `expr` references, in first-appearance order
/// and deduplicated. The first entry becomes the derived channel's
/// sample-time base (see [`evaluate_math_channel`]).
fn referenced_channels(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Number(_) => {}
        Expr::ChannelRef(name) => {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        Expr::Neg(inner) => referenced_channels(inner, out),
        Expr::BinOp(_, lhs, rhs) => {
            referenced_channels(lhs, out);
            referenced_channels(rhs, out);
        }
        Expr::Call(_, args) => {
            for a in args {
                referenced_channels(a, out);
            }
        }
    }
}

/// Same interpolation semantics as `sde-app::graph::value_at_raw`
/// (respecting `Channel::interpolate`), kept as its own small copy here
/// so `sde-core` doesn't need to depend on `sde-app` for it.
fn value_at(channel: &Channel, t: f64) -> Option<f64> {
    if channel.timecodes.is_empty() {
        return None;
    }
    if t <= channel.timecodes[0] {
        return Some(channel.values[0]);
    }
    let last = channel.timecodes.len() - 1;
    if t >= channel.timecodes[last] {
        return Some(channel.values[last]);
    }

    // `t` is strictly between the first and last timecode here, so
    // binary_search's Err(insertion point) is guaranteed to be in
    // 1..=last, making `i0 = idx - 1` safe.
    let idx = match channel
        .timecodes
        .binary_search_by(|probe| probe.partial_cmp(&t).unwrap())
    {
        Ok(i) => return Some(channel.values[i]),
        Err(i) => i,
    };
    let i0 = idx - 1;
    if channel.interpolate {
        let (t0, t1) = (channel.timecodes[i0], channel.timecodes[idx]);
        let (v0, v1) = (channel.values[i0], channel.values[idx]);
        Some(v0 + (v1 - v0) * (t - t0) / (t1 - t0))
    } else {
        Some(channel.values[i0])
    }
}

// Not a general-purpose/code `eval`: this only walks the small `Expr` AST
// built by `parse` above (arithmetic + a fixed set of math functions), so
// there's no arbitrary-code-execution surface here.
fn eval(expr: &Expr, session: &Session, t: f64) -> Result<f64, MathError> {
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::ChannelRef(name) => {
            let channel = session
                .channels
                .get(name)
                .ok_or_else(|| MathError::UnknownChannel(name.clone()))?;
            // A channel with no samples at all is a degenerate case that
            // should have already been caught by `evaluate_math_channel`
            // picking a non-empty base channel; NaN here just propagates
            // harmlessly through the rest of the expression.
            Ok(value_at(channel, t).unwrap_or(f64::NAN))
        }
        Expr::Neg(inner) => Ok(-eval(inner, session, t)?),
        Expr::BinOp(op, lhs, rhs) => {
            let l = eval(lhs, session, t)?;
            let r = eval(rhs, session, t)?;
            Ok(match op {
                Op::Add => l + r,
                Op::Sub => l - r,
                Op::Mul => l * r,
                Op::Div => l / r,
                Op::Pow => l.powf(r),
            })
        }
        Expr::Call(name, args) => {
            let values = args
                .iter()
                .map(|a| eval(a, session, t))
                .collect::<Result<Vec<f64>, MathError>>()?;
            match (name.as_str(), values.as_slice()) {
                ("abs", [v]) => Ok(v.abs()),
                ("sqrt", [v]) => Ok(v.sqrt()),
                ("min", [a, b]) => Ok(a.min(*b)),
                ("max", [a, b]) => Ok(a.max(*b)),
                ("abs" | "sqrt", _) => Err(MathError::WrongArgCount {
                    function: name.clone(),
                    expected: 1,
                    got: values.len(),
                }),
                ("min" | "max", _) => Err(MathError::WrongArgCount {
                    function: name.clone(),
                    expected: 2,
                    got: values.len(),
                }),
                _ => Err(MathError::UnknownFunction(name.clone())),
            }
        }
    }
}

/// Parse and evaluate `expr_str` against `session`'s channels, producing
/// a new derived [`Channel`] named `name`.
///
/// The first channel `expr_str` references (in left-to-right appearance
/// order) supplies the sample timecodes for the whole result; every
/// other referenced channel is resampled onto those timecodes (linearly
/// interpolated or held, per its own `interpolate` flag — same semantics
/// as looking up any channel at an arbitrary time). This means the
/// result's sample rate follows whichever channel was written first in
/// the expression, which matters for expressions mixing channels
/// recorded at different rates.
///
/// # Errors
///
/// Returns [`MathError::Parse`] for a syntax error, [`MathError::NoChannelReferenced`]
/// if `expr_str` contains no channel reference at all (there'd be no
/// sample-time base to build a channel from), [`MathError::UnknownChannel`]
/// if any referenced name isn't in `session.channels`, and
/// [`MathError::UnknownFunction`]/[`MathError::WrongArgCount`] for an
/// unsupported or misused function call.
pub fn evaluate_math_channel(
    session: &Session,
    name: &str,
    expr_str: &str,
) -> Result<Channel, MathError> {
    let expr = parse(expr_str)?;

    let mut refs = Vec::new();
    referenced_channels(&expr, &mut refs);
    let Some(base_name) = refs.first() else {
        return Err(MathError::NoChannelReferenced);
    };
    let base = session
        .channels
        .get(base_name)
        .ok_or_else(|| MathError::UnknownChannel(base_name.clone()))?;
    let timecodes = base.timecodes.clone();

    let values = timecodes
        .iter()
        .map(|&t| eval(&expr, session, t))
        .collect::<Result<Vec<f64>, MathError>>()?;

    Ok(Channel {
        name: name.to_string(),
        units: String::new(),
        dec_pts: 3,
        interpolate: true,
        timecodes,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn channel(name: &str, timecodes: Vec<f64>, values: Vec<f64>) -> Channel {
        Channel {
            name: name.to_string(),
            units: "u".into(),
            dec_pts: 2,
            interpolate: true,
            timecodes,
            values,
        }
    }

    fn session_with(channels: Vec<Channel>) -> Session {
        Session {
            channels: channels.into_iter().map(|c| (c.name.clone(), c)).collect(),
            laps: vec![],
            metadata: HashMap::new(),
            key_channel_map: crate::KeyChannelMap::default(),
            file_name: "test".into(),
        }
    }

    #[test]
    fn evaluates_arithmetic_over_a_bare_channel_name() {
        let session = session_with(vec![channel("RPM", vec![0.0, 10.0], vec![1000.0, 2000.0])]);
        let out = evaluate_math_channel(&session, "RPM x2", "RPM * 2").unwrap();
        assert_eq!(out.name, "RPM x2");
        assert_eq!(out.timecodes, vec![0.0, 10.0]);
        assert_eq!(out.values, vec![2000.0, 4000.0]);
    }

    #[test]
    fn bracket_syntax_handles_channel_names_with_spaces() {
        let session = session_with(vec![channel("Ground Speed", vec![0.0], vec![50.0])]);
        let out = evaluate_math_channel(&session, "kph", "[Ground Speed] * 3.6").unwrap();
        assert_eq!(out.values, vec![180.0]);
    }

    #[test]
    fn resamples_second_channel_onto_first_channels_base_timecodes() {
        // Base channel "A" samples at 0/10/20; "B" only has samples at
        // 0 and 20, so its value at t=10 must come from interpolation.
        let session = session_with(vec![
            channel("A", vec![0.0, 10.0, 20.0], vec![1.0, 1.0, 1.0]),
            channel("B", vec![0.0, 20.0], vec![0.0, 10.0]),
        ]);
        let out = evaluate_math_channel(&session, "sum", "A + B").unwrap();
        assert_eq!(out.timecodes, vec![0.0, 10.0, 20.0]);
        assert_eq!(out.values, vec![1.0, 6.0, 11.0]);
    }

    #[test]
    fn unary_minus_and_operator_precedence() {
        let session = session_with(vec![channel("X", vec![0.0], vec![2.0])]);
        // -X + 3 * 4 should be -2 + 12 = 10, not -(X + 3) * 4.
        let out = evaluate_math_channel(&session, "y", "-X + 3 * 4").unwrap();
        assert_eq!(out.values, vec![10.0]);
    }

    #[test]
    fn power_is_right_associative_and_binds_tighter_than_unary_minus() {
        let session = session_with(vec![channel("X", vec![0.0], vec![2.0])]);
        // 2^3^2 == 2^(3^2) == 2^9 == 512, independent of X.
        let out = evaluate_math_channel(&session, "y", "X - X + 2^3^2").unwrap();
        assert_eq!(out.values, vec![512.0]);
    }

    #[test]
    fn power_binds_tighter_than_unary_minus_on_both_sides() {
        let session = session_with(vec![channel("X", vec![0.0], vec![2.0])]);
        // -X^2 == -(X^2) == -4, not (-X)^2 == 4.
        let out = evaluate_math_channel(&session, "y", "-X^2").unwrap();
        assert_eq!(out.values, vec![-4.0]);
        // X^-2 == X^(-2) == 0.25.
        let out = evaluate_math_channel(&session, "y", "X^-2").unwrap();
        assert_eq!(out.values, vec![0.25]);
    }

    #[test]
    fn function_calls_abs_sqrt_min_max() {
        let session = session_with(vec![channel("X", vec![0.0], vec![-9.0])]);
        assert_eq!(
            evaluate_math_channel(&session, "y", "abs(X)").unwrap().values,
            vec![9.0]
        );
        assert_eq!(
            evaluate_math_channel(&session, "y", "sqrt(abs(X))")
                .unwrap()
                .values,
            vec![3.0]
        );
        assert_eq!(
            evaluate_math_channel(&session, "y", "min(X, 0)").unwrap().values,
            vec![-9.0]
        );
        assert_eq!(
            evaluate_math_channel(&session, "y", "max(X, 0)").unwrap().values,
            vec![0.0]
        );
    }

    #[test]
    fn wrong_arg_count_is_an_error() {
        let session = session_with(vec![channel("X", vec![0.0], vec![1.0])]);
        let err = evaluate_math_channel(&session, "y", "abs(X, X)").unwrap_err();
        assert_eq!(
            err,
            MathError::WrongArgCount {
                function: "abs".into(),
                expected: 1,
                got: 2
            }
        );
    }

    #[test]
    fn unknown_function_is_an_error() {
        let session = session_with(vec![channel("X", vec![0.0], vec![1.0])]);
        assert_eq!(
            evaluate_math_channel(&session, "y", "sin(X)").unwrap_err(),
            MathError::UnknownFunction("sin".into())
        );
    }

    #[test]
    fn unknown_channel_is_an_error() {
        let session = session_with(vec![channel("X", vec![0.0], vec![1.0])]);
        assert_eq!(
            evaluate_math_channel(&session, "y", "X + NOPE").unwrap_err(),
            MathError::UnknownChannel("NOPE".into())
        );
    }

    #[test]
    fn no_channel_referenced_is_an_error() {
        let session = session_with(vec![channel("X", vec![0.0], vec![1.0])]);
        assert_eq!(
            evaluate_math_channel(&session, "y", "1 + 2 * 3").unwrap_err(),
            MathError::NoChannelReferenced
        );
    }

    #[test]
    fn division_by_zero_yields_infinity_not_an_error() {
        let session = session_with(vec![channel("X", vec![0.0], vec![1.0])]);
        let out = evaluate_math_channel(&session, "y", "X / 0").unwrap();
        assert_eq!(out.values, vec![f64::INFINITY]);
    }

    #[test]
    fn malformed_expressions_are_parse_errors() {
        let session = session_with(vec![channel("X", vec![0.0], vec![1.0])]);
        assert!(matches!(
            evaluate_math_channel(&session, "y", "X +").unwrap_err(),
            MathError::Parse(_)
        ));
        assert!(matches!(
            evaluate_math_channel(&session, "y", "(X + 1").unwrap_err(),
            MathError::Parse(_)
        ));
        assert!(matches!(
            evaluate_math_channel(&session, "y", "X ) 1").unwrap_err(),
            MathError::Parse(_)
        ));
        assert!(matches!(
            evaluate_math_channel(&session, "y", "[unterminated").unwrap_err(),
            MathError::Parse(_)
        ));
        assert!(matches!(
            evaluate_math_channel(&session, "y", "").unwrap_err(),
            MathError::Parse(_)
        ));
    }

    #[test]
    fn parenthesized_expressions_override_precedence() {
        let session = session_with(vec![channel("X", vec![0.0], vec![2.0])]);
        let out = evaluate_math_channel(&session, "y", "(X + 3) * 4").unwrap();
        assert_eq!(out.values, vec![20.0]);
    }
}
