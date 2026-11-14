// SPDX-License-Identifier: GPL-3.0-only
//! Evaluating expressions: variables, cell paths, operators, row conditions.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::ast::*;
use crate::error::{ShellError, Span};
use crate::host::Host;
use crate::units;
use crate::value::{no_column, Record, Value};

/// What commands and expressions run with.
pub(crate) struct Ctx<'a> {
    pub vars: &'a BTreeMap<String, Value>,
    pub host: &'a mut dyn Host,
    pub cols: usize,
    pub color: bool,
}

/// Evaluates `e` with `$in` = `input`; in a row condition `row` is the row, its columns
/// reachable as bare words and as `$it`.
pub(crate) fn eval(ctx: &mut Ctx, e: &Expr, input: &Value, row: Option<&Value>) -> Result<Value, ShellError> {
    match &e.kind {
        ExprKind::Literal(v) => Ok(v.clone()),
        ExprKind::BareWord(w) => {
            if let Some(Value::Record(r)) = row {
                if let Some(v) = r.get(w) { return Ok(v.clone()); }
            }
            Ok(Value::String(w.clone()))
        }
        ExprKind::Var { name, path } => {
            let base = match name.as_str() {
                "in" => input.clone(),
                "it" => row.cloned().unwrap_or_else(|| input.clone()),
                "nothing" => Value::Nothing,
                _ => match ctx.vars.get(name) {
                    Some(v) => v.clone(),
                    None => {
                        let err = ShellError::at(format!("unknown variable `${}`", name), Span::new(e.span.start, e.span.start + 1 + name.len()), "not defined");
                        let names: Vec<&str> = ctx.vars.keys().map(|s| s.as_str()).chain(["in", "it"]).collect();
                        return Err(match crate::suggest::closest(name, names.into_iter()) {
                            Some(best) => err.help(format!("did you mean `${}`?", best)),
                            None => err.help(format!("define it with `let {} = ...`", name)),
                        });
                    }
                },
            };
            base.follow(path)
        }
        ExprKind::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for i in items { out.push(eval(ctx, i, input, row)?); }
            Ok(Value::List(out))
        }
        ExprKind::Record(fields) => {
            let mut r = Record::new();
            for (k, v) in fields { r.insert(k.clone(), eval(ctx, v, input, row)?); }
            Ok(Value::Record(r))
        }
        ExprKind::Not(inner) => match eval(ctx, inner, input, row)? {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            other => Err(ShellError::at(format!("`not` needs true or false, not a {}", other.type_name()), inner.span, "not a bool")),
        },
        ExprKind::Neg(inner) => match eval(ctx, inner, input, row)? {
            Value::Int(i) => Ok(Value::Int(-i)),
            Value::Float(f) => Ok(Value::Float(-f)),
            Value::Filesize(i) => Ok(Value::Filesize(-i)),
            Value::Duration(i) => Ok(Value::Duration(-i)),
            other => Err(ShellError::at(format!("can't negate a {}", other.type_name()), inner.span, "not a number")),
        },
        ExprKind::Subexpr(block) => run_block(ctx, block, input),
        ExprKind::Binary { op, op_span, lhs, rhs } => binary(ctx, *op, *op_span, lhs, rhs, input, row),
    }
}

fn binary(ctx: &mut Ctx, op: Op, op_span: Span, lhs: &Expr, rhs: &Expr, input: &Value, row: Option<&Value>) -> Result<Value, ShellError> {
    // A bare word on the left of a comparison in a row condition must be a column.
    if op.is_comparison() {
        if let (ExprKind::BareWord(name), Some(Value::Record(r))) = (&lhs.kind, row) {
            if !r.contains(name) { return Err(no_column(name, lhs.span, row.unwrap())); }
        }
    }
    let a = eval(ctx, lhs, input, row)?;
    if op == Op::And || op == Op::Or {
        let Value::Bool(x) = a else { return Err(bool_needed(op, op_span, &a, lhs.span)) };
        if (op == Op::And && !x) || (op == Op::Or && x) { return Ok(Value::Bool(x)); }
        let b = eval(ctx, rhs, input, row)?;
        return match b { Value::Bool(y) => Ok(Value::Bool(y)), other => Err(bool_needed(op, op_span, &other, rhs.span)) };
    }
    // On the right of a comparison, a bare word is text, never a column.
    let b = match (&rhs.kind, op.is_comparison()) {
        (ExprKind::BareWord(w), true) => Value::String(w.clone()),
        _ => eval(ctx, rhs, input, row)?,
    };
    apply(op, &a, &b).map_err(|msg| ShellError::at(msg, op_span, format!("{} {} {}", a.type_name(), op_text(op), b.type_name())))
}

