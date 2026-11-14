// SPDX-License-Identifier: GPL-3.0-only
//! Tab completion: commands, subcommands, flags, variables, columns and paths.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::Range;

use crate::ast::{Element, Statement};
use crate::eval::{self, Ctx};
use crate::host::{EntryKind, Host, ReadOnly};
use crate::lexer::{lex, TokKind, Token};
use crate::signature::{self, ArgKind, Signature, COMMANDS, NAMESPACES};
use crate::value::Value;
use crate::{builtins, parser, path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Command,
    External,
    Flag,
    Variable,
    Column,
    File,
    Dir,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// The bytes of the line the completion replaces.
    pub replace: Range<usize>,
    pub text: String,
    pub kind: CompletionKind,
    pub description: String,
}

pub(crate) fn complete(line: &str, cursor: usize, host: &dyn Host, vars: &BTreeMap<String, Value>) -> Vec<Completion> {
    let cursor = cursor.min(line.len());
    let prefix = &line[..cursor];
    let toks: Vec<Token> = lex(prefix).into_iter().filter(|t| t.kind != TokKind::Comment).collect();
    let partial_idx = match toks.last() {
        Some(t) if t.span.end == cursor && matches!(t.kind, TokKind::Word | TokKind::Var | TokKind::Error) => Some(toks.len() - 1),
        _ => None,
    };
    let (partial, replace) = match partial_idx {
        Some(i) => (line[toks[i].span.range()].to_string(), toks[i].span.range()),
        None => (String::new(), cursor..cursor),
    };
    let before = &toks[..partial_idx.unwrap_or(toks.len())];
    let elem_start = before.iter().rposition(|t| matches!(t.kind, TokKind::Pipe | TokKind::Semi | TokKind::Newline | TokKind::LParen)).map(|i| i + 1).unwrap_or(0);
    let elem = &before[elem_start..];
    let mut out = Vec::new();
    let push = |out: &mut Vec<Completion>, text: String, kind, description: &str| {
        out.push(Completion { replace: replace.clone(), text, kind, description: description.to_string() });
    };

    if partial.starts_with('$') {
        for name in vars.keys().map(|s| s.as_str()).chain(["in", "it"]) {
            let text = format!("${}", name);
            if text.starts_with(&partial) { push(&mut out, text, CompletionKind::Variable, "variable"); }
        }
        return finish(out);
    }

    // The command name itself.
    let let_head = elem.len() >= 3 && elem[0].text == "let" && elem[2].text == "=";
    let elem = if let_head { &elem[3..] } else { elem };
    if elem.is_empty() {
        for s in COMMANDS {
            if s.name.starts_with(partial.as_str()) { push(&mut out, s.name.to_string(), CompletionKind::Command, s.desc); }
        }
        for name in host.external_names() {
            if name.starts_with(partial.as_str()) && signature::lookup(&name).is_none() { push(&mut out, name, CompletionKind::External, "program"); }
        }
        if "let".starts_with(partial.as_str()) && !partial.is_empty() { push(&mut out, String::from("let"), CompletionKind::Command, "Keep a value in a variable: let x = ls"); }
        return finish(out);
    }
    if elem.len() == 1 && NAMESPACES.contains(&elem[0].text.as_str()) {
        let ns = elem[0].text.as_str();
        for s in signature::subcommands(ns) {
            let sub = &s.name[ns.len() + 1..];
            if sub.starts_with(partial.as_str()) {
                out.push(Completion { replace: replace.clone(), text: sub.to_string(), kind: CompletionKind::Command, description: s.desc.to_string() });
            }
        }
        return finish(out);
    }

    let sig: Option<&'static Signature> = match elem {
        [a, b, ..] if b.kind == TokKind::Word => signature::lookup(&format!("{} {}", a.text, b.text)).or_else(|| signature::lookup(&a.text)),
        [a, ..] => signature::lookup(&a.text),
        [] => None,
    };

    if partial.starts_with('-') {
        if let Some(sig) = sig {
            for f in sig.flags {
                let text = format!("--{}", f.long);
                if text.starts_with(&partial) || (partial.len() == 2 && f.short.map(|c| partial.ends_with(c)).unwrap_or(false)) {
                    push(&mut out, text, CompletionKind::Flag, f.desc);
                }
            }
        }
        return finish(out);
    }

    let kind = sig.map(|s| s.completes).unwrap_or(ArgKind::Path);
    match kind {
        ArgKind::Column => {
            let pipe_at = toks[..elem_start].last().filter(|t| t.kind == TokKind::Pipe).map(|t| t.span.start);
            if let Some(end) = pipe_at {
                let stmt_start = toks[..elem_start].iter().rposition(|t| matches!(t.kind, TokKind::Semi | TokKind::Newline | TokKind::LParen)).map(|i| toks[i].span.end).unwrap_or(0);
                for c in columns_before(&line[stmt_start..end], host, vars) {
                    if c.starts_with(partial.as_str()) { push(&mut out, c, CompletionKind::Column, "column"); }
                }
            }
        }
        ArgKind::Path => paths(&partial, host, false, &mut out, &replace),
        ArgKind::Dir => paths(&partial, host, true, &mut out, &replace),
        ArgKind::Command => for s in COMMANDS {
            if s.name.starts_with(partial.as_str()) { push(&mut out, s.name.to_string(), CompletionKind::Command, s.desc); }
        },
        ArgKind::None => {}
    }
    finish(out)
}

