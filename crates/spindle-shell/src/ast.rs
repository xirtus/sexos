// SPDX-License-Identifier: GPL-3.0-only
//! The parsed form of a command line.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Span;
use crate::signature::Signature;
use crate::value::{PathMember, Value};

/// Statements separated by `;` or newlines.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Statement>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    /// `let name = pipeline`
    Let { name: String, name_span: Span, pipeline: Pipeline },
    Pipeline(Pipeline),
}

/// `a | b | c`, with an optional `> file` and `&`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pipeline {
    pub elements: Vec<Element>,
    pub redirect: Option<Redirect>,
    pub background: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Redirect {
    pub path: Expr,
    pub append: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Call(Call),
    /// A value standing as a pipeline stage: `$x | ...`, `[1 2 3] | ...`.
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The command's full name, `math sum` for a two-word one.
    pub name: String,
    pub head_span: Span,
    /// The built-in this names; `None` for a program.
    pub builtin: Option<&'static Signature>,
    /// Written `^name`: always a program.
    pub forced_external: bool,
    pub args: Vec<Arg>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Positional(Expr),
    /// A flag by its long name.
    Flag { name: String, span: Span, value: Option<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Literal(Value),
    /// A bare word: a string, or a column of the row in a row condition.
    BareWord(String),
    Var { name: String, path: Vec<PathMember> },
    List(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Binary { op: Op, op_span: Span, lhs: Box<Expr>, rhs: Box<Expr> },
    Not(Box<Expr>),
    Neg(Box<Expr>),
    /// `( pipeline )`
    Subexpr(Box<Block>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq, Ne, Lt, Le, Gt, Ge,
    /// `=~`: contains (a leading `^` anchors at the start, a trailing `$` at the end).
    Match,
    NotMatch,
    In,
    NotIn,
    StartsWith,
    EndsWith,
    And,
    Or,
    Xor,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
}

impl Op {
    pub fn from_token(text: &str) -> Option<Op> {
        Some(match text {
            "==" => Op::Eq,
            "!=" => Op::Ne,
            "<" => Op::Lt,
            "<=" => Op::Le,
            ">" => Op::Gt,
            ">=" => Op::Ge,
            "=~" => Op::Match,
            "!~" => Op::NotMatch,
            "in" => Op::In,
            "not-in" => Op::NotIn,
            "starts-with" => Op::StartsWith,
            "ends-with" => Op::EndsWith,
            "and" => Op::And,
            "or" => Op::Or,
            "xor" => Op::Xor,
            "+" => Op::Add,
            "-" => Op::Sub,
            "*" => Op::Mul,
            "/" => Op::Div,
            "mod" => Op::Mod,
            "**" => Op::Pow,
            "++" => Op::Concat,
            _ => return None,
        })
    }
    pub fn precedence(self) -> u8 {
        match self {
            Op::Or => 1,
            Op::Xor => 2,
            Op::And => 3,
            Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Match | Op::NotMatch | Op::In | Op::NotIn
            | Op::StartsWith | Op::EndsWith => 5,
            Op::Add | Op::Sub | Op::Concat => 6,
            Op::Mul | Op::Div | Op::Mod => 7,
            Op::Pow => 8,
        }
    }
    pub fn is_comparison(self) -> bool { self.precedence() == 5 }
}
