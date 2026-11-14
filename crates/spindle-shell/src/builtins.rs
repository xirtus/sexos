// SPDX-License-Identifier: GPL-3.0-only
//! The built-in commands.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::ast::{Arg, Call, Expr, ExprKind, Op};
use crate::error::{ShellError, Span};
use crate::eval::{self, Ctx};
use crate::formats;
use crate::host::EntryKind;
use crate::path;
use crate::render;
use crate::signature::{self, COMMANDS};
use crate::value::{no_column, PathMember, Record, Value};

/// Runs the built-in `call` names on `input`.
pub(crate) fn run(ctx: &mut Ctx, call: &Call, input: Value) -> Result<Value, ShellError> {
    let a = Args { call };
    match call.name.as_str() {
        "ls" => ls(ctx, &a, &input),
        "cd" => cd(ctx, &a, &input),
        "pwd" => Ok(Value::String(ctx.host.cwd())),
        "mkdir" => {
            for (p, span) in a.paths(ctx, &input, 1)? {
                let abs = abs(ctx, &p);
                ctx.host.make_dir(&abs).map_err(|e| host_err("can't make", &p, span, e))?;
            }
            Ok(Value::Nothing)
        }
        "rm" => rm(ctx, &a, &input),
        "mv" => mv(ctx, &a, &input),
        "cp" => cp(ctx, &a, &input),
        "touch" => {
            for (p, span) in a.paths(ctx, &input, 1)? {
                let abs = abs(ctx, &p);
                if ctx.host.stat(&abs).is_err() {
                    ctx.host.write_file(&abs, &[], false).map_err(|e| host_err("can't make", &p, span, e))?;
                }
            }
            Ok(Value::Nothing)
        }
        "save" => save(ctx, &a, input),
        "open" => open(ctx, &a, &input),
        "sys" => Ok(ctx.host.sys_info()),
        "ps" => Ok(Value::List(ctx.host.processes().into_iter().map(|p| {
            Value::Record(Record::new()
                .with("pid", Value::Int(p.pid as i64))
                .with("name", Value::String(p.name))
                .with("state", Value::String(p.state))
                .with("mem", p.memory.map(|m| Value::Filesize(m as i64)).unwrap_or(Value::Nothing)))
        }).collect())),
        "jobs" => Ok(Value::List(ctx.host.jobs().into_iter().map(|j| {
            let state = match j.state {
                crate::host::JobState::Running => String::from("running"),
                crate::host::JobState::Stopped => String::from("stopped"),
                crate::host::JobState::Done(code) => format!("done ({})", code),
            };
            Value::Record(Record::new().with("id", Value::Int(j.id as i64)).with("command", Value::String(j.command)).with("state", Value::String(state)))
        }).collect())),
        "kill" => {
            let (v, span) = a.required(ctx, &input, 0, "a job number")?;
            let id = job_id(&v, span)?;
            ctx.host.kill_job(id).map_err(|e| ShellError::at(format!("can't stop job {}: {}", id, e), span, e))?;
            Ok(Value::Nothing)
        }
        "fg" => Err(ShellError::at("`fg` must be a command of its own", call.head_span, "not inside ( ) or a pipeline")),
        "date now" => Ok(Value::Date(ctx.host.now())),
        "where" => where_(ctx, &a, input),
        "select" | "reject" => select(ctx, &a, input, call.name == "reject"),
        "get" => {
            let (v, span) = a.required(ctx, &input, 0, "a cell path, like `name` or `0.name`")?;
            let path = match &v {
                Value::Int(i) if *i >= 0 => vec![PathMember::Index(*i as usize, span)],
                Value::String(s) => PathMember::parse_path(s, span.start),
                other => return Err(ShellError::at(format!("`get` needs a cell path, not a {}", other.type_name()), span, "")),
            };
            input.follow(&path)
        }
        "sort-by" => sort_by(ctx, &a, input),
        "first" | "last" => {
            let n = a.optional(ctx, &input, 0)?;
            let items = list_input(input, call)?;
            let len = items.len();
            match n {
                None => {
                    let item = if call.name == "first" { items.into_iter().next() } else { items.into_iter().last() };
                    item.ok_or_else(|| ShellError::at("the input is empty", call.head_span, "no rows"))
                }
                Some((v, span)) => {
                    let n = count(&v, span)?.min(len);
                    Ok(Value::List(if call.name == "first" { items.into_iter().take(n).collect() } else { items.into_iter().skip(len - n).collect() }))
                }
            }
        }
        "skip" | "take" => {
            let (v, span) = a.required(ctx, &input, 0, "how many rows")?;
            let n = count(&v, span)?;
            let items = list_input(input, call)?;
            Ok(Value::List(if call.name == "skip" { items.into_iter().skip(n).collect() } else { items.into_iter().take(n).collect() }))
        }
        "length" => match input {
            Value::List(items) => Ok(Value::Int(items.len() as i64)),
            Value::Nothing => Ok(Value::Int(0)),
            Value::String(_) => Err(ShellError::at("`length` counts rows, and the input is text", call.head_span, "").help("use `str length` for characters, or `lines | length` for lines")),
            Value::Record(r) => Ok(Value::Int(r.len() as i64)),
            other => Err(wrong_input(call, "a list or table", &other)),
        },
        "reverse" => { let mut items = list_input(input, call)?; items.reverse(); Ok(Value::List(items)) }
        "uniq" => {
            let items = list_input(input, call)?;
            let mut out: Vec<Value> = Vec::new();
            for it in items { if !out.iter().any(|o| *o == it) { out.push(it); } }
            Ok(Value::List(out))
        }
        "group-by" => {
            let (v, span) = a.required(ctx, &input, 0, "a column")?;
            let col = text_arg(&v, span)?;
            let whole = input.clone();
            let items = list_input(input, call)?;
            let mut groups = Record::new();
            for it in items {
                let key = match &it {
                    Value::Record(r) => match r.get(&col) { Some(k) => k.to_abbreviated(), None => return Err(no_column(&col, span, &whole)) },
                    other => other.to_abbreviated(),
                };
                match groups.get_mut(&key) {
                    Some(Value::List(l)) => l.push(it),
                    _ => groups.insert(key, Value::List(vec![it])),
                }
            }
            Ok(Value::Record(groups))
        }
        "update" | "insert" => update(ctx, &a, input, call.name == "insert"),
        "wrap" => {
            let (v, span) = a.required(ctx, &input, 0, "a column name")?;
            let col = text_arg(&v, span)?;
            Ok(match input {
                Value::List(items) => Value::List(items.into_iter().map(|x| Value::Record(Record::new().with(col.clone(), x))).collect()),
                other => Value::Record(Record::new().with(col, other)),
            })
        }
        "columns" => match &input {
            Value::List(_) | Value::Record(_) => Ok(Value::List(input.columns().into_iter().map(Value::String).collect())),
            other => Err(wrong_input(call, "a table or record", other)),
        },
        "describe" => Ok(Value::String(describe(&input))),
        "math sum" | "math avg" | "math min" | "math max" => math(call, input),
        "str contains" | "str replace" | "str upcase" | "str downcase" | "str trim" | "str length" => strings(ctx, &a, input),
        "split row" => {
            let (v, span) = a.required(ctx, &input, 0, "a separator")?;
            let sep = text_arg(&v, span)?;
            if sep.is_empty() { return Err(ShellError::at("the separator is empty", span, "")); }
            each_text(call, input, &|s| s.split(sep.as_str()).map(|p| Value::String(p.to_string())).collect())
        }
        "lines" => each_text(call, input, &|s| {
            let mut v: Vec<Value> = s.split('\n').map(|l| Value::String(l.strip_suffix('\r').unwrap_or(l).to_string())).collect();
            if s.ends_with('\n') { v.pop(); }
            v
        }),
        "to json" => Ok(Value::String(formats::to_json(&input, if a.has("raw") { 0 } else { 2 }))),
        "to csv" | "to tsv" => formats::to_csv(&input, if call.name == "to csv" { ',' } else { '\t' }).map(Value::String).map_err(|e| ShellError::at(e, call.head_span, "")),
        "to text" => Ok(Value::String(input.to_text())),
        "from json" | "from csv" | "from tsv" | "from toml" => {
            let text = text_input(call, &input)?;
            let parsed = match call.name.as_str() {
                "from json" => formats::from_json(&text),
                "from toml" => formats::from_toml(&text),
                "from tsv" => formats::from_csv(&text, '\t', !a.has("noheaders")),
                _ => {
                    let sep = match a.flag_value(ctx, &input, "separator")? {
                        Some((Value::String(s), span)) => { let mut c = s.chars(); match (c.next(), c.next()) { (Some(ch), None) => ch, _ => return Err(ShellError::at("the separator must be one character", span, "")) } }
                        Some((_, span)) => return Err(ShellError::at("the separator must be one character", span, "")),
                        None => ',',
                    };
                    formats::from_csv(&text, sep, !a.has("noheaders"))
                }
            };
            parsed.map_err(|e| ShellError::at(format!("can't parse the input: {}", e), call.head_span, "bad input"))
        }
        "table" => Ok(Value::String(render::render(&input, ctx.cols, ctx.color))),
        "echo" => {
            let mut vals: Vec<Value> = a.all_positional(ctx, &input)?.into_iter().map(|(v, _)| v).collect();
            Ok(match vals.len() { 0 => Value::Nothing, 1 => vals.pop().unwrap(), _ => Value::List(vals) })
        }
        "seq" => seq(ctx, &a, &input),
        "help" => help(ctx, &a, &input),
        other => Err(ShellError::at(format!("`{}` is not implemented", other), call.head_span, "")),
    }
}