fn finish(mut out: Vec<Completion>) -> Vec<Completion> {
    out.sort_by(|a, b| a.text.cmp(&b.text));
    out.dedup_by(|a, b| a.text == b.text);
    out
}

/// The columns of what an earlier pipeline produces, when it only reads.
fn columns_before(src: &str, host: &dyn Host, vars: &BTreeMap<String, Value>) -> Vec<String> {
    let Ok(block) = parser::parse(src) else { return Vec::new() };
    let [stmt] = block.stmts.as_slice() else { return Vec::new() };
    let p = match stmt { Statement::Pipeline(p) => p, Statement::Let { pipeline, .. } => pipeline };
    let mut ro = ReadOnly(host);
    let mut ctx = Ctx { vars, host: &mut ro, cols: 0, color: false };
    let mut v = Value::Nothing;
    for el in &p.elements {
        v = match el {
            Element::Expr(e) => match eval::eval(&mut ctx, e, &v, None) { Ok(x) => x, Err(_) => return Vec::new() },
            Element::Call(c) => match c.builtin {
                Some(sig) if sig.pure && !c.forced_external => match builtins::run(&mut ctx, c, v) { Ok(x) => x, Err(_) => return Vec::new() },
                _ => return Vec::new(),
            },
        };
    }
    v.columns()
}

fn paths(partial: &str, host: &dyn Host, dirs_only: bool, out: &mut Vec<Completion>, replace: &Range<usize>) {
    let raw = partial.trim_start_matches(['`', '"', '\'']);
    let (folder, stem) = match raw.rfind('/') {
        Some(i) => (&raw[..=i], &raw[i + 1..]),
        None => ("", raw),
    };
    let list_at = if folder.is_empty() { "." } else { folder };
    let full = path::resolve(&host.cwd(), &host.home(), list_at);
    let Ok(entries) = host.list_dir(&full) else { return };
    for e in entries {
        if !e.name.starts_with(stem) || (e.name.starts_with('.') && !stem.starts_with('.')) { continue; }
        let is_dir = e.kind == EntryKind::Dir;
        if dirs_only && !is_dir { continue; }
        let mut text = format!("{}{}", folder, e.name);
        if is_dir { text.push('/'); }
        if text.contains(' ') { text = format!("`{}`", text); }
        out.push(Completion {
            replace: replace.clone(),
            text,
            kind: if is_dir { CompletionKind::Dir } else { CompletionKind::File },
            description: String::from(if is_dir { "folder" } else { "file" }),
        });
    }
}
