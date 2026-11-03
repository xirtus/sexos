// SPDX-License-Identifier: GPL-3.0-only
//! A bounded script subset for inline `<script>` in the Browser reader.
//!
//! This is not a JavaScript engine. It accepts:
//! `var|let|const name = expr`, `name = expr` (also `+=`, `-=`, `++`, `--`),
//! `if (...) ... else ...`, `while (...)`, `for (...; ...; ...)` with
//! `break`/`continue`, `{ ... }`
//! blocks, `function name(a, b) { ... return x }` declared before use, and
//! `document.write(...)` / `document.writeln(...)`.
//! Expressions: number, string, template (`` `a ${x}` ``) and `true`/`false`
//! literals, variables,
//! `+ - * / %`, comparisons (`< > <= >= === !== == !=`), `! && ||`, `?:`,
//! parentheses, arrays (`[a, b]`, `x[i]`, `x[i] = v`, `.push(v)`, `.join(s)`),
//! objects (`{a: 1, 'b': x}`, `o.a`, `o[k]`, `o.a = v`, `Object.keys(o)`),
//! `JSON.stringify(v)`, `null`/`undefined`, `typeof`, `n.toFixed(d)`,
//! strings (`.length`, `.toUpperCase()`, `.toLowerCase()`, `.trim()`,
//! `.indexOf(s)`, `.charAt(i)`, `.slice(a, b)`, `.substring(a, b)`,
//! `.split(s)`), `String(x)`, `Number(x)`, `parseInt(s)` and
//! `Math.floor/ceil/round/trunc/sign/abs/min/max/sqrt/pow` and `Math.PI`
//! (no `Math.random`: a script holds no entropy source). Numbers are IEEE doubles printed the
//! way JavaScript prints ordinary values; `+` with a string concatenates.
//! Every executed statement spends one unit of a fixed step budget, so loops
//! always end (an exhausted budget refuses the script); there are no names
//! for DOM, timers, storage or network. Any unsupported construct means the whole
//! script writes nothing. Written text is inserted as text, never markup.
use alloc::vec::Vec;
use core::fmt::Write;

pub const MAX_SOURCE: usize = 4096;
pub const MAX_OUTPUT: usize = 1024;
const ARENA: usize = 8192;
/// Statements executed per script, loop bodies included.
const STEPS: u32 = 10_000;
// Arena and output live on the heap (fallibly): the Browser stack is tight.
const VARS: usize = 16;
const FUNCS: usize = 8;
/// Array elements (all arrays) and array count; arrays are shared by reference.
const ELEMS: usize = 512;
const ARRAYS: usize = 32;
/// Objects and their properties (name in the arena, value), by reference.
const OBJECTS: usize = 16;
const PROPS: usize = 256;
const PARAMS: usize = 4;
const NAME: usize = 16;
// Nesting budget (expressions, statements, calls). Measured natively at 32
// with fact(7): the Browser stack stays below its 46.7 KiB construction peak
// (feature ledger F30).
const MAX_DEPTH: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Unsupported, Syntax, Limit }

#[derive(Clone, Copy)]
enum Value { Num(f64), Str(u16, u16), Bool(bool), Undef, Null, Arr(u8), Obj(u8) }

#[derive(Clone, Copy, PartialEq)]
enum Tok<'a> { Num(f64), Str(&'a [u8]), Template(&'a [u8]), Ident(&'a [u8]), Punct(u8), End }


struct Machine<'a> {
    src: &'a [u8],
    at: usize,
    tok: Tok<'a>,
    /// A newline was skipped before the current token (statement end).
    nl: bool,
    arena: Vec<u8>,
    vars: [([u8; NAME], u8, Value); VARS],
    nvars: usize,
    out: Vec<u8>,
    depth: u8,
    steps: u32,
    funcs: [([u8; NAME], u8, Pos<'a>); FUNCS],
    nfuncs: usize,
    /// Checking a function body at its declaration: names may be parameters.
    skim: bool,
    /// The current statement runs (calls only happen when live).
    live: bool,
    /// Set by `return` until the enclosing call takes it.
    ret: Option<Value>,
    /// `break` (true) or `continue` (false) pending for the innermost loop.
    jump: Option<bool>,
    /// Loops open in the current function (0 = `break` is a syntax error).
    loops: u8,
    elems: Vec<Value>,
    /// (start, len, cap) of each array inside `elems`.
    arrays: [(u16, u16, u16); ARRAYS],
    narrays: usize,
    props: Vec<(u16, u16, Value)>,
    /// (start, len, cap) of each object inside `props`.
    objects: [(u16, u16, u16); OBJECTS],
    nobjects: usize,
}

/// A saved lexer position, to re-run a loop condition or body.
#[derive(Clone, Copy)]
struct Pos<'a> { at: usize, tok: Tok<'a>, nl: bool }

fn ident_start(b: u8) -> bool { b.is_ascii_alphabetic() || b == b'_' || b == b'$' }

impl<'a> Machine<'a> {
    fn next(&mut self) -> Result<(), Error> {
        let s = self.src;
        self.nl = false;
        loop {
            while self.at < s.len() && s[self.at].is_ascii_whitespace() { self.nl |= s[self.at] == b'\n'; self.at += 1; }
            if s[self.at..].starts_with(b"//") {
                while self.at < s.len() && s[self.at] != b'\n' { self.at += 1; }
                continue;
            }
            if s[self.at..].starts_with(b"/*") {
                let end = s[self.at + 2..].windows(2).position(|w| w == b"*/").ok_or(Error::Syntax)?;
                self.at += end + 4;
                continue;
            }
            break;
        }
        let Some(&c) = s.get(self.at) else { self.tok = Tok::End; return Ok(()) };
        self.tok = if c.is_ascii_digit() || (c == b'.' && s.get(self.at + 1).is_some_and(u8::is_ascii_digit)) {
            let start = self.at;
            while self.at < s.len() && (s[self.at].is_ascii_alphanumeric() || s[self.at] == b'.'
                || (matches!(s[self.at], b'+' | b'-') && matches!(s[self.at - 1], b'e' | b'E'))) { self.at += 1; }
            let text = core::str::from_utf8(&s[start..self.at]).map_err(|_| Error::Syntax)?;
            Tok::Num(text.parse::<f64>().map_err(|_| Error::Syntax)?)
        } else if c == b'`' {
            let start = self.at + 1;
            let end = s[start..].iter().position(|&b| b == b'`').ok_or(Error::Syntax)? + start;
            self.at = end + 1;
            Tok::Template(&s[start..end])
        } else if c == b'"' || c == b'\'' {
            let start = self.at + 1;
            let mut i = start;
            while i < s.len() && s[i] != c { if s[i] == b'\\' { i += 1; } if s.get(i) == Some(&b'\n') { return Err(Error::Syntax); } i += 1; }
            if i >= s.len() { return Err(Error::Syntax); }
            self.at = i + 1;
            Tok::Str(&s[start..i])
        } else if ident_start(c) {
            let start = self.at;
            while self.at < s.len() && (ident_start(s[self.at]) || s[self.at].is_ascii_digit()) { self.at += 1; }
            Tok::Ident(&s[start..self.at])
        } else if let Some((op, len)) = [(&b"==="[..], b'E'), (b"!==", b'N'), (b"==", b'e'), (b"!=", b'n'), (b"<=", b'L'), (b">=", b'G'), (b"&&", b'&'), (b"||", b'|'),
                (b"++", b'I'), (b"--", b'D'), (b"+=", b'P'), (b"-=", b'M')]
            .iter().find(|(t, _)| s[self.at..].starts_with(t)).map(|(t, op)| (*op, t.len())) {
            self.at += len;
            Tok::Punct(op)
        } else if b"+-*/%()=;,.<>!?:{}[]".contains(&c) {
            self.at += 1;
            Tok::Punct(c)
        } else {
            return Err(Error::Unsupported);
        };
        Ok(())
    }
    fn expect(&mut self, p: u8) -> Result<(), Error> {
        if self.tok != Tok::Punct(p) { return Err(Error::Syntax); }
        self.next()
    }