struct Args<'c> {
    call: &'c Call,
}

impl<'c> Args<'c> {
    fn positional_exprs(&self) -> Vec<&'c Expr> {
        self.call.args.iter().filter_map(|a| if let Arg::Positional(e) = a { Some(e) } else { None }).collect()
    }
    fn has(&self, long: &str) -> bool {
        self.call.args.iter().any(|a| matches!(a, Arg::Flag { name, .. } if name == long))
    }
    fn flag_value(&self, ctx: &mut Ctx, input: &Value, long: &str) -> Result<Option<(Value, Span)>, ShellError> {
        for a in &self.call.args {
            if let Arg::Flag { name, value: Some(e), .. } = a {
                if name == long { return Ok(Some((eval::eval(ctx, e, input, None)?, e.span))); }
            }
        }
        Ok(None)
    }
    fn all_positional(&self, ctx: &mut Ctx, input: &Value) -> Result<Vec<(Value, Span)>, ShellError> {
        let mut out = Vec::new();
        for e in self.positional_exprs() { out.push((eval::eval(ctx, e, input, None)?, e.span)); }
        Ok(out)
    }
    fn optional(&self, ctx: &mut Ctx, input: &Value, i: usize) -> Result<Option<(Value, Span)>, ShellError> {
        match self.positional_exprs().get(i) {
            Some(e) => Ok(Some((eval::eval(ctx, e, input, None)?, e.span))),
            None => Ok(None),
        }
    }
    fn required(&self, ctx: &mut Ctx, input: &Value, i: usize, what: &str) -> Result<(Value, Span), ShellError> {
        self.optional(ctx, input, i)?.ok_or_else(|| self.missing(what))
    }
    fn missing(&self, what: &str) -> ShellError {
        let sig = self.call.builtin.map(|s| format!("{} {}", s.name, s.args)).unwrap_or_default();
        ShellError::at(format!("`{}` needs {}", self.call.name, what), Span::new(self.call.span.end, self.call.span.end + 1), format!("missing {}", what))
            .help(format!("usage: {}", sig.trim_end()))
    }
    /// Every positional as text (paths), at least `min` of them.
    fn paths(&self, ctx: &mut Ctx, input: &Value, min: usize) -> Result<Vec<(String, Span)>, ShellError> {
        let vals = self.all_positional(ctx, input)?;
        if vals.len() < min { return Err(self.missing(if min == 1 { "a path" } else { "two paths" })); }
        vals.into_iter().map(|(v, s)| text_arg(&v, s).map(|t| (t, s))).collect()
    }
}

