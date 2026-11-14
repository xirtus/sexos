// SPDX-License-Identifier: GPL-3.0-only
//! Command line to [`Block`].
//!
//! Precedence, loosest first: `or`, `xor`, `and`, `not`, comparisons (`== != < <= > >=
//! =~ !~ in not-in starts-with ends-with`), `+ - ++`, `* / mod`, `**`. Operators need
//! spaces around them except the comparison ones (`size>1mb` works).
//!
//! `>` is a comparison inside conditions (`where`), expressions after `update`/`insert`
//! and parentheses; elsewhere `>`/`>>` at the end of a pipeline redirect to a file.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::*;
use crate::error::{ShellError, Span};
use crate::lexer::{lex, TokKind, Token};
use crate::signature::{self, Shape, Signature, NAMESPACES};
use crate::units;
use crate::value::PathMember;

/// Parses a whole command line.
pub fn parse(src: &str) -> Result<Block, ShellError> {
    let toks: Vec<Token> = lex(src).into_iter().filter(|t| t.kind != TokKind::Comment).collect();
    if let Some(t) = toks.iter().find(|t| t.kind == TokKind::Error) {
        return Err(ShellError::at(t.text.clone(), t.span, ""));
    }
    let mut p = Parser { toks, pos: 0, end: src.len(), parens: 0 };
    p.block(false)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    end: usize,
    parens: usize,
}

fn describe(t: &Token) -> String {
    match t.kind {
        TokKind::Newline => String::from("end of line"),
        TokKind::Str => String::from("a string"),
        _ => format!("`{}`", t.text),
    }
}

fn is_flag_word(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() < 2 || b[0] != b'-' || units::parse_number(text).is_some() { return false; }
    if b[1] == b'-' { b.len() > 2 && b[2].is_ascii_alphabetic() } else { b[1].is_ascii_alphabetic() }
}

fn is_command_word(text: &str) -> bool {
    signature::lookup(text).is_some() || NAMESPACES.contains(&text) || text.starts_with('^')
}