    fn store(&mut self, bytes: &[u8]) -> Result<Value, Error> {
        let start = self.arena.len();
        if start + bytes.len() > ARENA { return Err(Error::Limit); }
        self.arena.extend_from_slice(bytes);
        Ok(Value::Str(start as u16, bytes.len() as u16))
    }
    fn literal(&mut self, raw: &[u8]) -> Result<Value, Error> {
        let start = self.arena.len();
        let mut i = 0;
        while i < raw.len() {
            let mut b = raw[i];
            if b == b'\\' {
                i += 1;
                b = match raw.get(i).ok_or(Error::Syntax)? { b'n' => b'\n', b't' => b'\t', b'r' => b'\r', b'0' => 0, &other => other };
            }
            if self.arena.len() >= ARENA { return Err(Error::Limit); }
            self.arena.push(b);
            i += 1;
        }
        Ok(Value::Str(start as u16, (self.arena.len() - start) as u16))
    }
    /// `` `text ${expr} text` ``: each expression is parsed in place from the
    /// script source, then the lexer resumes after the template.
    fn template(&mut self, raw: &'a [u8]) -> Result<Value, Error> {
        let base = raw.as_ptr() as usize - self.src.as_ptr() as usize;
        let resume = self.pos();
        let mut text: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            if raw[i..].starts_with(b"${") {
                self.at = base + i + 2;
                self.next()?;
                let v = self.expr()?;
                if self.tok != Tok::Punct(b'}') { return Err(Error::Syntax); }
                let t = self.to_text(v)?;
                text.extend_from_slice(self.text(t));
                i = self.at - base;
            } else {
                let mut b = raw[i];
                if b == b'\\' && i + 1 < raw.len() { i += 1; b = match raw[i] { b'n' => b'\n', b't' => b'\t', other => other }; }
                text.push(b);
                i += 1;
            }
            if text.len() > ARENA { return Err(Error::Limit); }
        }
        self.restore(resume);
        self.store(&text)
    }
    fn text(&self, v: Value) -> &[u8] {
        match v { Value::Str(s, n) => &self.arena[s as usize..s as usize + n as usize], _ => &[] }
    }
    /// JavaScript's Number-to-String for ordinary values.
    fn number_text(&mut self, x: f64) -> Result<Value, Error> {
        let mut buf = Buf { b: [0; 40], n: 0 };
        if x.is_nan() { buf.push(b"NaN"); }
        else if x.is_infinite() { buf.push(if x > 0.0 { b"Infinity" } else { b"-Infinity" }); }
        else if x == 0.0 { buf.push(b"0"); }
        else {
            let a = if x < 0.0 { -x } else { x };
            if a >= 1e21 || a < 1e-6 {
                let _ = write!(buf, "{:e}", x);
                // JavaScript writes the exponent sign: 1e+21.
                if let Some(e) = buf.b[..buf.n].iter().position(|&c| c == b'e') {
                    if buf.b.get(e + 1) != Some(&b'-') && buf.n < 40 {
                        buf.b.copy_within(e + 1..buf.n, e + 2); buf.b[e + 1] = b'+'; buf.n += 1;
                    }
                }
            } else { let _ = write!(buf, "{}", x); }
        }
        let n = buf.n;
        self.store(&buf.b[..n])
    }
    fn to_text(&mut self, v: Value) -> Result<Value, Error> {
        match v { Value::Num(x) => self.number_text(x), Value::Bool(b) => self.store(if b { b"true" } else { b"false" }), Value::Undef => self.store(b"undefined"), Value::Null => self.store(b"null"), Value::Arr(id) => self.join(id, b","), Value::Obj(_) => self.store(b"[object Object]"), s => Ok(s) }
    }
    fn truthy(&self, v: Value) -> bool {
        match v { Value::Num(x) => x != 0.0 && !x.is_nan(), Value::Bool(b) => b, Value::Undef | Value::Null => false, Value::Arr(_) | Value::Obj(_) => true, s => !self.text(s).is_empty() }
    }
    fn to_number(&self, v: Value) -> f64 {
        match v {
            Value::Num(x) => x,
            Value::Bool(b) => b as u8 as f64,
            Value::Null => 0.0,
            Value::Undef | Value::Arr(_) | Value::Obj(_) => f64::NAN,
            s => {
                let t = self.text(s);
                let t = core::str::from_utf8(t).unwrap_or("x").trim();
                if t.is_empty() { 0.0 } else { t.parse::<f64>().unwrap_or(f64::NAN) }
            }
        }
    }

    /// Full expression, including `?:`. Each nesting level (parentheses,
    /// unary operators, branches) costs one unit of the depth budget.
    fn expr(&mut self) -> Result<Value, Error> {
        if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
        self.depth += 1;
        let v = self.binary(1);
        self.depth -= 1;
        v
    }
    /// Precedence climbing keeps stack use per nesting level small.
    fn binary(&mut self, min: u8) -> Result<Value, Error> {
        let mut v = self.unary()?;
        loop {
            let Tok::Punct(op) = self.tok else { return Ok(v) };
            let prec = match op { b'?' => 1, b'|' => 2, b'&' => 3, b'E' | b'N' | b'e' | b'n' => 4, b'<' | b'>' | b'L' | b'G' => 5, b'+' | b'-' => 6, b'*' | b'/' | b'%' => 7, _ => 0 };
            if prec == 0 || prec < min { return Ok(v); }
            self.next()?;
            if op == b'?' {
                let yes = self.expr()?;
                self.expect(b':')?;
                let no = self.binary(1)?;
                v = if self.truthy(v) { yes } else { no };
                continue;
            }
            let r = self.binary(prec + 1)?;
            v = self.apply(op, v, r)?;
        }
    }
    fn apply(&mut self, op: u8, v: Value, r: Value) -> Result<Value, Error> {
        Ok(match op {
            b'|' => if self.truthy(v) { v } else { r },
            b'&' => if self.truthy(v) { r } else { v },
            b'E' | b'N' | b'e' | b'n' => {
                let strict = matches!(op, b'E' | b'N');
                let same = match (v, r) {
                    (Value::Str(..), Value::Str(..)) => self.text(v) == self.text(r),
                    (Value::Bool(a), Value::Bool(b)) => a == b,
                    (Value::Num(a), Value::Num(b)) => a == b,
                    (Value::Null | Value::Undef, Value::Null | Value::Undef) => !strict || core::mem::discriminant(&v) == core::mem::discriminant(&r),
                    (Value::Null | Value::Undef, _) | (_, Value::Null | Value::Undef) => false,
                    (Value::Arr(a), Value::Arr(b)) => a == b,
                    (Value::Obj(a), Value::Obj(b)) => a == b,
                    _ if strict => false,
                    _ => self.to_number(v) == self.to_number(r),
                };
                Value::Bool(same == matches!(op, b'E' | b'e'))
            }
            b'<' | b'>' | b'L' | b'G' => {
                let order = if let (Value::Str(..), Value::Str(..)) = (v, r) { Some(self.text(v).cmp(self.text(r))) }
                    else { self.to_number(v).partial_cmp(&self.to_number(r)) };
                Value::Bool(order.is_some_and(|o| match op { b'<' => o.is_lt(), b'>' => o.is_gt(), b'L' => o.is_le(), _ => o.is_ge() }))
            }
            b'+' if matches!(v, Value::Str(..)) || matches!(r, Value::Str(..)) => {
                let (l, r) = (self.to_text(v)?, self.to_text(r)?);
                let start = self.arena.len();
                let (ls, ln) = match l { Value::Str(s, n) => (s as usize, n as usize), _ => (0, 0) };
                let (rs, rn) = match r { Value::Str(s, n) => (s as usize, n as usize), _ => (0, 0) };
                if start + ln + rn > ARENA { return Err(Error::Limit); }
                self.arena.extend_from_within(ls..ls + ln);
                self.arena.extend_from_within(rs..rs + rn);
                Value::Str(start as u16, (ln + rn) as u16)
            }
            _ => {
                let (a, b) = (self.to_number(v), self.to_number(r));
                Value::Num(match op { b'+' => a + b, b'-' => a - b, b'*' => a * b, b'/' => a / b, _ => rem(a, b) })
            }
        })
    }
    fn unary(&mut self) -> Result<Value, Error> {
        if self.tok == Tok::Ident(b"typeof") {
            if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
            self.next()?;
            self.depth += 1;
            // An undeclared name is "undefined" here, as in JavaScript.
            let v = if let Tok::Ident(name) = self.tok { if self.lookup(name).is_none() && !matches!(name, b"NaN" | b"Infinity" | b"true" | b"false" | b"null" | b"undefined" | b"Math" | b"JSON" | b"Object" | b"String" | b"Number" | b"parseInt") && self.funcs[..self.nfuncs].iter().all(|(n, l, _)| &n[..*l as usize] != name) { self.next()?; Value::Undef } else { self.unary()? } } else { self.unary()? };
            self.depth -= 1;
            let kind: &[u8] = match v { Value::Num(_) => b"number", Value::Str(..) => b"string", Value::Bool(_) => b"boolean", Value::Undef => b"undefined", _ => b"object" };
            return self.store(kind);
        }
        if let Tok::Punct(op @ (b'-' | b'+' | b'!')) = self.tok {
            if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
            self.next()?;
            self.depth += 1;
            let v = self.unary()?;
            self.depth -= 1;
            if op == b'!' { return Ok(Value::Bool(!self.truthy(v))); }
            let x = self.to_number(v);
            return Ok(Value::Num(if op == b'-' { -x } else { x }));
        }
        let v = self.atom()?;
        self.postfix(v)
    }
    /// `.length`, `.toUpperCase()`, `.toLowerCase()` on strings.
    fn postfix(&mut self, mut v: Value) -> Result<Value, Error> {
        loop {
            if self.tok == Tok::Punct(b'[') {
                self.next()?;
                let i = self.expr()?;
                self.expect(b']')?;
                v = if let Value::Obj(id) = v { let k = self.to_text(i)?; let key = self.text(k).to_vec(); self.get_prop(id, &key) }
                    else { let i = self.to_number(i); self.index(v, i)? };
                continue;
            }
            if self.tok != Tok::Punct(b'.') { return Ok(v); }
            self.next()?;
            let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            if let Value::Obj(id) = v { v = self.get_prop(id, name); continue; }
            if let (Value::Num(x), b"toFixed") = (v, name) {
                self.expect(b'(')?;
                let d = if self.tok == Tok::Punct(b')') { Value::Num(0.0) } else { self.expr()? };
                self.expect(b')')?;
                let d = self.to_number(d);
                if !(0.0..=20.0).contains(&d) || d != floor(d) { return Err(Error::Unsupported); }
                v = self.fixed(x, d as usize)?;
                continue;
            }
            if let Value::Arr(id) = v {
                let (_, len, _) = self.arrays[id as usize];
                v = match name {
                    b"length" => Value::Num(len as f64),
                    b"push" => {
                        self.expect(b'(')?;
                        let x = self.expr()?;
                        self.expect(b')')?;
                        if self.live { self.push(id, x)?; }
                        Value::Num(self.arrays[id as usize].1 as f64)
                    }
                    b"join" => {
                        self.expect(b'(')?;
                        let sep = if self.tok == Tok::Punct(b')') { None } else { Some(self.expr()?) };
                        self.expect(b')')?;
                        let sep = match sep { Some(x) => { let t = self.to_text(x)?; self.text(t).to_vec() } None => b",".to_vec() };
                        self.join(id, &sep)?
                    }
                    _ => return Err(Error::Unsupported),
                };
                continue;
            }
            let s = self.to_text(v)?;
            v = match name {
                b"length" => Value::Num(self.text(s).len() as f64),
                b"trim" | b"indexOf" | b"charAt" | b"slice" | b"substring" | b"split" => {
                    self.expect(b'(')?;
                    let mut args = [Value::Undef; 2];
                    let mut n = 0;
                    if self.tok != Tok::Punct(b')') {
                        loop {
                            if n == 2 { return Err(Error::Unsupported); }
                            args[n] = self.expr()?;
                            n += 1;
                            if self.tok != Tok::Punct(b',') { break; }
                            self.next()?;
                        }
                    }
                    self.expect(b')')?;
                    self.string_method(name, s, args)?
                }
                b"toUpperCase" | b"toLowerCase" => {
                    self.expect(b'(')?;
                    self.expect(b')')?;
                    let Value::Str(start, n) = s else { return Err(Error::Syntax) };
                    let at = self.arena.len();
                    if at + n as usize > ARENA { return Err(Error::Limit); }
                    self.arena.extend_from_within(start as usize..start as usize + n as usize);
                    for b in &mut self.arena[at..] { *b = if name == b"toUpperCase" { b.to_ascii_uppercase() } else { b.to_ascii_lowercase() }; }
                    Value::Str(at as u16, n)
                }
                _ => return Err(Error::Unsupported),
            };
        }
    }
    /// String methods on byte offsets (the subset treats text as bytes).
    fn string_method(&mut self, name: &[u8], s: Value, args: [Value; 2]) -> Result<Value, Error> {
        let Value::Str(start, n) = s else { return Err(Error::Syntax) };
        let (start, n) = (start as usize, n as usize);
        // JavaScript index rules: negatives count from the end for slice only.
        let pos = |m: &Self, v: Value, dflt: usize, from_end: bool| -> usize {
            if matches!(v, Value::Undef) { return dflt; }
            let x = m.to_number(v);
            let x = if x.is_nan() { 0.0 } else { floor(x) };
            let x = if x < 0.0 && from_end { n as f64 + x } else { x };
            if x < 0.0 { 0 } else if x > n as f64 { n } else { x as usize }
        };
        Ok(match name {
            b"trim" => {
                let t = &self.arena[start..start + n];
                let a = t.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(n);
                let b = t.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(a, |p| p + 1);
                Value::Str((start + a) as u16, (b - a) as u16)
            }
            b"indexOf" => {
                let needle = self.to_text(args[0])?;
                let (h, k) = (self.text(s), self.text(needle));
                Value::Num(if k.is_empty() { 0.0 } else { h.windows(k.len()).position(|w| w == k).map_or(-1.0, |p| p as f64) })
            }
            b"charAt" => { let i = pos(self, args[0], 0, false); if i < n { Value::Str((start + i) as u16, 1) } else { self.store(b"")? } }
            b"slice" | b"substring" => {
                let slice = name == b"slice";
                let (mut a, mut b) = (pos(self, args[0], 0, slice), pos(self, args[1], n, slice));
                if !slice && a > b { core::mem::swap(&mut a, &mut b); }
                if b < a { b = a; }
                Value::Str((start + a) as u16, (b - a) as u16)
            }
            _ => {
                // split(sep): a new array of pieces (no separator: one piece).
                let sep = match args[0] { Value::Undef => None, v => { let t = self.to_text(v)?; Some(t) } };
                let mut items = [Value::Undef; 16];
                let mut count = 0;
                let mut put = |v: Value, c: &mut usize| -> Result<(), Error> { if *c == 16 { return Err(Error::Limit); } items[*c] = v; *c += 1; Ok(()) };
                match sep {
                    None => put(s, &mut count)?,
                    Some(Value::Str(_, 0)) => for i in 0..n { put(Value::Str((start + i) as u16, 1), &mut count)?; },
                    Some(Value::Str(ss, sn)) => {
                        let k = sn as usize;
                        let (mut from, mut i) = (0, 0);
                        while i + k <= n {
                            if self.arena[start + i..start + i + k] == self.arena[ss as usize..ss as usize + k] {
                                put(Value::Str((start + from) as u16, (i - from) as u16), &mut count)?;
                                i += k; from = i;
                            } else { i += 1; }
                        }
                        put(Value::Str((start + from) as u16, (n - from) as u16), &mut count)?;
                    }
                    _ => return Err(Error::Unsupported),
                }
                self.new_array(&items[..count])?
            }
        })
    }
    /// `String(x)`, `Number(x)`, `parseInt(s)`.
    fn convert(&mut self, name: &[u8]) -> Result<Value, Error> {
        self.expect(b'(')?;
        let v = if self.tok == Tok::Punct(b')') { Value::Undef } else { self.expr()? };
        self.expect(b')')?;
        Ok(match name {
            b"String" => self.to_text(v)?,
            b"Number" => Value::Num(if matches!(v, Value::Undef) { f64::NAN } else { self.to_number(v) }),
            _ => {
                let t = self.to_text(v)?;
                let t = self.text(t);
                let t = &t[t.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(t.len())..];
                let (neg, digits) = match t.first() { Some(b'-') => (true, &t[1..]), Some(b'+') => (false, &t[1..]), _ => (false, t) };
                let k = digits.iter().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
                if k == 0 { Value::Num(f64::NAN) } else {
                    let mut x = 0.0f64;
                    for &d in &digits[..k] { x = x * 10.0 + (d - b'0') as f64; }
                    Value::Num(if neg { -x } else { x })
                }
            }
        })
    }
    /// `x[i]`: an array element or a one-byte string; out of range is undefined.
    fn index(&mut self, v: Value, i: f64) -> Result<Value, Error> {
        let whole = i >= 0.0 && i == floor(i) && i < 65536.0;
        Ok(match v {
            Value::Arr(id) => {
                let (start, len, _) = self.arrays[id as usize];
                if whole && (i as u16) < len { self.elems[start as usize + i as usize] } else { Value::Undef }
            }
            Value::Str(start, n) if whole && (i as u16) < n => { let b = self.arena[start as usize + i as usize]; self.store(&[b])? }
            Value::Str(..) => Value::Undef,
            _ => return Err(Error::Unsupported),
        })
    }
    fn find_prop(&self, id: u8, key: &[u8]) -> Option<usize> {
        let (start, len, _) = self.objects[id as usize];
        (start as usize..start as usize + len as usize).find(|&k| { let (s0, n, _) = self.props[k]; &self.arena[s0 as usize..s0 as usize + n as usize] == key })
    }
    fn get_prop(&self, id: u8, key: &[u8]) -> Value {
        self.find_prop(id, key).map_or(Value::Undef, |k| self.props[k].2)
    }
    fn set_prop(&mut self, id: u8, key: &[u8], v: Value) -> Result<(), Error> {
        if let Some(k) = self.find_prop(id, key) { self.props[k].2 = v; return Ok(()); }
        let name = self.store(key)?;
        let Value::Str(ns, nn) = name else { return Err(Error::Syntax) };
        let (start, len, cap) = self.objects[id as usize];
        if len == cap {
            let cap2 = (cap as usize * 2).max(4);
            let at = self.props.len();
            if at + cap2 > PROPS { return Err(Error::Limit); }
            self.props.extend_from_within(start as usize..start as usize + len as usize);
            self.props.resize(at + cap2, (0, 0, Value::Undef));
            self.objects[id as usize] = (at as u16, len, cap2 as u16);
        }
        let (start, len, cap) = self.objects[id as usize];
        self.props[start as usize + len as usize] = (ns, nn, v);
        self.objects[id as usize] = (start, len + 1, cap);
        Ok(())
    }
    fn new_object(&mut self) -> Result<u8, Error> {
        if self.nobjects == OBJECTS || self.props.len() + 4 > PROPS { return Err(Error::Limit); }
        let at = self.props.len();
        self.props.resize(at + 4, (0, 0, Value::Undef));
        self.objects[self.nobjects] = (at as u16, 0, 4);
        self.nobjects += 1;
        Ok((self.nobjects - 1) as u8)
    }
    /// JSON text of a value; undefined members are left out like JavaScript.
    fn json(&mut self, v: Value, into: &mut Vec<u8>) -> Result<(), Error> {
        if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
        if into.len() > ARENA { return Err(Error::Limit); }
        self.depth += 1;
        match v {
            Value::Num(x) if x.is_nan() || x.is_infinite() => into.extend_from_slice(b"null"),
            Value::Str(..) => self.quote(v, into),
            Value::Arr(id) => {
                into.push(b'[');
                let (start, len, _) = self.arrays[id as usize];
                for k in 0..len as usize {
                    if k > 0 { into.push(b','); }
                    let e = self.elems[start as usize + k];
                    if matches!(e, Value::Undef) { into.extend_from_slice(b"null"); } else { self.json(e, into)?; }
                }
                into.push(b']');
            }
            Value::Obj(id) => {
                into.push(b'{');
                let (start, len, _) = self.objects[id as usize];
                let mut first = true;
                for k in start as usize..start as usize + len as usize {
                    let (ns, nn, value) = self.props[k];
                    if matches!(value, Value::Undef) { continue; }
                    if !first { into.push(b','); }
                    first = false;
                    self.quote(Value::Str(ns, nn), into);
                    into.push(b':');
                    self.json(value, into)?;
                }
                into.push(b'}');
            }
            Value::Undef => into.extend_from_slice(b"null"),
            other => { let t = self.to_text(other)?; into.extend_from_slice(self.text(t)); }
        }
        self.depth -= 1;
        Ok(())
    }
    fn quote(&self, v: Value, into: &mut Vec<u8>) {
        into.push(b'"');
        for &b in self.text(v) {
            match b { b'"' => into.extend_from_slice(b"\\\""), b'\\' => into.extend_from_slice(b"\\\\"), b'\n' => into.extend_from_slice(b"\\n"), b'\t' => into.extend_from_slice(b"\\t"), 0..=31 => into.extend_from_slice(b"?"), _ => into.push(b) }
        }
        into.push(b'"');
    }
    /// `x.toFixed(d)`: the exact binary value rounded half away from zero.
    fn fixed(&mut self, x: f64, d: usize) -> Result<Value, Error> {
        if x.is_nan() || x.is_infinite() || x.abs_ge(1e21) { return self.number_text(x); }
        // Print with 40 digits (exact for these magnitudes in practice), then
        // round the decimal string ourselves so ties go away from zero.
        let mut wide = alloc::string::String::new();
        let _ = write!(wide, "{:.40}", if x < 0.0 { -x } else { x });
        let bytes = wide.as_bytes();
        let dot = bytes.iter().position(|&c| c == b'.').unwrap_or(bytes.len());
        let mut digits: Vec<u8> = bytes[..dot].iter().chain(bytes[dot + 1..].iter()).copied().collect();
        let keep = dot + d;
        let up = digits.get(keep).is_some_and(|&c| c >= b'5');
        digits.truncate(keep);
        if up {
            let mut i = digits.len();
            loop {
                if i == 0 { digits.insert(0, b'1'); break; }
                i -= 1;
                if digits[i] == b'9' { digits[i] = b'0'; } else { digits[i] += 1; break; }
            }
        }
        let int_len = digits.len() - d;
        let negative = x < 0.0 && digits.iter().any(|&c| c != b'0');
        let mut text: Vec<u8> = Vec::new();
        if negative { text.push(b'-'); }
        text.extend_from_slice(&digits[..int_len]);
        if d > 0 { text.push(b'.'); text.extend_from_slice(&digits[int_len..]); }
        self.store(&text)
    }
    fn new_array(&mut self, items: &[Value]) -> Result<Value, Error> {
        let cap = items.len().max(4);
        if self.narrays == ARRAYS || self.elems.len() + cap > ELEMS { return Err(Error::Limit); }
        let start = self.elems.len();
        self.elems.extend_from_slice(items);
        self.elems.resize(start + cap, Value::Undef);
        self.arrays[self.narrays] = (start as u16, items.len() as u16, cap as u16);
        self.narrays += 1;
        Ok(Value::Arr((self.narrays - 1) as u8))
    }
    fn push(&mut self, id: u8, x: Value) -> Result<(), Error> {
        let (start, len, cap) = self.arrays[id as usize];
        if len == cap {
            // Grow by moving the array to the end of the pool, doubled.
            let cap2 = (cap as usize * 2).max(4);
            let at = self.elems.len();
            if at + cap2 > ELEMS { return Err(Error::Limit); }
            self.elems.extend_from_within(start as usize..start as usize + len as usize);
            self.elems.resize(at + cap2, Value::Undef);
            self.arrays[id as usize] = (at as u16, len, cap2 as u16);
        }
        let (start, len, cap) = self.arrays[id as usize];
        self.elems[start as usize + len as usize] = x;
        self.arrays[id as usize] = (start, len + 1, cap);
        Ok(())
    }
    fn join(&mut self, id: u8, sep: &[u8]) -> Result<Value, Error> {
        if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
        self.depth += 1;
        let (start, len, _) = self.arrays[id as usize];
        let at = self.arena.len();
        let mut joined: Vec<u8> = Vec::new();
        for k in 0..len as usize {
            if k > 0 { joined.extend_from_slice(sep); }
            // Undefined prints empty; nested arrays print their own join.
            let e = self.elems[start as usize + k];
            if matches!(e, Value::Undef) { continue; }
            let t = self.to_text(e)?;
            joined.extend_from_slice(self.text(t));
            if joined.len() > ARENA { return Err(Error::Limit); }
        }
        // Pieces converted here were scratch; only the joined text is kept.
        self.arena.truncate(at);
        self.depth -= 1;
        self.store(&joined)
    }
    fn math(&mut self) -> Result<Value, Error> {
        self.expect(b'.')?;
        let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        if name == b"PI" { return Ok(Value::Num(core::f64::consts::PI)); }
        self.expect(b'(')?;
        let mut args = [0.0f64; 4];
        let mut n = 0;
        if self.tok != Tok::Punct(b')') {
            loop {
                if n == args.len() { return Err(Error::Limit); }
                let v = self.expr()?;
                args[n] = self.to_number(v);
                n += 1;
                if self.tok != Tok::Punct(b',') { break; }
                self.next()?;
            }
        }
        self.expect(b')')?;
        let a = args[0];
        let one = |f: fn(f64) -> f64| if n == 0 { f64::NAN } else { f(a) };
        Ok(Value::Num(match name {
            b"floor" => one(floor), b"ceil" => one(|x| -floor(-x)), b"round" => one(|x| floor(x + 0.5)),
            b"abs" => one(|x| if x < 0.0 { -x } else { x }),
            b"trunc" => one(|x| if x < 0.0 { -floor(-x) } else { floor(x) }),
            b"sign" => one(|x| if x.is_nan() { x } else if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { x }),
            b"sqrt" => one(sqrt),
            b"pow" => if n < 2 { f64::NAN } else { pow(a, args[1]) },
            b"max" => args[..n].iter().fold(f64::NEG_INFINITY, |m, &x| if x.is_nan() || m.is_nan() { f64::NAN } else if x > m { x } else { m }),
            b"min" => args[..n].iter().fold(f64::INFINITY, |m, &x| if x.is_nan() || m.is_nan() { f64::NAN } else if x < m { x } else { m }),
            _ => return Err(Error::Unsupported),
        }))
    }
    fn atom(&mut self) -> Result<Value, Error> {
        match self.tok {
            Tok::Num(x) => { self.next()?; Ok(Value::Num(x)) }
            Tok::Str(raw) => { self.next()?; self.literal(raw) }
            Tok::Template(raw) => { self.next()?; self.template(raw) }
            Tok::Punct(b'(') => { self.next()?; let v = self.expr()?; self.expect(b')')?; Ok(v) }
            Tok::Punct(b'{') => {
                self.next()?;
                let id = self.new_object()?;
                while self.tok != Tok::Punct(b'}') {
                    let key: &[u8] = match self.tok { Tok::Ident(k) | Tok::Str(k) => k, _ => return Err(Error::Syntax) };
                    self.next()?;
                    self.expect(b':')?;
                    let v = self.expr()?;
                    self.set_prop(id, key, v)?;
                    if self.tok != Tok::Punct(b',') { break; }
                    self.next()?;
                }
                self.expect(b'}')?;
                Ok(Value::Obj(id))
            }
            Tok::Punct(b'[') => {
                self.next()?;
                let mut items = [Value::Undef; 16];
                let mut n = 0;
                if self.tok != Tok::Punct(b']') {
                    loop {
                        if n == items.len() { return Err(Error::Limit); }
                        items[n] = self.expr()?;
                        n += 1;
                        if self.tok != Tok::Punct(b',') { break; }
                        self.next()?;
                    }
                }
                self.expect(b']')?;
                self.new_array(&items[..n])
            }
            Tok::Ident(name) => {
                self.next()?;
                match name {
                    b"NaN" => Ok(Value::Num(f64::NAN)),
                    b"null" => Ok(Value::Null),
                    b"undefined" => Ok(Value::Undef),
                    b"Infinity" => Ok(Value::Num(f64::INFINITY)),
                    b"true" => Ok(Value::Bool(true)),
                    b"false" => Ok(Value::Bool(false)),
                    b"Math" => self.math(),
                    b"JSON" | b"Object" => {
                        self.expect(b'.')?;
                        let Tok::Ident(f) = self.tok else { return Err(Error::Syntax) };
                        self.next()?;
                        self.expect(b'(')?;
                        let v = self.expr()?;
                        self.expect(b')')?;
                        match (name, f, v) {
                            (b"JSON", b"stringify", v) => { let mut j = Vec::new(); self.json(v, &mut j)?; self.store(&j) }
                            (b"Object", b"keys", Value::Obj(id)) => {
                                let (start, len, _) = self.objects[id as usize];
                                let mut keys = [Value::Undef; 16];
                                if len as usize > keys.len() { return Err(Error::Limit); }
                                for (k, slot) in (start as usize..start as usize + len as usize).zip(keys.iter_mut()) { let (ns, nn, _) = self.props[k]; *slot = Value::Str(ns, nn); }
                                self.new_array(&keys[..len as usize])
                            }
                            _ => Err(Error::Unsupported),
                        }
                    }
                    b"String" | b"Number" | b"parseInt" if self.tok == Tok::Punct(b'(') => self.convert(name),
                    _ if self.tok == Tok::Punct(b'(') => self.call(name),
                    _ => self.lookup(name).or(self.skim.then_some(Value::Undef)).ok_or(Error::Unsupported),
                }
            }
            _ => Err(Error::Syntax),
        }
    }
    /// Call a declared function: arguments bind to its parameters for the
    /// call (shadowing, then restored); names it declares end with it.
    fn call(&mut self, name: &[u8]) -> Result<Value, Error> {
        let f = self.funcs[..self.nfuncs].iter().find(|(n, l, _)| &n[..*l as usize] == name).map(|f| f.2);
        if f.is_none() && !self.skim { return Err(Error::Unsupported); }
        self.expect(b'(')?;
        let mut args = [Value::Undef; PARAMS];
        let mut n = 0;
        if self.tok != Tok::Punct(b')') {
            loop {
                if n == PARAMS { return Err(Error::Limit); }
                args[n] = self.expr()?;
                n += 1;
                if self.tok != Tok::Punct(b',') { break; }
                self.next()?;
            }
        }
        self.expect(b')')?;
        let (Some(f), true) = (f, self.live) else { return Ok(Value::Undef) };
        if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
        self.depth += 1;
        let back = self.pos();
        let base = self.nvars;
        let mut shadowed = [None; PARAMS];
        self.restore(f);
        self.expect(b'(')?;
        let mut i = 0;
        while let Tok::Ident(param) = self.tok {
            if i == PARAMS { return Err(Error::Limit); }
            if let Some(slot) = self.vars[..base].iter().position(|(n, l, _)| &n[..*l as usize] == param) { shadowed[i] = Some((slot, self.vars[slot].2)); }
            self.assign(param, args[i], true)?;
            i += 1;
            self.next()?;
            if self.tok != Tok::Punct(b',') { break; }
            self.next()?;
        }
        self.expect(b')')?;
        let outer = self.ret.take();
        let loops = core::mem::replace(&mut self.loops, 0);
        let body = self.statement(true);
        self.loops = loops;
        let result = self.ret.take().unwrap_or(Value::Undef);
        self.ret = outer;
        for (slot, value) in shadowed.iter().flatten() { self.vars[*slot].2 = *value; }
        self.nvars = base;
        self.restore(back);
        self.depth -= 1;
        body.map(|_| result)
    }
    fn lookup(&self, name: &[u8]) -> Option<Value> {
        self.vars[..self.nvars].iter().find(|(n, l, _)| &n[..*l as usize] == name).map(|v| v.2)
    }
    fn assign(&mut self, name: &[u8], v: Value, declare: bool) -> Result<(), Error> {
        if let Some(slot) = self.vars[..self.nvars].iter_mut().find(|(n, l, _)| &n[..*l as usize] == name) {
            slot.2 = v;
            return Ok(());
        }
        if !declare { return Err(Error::Unsupported); }
        if self.nvars == VARS || name.len() > NAME { return Err(Error::Limit); }
        let mut n = [0; NAME];
        n[..name.len()].copy_from_slice(name);
        self.vars[self.nvars] = (n, name.len() as u8, v);
        self.nvars += 1;
        Ok(())
    }
    fn emit(&mut self, v: Value) -> Result<(), Error> {
        let v = self.to_text(v)?;
        let (s, n) = match v { Value::Str(s, n) => (s as usize, n as usize), _ => (0, 0) };
        if self.out.len() + n > MAX_OUTPUT { return Err(Error::Limit); }
        self.out.extend_from_slice(&self.arena[s..s + n]);
        Ok(())
    }
    /// One statement. With `run == false` (an untaken branch) it is parsed
    /// and checked the same way but writes and assigns nothing.
    fn statement(&mut self, run: bool) -> Result<(), Error> {
        if self.depth >= MAX_DEPTH { return Err(Error::Limit); }
        // After `return`, the rest of the function body is checked, not run.
        let run = run && self.ret.is_none() && self.jump.is_none() && !self.skim;
        if run { self.steps += 1; if self.steps > STEPS { return Err(Error::Limit); } }
        self.depth += 1;
        let live = self.live;
        self.live = run;
        let result = self.statement_inner(run);
        self.live = live;
        self.depth -= 1;
        result
    }
    fn statement_inner(&mut self, run: bool) -> Result<(), Error> {
        match self.tok {
            Tok::Punct(b';') => self.next(),
            Tok::Punct(b'{') => {
                self.next()?;
                while self.tok != Tok::Punct(b'}') {
                    if self.tok == Tok::End { return Err(Error::Syntax); }
                    self.statement(run)?;
                }
                self.next()
            }
            Tok::Ident(b"if") => {
                self.next()?;
                self.expect(b'(')?;
                let c = self.expr()?;
                self.expect(b')')?;
                let yes = self.truthy(c);
                self.statement(run && yes)?;
                if self.tok == Tok::Ident(b"else") {
                    self.next()?;
                    self.statement(run && !yes)?;
                }
                Ok(())
            }
            Tok::Ident(b"var" | b"let" | b"const") => {
                self.next()?;
                let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
                if matches!(name, b"var" | b"let" | b"const" | b"document" | b"if" | b"else" | b"while" | b"for" | b"function" | b"return" | b"break" | b"continue" | b"JSON" | b"Object" | b"null" | b"undefined" | b"typeof" | b"Math" | b"true" | b"false") { return Err(Error::Syntax); }
                self.next()?;
                self.expect(b'=')?;
                let v = self.expr()?;
                if run { self.assign(name, v, true)?; }
                else if self.skim && self.lookup(name).is_none() { /* body-local name, checked only */ }
                self.end()
            }
            Tok::Ident(b"document") => {
                self.next()?;
                self.expect(b'.')?;
                let line = match self.tok { Tok::Ident(b"write") => false, Tok::Ident(b"writeln") => true, _ => return Err(Error::Unsupported) };
                self.next()?;
                self.expect(b'(')?;
                if self.tok != Tok::Punct(b')') {
                    loop {
                        let v = self.expr()?;
                        if run { self.emit(v)?; }
                        if self.tok != Tok::Punct(b',') { break; }
                        self.next()?;
                    }
                }
                self.expect(b')')?;
                if line && run { let nl = self.store(b"\n")?; self.emit(nl)?; }
                self.end()
            }
            Tok::Ident(b"function") => {
                self.next()?;
                let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
                self.next()?;
                let at = self.pos();
                self.expect(b'(')?;
                while let Tok::Ident(_) = self.tok { self.next()?; if self.tok != Tok::Punct(b',') { break; } self.next()?; }
                self.expect(b')')?;
                if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
                let skim = self.skim;
                self.skim = true;
                let body = self.statement(false);
                self.skim = skim;
                body?;
                if run {
                    if self.nfuncs == FUNCS || name.len() > NAME { return Err(Error::Limit); }
                    let mut n = [0; NAME];
                    n[..name.len()].copy_from_slice(name);
                    self.funcs[self.nfuncs] = (n, name.len() as u8, at);
                    self.nfuncs += 1;
                }
                Ok(())
            }
            Tok::Ident(b"return") => {
                self.next()?;
                let v = if matches!(self.tok, Tok::Punct(b';' | b'}') | Tok::End) || self.nl { Value::Undef } else { self.expr()? };
                if run { self.ret = Some(v); }
                self.end()
            }
            Tok::Ident(kind @ (b"break" | b"continue")) => {
                if self.loops == 0 { return Err(Error::Syntax); }
                self.next()?;
                if run { self.jump = Some(kind == b"break"); }
                self.end()
            }
            Tok::Ident(b"while") => {
                self.next()?;
                let top = self.pos();
                self.loops += 1;
                loop {
                    self.restore(top);
                    self.expect(b'(')?;
                    let c = self.expr()?;
                    self.expect(b')')?;
                    let go = run && self.ret.is_none() && self.truthy(c);
                    self.statement(go)?;
                    if !go || self.jump.take() == Some(true) { self.loops -= 1; return Ok(()); }
                }
            }
            Tok::Ident(b"for") => {
                self.next()?;
                self.expect(b'(')?;
                if self.tok == Tok::Punct(b';') { self.next()?; } else { self.statement(run)?; }
                let top = self.pos();
                self.loops += 1;
                loop {
                    self.restore(top);
                    let go = if self.tok == Tok::Punct(b';') { true } else { let c = self.expr()?; self.truthy(c) };
                    self.expect(b';')?;
                    let update = self.pos();
                    if self.tok != Tok::Punct(b')') { self.assignment(false)?; }
                    self.expect(b')')?;
                    let go = run && self.ret.is_none() && go;
                    self.statement(go)?;
                    // The body statement was parsed to its end either way.
                    if !go || self.jump.take() == Some(true) { self.loops -= 1; return Ok(()); }
                    self.restore(update);
                    if self.tok != Tok::Punct(b')') { self.assignment(true)?; }
                }
            }
            Tok::Ident(name) if !matches!(name, b"else") => {
                // `f(...)` as a statement calls for its effects.
                let at = self.pos();
                self.next()?;
                let mut call = matches!(self.tok, Tok::Punct(b'(' | b'.'));
                if self.tok == Tok::Punct(b'.') {
                    // `o.key = v` assigns; `o.key(...)` calls.
                    self.next()?;
                    if matches!(self.tok, Tok::Ident(_)) { self.next()?; call = !matches!(self.tok, Tok::Punct(b'=' | b'P' | b'M' | b'I' | b'D')); }
                }
                self.restore(at);
                if call { self.expr()?; } else { self.assignment(run)?; }
                self.end()
            }
            _ => Err(Error::Unsupported),
        }
    }
    /// `name = expr`, `name += expr`, `name -= expr`, `name++`, `name--`.
    fn assignment(&mut self, run: bool) -> Result<(), Error> {
        let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        let Some(old) = self.lookup(name).or(self.skim.then_some(Value::Undef)) else { return Err(Error::Unsupported) };
        if self.tok == Tok::Punct(b'.') {
            self.next()?;
            let Tok::Ident(key) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            let id = match old { Value::Obj(id) => Some(id), _ if !run => None, _ => return Err(Error::Unsupported) };
            let current = id.map_or(Value::Undef, |id| self.get_prop(id, key));
            let v = self.update(current)?;
            if let (true, Some(id)) = (run, id) { self.set_prop(id, key, v)?; }
            return Ok(());
        }
        if self.tok == Tok::Punct(b'[') {
            self.next()?;
            let i = self.expr()?;
            self.expect(b']')?;
            self.expect(b'=')?;
            let v = self.expr()?;
            if run && matches!(old, Value::Obj(_)) {
                let Value::Obj(id) = old else { unreachable!() };
                let k = self.to_text(i)?;
                let key = self.text(k).to_vec();
                self.set_prop(id, &key, v)?;
                return Ok(());
            }
            if run {
                let (Value::Arr(id), i) = (old, self.to_number(i)) else { return Err(Error::Unsupported) };
                let (start, len, _) = self.arrays[id as usize];
                if !(i >= 0.0 && i == floor(i)) { return Err(Error::Unsupported); }
                if (i as u16) < len && i < 65536.0 { self.elems[start as usize + i as usize] = v; }
                else if i as u16 == len { self.push(id, v)?; }
                else { return Err(Error::Unsupported); }
            }
            return Ok(());
        }
        let v = self.update(old)?;
        if run { self.assign(name, v, false)?; }
        Ok(())
    }
    /// The new value for `= e`, `+= e`, `-= e`, `++` or `--` applied to `old`.
    fn update(&mut self, old: Value) -> Result<Value, Error> {
        Ok(match self.tok {
            Tok::Punct(b'=') => { self.next()?; self.expr()? }
            Tok::Punct(op @ (b'P' | b'M')) => { self.next()?; let r = self.expr()?; self.apply(if op == b'P' { b'+' } else { b'-' }, old, r)? }
            Tok::Punct(op @ (b'I' | b'D')) => { self.next()?; Value::Num(self.to_number(old) + if op == b'I' { 1.0 } else { -1.0 }) }
            _ => return Err(Error::Syntax),
        })
    }
    fn pos(&self) -> Pos<'a> { Pos { at: self.at, tok: self.tok, nl: self.nl } }
    fn restore(&mut self, p: Pos<'a>) { self.at = p.at; self.tok = p.tok; self.nl = p.nl; }
    /// A statement ends at `;`, the end of the script, or a new line (ASI).
    fn end(&mut self) -> Result<(), Error> {
        match self.tok {
            Tok::Punct(b';') => self.next(),
            Tok::End | Tok::Punct(b'}') => Ok(()),
            _ if self.nl => Ok(()),
            _ => Err(Error::Syntax),
        }
    }
}