fn bool_needed(op: Op, op_span: Span, v: &Value, span: Span) -> ShellError {
    let _ = op_span;
    ShellError::at(format!("`{}` needs true or false on both sides, not a {}", op_text(op), v.type_name()), span, "not a bool")
}

pub(crate) fn op_text(op: Op) -> &'static str {
    match op {
        Op::Eq => "==", Op::Ne => "!=", Op::Lt => "<", Op::Le => "<=", Op::Gt => ">", Op::Ge => ">=",
        Op::Match => "=~", Op::NotMatch => "!~", Op::In => "in", Op::NotIn => "not-in",
        Op::StartsWith => "starts-with", Op::EndsWith => "ends-with",
        Op::And => "and", Op::Or => "or", Op::Xor => "xor",
        Op::Add => "+", Op::Sub => "-", Op::Mul => "*", Op::Div => "/", Op::Mod => "mod", Op::Pow => "**", Op::Concat => "++",
    }
}

/// `=~`: a substring test; `^` anchors at the start and `$` at the end.
pub fn text_matches(hay: &str, pattern: &str) -> bool {
    let (start, p) = match pattern.strip_prefix('^') { Some(p) => (true, p), None => (false, pattern) };
    let (end, p) = match p.strip_suffix('$') { Some(p) => (true, p), None => (false, p) };
    match (start, end) {
        (true, true) => hay == p,
        (true, false) => hay.starts_with(p),
        (false, true) => hay.ends_with(p),
        (false, false) => hay.contains(p),
    }
}

pub(crate) fn apply(op: Op, a: &Value, b: &Value) -> Result<Value, String> {
    use Value::*;
    let mismatch = || format!("can't use `{}` on {} and {}", op_text(op), a.type_name(), b.type_name());
    let ord = || a.compare(b).ok_or_else(|| format!("can't compare {} with {}", a.type_name(), b.type_name()));
    Ok(match op {
        Op::Eq => Bool(a.loose_eq(b)),
        Op::Ne => Bool(!a.loose_eq(b)),
        Op::Lt => Bool(ord()? == Ordering::Less),
        Op::Le => Bool(ord()? != Ordering::Greater),
        Op::Gt => Bool(ord()? == Ordering::Greater),
        Op::Ge => Bool(ord()? != Ordering::Less),
        Op::Match | Op::NotMatch => match (a, b) {
            (String(h), String(p)) => Bool(text_matches(h, p) == (op == Op::Match)),
            _ => return Err(format!("`{}` needs text on both sides, not {} and {}", op_text(op), a.type_name(), b.type_name())),
        },
        Op::StartsWith | Op::EndsWith => match (a, b) {
            (String(h), String(p)) => Bool(if op == Op::StartsWith { h.starts_with(p.as_str()) } else { h.ends_with(p.as_str()) }),
            _ => return Err(mismatch()),
        },
        Op::In | Op::NotIn => {
            let found = match b {
                List(items) => items.iter().any(|x| x.loose_eq(a)),
                String(h) => match a { String(n) => h.contains(n.as_str()), _ => return Err(mismatch()) },
                Record(r) => match a { String(n) => r.contains(n), _ => return Err(mismatch()) },
                _ => return Err(mismatch()),
            };
            Bool(found == (op == Op::In))
        }
        Op::And | Op::Or | Op::Xor => match (a, b) {
            (Bool(x), Bool(y)) => Bool(match op { Op::And => *x && *y, Op::Or => *x || *y, _ => x ^ y }),
            _ => return Err(mismatch()),
        },
        Op::Concat => match (a, b) {
            (String(x), String(y)) => String(format!("{}{}", x, y)),
            (List(x), List(y)) => List(x.iter().chain(y.iter()).cloned().collect()),
            (List(x), y) => { let mut v = x.clone(); v.push(y.clone()); List(v) }
            _ => return Err(mismatch()),
        },
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Pow => arith(op, a, b).ok_or_else(mismatch)?,
    })
}

fn checked(op: Op, x: i64, y: i64) -> Option<i64> {
    match op {
        Op::Add => x.checked_add(y),
        Op::Sub => x.checked_sub(y),
        Op::Mul => x.checked_mul(y),
        Op::Mod => if y == 0 { None } else { Some(x.rem_euclid(y)) },
        Op::Pow => u32::try_from(y).ok().and_then(|e| x.checked_pow(e)),
        _ => None,
    }
}