fn abs(ctx: &Ctx, p: &str) -> String { path::resolve(&ctx.host.cwd(), &ctx.host.home(), p) }

fn host_err(what: &str, p: &str, span: Span, e: String) -> ShellError {
    ShellError::at(format!("{} `{}`: {}", what, p, e), span, e)
}

fn text_arg(v: &Value, span: Span) -> Result<String, ShellError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::List(_) | Value::Record(_) | Value::Binary(_) | Value::Nothing | Value::Error(_) => Err(ShellError::at(format!("expected text, got a {}", v.type_name()), span, "not text")),
        other => Ok(other.to_text()),
    }
}

fn count(v: &Value, span: Span) -> Result<usize, ShellError> {
    match v {
        Value::Int(i) if *i >= 0 => Ok(*i as usize),
        other => Err(ShellError::at(format!("expected a count, got {}", other.to_abbreviated()), span, "not a whole number of zero or more")),
    }
}

fn job_id(v: &Value, span: Span) -> Result<u32, ShellError> {
    match v {
        Value::Int(i) if *i >= 0 => Ok(*i as u32),
        Value::String(s) => s.trim_start_matches('%').parse().map_err(|_| ShellError::at("expected a job number", span, "see `jobs`")),
        _ => Err(ShellError::at("expected a job number", span, "see `jobs`")),
    }
}

fn wrong_input(call: &Call, want: &str, got: &Value) -> ShellError {
    ShellError::at(format!("`{}` needs {} as input, not a {}", call.name, want, got.type_name()), call.head_span, format!("got {}", got.type_name()))
}

fn list_input(input: Value, call: &Call) -> Result<Vec<Value>, ShellError> {
    match input {
        Value::List(items) => Ok(items),
        Value::Nothing => Ok(Vec::new()),
        Value::Binary(_) | Value::String(_) | Value::Record(_) => Err(wrong_input(call, "a list or table", &input)),
        other => Ok(vec![other]),
    }
}

fn text_input(call: &Call, input: &Value) -> Result<String, ShellError> {
    match input {
        Value::String(s) => Ok(s.clone()),
        Value::Binary(b) => String::from_utf8(b.clone()).map_err(|_| ShellError::at("the input is not UTF-8 text", call.head_span, "")),
        other => Err(wrong_input(call, "text", other)),
    }
}