fn rem(a: f64, b: f64) -> f64 {
    if b == 0.0 || a.is_infinite() || a.is_nan() || b.is_nan() { return f64::NAN; }
    if b.is_infinite() { return a; }
    // Truncated remainder like JavaScript `%`, without libm.
    let q = a / b;
    let t = if q >= 0.0 { trunc(q) } else { -trunc(-q) };
    a - t * b
}
/// Correctly rounded square root without libm: an exact integer square
/// root of the scaled significand, rounded once with a sticky bit.
fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 { return f64::NAN; }
    if x == 0.0 || x.is_infinite() { return x; }
    let bits = x.to_bits();
    let (mut m, mut e) = ((bits & ((1 << 52) - 1)) as u128, ((bits >> 52) & 0x7ff) as i32);
    if e == 0 { e = 1; } else { m |= 1 << 52; }
    let mut e = e - 1075; // x = m * 2^e
    if e & 1 != 0 { m <<= 1; e -= 1; }
    let big = m << 74; // < 2^128; its root has at least 63 bits
    // Newton from above (u64::MAX >= the root) decreases to floor(sqrt).
    let mut r: u128 = u64::MAX as u128;
    loop { let n = (r + big / r) >> 1; if n >= r { break; } r = n; }
    let sticky = (r * r != big) as u128;
    let v = ((r | sticky) as u64) as f64; // one rounding, sticky keeps it exact
    let mut scale = (e / 2) - 37;
    let mut out = v;
    while scale > 0 { let k = scale.min(1000); out *= f64::from_bits(((k + 1023) as u64) << 52); scale -= k; }
    while scale < 0 { let k = (-scale).min(1000); out /= f64::from_bits(((k + 1023) as u64) << 52); scale += k; }
    out
}
/// `Math.pow` for whole exponents; others are NaN in this subset.
fn pow(a: f64, e: f64) -> f64 {
    if e.is_nan() || e != floor(e) || e > 1024.0 || e < -1024.0 { return f64::NAN; }
    let mut r = 1.0;
    let (mut b, mut k) = (a, if e < 0.0 { -e } else { e } as u32);
    while k > 0 { if k & 1 == 1 { r *= b; } b *= b; k >>= 1; }
    if e < 0.0 { 1.0 / r } else { r }
}
trait AbsGe { fn abs_ge(self, limit: f64) -> bool; }
impl AbsGe for f64 { fn abs_ge(self, limit: f64) -> bool { self >= limit || self <= -limit } }
fn floor(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() || x >= 4503599627370496.0 || x <= -4503599627370496.0 { return x; }
    let t = if x >= 0.0 { trunc(x) } else { -trunc(-x) };
    if t > x { t - 1.0 } else { t }
}
fn trunc(x: f64) -> f64 {
    if x >= 4503599627370496.0 { return x; }
    (x as u64) as f64
}