fn float_op(op: Op, x: f64, y: f64) -> Value {
    Value::Float(match op {
        Op::Add => x + y,
        Op::Sub => x - y,
        Op::Mul => x * y,
        Op::Div => x / y,
        Op::Mod => x - y * units::floor_f64(x / y),
        _ => {
            let mut r = 1.0;
            for _ in 0..(y as i64).clamp(0, 1024) { r *= x; }
            r
        }
    })
}

fn arith(op: Op, a: &Value, b: &Value) -> Option<Value> {
    use Value::*;
    Some(match (a, b) {
        (Int(x), Int(y)) => {
            if op == Op::Div {
                if *y == 0 { return None; }
                if x % y == 0 { Int(x / y) } else { Float(*x as f64 / *y as f64) }
            } else { Int(checked(op, *x, *y)?) }
        }
        (Int(x), Float(y)) => float_op(op, *x as f64, *y),
        (Float(x), Int(y)) => float_op(op, *x, *y as f64),
        (Float(x), Float(y)) => float_op(op, *x, *y),
        (Filesize(x), Filesize(y)) => match op {
            Op::Add | Op::Sub | Op::Mod => Filesize(checked(op, *x, *y)?),
            Op::Div if *y != 0 => Float(*x as f64 / *y as f64),
            _ => return None,
        },
        (Duration(x), Duration(y)) => match op {
            Op::Add | Op::Sub | Op::Mod => Duration(checked(op, *x, *y)?),
            Op::Div if *y != 0 => Float(*x as f64 / *y as f64),
            _ => return None,
        },
        (Filesize(x), Int(y)) | (Int(y), Filesize(x)) if op == Op::Mul => Filesize(x.checked_mul(*y)?),
        (Duration(x), Int(y)) | (Int(y), Duration(x)) if op == Op::Mul => Duration(x.checked_mul(*y)?),
        (Filesize(x), Float(y)) | (Float(y), Filesize(x)) if op == Op::Mul => Filesize(units::round_f64(*x as f64 * y)),
        (Duration(x), Float(y)) | (Float(y), Duration(x)) if op == Op::Mul => Duration(units::round_f64(*x as f64 * y)),
        (Filesize(x), Int(y)) if op == Op::Div && *y != 0 => Filesize(x / y),
        (Duration(x), Int(y)) if op == Op::Div && *y != 0 => Duration(x / y),
        (Filesize(x), Float(y)) if op == Op::Div && *y != 0.0 => Filesize(units::round_f64(*x as f64 / y)),
        (Duration(x), Float(y)) if op == Op::Div && *y != 0.0 => Duration(units::round_f64(*x as f64 / y)),
        (Date(x), Duration(y)) if op == Op::Add => Date(x + y / 1_000_000_000),
        (Date(x), Duration(y)) if op == Op::Sub => Date(x - y / 1_000_000_000),
        (Duration(y), Date(x)) if op == Op::Add => Date(x + y / 1_000_000_000),
        (Date(x), Date(y)) if op == Op::Sub => Duration((x - y).checked_mul(1_000_000_000)?),
        (String(x), String(y)) if op == Op::Add => String(format!("{}{}", x, y)),
        (List(x), List(y)) if op == Op::Add => List(x.iter().chain(y.iter()).cloned().collect()),
        _ => return None,
    })
}

/// Runs a `( ... )` block of built-ins and returns its last value.
pub(crate) fn run_block(ctx: &mut Ctx, block: &Block, input: &Value) -> Result<Value, ShellError> {
    let mut last = Value::Nothing;
    for stmt in &block.stmts {
        let p = match stmt {
            Statement::Pipeline(p) => p,
            Statement::Let { name_span, .. } => return Err(ShellError::at("`let` can't be used inside ( )", *name_span, "move it before this command")),
        };
        if let Some(r) = &p.redirect { return Err(ShellError::at("can't redirect inside ( )", r.span, "")); }
        if p.background { return Err(ShellError::at("can't start a background job inside ( )", p.span, "")); }
        let mut value = input.clone();
        for el in &p.elements {
            value = match el {
                Element::Expr(e) => eval(ctx, e, &value, None)?,
                Element::Call(c) => match c.builtin {
                    Some(_) if !c.forced_external => crate::builtins::run(ctx, c, value)?,
                    _ => return Err(ShellError::at(format!("the program `{}` can't run inside ( )", c.name), c.head_span, "only built-in commands run here").help("run the program as its own pipeline and keep its output with `let`")),
                },
            };
        }
        last = value;
    }
    Ok(last)
}