fn each_text(call: &Call, input: Value, f: &dyn Fn(&str) -> Vec<Value>) -> Result<Value, ShellError> {
    match input {
        Value::String(s) => Ok(Value::List(f(&s))),
        Value::Binary(b) => Ok(Value::List(f(&String::from_utf8_lossy(&b)))),
        Value::List(items) => {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Value::String(s) => out.extend(f(&s)),
                    other => return Err(wrong_input(call, "text or a list of text", &other)),
                }
            }
            Ok(Value::List(out))
        }
        other => Err(wrong_input(call, "text", &other)),
    }
}

pub(crate) fn describe(v: &Value) -> String {
    match v {
        Value::List(items) if v.is_table() => {
            let cols = v.columns();
            format!("table<{}> ({} row{})", cols.join(", "), items.len(), crate::value::plural(items.len()))
        }
        Value::List(items) => format!("list ({} item{})", items.len(), crate::value::plural(items.len())),
        Value::Record(r) => format!("record<{}>", r.columns().join(", ")),
        other => other.type_name().to_string(),
    }
}

// ---------------------------------------------------------------- files

fn entry_record(name: String, e: &crate::host::DirEntry) -> Value {
    Value::Record(Record::new()
        .with("name", Value::String(name))
        .with("type", Value::String(e.kind.name().to_string()))
        .with("size", Value::Filesize(e.size as i64))
        .with("modified", e.modified.map(Value::Date).unwrap_or(Value::Nothing)))
}

fn ls(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let all = a.has("all");
    let mut targets = a.paths(ctx, input, 0)?;
    if targets.is_empty() { targets.push((String::from("."), a.call.head_span)); }
    let mut rows = Vec::new();
    for (target, span) in targets {
        let (folder, last) = path::split(&target);
        if path::is_pattern(last) {
            let folder_rel = if folder.is_empty() { "." } else { folder };
            let mut entries = ctx.host.list_dir(&abs(ctx, folder_rel)).map_err(|e| host_err("can't list", folder_rel, span, e))?;
            entries.sort_by(|x, y| x.name.cmp(&y.name));
            for e in entries {
                if !path::matches(last, &e.name) || (e.name.starts_with('.') && !all && !last.starts_with('.')) { continue; }
                let name = if folder.is_empty() { e.name.clone() } else { path::join(folder, &e.name) };
                rows.push(entry_record(name, &e));
            }
            continue;
        }
        let full = abs(ctx, &target);
        let st = ctx.host.stat(&full).map_err(|e| host_err("can't list", &target, span, e))?;
        if st.kind != EntryKind::Dir {
            rows.push(entry_record(target.clone(), &st));
            continue;
        }
        let mut entries = ctx.host.list_dir(&full).map_err(|e| host_err("can't list", &target, span, e))?;
        entries.sort_by(|x, y| x.name.cmp(&y.name));
        for e in entries {
            if e.name.starts_with('.') && !all { continue; }
            let name = if target == "." { e.name.clone() } else { path::join(&target, &e.name) };
            rows.push(entry_record(name, &e));
        }
    }
    Ok(Value::List(rows))
}

fn cd(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let (target, span) = match a.optional(ctx, input, 0)? {
        Some((v, span)) => (text_arg(&v, span)?, span),
        None => (ctx.host.home(), a.call.head_span),
    };
    let full = abs(ctx, &target);
    let st = ctx.host.stat(&full).map_err(|e| host_err("can't go to", &target, span, e))?;
    if st.kind != EntryKind::Dir && full != "/" {
        return Err(ShellError::at(format!("`{}` is not a folder", target), span, "a file"));
    }
    ctx.host.set_cwd(&full).map_err(|e| host_err("can't go to", &target, span, e))?;
    Ok(Value::Nothing)
}

fn rm(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let recursive = a.has("recursive");
    for (p, span) in a.paths(ctx, input, 1)? {
        let (folder, last) = path::split(&p);
        let targets: Vec<String> = if path::is_pattern(last) {
            let folder_rel = if folder.is_empty() { "." } else { folder };
            let folder_abs = abs(ctx, folder_rel);
            let entries = ctx.host.list_dir(&folder_abs).map_err(|e| host_err("can't list", folder_rel, span, e))?;
            let found: Vec<String> = entries.into_iter().filter(|e| path::matches(last, &e.name)).map(|e| path::join(&folder_abs, &e.name)).collect();
            if found.is_empty() { return Err(ShellError::at(format!("nothing matches `{}`", p), span, "no match")); }
            found
        } else { vec![abs(ctx, &p)] };
        for t in targets {
            let st = ctx.host.stat(&t).map_err(|e| host_err("can't remove", &p, span, e))?;
            if st.kind == EntryKind::Dir && !recursive {
                return Err(ShellError::at(format!("`{}` is a folder", p), span, "a folder").help("use `rm -r` to remove it and what it holds"));
            }
            ctx.host.remove(&t, recursive).map_err(|e| host_err("can't remove", &p, span, e))?;
        }
    }
    Ok(Value::Nothing)
}