impl Parser {
    fn peek(&self) -> Option<&Token> { self.toks.get(self.pos) }
    fn peek_kind(&self) -> Option<TokKind> { self.peek().map(|t| t.kind) }
    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() { self.pos += 1; }
        t
    }
    fn eof_span(&self) -> Span { Span::new(self.end, self.end) }
    fn here(&self) -> Span { self.peek().map(|t| t.span).unwrap_or(self.eof_span()) }
    fn prev_end(&self) -> usize { if self.pos == 0 { 0 } else { self.toks[self.pos - 1].span.end } }

    fn at_element_end(&self) -> bool {
        matches!(self.peek_kind(), None | Some(TokKind::Pipe | TokKind::Semi | TokKind::Newline | TokKind::Amp))
            || (self.parens > 0 && self.peek_kind() == Some(TokKind::RParen))
    }

    fn unexpected(&self, t: &Token) -> ShellError {
        match t.kind {
            TokKind::RParen => ShellError::at("unexpected `)`", t.span, "nothing to close here"),
            TokKind::RBracket => ShellError::at("unexpected `]`", t.span, "nothing to close here"),
            TokKind::RBrace => ShellError::at("unexpected `}`", t.span, "nothing to close here"),
            TokKind::Amp => ShellError::at("`&` goes at the end of a pipeline", t.span, "runs the pipeline in the background"),
            _ => ShellError::at(format!("unexpected {}", describe(t)), t.span, "not expected here"),
        }
    }

    fn block(&mut self, inner: bool) -> Result<Block, ShellError> {
        let mut stmts = Vec::new();
        loop {
            while matches!(self.peek_kind(), Some(TokKind::Newline | TokKind::Semi)) { self.pos += 1; }
            match self.peek_kind() {
                None => break,
                Some(TokKind::RParen) if inner => break,
                _ => {}
            }
            stmts.push(self.statement()?);
            match self.peek() {
                None => {}
                Some(t) if matches!(t.kind, TokKind::Newline | TokKind::Semi) => {}
                Some(t) if inner && t.kind == TokKind::RParen => {}
                Some(t) => return Err(self.unexpected(&t.clone())),
            }
        }
        Ok(Block { stmts })
    }

    fn statement(&mut self) -> Result<Statement, ShellError> {
        let is_let = matches!(self.peek(), Some(t) if t.kind == TokKind::Word && t.text == "let");
        if !is_let { return Ok(Statement::Pipeline(self.pipeline()?)); }
        let let_tok = self.next().unwrap();
        let name_tok = match self.peek() {
            Some(t) if t.kind == TokKind::Word => self.next().unwrap(),
            _ => return Err(ShellError::at("`let` needs a name", let_tok.span, "like `let x = ls`")),
        };
        let name = name_tok.text.strip_prefix('$').unwrap_or(&name_tok.text).to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') || name.starts_with('-') {
            return Err(ShellError::at(format!("`{}` can't be a variable name", name_tok.text), name_tok.span, "use letters, digits, `_` and `-`"));
        }
        match self.peek() {
            Some(t) if t.kind == TokKind::Op && t.text == "=" => { self.pos += 1; }
            _ => return Err(ShellError::at(format!("expected `=` after `let {}`", name), Span::new(name_tok.span.end, name_tok.span.end + 1), "like `let x = 5`")),
        }
        if self.at_element_end() {
            return Err(ShellError::at(format!("`let {} =` needs a value", name), self.here(), "expected a value or a pipeline"));
        }
        let pipeline = self.pipeline()?;
        if pipeline.background {
            return Err(ShellError::at("`&` can't be used with `let`", pipeline.span, "a background job has no value yet"));
        }
        Ok(Statement::Let { name, name_span: name_tok.span, pipeline })
    }

    fn pipeline(&mut self) -> Result<Pipeline, ShellError> {
        let start = self.here().start;
        let mut elements = Vec::new();
        let mut redirect: Option<Redirect> = None;
        let mut last_pipe: Option<Span> = None;
        loop {
            if self.at_element_end() {
                return Err(match (last_pipe, self.peek()) {
                    (Some(pipe), _) => ShellError::at("expected a command after `|`", pipe, "nothing after this pipe"),
                    (None, Some(t)) if t.kind == TokKind::Pipe => ShellError::at("expected a command before `|`", t.span, "nothing goes into this pipe"),
                    (None, Some(t)) => self.unexpected(&t.clone()),
                    (None, None) => ShellError::at("expected a command", self.eof_span(), ""),
                });
            }
            elements.push(self.element(&mut redirect)?);
            if self.peek_kind() == Some(TokKind::Pipe) {
                let pipe = self.next().unwrap();
                if let Some(r) = &redirect {
                    return Err(ShellError::at("a redirect must end its pipeline", r.span, "nothing can follow this").help("to keep going, use `| save file | ...` or put the redirect last"));
                }
                last_pipe = Some(pipe.span);
                continue;
            }
            break;
        }
        let mut background = false;
        if self.peek_kind() == Some(TokKind::Amp) {
            self.pos += 1;
            background = true;
        }
        Ok(Pipeline { elements, redirect, background, span: Span::new(start, self.prev_end()) })
    }

    fn element(&mut self, redirect: &mut Option<Redirect>) -> Result<Element, ShellError> {
        let t = self.peek().unwrap().clone();
        match t.kind {
            TokKind::Word => {
                if let Some(name) = t.text.strip_prefix('^') {
                    self.pos += 1;
                    if name.is_empty() { return Err(ShellError::at("`^` needs a program name after it", t.span, "like `^ls`")); }
                    return Ok(Element::Call(self.call_args(name.to_string(), t.span, None, true, redirect)?));
                }
                let literal = units::parse_literal(&t.text).is_some() || t.text == "not" || t.text == "-";
                if literal || (self.parens > 0 && !is_command_word(&t.text)) {
                    return self.expr_element(redirect);
                }
                self.call(redirect).map(Element::Call)
            }
            TokKind::Backtick => {
                self.pos += 1;
                let builtin = signature::lookup(&t.text);
                Ok(Element::Call(self.call_args(t.text.clone(), t.span, builtin, false, redirect)?))
            }
            TokKind::Str | TokKind::Var | TokKind::LBracket | TokKind::LBrace | TokKind::LParen => self.expr_element(redirect),
            _ => Err(self.unexpected(&t)),
        }
    }

    fn expr_element(&mut self, redirect: &mut Option<Redirect>) -> Result<Element, ShellError> {
        let e = self.expression(false)?;
        self.maybe_redirect(redirect)?;
        if !self.at_element_end() {
            let t = self.peek().unwrap().clone();
            return Err(ShellError::at(format!("unexpected {} after a value", describe(&t)), t.span, "a value stands alone in a pipeline stage")
                .help("put operators inside ( ), like `(1 + 2)`"));
        }
        Ok(Element::Expr(e))
    }

    fn maybe_redirect(&mut self, redirect: &mut Option<Redirect>) -> Result<bool, ShellError> {
        let Some(t) = self.peek() else { return Ok(false) };
        if !(t.kind == TokKind::Op && (t.text == ">" || t.text == ">>")) || self.parens > 0 { return Ok(false); }
        let t = self.next().unwrap();
        if self.at_element_end() {
            return Err(ShellError::at(format!("`{}` needs a file name", t.text), t.span, "write to which file?"));
        }
        let path = self.atom()?;
        let span = t.span.to(path.span);
        *redirect = Some(Redirect { path, append: t.text == ">>", span });
        if !self.at_element_end() {
            let n = self.peek().unwrap().clone();
            return Err(ShellError::at(format!("unexpected {} after the redirect", describe(&n)), n.span, "only `&` can follow a redirect"));
        }
        Ok(true)
    }

    fn call(&mut self, redirect: &mut Option<Redirect>) -> Result<Call, ShellError> {
        let head = self.next().unwrap();
        let mut name = head.text.clone();
        let mut head_span = head.span;
        let mut builtin: Option<&'static Signature> = None;
        if let Some(n) = self.peek() {
            if n.kind == TokKind::Word {
                let two = format!("{} {}", name, n.text);
                if let Some(sig) = signature::lookup(&two) {
                    head_span = head_span.to(n.span);
                    self.pos += 1;
                    name = two;
                    builtin = Some(sig);
                }
            }
        }
        if builtin.is_none() {
            builtin = signature::lookup(&name);
            if builtin.is_none() && NAMESPACES.contains(&name.as_str()) {
                let subs: Vec<&str> = signature::subcommands(&name).map(|s| s.name).collect();
                let err = match self.peek() {
                    Some(n) if n.kind == TokKind::Word => ShellError::at(format!("`{}` has no subcommand `{}`", name, n.text), n.span, "unknown subcommand"),
                    _ => ShellError::at(format!("`{}` needs a subcommand", name), head_span, "which one?"),
                };
                return Err(err.help(format!("try {}", subs.join(", "))));
            }
        }
        self.call_args(name, head_span, builtin, false, redirect)
    }

    fn call_args(&mut self, name: String, head_span: Span, builtin: Option<&'static Signature>, forced_external: bool, redirect: &mut Option<Redirect>) -> Result<Call, ShellError> {
        let mut args = Vec::new();
        let shape = builtin.map(|s| s.shape).unwrap_or(Shape::Atoms);
        let mut positionals = 0;
        loop {
            if self.at_element_end() { break; }
            if self.maybe_redirect(redirect)? { break; }
            let t = self.peek().unwrap().clone();
            if t.kind == TokKind::Word && is_flag_word(&t.text) {
                if let Some(sig) = builtin {
                    self.pos += 1;
                    self.flags(sig, &t, &mut args)?;
                    continue;
                }
            }
            let expr_now = match shape {
                Shape::Condition => true,
                Shape::ColumnThenExpr => positionals >= 1,
                Shape::Atoms => false,
            };
            if expr_now {
                let e = self.expression(true)?;
                args.push(Arg::Positional(e));
                if !self.at_element_end() {
                    let n = self.peek().unwrap().clone();
                    return Err(ShellError::at(format!("unexpected {} in the condition", describe(&n)), n.span, "expected an operator such as `and`, or the end"));
                }
                break;
            }
            let e = if builtin.is_none() { self.word_arg()? } else { self.atom_or_word()? };
            args.push(Arg::Positional(e));
            positionals += 1;
        }
        let end = self.prev_end().max(head_span.end);
        Ok(Call { name, head_span, builtin, forced_external, args, span: Span::new(head_span.start, end) })
    }

    /// One flag word (`--long`, `-r`, `-rf`), checked against the command's flags.
    fn flags(&mut self, sig: &'static Signature, t: &Token, args: &mut Vec<Arg>) -> Result<(), ShellError> {
        let list = || {
            if sig.flags.is_empty() { String::from("it takes no flags") } else {
                let names: Vec<String> = sig.flags.iter().map(|f| match f.short {
                    Some(c) => format!("--{} (-{})", f.long, c),
                    None => format!("--{}", f.long),
                }).collect();
                format!("its flags are {}", names.join(", "))
            }
        };
        let mut found = Vec::new();
        if let Some(long) = t.text.strip_prefix("--") {
            match sig.flag(long) {
                Some(fl) => found.push((fl, t.span)),
                None => return Err(ShellError::at(format!("`{}` has no flag `{}`", sig.name, t.text), t.span, "unknown flag").help(list())),
            }
        } else {
            for (k, c) in t.text[1..].char_indices() {
                let span = Span::new(t.span.start + 1 + k, t.span.start + 2 + k);
                match sig.short_flag(c) {
                    Some(fl) => found.push((fl, span)),
                    None => return Err(ShellError::at(format!("`{}` has no flag `-{}`", sig.name, c), span, "unknown flag").help(list())),
                }
            }
        }
        let n = found.len();
        for (i, (fl, span)) in found.into_iter().enumerate() {
            let value = if fl.value {
                if i + 1 != n { return Err(ShellError::at(format!("`--{}` takes a value, so it must come last", fl.long), span, "")); }
                if self.at_element_end() { return Err(ShellError::at(format!("`--{}` needs a value", fl.long), span, "")); }
                Some(self.atom_or_word()?)
            } else { None };
            args.push(Arg::Flag { name: fl.long.to_string(), span: t.span, value });
        }
        Ok(())
    }

    /// An argument of a built-in: a value, or a bare word glued to the words around it
    /// (`a=b`).
    fn atom_or_word(&mut self) -> Result<Expr, ShellError> {
        match self.peek_kind() {
            Some(TokKind::Word | TokKind::Op) => self.word_arg(),
            _ => self.atom(),
        }
    }

    /// A program's argument: tokens written without space between them are one word.
    fn word_arg(&mut self) -> Result<Expr, ShellError> {
        if !matches!(self.peek_kind(), Some(TokKind::Word | TokKind::Op)) { return self.atom(); }
        let first = self.peek().unwrap().clone();
        if first.kind == TokKind::Word && !self.glued_next(1) { return self.atom(); }
        let mut text = String::new();
        let mut span = first.span;
        while let Some(t) = self.peek() {
            if !matches!(t.kind, TokKind::Word | TokKind::Op) || (t.span.start != span.end && !text.is_empty()) { break; }
            if t.kind == TokKind::Op && (t.text == ">" || t.text == ">>") { break; }
            text.push_str(&t.text);
            span = span.to(t.span);
            self.pos += 1;
        }
        if text.is_empty() { return self.atom(); }
        Ok(Expr { kind: ExprKind::BareWord(text), span })
    }

    fn glued_next(&self, ahead: usize) -> bool {
        match (self.toks.get(self.pos + ahead - 1), self.toks.get(self.pos + ahead)) {
            (Some(a), Some(b)) => b.span.start == a.span.end && b.kind == TokKind::Op && b.text != ">" && b.text != ">>",
            _ => false,
        }
    }

    fn expression(&mut self, allow_gt: bool) -> Result<Expr, ShellError> { self.binary(0, allow_gt) }

    fn binary(&mut self, min_prec: u8, allow_gt: bool) -> Result<Expr, ShellError> {
        let mut lhs = self.unary(allow_gt)?;
        loop {
            let Some(t) = self.peek() else { break };
            let op = match t.kind {
                TokKind::Op | TokKind::Word => Op::from_token(&t.text),
                _ => None,
            };
            let Some(op) = op else { break };
            if op == Op::Gt && !allow_gt && self.parens == 0 { break; }
            let prec = op.precedence();
            if prec < min_prec || prec == 0 { break; }
            let op_tok = self.next().unwrap();
            if self.at_element_end() {
                return Err(ShellError::at(format!("`{}` needs a value on its right", op_tok.text), op_tok.span, "nothing after this"));
            }
            let rhs = self.binary(prec + 1, allow_gt)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary { op, op_span: op_tok.span, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span };
        }
        Ok(lhs)
    }

    fn unary(&mut self, allow_gt: bool) -> Result<Expr, ShellError> {
        if let Some(t) = self.peek() {
            if t.kind == TokKind::Word && (t.text == "not" || t.text == "-") {
                let t = self.next().unwrap();
                if self.at_element_end() {
                    return Err(ShellError::at(format!("`{}` needs a value after it", t.text), t.span, ""));
                }
                let inner = if t.text == "not" { self.binary(4, allow_gt)? } else { self.unary(allow_gt)? };
                let span = t.span.to(inner.span);
                let kind = if t.text == "not" { ExprKind::Not(Box::new(inner)) } else { ExprKind::Neg(Box::new(inner)) };
                return Ok(Expr { kind, span });
            }
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, ShellError> {
        let Some(t) = self.peek().cloned() else {
            return Err(ShellError::at("expected a value", self.eof_span(), "the line ends here"));
        };
        match t.kind {
            TokKind::Word => {
                self.pos += 1;
                Ok(word_expr(&t.text, t.span))
            }
            TokKind::Str | TokKind::Backtick => {
                self.pos += 1;
                Ok(Expr { kind: ExprKind::Literal(crate::value::Value::String(t.text.clone())), span: t.span })
            }
            TokKind::Var => {
                self.pos += 1;
                let (name, rest) = match t.text.split_once('.') {
                    Some((n, r)) => (n.to_string(), r),
                    None => (t.text.clone(), ""),
                };
                let path = PathMember::parse_path(rest, t.span.start + 1 + name.len() + 1);
                Ok(Expr { kind: ExprKind::Var { name, path }, span: t.span })
            }
            TokKind::LBracket => self.list(),
            TokKind::LBrace => self.record(),
            TokKind::LParen => self.subexpr(),
            _ => Err(match t.kind {
                TokKind::Pipe | TokKind::Semi | TokKind::Newline | TokKind::Amp => ShellError::at("expected a value", t.span, format!("found {}", describe(&t))),
                _ => self.unexpected(&t),
            }),
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek_kind(), Some(TokKind::Newline | TokKind::Comma)) { self.pos += 1; }
    }

    fn list(&mut self) -> Result<Expr, ShellError> {
        let open = self.next().unwrap();
        let saved = self.parens;
        self.parens = 0;
        let mut items = Vec::new();
        let close = loop {
            self.skip_separators();
            match self.peek_kind() {
                None => { self.parens = saved; return Err(ShellError::at("this `[` is not closed", open.span, "opened here").help("add `]` at the end of the list")); }
                Some(TokKind::RBracket) => break self.next().unwrap(),
                _ => items.push(self.atom_or_word()?),
            }
        };
        self.parens = saved;
        Ok(Expr { kind: ExprKind::List(items), span: open.span.to(close.span) })
    }

    fn record(&mut self) -> Result<Expr, ShellError> {
        let open = self.next().unwrap();
        let saved = self.parens;
        self.parens = 0;
        let mut fields = Vec::new();
        let result = loop {
            self.skip_separators();
            let Some(t) = self.peek().cloned() else {
                break Err(ShellError::at("this `{` is not closed", open.span, "opened here").help("add `}` at the end of the record"));
            };
            match t.kind {
                TokKind::RBrace => { self.pos += 1; break Ok(t.span); }
                TokKind::Word | TokKind::Str | TokKind::Backtick => {
                    self.pos += 1;
                    if t.kind == TokKind::Word && self.peek_kind() != Some(TokKind::Colon) {
                        if let Some((k, v)) = t.text.split_once(':') {
                            let vstart = t.span.start + k.len() + 1;
                            fields.push((k.to_string(), word_expr(v, Span::new(vstart, t.span.end))));
                            continue;
                        }
                    }
                    if self.peek_kind() != Some(TokKind::Colon) {
                        break Err(ShellError::at(format!("expected `:` after the field name `{}`", t.text), Span::new(t.span.end, t.span.end + 1), "like `{name: value}`"));
                    }
                    self.pos += 1;
                    match self.atom_or_word() {
                        Ok(v) => fields.push((t.text.clone(), v)),
                        Err(e) => break Err(e),
                    }
                }
                _ => break Err(ShellError::at(format!("expected a field name, found {}", describe(&t)), t.span, "")),
            }
        };
        self.parens = saved;
        let close = result?;
        Ok(Expr { kind: ExprKind::Record(fields), span: open.span.to(close) })
    }

    fn subexpr(&mut self) -> Result<Expr, ShellError> {
        let open = self.next().unwrap();
        self.parens += 1;
        let block = self.block(true);
        self.parens -= 1;
        let block = block?;
        let close = match self.peek_kind() {
            Some(TokKind::RParen) => self.next().unwrap(),
            _ => return Err(ShellError::at("this `(` is not closed", open.span, "opened here").help("add `)` at the end")),
        };
        let span = open.span.to(close.span);
        if block.stmts.is_empty() {
            return Ok(Expr { kind: ExprKind::Literal(crate::value::Value::Nothing), span });
        }
        if let [Statement::Pipeline(p)] = block.stmts.as_slice() {
            if let [Element::Expr(e)] = p.elements.as_slice() {
                if p.redirect.is_none() && !p.background {
                    return Ok(Expr { kind: e.kind.clone(), span });
                }
            }
        }
        Ok(Expr { kind: ExprKind::Subexpr(Box::new(block)), span })
    }
}

fn word_expr(text: &str, span: Span) -> Expr {
    match units::parse_literal(text) {
        Some(v) => Expr { kind: ExprKind::Literal(v), span },
        None => Expr { kind: ExprKind::BareWord(text.to_string()), span },
    }
}