struct Buf { b: [u8; 40], n: usize }
impl Buf { fn push(&mut self, s: &[u8]) { for &c in s { if self.n < 40 { self.b[self.n] = c; self.n += 1; } } } }
impl Write for Buf { fn write_str(&mut self, s: &str) -> core::fmt::Result { self.push(s.as_bytes()); Ok(()) } }

/// Run one inline script and return everything it wrote. On any error,
/// including a failed allocation, nothing is returned.
pub fn run(source: &[u8]) -> Result<Vec<u8>, Error> {
    if source.len() > MAX_SOURCE { return Err(Error::Limit); }
    let (mut arena, mut out) = (Vec::new(), Vec::new());
    arena.try_reserve_exact(ARENA).map_err(|_| Error::Limit)?;
    out.try_reserve_exact(MAX_OUTPUT).map_err(|_| Error::Limit)?;
    let mut elems = Vec::new();
    elems.try_reserve_exact(ELEMS).map_err(|_| Error::Limit)?;
    let mut props = Vec::new();
    props.try_reserve_exact(PROPS).map_err(|_| Error::Limit)?;
    let mut m = Machine { src: source, at: 0, tok: Tok::End, nl: false, arena, vars: [([0; NAME], 0, Value::Num(0.0)); VARS], nvars: 0, out, depth: 0, steps: 0,
        funcs: [([0; NAME], 0, Pos { at: 0, tok: Tok::End, nl: false }); FUNCS], nfuncs: 0, skim: false, live: true, ret: None, jump: None, loops: 0,
        elems, arrays: [(0, 0, 0); ARRAYS], narrays: 0, props, objects: [(0, 0, 0); OBJECTS], nobjects: 0 };
    m.next()?;
    while m.tok != Tok::End { m.statement(true)?; }
    Ok(m.out)
}