/// The destination: inside `to` when `to` is a folder.
fn destination(ctx: &Ctx, from: &str, to: &str) -> String {
    let to_abs = abs(ctx, to);
    match ctx.host.stat(&to_abs) {
        Ok(st) if st.kind == EntryKind::Dir => path::join(&to_abs, path::split(&abs(ctx, from)).1),
        _ => to_abs,
    }
}

fn two_paths(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<((String, Span), (String, Span)), ShellError> {
    let mut p = a.paths(ctx, input, 2)?;
    if p.len() > 2 { return Err(ShellError::at(format!("`{}` takes two paths", a.call.name), p[2].1, "one too many")); }
    let to = p.pop().unwrap();
    Ok((p.pop().unwrap(), to))
}

fn mv(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let ((from, fspan), (to, _)) = two_paths(ctx, a, input)?;
    let dest = destination(ctx, &from, &to);
    let src = abs(ctx, &from);
    ctx.host.stat(&src).map_err(|e| host_err("can't move", &from, fspan, e))?;
    ctx.host.rename(&src, &dest).map_err(|e| host_err("can't move", &from, fspan, e))?;
    Ok(Value::Nothing)
}

fn cp(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let ((from, fspan), (to, _)) = two_paths(ctx, a, input)?;
    let dest = destination(ctx, &from, &to);
    let src = abs(ctx, &from);
    let st = ctx.host.stat(&src).map_err(|e| host_err("can't copy", &from, fspan, e))?;
    if st.kind == EntryKind::Dir {
        if !a.has("recursive") { return Err(ShellError::at(format!("`{}` is a folder", from), fspan, "a folder").help("use `cp -r` to copy it and what it holds")); }
        copy_tree(ctx, &src, &dest).map_err(|e| host_err("can't copy", &from, fspan, e))?;
    } else {
        let data = ctx.host.read_file(&src).map_err(|e| host_err("can't read", &from, fspan, e))?;
        ctx.host.write_file(&dest, &data, false).map_err(|e| host_err("can't write", &to, fspan, e))?;
    }
    Ok(Value::Nothing)
}

fn copy_tree(ctx: &mut Ctx, from: &str, to: &str) -> Result<(), String> {
    if to.starts_with(from) && to[from.len()..].starts_with('/') { return Err(String::from("can't copy a folder into itself")); }
    ctx.host.make_dir(to)?;
    for e in ctx.host.list_dir(from)? {
        let (s, d) = (path::join(from, &e.name), path::join(to, &e.name));
        if e.kind == EntryKind::Dir { copy_tree(ctx, &s, &d)?; }
        else { let data = ctx.host.read_file(&s)?; ctx.host.write_file(&d, &data, false)?; }
    }
    Ok(())
}

/// The bytes `save` writes for a value: tables convert by the file's extension.
pub(crate) fn encode(v: &Value, ext: &str, raw: bool) -> Result<Vec<u8>, String> {
    match v {
        Value::String(s) => return Ok(s.as_bytes().to_vec()),
        Value::Binary(b) => return Ok(b.clone()),
        Value::Nothing => return Ok(Vec::new()),
        _ => {}
    }
    let mut text = if raw { v.to_text() } else {
        match ext {
            "json" => formats::to_json(v, 2),
            "csv" => formats::to_csv(v, ',')?,
            "tsv" => formats::to_csv(v, '\t')?,
            _ => v.to_text(),
        }
    };
    if !text.ends_with('\n') { text.push('\n'); }
    Ok(text.into_bytes())
}

fn save(ctx: &mut Ctx, a: &Args, input: Value) -> Result<Value, ShellError> {
    let (v, span) = a.required(ctx, &input, 0, "a file name")?;
    let p = text_arg(&v, span)?;
    let full = abs(ctx, &p);
    let append = a.has("append");
    if !append && !a.has("force") && ctx.host.stat(&full).is_ok() {
        return Err(ShellError::at(format!("`{}` already exists", p), span, "exists").help("use `save -f` to replace it, or `save -a` to add to it"));
    }
    let data = encode(&input, &path::extension(&p), a.has("raw")).map_err(|e| ShellError::at(e, span, ""))?;
    ctx.host.write_file(&full, &data, append).map_err(|e| host_err("can't write", &p, span, e))?;
    Ok(Value::Nothing)
}

/// A file's bytes as a value, by extension.
pub(crate) fn decode(bytes: Vec<u8>, ext: &str) -> Result<Value, String> {
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => return Ok(Value::Binary(e.into_bytes())),
    };
    match ext {
        "json" => formats::from_json(&text),
        "toml" => formats::from_toml(&text),
        "csv" => formats::from_csv(&text, ',', true),
        "tsv" => formats::from_csv(&text, '\t', true),
        "txt" => {
            let mut lines: Vec<Value> = text.split('\n').map(|l| Value::String(l.strip_suffix('\r').unwrap_or(l).to_string())).collect();
            if text.ends_with('\n') { lines.pop(); }
            Ok(Value::List(lines))
        }
        _ => Ok(Value::String(text)),
    }
}

