// SPDX-License-Identifier: GPL-3.0-only
//! A bounded script subset for inline `<script>` in the Browser reader.
//!
//! This is not a JavaScript engine. It accepts statements without loops or
//! functions: `var|let|const name = expr`, `name = expr`, `if (...) ... else
//! ...`, `{ ... }` blocks and `document.write(...)` / `document.writeln(...)`.
//! Expressions: number, string and `true`/`false` literals, variables,
//! `+ - * / %`, comparisons (`< > <= >= === !== == !=`), `! && ||`, `?:`,
//! parentheses, `.length`, `.toUpperCase()`, `.toLowerCase()` and
//! `Math.floor/ceil/round/abs/min/max`. Numbers are IEEE doubles printed the
//! way JavaScript prints ordinary values; `+` with a string concatenates.
//! Without loops or calls a script always terminates, and it has no names for
//! DOM, timers, storage or network. Any unsupported construct means the whole
//! script writes nothing. Written text is inserted as text, never markup.
use alloc::vec::Vec;
use core::fmt::Write;

pub const MAX_SOURCE: usize = 4096;
pub const MAX_OUTPUT: usize = 1024;
const ARENA: usize = 2048;
// Arena and output live on the heap (fallibly): the Browser stack is tight.
const VARS: usize = 16;
const NAME: usize = 16;
// Each nesting level costs ~4 stack frames on the Browser main task.
const MAX_DEPTH: u8 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Unsupported, Syntax, Limit }

#[derive(Clone, Copy)]
enum Value { Num(f64), Str(u16, u16), Bool(bool) }

#[derive(Clone, Copy, PartialEq)]
enum Tok<'a> { Num(f64), Str(&'a [u8]), Ident(&'a [u8]), Punct(u8), End }


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
}

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
        } else if let Some((op, len)) = [(&b"==="[..], b'E'), (b"!==", b'N'), (b"==", b'e'), (b"!=", b'n'), (b"<=", b'L'), (b">=", b'G'), (b"&&", b'&'), (b"||", b'|')]
            .iter().find(|(t, _)| s[self.at..].starts_with(t)).map(|(t, op)| (*op, t.len())) {
            self.at += len;
            Tok::Punct(op)
        } else if b"+-*/%()=;,.<>!?:{}".contains(&c) {
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
        match v { Value::Num(x) => self.number_text(x), Value::Bool(b) => self.store(if b { b"true" } else { b"false" }), s => Ok(s) }
    }
    fn truthy(&self, v: Value) -> bool {
        match v { Value::Num(x) => x != 0.0 && !x.is_nan(), Value::Bool(b) => b, s => !self.text(s).is_empty() }
    }
    fn to_number(&self, v: Value) -> f64 {
        match v {
            Value::Num(x) => x,
            Value::Bool(b) => b as u8 as f64,
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
        while self.tok == Tok::Punct(b'.') {
            self.next()?;
            let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            let s = self.to_text(v)?;
            v = match name {
                b"length" => Value::Num(self.text(s).len() as f64),
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
        Ok(v)
    }
    fn math(&mut self) -> Result<Value, Error> {
        self.expect(b'.')?;
        let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
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
            b"max" => args[..n].iter().fold(f64::NEG_INFINITY, |m, &x| if x.is_nan() || m.is_nan() { f64::NAN } else if x > m { x } else { m }),
            b"min" => args[..n].iter().fold(f64::INFINITY, |m, &x| if x.is_nan() || m.is_nan() { f64::NAN } else if x < m { x } else { m }),
            _ => return Err(Error::Unsupported),
        }))
    }
    fn atom(&mut self) -> Result<Value, Error> {
        match self.tok {
            Tok::Num(x) => { self.next()?; Ok(Value::Num(x)) }
            Tok::Str(raw) => { self.next()?; self.literal(raw) }
            Tok::Punct(b'(') => { self.next()?; let v = self.expr()?; self.expect(b')')?; Ok(v) }
            Tok::Ident(name) => {
                self.next()?;
                match name {
                    b"NaN" => Ok(Value::Num(f64::NAN)),
                    b"Infinity" => Ok(Value::Num(f64::INFINITY)),
                    b"true" => Ok(Value::Bool(true)),
                    b"false" => Ok(Value::Bool(false)),
                    b"Math" => self.math(),
                    _ => self.lookup(name).ok_or(Error::Unsupported),
                }
            }
            _ => Err(Error::Syntax),
        }
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
        self.depth += 1;
        let result = self.statement_inner(run);
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
                if matches!(name, b"var" | b"let" | b"const" | b"document" | b"if" | b"else" | b"Math" | b"true" | b"false") { return Err(Error::Syntax); }
                self.next()?;
                self.expect(b'=')?;
                let v = self.expr()?;
                if run { self.assign(name, v, true)?; }
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
            Tok::Ident(name) if !matches!(name, b"else") => {
                self.next()?;
                self.expect(b'=')?;
                let v = self.expr()?;
                if run { self.assign(name, v, false)?; }
                else if self.lookup(name).is_none() { return Err(Error::Unsupported); }
                self.end()
            }
            _ => Err(Error::Unsupported),
        }
    }
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
    let mut m = Machine { src: source, at: 0, tok: Tok::End, nl: false, arena, vars: [([0; NAME], 0, Value::Num(0.0)); VARS], nvars: 0, out, depth: 0 };
    m.next()?;
    while m.tok != Tok::End { m.statement(true)?; }
    Ok(m.out)
}