fn open(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let (v, span) = a.required(ctx, input, 0, "a file name")?;
    let p = text_arg(&v, span)?;
    let full = abs(ctx, &p);
    let bytes = ctx.host.read_file(&full).map_err(|e| host_err("can't open", &p, span, e))?;
    if a.has("raw") {
        return Ok(match String::from_utf8(bytes) { Ok(t) => Value::String(t), Err(e) => Value::Binary(e.into_bytes()) });
    }
    decode(bytes, &path::extension(&p)).map_err(|e| ShellError::at(format!("can't read `{}`: {}", p, e), span, "not valid").help("use `open --raw` for the text as it is"))
}

// ---------------------------------------------------------------- tables

fn where_(ctx: &mut Ctx, a: &Args, input: Value) -> Result<Value, ShellError> {
    let cond = *a.positional_exprs().first().ok_or_else(|| a.missing("a condition, like `size > 1mb`"))?;
    let items = list_input(input, a.call)?;
    let mut out = Vec::new();
    for it in items {
        match eval::eval(ctx, cond, &it, Some(&it))? {
            Value::Bool(true) => out.push(it),
            Value::Bool(false) => {}
            other => {
                let mut err = ShellError::at(format!("the condition must be true or false, but it is a {}", other.type_name()), cond.span, "not a condition");
                if let ExprKind::BareWord(_) = cond.kind { err = err.help("compare it with something, like `where size > 1mb`"); }
                return Err(err);
            }
        }
    }
    Ok(Value::List(out))
}

fn select(ctx: &mut Ctx, a: &Args, input: Value, reject: bool) -> Result<Value, ShellError> {
    let vals = a.all_positional(ctx, &input)?;
    if vals.is_empty() { return Err(a.missing("one or more columns")); }
    let mut cols = Vec::new();
    for (v, span) in vals { cols.push((text_arg(&v, span)?, span)); }
    let present = input.columns();
    for (c, span) in &cols {
        if !present.contains(c) && matches!(input, Value::List(_) | Value::Record(_)) && !present.is_empty() {
            return Err(no_column(c, *span, &input));
        }
    }
    let pick = |r: &Record| -> Record {
        if reject {
            r.iter().filter(|(k, _)| !cols.iter().any(|(c, _)| c == *k)).map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            cols.iter().map(|(c, _)| (c.clone(), r.get(c).cloned().unwrap_or(Value::Nothing))).collect()
        }
    };
    match input {
        Value::Record(r) => Ok(Value::Record(pick(&r))),
        Value::List(items) => {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Value::Record(r) => out.push(Value::Record(pick(&r))),
                    other => return Err(wrong_input(a.call, "a table", &other)),
                }
            }
            Ok(Value::List(out))
        }
        other => Err(wrong_input(a.call, "a table or record", &other)),
    }
}

fn sort_by(ctx: &mut Ctx, a: &Args, input: Value) -> Result<Value, ShellError> {
    let whole = input.clone();
    let mut cols = Vec::new();
    for (v, span) in a.all_positional(ctx, &input)? { cols.push((text_arg(&v, span)?, span)); }
    let reverse = a.has("reverse");
    let fold = a.has("ignore-case");
    let mut items = list_input(input, a.call)?;
    for (c, span) in &cols {
        if !items.iter().any(|it| matches!(it, Value::Record(r) if r.contains(c))) && !items.is_empty() {
            return Err(no_column(c, *span, &whole));
        }
    }
    let key = |v: &Value| -> Value {
        match v { Value::String(s) if fold => Value::String(s.to_lowercase()), other => other.clone() }
    };
    items.sort_by(|x, y| {
        let ord = if cols.is_empty() { key(x).sort_cmp(&key(y)) } else {
            let mut o = core::cmp::Ordering::Equal;
            for (c, _) in &cols {
                let get = |v: &Value| match v { Value::Record(r) => r.get(c).map(&key).unwrap_or(Value::Nothing), _ => Value::Nothing };
                o = get(x).sort_cmp(&get(y));
                if o != core::cmp::Ordering::Equal { break; }
            }
            o
        };
        if reverse { ord.reverse() } else { ord }
    });
    Ok(Value::List(items))
}

fn update(ctx: &mut Ctx, a: &Args, input: Value, insert: bool) -> Result<Value, ShellError> {
    let exprs = a.positional_exprs();
    let (col_expr, value_expr) = match exprs.as_slice() {
        [c, v] => (*c, *v),
        [_] => return Err(a.missing("a value or expression after the column")),
        _ => return Err(a.missing("a column and a value")),
    };
    let col = text_arg(&eval::eval(ctx, col_expr, &input, None)?, col_expr.span)?;
    let one = |ctx: &mut Ctx, row: Record, whole: &Value| -> Result<Value, ShellError> {
        let has = row.contains(&col);
        if insert && has {
            return Err(ShellError::at(format!("column `{}` already exists", col), col_expr.span, "exists").help("use `update` to change it"));
        }
        if !insert && !has {
            let err = no_column(&col, col_expr.span, whole);
            return Err(match &err.help {
                Some(h) if h.starts_with("did you mean") => err,
                _ => err.help("use `insert` to add a column"),
            });
        }
        let cell = row.get(&col).cloned().unwrap_or(Value::Nothing);
        let row_v = Value::Record(row);
        let v = eval::eval(ctx, value_expr, &cell, Some(&row_v))?;
        let Value::Record(mut row) = row_v else { unreachable!() };
        row.insert(col.clone(), v);
        Ok(Value::Record(row))
    };
    let whole = input.clone();
    match input {
        Value::Record(r) => one(ctx, r, &whole),
        Value::List(items) => {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Value::Record(r) => out.push(one(ctx, r, &whole)?),
                    other => return Err(wrong_input(a.call, "a table", &other)),
                }
            }
            Ok(Value::List(out))
        }
        other => Err(wrong_input(a.call, "a table or record", &other)),
    }
}

fn math(call: &Call, input: Value) -> Result<Value, ShellError> {
    let op = call.name.trim_start_matches("math ");
    let reduce = |vals: &[Value]| -> Result<Value, String> {
        match op {
            "sum" | "avg" => {
                let mut acc = Value::Int(0);
                for (i, v) in vals.iter().enumerate() {
                    acc = if i == 0 { v.clone() } else { eval::apply(Op::Add, &acc, v)? };
                    if !matches!(acc, Value::Int(_) | Value::Float(_) | Value::Filesize(_) | Value::Duration(_)) {
                        return Err(format!("can't add up a {}", v.type_name()));
                    }
                }
                if op == "avg" {
                    if vals.is_empty() { return Err(String::from("no values to average")); }
                    acc = eval::apply(Op::Div, &acc, &Value::Int(vals.len() as i64))?;
                }
                Ok(acc)
            }
            _ => {
                let mut best: Option<&Value> = None;
                for v in vals {
                    best = Some(match best {
                        None => v,
                        Some(b) => {
                            let o = v.compare(b).ok_or_else(|| format!("can't compare {} with {}", v.type_name(), b.type_name()))?;
                            if (op == "min" && o.is_lt()) || (op == "max" && o.is_gt()) { v } else { b }
                        }
                    });
                }
                best.cloned().ok_or_else(|| String::from("no values"))
            }
        }
    };
    let err = |e: String| ShellError::at(e, call.head_span, "");
    match input {
        Value::List(items) if !items.is_empty() && items.iter().all(|i| matches!(i, Value::Record(_))) => {
            let whole = Value::List(items);
            let mut out = Record::new();
            for c in whole.columns() {
                let Value::List(rows) = &whole else { unreachable!() };
                let vals: Vec<Value> = rows.iter().filter_map(|r| if let Value::Record(r) = r { r.get(&c).cloned() } else { None }).collect();
                if let Ok(v) = reduce(&vals) { out.insert(c, v); }
            }
            Ok(Value::Record(out))
        }
        Value::List(items) => reduce(&items).map_err(err),
        Value::Nothing if op == "sum" => Ok(Value::Int(0)),
        other @ (Value::Int(_) | Value::Float(_) | Value::Filesize(_) | Value::Duration(_)) => reduce(&[other]).map_err(err),
        other => Err(wrong_input(call, "a list of numbers", &other)),
    }
}

fn strings(ctx: &mut Ctx, a: &Args, input: Value) -> Result<Value, ShellError> {
    let name = a.call.name.trim_start_matches("str ");
    let vals = a.all_positional(ctx, &input)?;
    let fixed = match name { "contains" => 1, "replace" => 2, _ => 0 };
    if vals.len() < fixed {
        return Err(a.missing(if fixed == 1 { "the text to look for" } else { "the text to find and its replacement" }));
    }
    let mut texts = Vec::new();
    for (v, span) in &vals[..fixed] { texts.push(text_arg(v, *span)?); }
    let mut cols = Vec::new();
    for (v, span) in &vals[fixed..] { cols.push((text_arg(v, *span)?, *span)); }
    let fold = a.has("ignore-case");
    let all = a.has("all");
    let f = |s: &str| -> Value {
        match name {
            "contains" => Value::Bool(if fold { s.to_lowercase().contains(&texts[0].to_lowercase()) } else { s.contains(texts[0].as_str()) }),
            "replace" => Value::String(if all { s.replace(texts[0].as_str(), &texts[1]) } else { s.replacen(texts[0].as_str(), &texts[1], 1) }),
            "upcase" => Value::String(s.to_uppercase()),
            "downcase" => Value::String(s.to_lowercase()),
            "trim" => Value::String(s.trim().to_string()),
            _ => Value::Int(s.chars().count() as i64),
        }
    };
    map_text(a.call, &input, &input, &cols, &f)
}

fn map_text(call: &Call, v: &Value, whole: &Value, cols: &[(String, Span)], f: &dyn Fn(&str) -> Value) -> Result<Value, ShellError> {
    match v {
        Value::String(s) if cols.is_empty() => Ok(f(s)),
        Value::List(items) => {
            let mut out = Vec::new();
            for it in items { out.push(map_text(call, it, whole, cols, f)?); }
            Ok(Value::List(out))
        }
        Value::Record(r) if !cols.is_empty() => {
            let mut r = r.clone();
            for (c, span) in cols {
                match r.get(c) {
                    Some(Value::String(s)) => { let n = f(s); r.insert(c.clone(), n); }
                    Some(other) => return Err(ShellError::at(format!("column `{}` holds a {}, not text", c, other.type_name()), *span, "not text")),
                    None => return Err(no_column(c, *span, whole)),
                }
            }
            Ok(Value::Record(r))
        }
        Value::Record(_) => Err(ShellError::at(format!("`{}` on a table needs the columns to change", call.name), call.head_span, "which columns?").help(format!("like `{} name`", call.name))),
        other => Err(wrong_input(call, "text", other)),
    }
}

fn seq(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let vals = a.all_positional(ctx, input)?;
    let mut nums = Vec::new();
    for (v, span) in &vals {
        match v { Value::Int(i) => nums.push(*i), _ => return Err(ShellError::at("`seq` takes whole numbers", *span, "not an int")) }
    }
    let (first, step, last) = match nums.as_slice() {
        [last] => (1, 1, *last),
        [first, last] => (*first, if last >= first { 1 } else { -1 }, *last),
        [first, step, last] => (*first, *step, *last),
        _ => return Err(a.missing("one to three numbers: [first] [step] last")),
    };
    if step == 0 { return Err(ShellError::at("the step can't be zero", vals[1].1, "")); }
    let span = (last - first) / step;
    if span > 1_000_000 { return Err(ShellError::at("that is more than a million numbers", a.call.span, "")); }
    let mut out = Vec::new();
    let mut x = first;
    while (step > 0 && x <= last) || (step < 0 && x >= last) {
        out.push(Value::Int(x));
        x += step;
    }
    Ok(Value::List(out))
}

fn help(ctx: &mut Ctx, a: &Args, input: &Value) -> Result<Value, ShellError> {
    let words: Vec<String> = a.all_positional(ctx, input)?.into_iter().map(|(v, _)| v.to_text()).collect();
    if words.is_empty() {
        return Ok(Value::List(COMMANDS.iter().map(|s| {
            Value::Record(Record::new().with("command", Value::String(s.name.to_string())).with("description", Value::String(s.desc.to_string())))
        }).collect()));
    }
    let name = words.join(" ");
    if let Some(sig) = signature::lookup(&name) {
        let mut t = format!("{}\n\nUsage:\n  {} {}\n", sig.desc, sig.name, sig.args);
        if !sig.flags.is_empty() {
            t.push_str("\nFlags:\n");
            let names: Vec<String> = sig.flags.iter().map(|f| {
                let short = f.short.map(|c| format!("-{}, ", c)).unwrap_or_else(|| String::from("    "));
                format!("{}--{}{}", short, f.long, if f.value { " <value>" } else { "" })
            }).collect();
            let w = names.iter().map(|n| n.len()).max().unwrap_or(0);
            for (n, f) in names.iter().zip(sig.flags) {
                t.push_str(&format!("  {:w$}  {}\n", n, f.desc, w = w));
            }
        }
        return Ok(Value::String(t));
    }
    if signature::NAMESPACES.contains(&name.as_str()) {
        return Ok(Value::List(signature::subcommands(&name).map(|s| {
            Value::Record(Record::new().with("command", Value::String(s.name.to_string())).with("description", Value::String(s.desc.to_string())))
        }).collect()));
    }
    let span = a.positional_exprs().first().map(|e| e.span).unwrap_or(a.call.span);
    let err = ShellError::at(format!("no command `{}`", name), span, "unknown");
    Err(match crate::suggest::closest(&name, COMMANDS.iter().map(|s| s.name)) {
        Some(best) => err.help(format!("did you mean `{}`?", best)),
        None => err.help("`help` lists every command"),
    })
}
