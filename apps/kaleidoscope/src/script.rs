// SPDX-License-Identifier: GPL-3.0-only
//! A bounded script subset for inline `<script>` in the Browser reader.
//!
//! This is not a JavaScript engine. It accepts:
//! `var|let|const name = expr, other` (no value: `undefined`), `target = expr` where the
//! target is a name or a member chain `a.b[c].d` (also `+= -= *= /= %= ++ --`), `n++`/`++n` in expressions,
//! `if (...) ... else ...`, `while (...)`, `do ... while (...)`,
//! `switch` with `case`/`default`, `for (...; ...; ...)`, `for (x of xs)`,
//! labelled statements with `break label` / `continue label`, `for (k in o)` with
//! `break`/`continue`, `{ ... }`
//! blocks, `function name(a, b = 2, ...rest) { ... return x }` declared before use,
//! closures (a function returned, alone or in an object/array, keeps one shared
//! copy of its creating call's variables), function-scoped `var`,
//! function values (`function (x) {...}`, `x => x * 2`, `(a, b) => {...}`) and
//! array callbacks (`forEach map filter find findIndex some every reduce`,
//! `sort(cmp)`), and
//! `document.write(...)` / `document.writeln(...)`.
//! Expressions: number, string, template (`` `a ${x}` ``) and `true`/`false`
//! literals, variables,
//! `**`, bitwise `& | ^ ~ << >> >>>`, `void`, `**= |= &= ^= ||= &&= ??=`,
//! calling function values (`(function () {...})()`), `+ - * / %`, comparisons (`< > <= >= === !== == !=`), `! && ||`, `?:`,
//! `??`, optional chaining `a?.b` / `a?.[k]` (`&& || ?? ?:` evaluate only the side JavaScript does),
//! parentheses, arrays (`[a, b]`, `x[i]`, `x[i] = v`, `.push .pop .shift .join
//! .indexOf .includes .slice .concat .reverse .sort`),
//! objects (`{a: 1, 'b': x}`, `o.a`, `o[k]`, `o.a = v`, `Object.keys(o)`, `Object.assign(t, ...s)`),
//! regular expressions (`/a(b+)/gi`, `.test`, and `match search replace
//! replaceAll split` with `$1`/`$&` or a callback; see `regex.rs`),
//! `class A extends B { constructor() { super(...) } m() { super.m() } static s() {} }`,
//! `Set`/`Map` (add/set/get/has/delete/clear/size/forEach/keys/values/entries, spread,
//! `for...of`), assignment as an expression (`a = b = 0`, `x => t += x`),
//! `re.exec` with `lastIndex`, `matchAll`, `toPrecision`, `toLocaleString`, `localeCompare`,
//! `Number.MAX_SAFE_INTEGER` etc., `hasOwnProperty`/`Object.hasOwn`, `Object.freeze`,
//! `get`/`set` accessors in classes and object literals (not listed by `Object.keys`),
//! `new`, `instanceof`,
//! `findLast findLastIndex flatMap lastIndexOf splice unshift toString`, `Array.of`,
//! `Object.fromEntries`, `codePointAt`, `encodeURIComponent`/`decodeURIComponent`,
//! `JSON.stringify(v)`, spread `...array` in array literals and calls,
//! destructuring `var [a, , b = 1, ...r] = x` / `var {a, b: c = 1, ...r} = o` (also in `for...of`),
//! object literals with shorthand, methods, `[computed]` keys and `...spread`, `null`/`undefined`, `typeof`, `n.toFixed(d)`,
//! strings (`.length`, `.toUpperCase()`, `.toLowerCase()`, `.trim()`,
//! `.indexOf(s)`, `.charAt(i)`, `.slice(a, b)`, `.substring(a, b)`,
//! `.split(s)`), `String(x)`, `Number(x)`, `Boolean(x)` (also as callbacks: `xs.map(Number)`), `parseInt(s)` and
//! `Math.floor/ceil/round/trunc/sign/abs/min/max/sqrt/pow` and `Math.PI`
//! (no `Math.random`: a script holds no entropy source). Numbers are IEEE doubles printed the
//! way JavaScript prints ordinary values; `+` with a string concatenates.
//! Every executed statement spends one unit of a fixed step budget, so loops
//! always end (an exhausted budget refuses the script); there are no names
//! for DOM, timers, storage or network. Any unsupported construct means the whole
//! script writes nothing. Written text is inserted as text, never markup.
use alloc::vec::Vec;
use crate::regex::{Regex, CAPTURES};
use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const MAX_SOURCE: usize = 16384;
pub const MAX_OUTPUT: usize = 4096;
const ARENA: usize = 12288;
/// Contiguous heap a script needs before it starts (buffers plus slack).
const HEADROOM: usize = 40 * 1024;
/// Statements executed per script, loop bodies included.
const STEPS: u32 = 50_000;
// Arena and output live on the heap (fallibly): the Browser stack is tight.
const VARS: usize = 48;
const FUNCS: usize = 32;
/// Function values that escaped their creating call, and the variable
/// sets (environments) they carry; `Value::Func(FUNCS + k)` is closure k.
const CLOSURES: usize = 64;
const ENVS: usize = 16;
/// `String`, `Number` and `Boolean` used as function values (`xs.map(Number)`).
const BUILTIN_FUNCS: u8 = 250;
/// A Promise executor's `resolve`/`reject` functions: `Value::Func(RESOLVERS + k)`.
const RESOLVERS: u8 = 200;
/// Array elements (all arrays) and array count; arrays are shared by reference.
const ELEMS: usize = 512;
const ARRAYS: usize = 128;
/// Objects and their properties (name in the arena, value), by reference.
const OBJECTS: usize = 64;
const PROPS: usize = 256;
const PARAMS: usize = 6;
const NAME: usize = 16;
// Nesting budget (expressions, statements, calls). Measured natively at 32
// with fact(7): the Browser stack stays below its 46.7 KiB construction peak
// (feature ledger F30).
const MAX_DEPTH: u8 = 64;
/// Lowest stack address a script may reach (0: no floor). The Browser sets
/// it from its mapped stack, so deep nesting is refused (`Limit`) before the
/// stack runs out, whatever the frame sizes; host tests run without one.
static STACK_FLOOR: AtomicUsize = AtomicUsize::new(0);
static FLOOR_HITS: AtomicUsize = AtomicUsize::new(0);
pub fn set_stack_floor(addr: usize) { STACK_FLOOR.store(addr, Ordering::Relaxed); }
/// Scripts refused at the stack floor so far.
pub fn floor_hits() -> usize { FLOOR_HITS.load(Ordering::Relaxed) }
#[inline(always)]
fn stack_low() -> bool {
    let floor = STACK_FLOOR.load(Ordering::Relaxed);
    if floor == 0 { return false; }
    let marker = 0u8;
    let sp = core::hint::black_box(&marker) as *const u8 as usize;
    if sp >= floor { return false; }
    FLOOR_HITS.fetch_add(1, Ordering::Relaxed);
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Unsupported, Syntax, Limit, Thrown }

#[derive(Clone, Copy)]
enum Value { Num(f64), Str(u16, u16), Bool(bool), Undef, Null, Arr(u8), Obj(u8), Func(u8), Re(u8) }

/// What a settled promise does next: call `fulfilled`/`rejected` (absent:
/// pass the state on) and settle `child` with the outcome. `finally`
/// handlers see no value and pass the original outcome on; `slot` feeds a
/// `Promise.all` (gather, index).
#[derive(Clone, Copy)]
struct Reaction { fulfilled: Option<u8>, rejected: Option<u8>, child: Option<u8>, finally: bool, slot: Option<(u8, u16)> }
/// `JSON.stringify` options: the indent unit and a replacer key list.
struct Layout { indent: Vec<u8>, keys: Option<Vec<Vec<u8>>> }
/// An assignment target's last key: `.name` or `[value]`.
#[derive(Clone, Copy)]
enum Key<'a> { Name(&'a [u8]), Value(Value) }
#[derive(Clone, Copy, PartialEq)]
enum Tok<'a> { Num(f64), Str(&'a [u8]), Template(&'a [u8]), Regex(&'a [u8], &'a [u8]), Ident(&'a [u8]), Punct(u8), End }


struct Machine<'a> {
    src: &'a [u8],
    at: usize,
    tok: Tok<'a>,
    /// A newline was skipped before the current token (statement end).
    nl: bool,
    arena: Vec<u8>,
    vars: Vec<([u8; NAME], u8, Value)>,
    /// Per variable: 0 `var`/parameter, 1 `let`, 2 `const` (block scoped).
    kinds: Vec<u8>,
    /// First variable slot of the innermost open block.
    block_base: usize,
    nvars: usize,
    out: Vec<u8>,
    depth: u8,
    steps: u32,
    funcs: Vec<([u8; NAME], u8, Pos<'a>)>,
    nfuncs: usize,
    /// Checking a function body at its declaration: names may be parameters.
    skim: bool,
    /// The current statement runs (calls only happen when live).
    live: bool,
    /// Set by `return` until the enclosing call takes it.
    ret: Option<Value>,
    /// `break` (true) or `continue` (false) pending for the innermost loop,
    /// or for the loop/statement with label `jump_label`.
    jump: Option<bool>,
    jump_label: Option<&'a [u8]>,
    /// The label of the loop about to run, and labels open now.
    label_next: Option<&'a [u8]>,
    labels: u8,
    /// `this` inside the current call, and the receiver for the next call.
    this: Value,
    this_next: Option<Value>,
    /// The value of a `throw` travelling to the nearest `catch`.
    thrown: Option<Value>,
    /// Compiled regular expression literals, by source offset.
    regexes: Vec<(usize, &'a [u8], &'a [u8], Regex)>,
    /// `lastIndex` of each regular expression (for `exec` with `g`).
    re_last: Vec<usize>,
    /// Objects passed to `Object.freeze` (assignments to them are ignored).
    frozen: Vec<u8>,
    /// `class` declarations: name, constructor, methods, parent class, static methods.
    classes: Vec<(&'a [u8], Option<u8>, Vec<(&'a [u8], u8)>, Option<u8>, Vec<(&'a [u8], u8)>, Vec<(&'a [u8], u8, bool)>)>,
    /// `Set`/`Map` objects: (object, keys array, values array; a Set's
    /// values array is its keys array).
    collections: Vec<(u8, u8, u8)>,
    /// Promises: (object, state 0 pending / 1 fulfilled / 2 rejected, value,
    /// reactions waiting for it).
    promises: Vec<(u8, u8, Value, Vec<Reaction>)>,
    /// Settled reactions to run, in order, after the script (microtasks).
    jobs: Vec<(Reaction, u8, Value)>,
    /// `resolve`/`reject` handed to executors: (promise, rejects).
    resolvers: Vec<(u8, bool)>,
    /// `Promise.all`: (result promise, results array, items still pending).
    gathers: Vec<(u8, u8, usize)>,
    /// Accessor properties: (object, name, getter, setter).
    accessors: Vec<(u8, Vec<u8>, Option<u8>, Option<u8>)>,
    /// What made each object, for `instanceof`: 0 a literal, 1+k class k,
    /// 256+f function `f` as a constructor, 64 an Error, 65 a Set, 66 a Map.
    made_by: Vec<u16>,
    /// The function running now (for `super`).
    func_now: Option<u8>,
    /// First variable slot of the running call (its parameters and locals).
    frame_base: usize,
    /// Closures: (function, environment); environments: captured variables.
    closures: Vec<(u8, u8)>,
    envs: Vec<Vec<([u8; NAME], u8, Value)>>,
    /// Loops open in the current function (0 = `break` is a syntax error).
    loops: u8,
    elems: Vec<Value>,
    /// (start, len, cap) of each array inside `elems`.
    arrays: Vec<(u16, u16, u16)>,
    narrays: usize,
    props: Vec<(u16, u16, Value)>,
    /// (start, len, cap) of each object inside `props`.
    objects: Vec<(u16, u16, u16)>,
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
        // `/` starts a regular expression where a value cannot end.
        let after_value = match self.tok { Tok::Num(_) | Tok::Str(_) | Tok::Template(_) | Tok::Regex(..) => true,
            Tok::Ident(k) => !matches!(k, b"return" | b"typeof" | b"case" | b"in" | b"delete" | b"throw" | b"do" | b"else"),
            Tok::Punct(p) => matches!(p, b')' | b']'), Tok::End => false };
        if c == b'/' && !after_value {
            let start = self.at + 1;
            let (mut i, mut class) = (start, false);
            while i < s.len() && (s[i] != b'/' || class) {
                match s[i] { b'\\' => i += 1, b'[' => class = true, b']' => class = false, b'\n' => return Err(Error::Syntax), _ => {} }
                i += 1;
            }
            if i >= s.len() { return Err(Error::Syntax); }
            let mut f = i + 1;
            while f < s.len() && s[f].is_ascii_alphabetic() { f += 1; }
            self.tok = Tok::Regex(&s[start..i], &s[i + 1..f]);
            self.at = f;
            return Ok(());
        }
        self.tok = if c.is_ascii_digit() || (c == b'.' && s.get(self.at + 1).is_some_and(u8::is_ascii_digit)) {
            let start = self.at;
            while self.at < s.len() && (s[self.at].is_ascii_alphanumeric() || s[self.at] == b'.'
                || (matches!(s[self.at], b'+' | b'-') && matches!(s[self.at - 1], b'e' | b'E'))) { self.at += 1; }
            let raw = &s[start..self.at];
            if raw.len() > 2 && (raw.starts_with(b"0x") || raw.starts_with(b"0X")) {
                // Hexadecimal integer literal.
                let mut v = 0f64;
                for &d in &raw[2..] { v = v * 16.0 + (d as char).to_digit(16).ok_or(Error::Syntax)? as f64; }
                Tok::Num(v)
            } else {
                let text = core::str::from_utf8(raw).map_err(|_| Error::Syntax)?;
                Tok::Num(text.parse::<f64>().map_err(|_| Error::Syntax)?)
            }
        } else if c == b'`' {
            let start = self.at + 1;
            let end = template_end(s, start).ok_or(Error::Syntax)?;
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
        } else if s[self.at..].starts_with(b"?.") && !s.get(self.at + 2).is_some_and(u8::is_ascii_digit) {
            self.at += 2;
            Tok::Punct(b'O')
        } else if let Some((op, len)) = [(&b"==="[..], b'E'), (b"!==", b'N'), (b"==", b'e'), (b"!=", b'n'), (b"<=", b'L'), (b">=", b'G'), (b"&&=", b'z'), (b"||=", b'y'), (b"??=", b'q'), (b"&&", b'&'), (b"||", b'|'), (b"??", b'C'),
                (b"**=", b'W'), (b"**", b'X'), (b"|=", b'v'), (b"&=", b'w'), (b"^=", b'x'), (b"<<", b'l'), (b">>>", b'u'), (b">>", b'r'),
                (b"++", b'I'), (b"--", b'D'), (b"+=", b'P'), (b"-=", b'M'), (b"*=", b'T'), (b"/=", b'Q'), (b"%=", b'R'), (b"=>", b'F'), (b"...", b'S')]
            .iter().find(|(t, _)| s[self.at..].starts_with(t)).map(|(t, op)| (*op, t.len())) {
            self.at += len;
            Tok::Punct(op)
        } else if b"+-*/%()=;,.<>!?:{}[]&|^~".contains(&c) {
            self.at += 1;
            // Single `&` `|` are bitwise: codes `a` `o` (`&&` `||` are `&` `|`).
            Tok::Punct(match c { b'&' => b'a', b'|' => b'o', c => c })
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
                let e = *raw.get(i).ok_or(Error::Syntax)?;
                if e == b'u' || e == b'x' {
                    // \xHH and \uHHHH (or \u{H...}) as UTF-8.
                    let (digits, skip) = if e == b'u' && raw.get(i + 1) == Some(&b'{') {
                        let end = raw[i + 2..].iter().position(|&c| c == b'}').ok_or(Error::Syntax)?;
                        (&raw[i + 2..i + 2 + end], end + 3)
                    } else {
                        let n = if e == b'x' { 2 } else { 4 };
                        (raw.get(i + 1..i + 1 + n).ok_or(Error::Syntax)?, n + 1)
                    };
                    let mut code = 0u32;
                    for &d in digits { code = code.checked_mul(16).and_then(|c| c.checked_add((d as char).to_digit(16)?)).ok_or(Error::Syntax)?; }
                    let ch = char::from_u32(code).ok_or(Error::Unsupported)?;
                    let mut buf = [0u8; 4];
                    for &c in ch.encode_utf8(&mut buf).as_bytes() { if self.arena.len() >= ARENA { return Err(Error::Limit); } self.arena.push(c); }
                    i += skip;
                    continue;
                }
                b = match e { b'n' => b'\n', b't' => b'\t', b'r' => b'\r', b'0' => 0, other => other };
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
        match v { Value::Num(x) => self.number_text(x), Value::Bool(b) => self.store(if b { b"true" } else { b"false" }), Value::Undef => self.store(b"undefined"), Value::Null => self.store(b"null"), Value::Arr(id) => self.join(id, b","), Value::Obj(_) => self.store(b"[object Object]"), Value::Func(_) => self.store(b"function"),
            Value::Re(k) => { let (_, p, f, _) = &self.regexes[k as usize]; let t: Vec<u8> = [&b"/"[..], p, b"/", f].concat(); self.store(&t) }, s => Ok(s) }
    }
    fn truthy(&self, v: Value) -> bool {
        match v { Value::Num(x) => x != 0.0 && !x.is_nan(), Value::Bool(b) => b, Value::Undef | Value::Null => false, Value::Arr(_) | Value::Obj(_) | Value::Func(_) | Value::Re(_) => true, s => !self.text(s).is_empty() }
    }
    fn to_number(&self, v: Value) -> f64 {
        match v {
            Value::Num(x) => x,
            Value::Bool(b) => b as u8 as f64,
            Value::Null => 0.0,
            Value::Undef | Value::Arr(_) | Value::Obj(_) | Value::Func(_) | Value::Re(_) => f64::NAN,
            s => {
                let t = self.text(s);
                let t = core::str::from_utf8(t).unwrap_or("x").trim();
                // JavaScript StringToNumber: 0x/0o/0b integers, `Infinity`, and
                // decimals (Rust's own `inf`/`nan` spellings are not numbers here).
                let radix = match t.get(..2) { Some("0x" | "0X") => 16, Some("0o" | "0O") => 8, Some("0b" | "0B") => 2, _ => 0 };
                if t.is_empty() { 0.0 }
                else if radix != 0 { let d = &t[2..]; if d.is_empty() { f64::NAN } else { d.chars().try_fold(0.0f64, |v, c| c.to_digit(radix).map(|k| v * radix as f64 + k as f64)).unwrap_or(f64::NAN) } }
                else if let Some(inf) = t.strip_prefix(['+', '-']).or(Some(t)).filter(|u| *u == "Infinity") { let _ = inf; if t.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY } }
                else if t.bytes().any(|b| b.is_ascii_alphabetic() && !matches!(b, b'e' | b'E')) { f64::NAN }
                else { t.parse::<f64>().unwrap_or(f64::NAN) }
            }
        }
    }

    /// Full expression, including `?:`. Each nesting level (parentheses,
    /// unary operators, branches) costs one unit of the depth budget.
    fn expr(&mut self) -> Result<Value, Error> {
        if self.too_deep() { return Err(Error::Limit); }
        self.depth += 1;
        // An assignment is an expression too (`a = b = 0`, `x => t += x`);
        // its value is the value assigned. (`n++` is handled in `atom`.)
        if let Tok::Ident(_) = self.tok {
            let at = self.pos();
            self.next()?;
            let assigns = self.assigns_ahead_with(false)?;
            self.restore(at);
            if assigns { let v = self.assignment(self.live); self.depth -= 1; return v; }
        }
        let v = self.binary(1);
        self.depth -= 1;
        v
    }
    /// Precedence climbing keeps stack use per nesting level small.
    fn binary(&mut self, min: u8) -> Result<Value, Error> {
        let mut v = self.unary()?;
        loop {
            if self.tok == Tok::Ident(b"in") && min <= 8 {
                self.next()?;
                let r = self.binary(9)?;
                let k = self.to_text(v)?;
                let key = self.text(k).to_vec();
                v = Value::Bool(match r { Value::Obj(id) => self.find_prop(id, &key).is_some(), Value::Arr(id) => core::str::from_utf8(&key).ok().and_then(|t| t.parse::<usize>().ok()).is_some_and(|i| i < self.arrays[id as usize].1 as usize), _ => return Err(Error::Unsupported) });
                continue;
            }
            if self.tok == Tok::Ident(b"instanceof") && min <= 8 {
                self.next()?;
                let Tok::Ident(c) = self.tok else { return Err(Error::Unsupported) };
                self.next()?;
                v = Value::Bool(self.instance_of(v, c)?);
                continue;
            }
            let Tok::Punct(op) = self.tok else { return Ok(v) };
            let prec = match op { b'?' => 1, b'|' | b'C' => 2, b'&' => 3, b'o' => 4, b'^' => 5, b'a' => 6, b'E' | b'N' | b'e' | b'n' => 7,
                b'<' | b'>' | b'L' | b'G' => 8, b'l' | b'r' | b'u' => 9, b'+' | b'-' => 10, b'*' | b'/' | b'%' => 11, b'X' => 12, _ => 0 };
            if prec == 0 || prec < min { return Ok(v); }
            self.next()?;
            // Only the branch JavaScript evaluates runs; the other is checked.
            if op == b'?' {
                let t = self.truthy(v);
                let yes = self.only_if(t, |m| m.expr())?;
                self.expect(b':')?;
                let no = self.only_if(!t, |m| m.binary(1))?;
                v = if t { yes } else { no };
                continue;
            }
            if matches!(op, b'|' | b'&' | b'C') {
                let decided = match op { b'|' => self.truthy(v), b'&' => !self.truthy(v), _ => !matches!(v, Value::Undef | Value::Null) };
                let r = self.only_if(!decided, |m| m.binary(prec + 1))?;
                if !decided { v = r; }
                continue;
            }
            // `**` is right-associative.
            let r = self.binary(if op == b'X' { prec } else { prec + 1 })?;
            v = self.apply(op, v, r)?;
        }
    }
    /// Evaluates `f` for real only when `run` (otherwise it is only checked).
    fn only_if(&mut self, run: bool, f: impl FnOnce(&mut Self) -> Result<Value, Error>) -> Result<Value, Error> {
        let live = self.live;
        self.live = live && run;
        let v = f(self);
        self.live = live;
        v
    }
    fn apply(&mut self, op: u8, v: Value, r: Value) -> Result<Value, Error> {
        Ok(match op {
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
                    (Value::Func(a), Value::Func(b)) => a == b,
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
            b'a' | b'o' | b'^' | b'l' | b'r' => {
                let (a, b) = (int32(self.to_number(v)), int32(self.to_number(r)));
                Value::Num(match op { b'a' => a & b, b'o' => a | b, b'^' => a ^ b, b'l' => a.wrapping_shl(b as u32 & 31), _ => a >> (b as u32 & 31) } as f64)
            }
            b'u' => Value::Num((int32(self.to_number(v)) as u32 >> (int32(self.to_number(r)) as u32 & 31)) as f64),
            b'X' => Value::Num(pow(self.to_number(v), self.to_number(r))),
            _ => {
                let (a, b) = (self.to_number(v), self.to_number(r));
                Value::Num(match op { b'+' => a + b, b'-' => a - b, b'*' => a * b, b'/' => a / b, _ => rem(a, b) })
            }
        })
    }
    fn unary(&mut self) -> Result<Value, Error> {
        if self.tok == Tok::Ident(b"typeof") {
            if self.too_deep() { return Err(Error::Limit); }
            self.next()?;
            self.depth += 1;
            // An undeclared name is "undefined" here, as in JavaScript.
            let v = if let Tok::Ident(name) = self.tok { if self.lookup(name).is_none() && !matches!(name, b"NaN" | b"Infinity" | b"true" | b"false" | b"null" | b"undefined" | b"Math" | b"JSON" | b"Object" | b"String" | b"Number" | b"Boolean" | b"parseInt" | b"encodeURIComponent" | b"decodeURIComponent") && self.funcs[..self.nfuncs].iter().all(|(n, l, _)| &n[..*l as usize] != name) { self.next()?; Value::Undef } else { self.unary()? } } else { self.unary()? };
            self.depth -= 1;
            let kind: &[u8] = match v { Value::Num(_) => b"number", Value::Str(..) => b"string", Value::Bool(_) => b"boolean", Value::Undef => b"undefined", Value::Func(_) => b"function", _ => b"object" };
            return self.store(kind);
        }
        if let (Tok::Punct(b'I' | b'D'), Tok::Ident(name)) = (self.tok, self.peek_after()?) {
            // `++n` / `--n`: the new value.
            let up = self.tok == Tok::Punct(b'I');
            self.next()?;
            self.next()?;
            return self.step_name(name, up, true);
        }
        if self.tok == Tok::Ident(b"void") {
            self.next()?;
            if self.too_deep() { return Err(Error::Limit); }
            self.depth += 1;
            let v = self.unary();
            self.depth -= 1;
            return v.map(|_| Value::Undef);
        }
        if let Tok::Punct(op @ (b'-' | b'+' | b'!' | b'~')) = self.tok {
            if self.too_deep() { return Err(Error::Limit); }
            self.next()?;
            self.depth += 1;
            let v = self.unary()?;
            self.depth -= 1;
            if op == b'!' { return Ok(Value::Bool(!self.truthy(v))); }
            if op == b'~' { return Ok(Value::Num(!int32(self.to_number(v)) as f64)); }
            let x = self.to_number(v);
            return Ok(Value::Num(if op == b'-' { -x } else { x }));
        }
        let v = self.atom()?;
        self.postfix(v)
    }
    /// `.length`, `.toUpperCase()`, `.toLowerCase()` on strings.
    /// A member chain; `?.` on `null`/`undefined` makes the rest of the
    /// chain `undefined` (it is still checked, not run).
    fn postfix(&mut self, v: Value) -> Result<Value, Error> {
        let live = self.live;
        let r = self.chain(v);
        let short = live && !self.live;
        self.live = live;
        r.map(|v| if short { Value::Undef } else { v })
    }
    fn chain(&mut self, mut v: Value) -> Result<Value, Error> {
        loop {
            // Calling a function value: `(function () {...})()`, `fs[0](x)`.
            if let (Value::Func(f), Tok::Punct(b'(')) = (v, self.tok) {
                let mut args = [Value::Undef; PARAMS];
                let n = self.argument_list(&mut args)?;
                v = if self.live { self.invoke(f, &args[..n])? } else { Value::Undef };
                continue;
            }
            let opt = self.tok == Tok::Punct(b'O');
            if opt {
                self.next()?;
                if matches!(v, Value::Undef | Value::Null) { self.live = false; }
                if self.tok == Tok::Punct(b'(') { return Err(Error::Unsupported); }
            }
            if self.tok == Tok::Punct(b'[') {
                self.next()?;
                let i = self.expr()?;
                self.expect(b']')?;
                if !self.live && !matches!(v, Value::Obj(_) | Value::Arr(_) | Value::Str(..)) { v = Value::Undef; continue; }
                v = if let Value::Obj(id) = v { let k = self.to_text(i)?; let key = self.text(k).to_vec(); self.get_prop(id, &key) }
                    else { let i = self.to_number(i); self.index(v, i)? };
                continue;
            }
            if !opt {
                if self.tok != Tok::Punct(b'.') { return Ok(v); }
                self.next()?;
            }
            let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            if !self.live && !matches!(v, Value::Obj(_) | Value::Arr(_) | Value::Str(..)) {
                // Only checked, not run (e.g. `this.x` while a body is checked).
                if self.tok == Tok::Punct(b'(') { let mut args = [Value::Undef; PARAMS]; self.argument_list(&mut args)?; }
                v = Value::Undef;
                continue;
            }
            if let (Value::Obj(id), Value::Num(ms)) = (v, if let Value::Obj(id) = v { self.get_prop(id, b"__date") } else { Value::Undef }) {
                if self.find_prop(id, name).is_none() {
                    let _ = id;
                    self.arguments()?;
                    v = self.date_method(ms, name)?;
                    continue;
                }
            }
            if let (Value::Obj(id), true) = (v, self.live) {
                if matches!(self.made_by[id as usize], 65 | 66) && self.find_prop(id, name).is_none() {
                    v = self.collection_member(id, name)?;
                    continue;
                }
                if self.made_by[id as usize] == 67 && matches!(name, b"then" | b"catch" | b"finally") {
                    v = self.promise_then(id, name)?;
                    continue;
                }
            }
            if let (Value::Obj(id), b"hasOwnProperty") = (v, name) {
                if self.find_prop(id, name).is_none() {
                    let (args, _) = self.arguments()?;
                    let k = self.to_text(args[0])?;
                    let key = self.text(k).to_vec();
                    v = Value::Bool(self.find_prop(id, &key).is_some() || self.accessor(id, &key).is_some());
                    continue;
                }
            }
            if let Value::Obj(id) = v {
                let prop = self.read_prop(id, name)?;
                // `o.f(...)` calls with `this` bound to `o`.
                if let (Value::Func(f), Tok::Punct(b'(')) = (prop, self.tok) {
                    let mut args = [Value::Undef; PARAMS];
                    let n = self.argument_list(&mut args)?;
                    v = if self.live { self.this_next = Some(Value::Obj(id)); self.invoke(f, &args[..n])? } else { Value::Undef };
                } else { v = prop; }
                continue;
            }
            v = self.member(v, name)?;
        }
    }
    /// `.name` / `.name(...)` on numbers, regular expressions, arrays and
    /// strings. Kept out of `chain` (whose frame is on every call path).
    #[inline(never)]
    fn member(&mut self, mut v: Value, name: &'a [u8]) -> Result<Value, Error> {
        if let (Value::Num(x), b"toString") = (v, name) {
            let (args, n) = self.arguments()?;
            let radix = if n == 0 { 10.0 } else { self.to_number(args[0]) };
            v = if radix == 10.0 { self.number_text(x)? }
            else if !(2.0..=36.0).contains(&radix) || radix != floor(radix) || !x.is_finite() || x != floor(x) || x.abs_ge(9.007e15) {
                // Fractions in other bases are not in the subset.
                return Err(Error::Unsupported);
            } else {
                let (mut n, r) = (if x < 0.0 { -x } else { x } as u64, radix as u64);
                let mut digits = Vec::new();
                loop { digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % r) as usize]); n /= r; if n == 0 { break; } }
                if x < 0.0 { digits.push(b'-'); }
                digits.reverse();
                self.store(&digits)?
            };
            return Ok(v);
        }
        if let (Value::Num(x), b"toPrecision" | b"toLocaleString") = (v, name) {
            let (args, n) = self.arguments()?;
            return if name == b"toLocaleString" { self.locale_number(x) } else if n == 0 || matches!(args[0], Value::Undef) { self.number_text(x) } else {
                let p = self.to_number(args[0]);
                if !(1.0..=21.0).contains(&p) || p != floor(p) { return Err(Error::Unsupported); }
                self.precision(x, p as usize)
            };
        }
        if let (Value::Num(x), b"toFixed") = (v, name) {
            self.expect(b'(')?;
            let d = if self.tok == Tok::Punct(b')') { Value::Num(0.0) } else { self.expr()? };
            self.expect(b')')?;
            let d = self.to_number(d);
            if !(0.0..=20.0).contains(&d) || d != floor(d) { return Err(Error::Unsupported); }
            v = self.fixed(x, d as usize)?;
            return Ok(v);
        }
        if let Value::Re(k) = v {
            v = match name {
                b"test" => { let (args, _) = self.arguments()?; let t = self.to_text(args[0])?; let text = self.text(t).to_vec(); Value::Bool(self.re_find(k, &text, 0)?.is_some()) }
                b"source" => { let p = self.regexes[k as usize].1; self.store(p)? }
                b"flags" => { let f = self.regexes[k as usize].2; self.store(f)? }
                b"lastIndex" => Value::Num(self.re_last[k as usize] as f64),
                b"exec" => {
                    // With `g`, starts at `lastIndex` and moves it past the match
                    // (back to 0 when nothing more matches).
                    let (args, _) = self.arguments()?;
                    if !self.live { return Ok(Value::Undef); }
                    let t = self.to_text(args[0])?;
                    let text = self.text(t).to_vec();
                    let global = self.regexes[k as usize].2.contains(&b'g');
                    let from = if global { self.re_last[k as usize] } else { 0 };
                    let found = if from > text.len() { None } else { self.re_find(k, &text, from)? };
                    match found {
                        None => { self.re_last[k as usize] = 0; Value::Null }
                        Some(c) => {
                            if global { self.re_last[k as usize] = c[0].map_or(0, |(_, b)| b); }
                            let groups = self.regexes[k as usize].3.groups;
                            let mut items = Vec::new();
                            for g in 0..=groups { items.push(match c[g] { Some((a, b)) => self.store(&text[a..b])?, None => Value::Undef }); }
                            self.new_array(&items)?
                        }
                    }
                }
                _ => return Err(Error::Unsupported),
            };
            return Ok(v);
        }
        if let (Value::Str(..), b"match" | b"matchAll" | b"search" | b"replace" | b"replaceAll" | b"split") = (v, name) {
            // String methods with a regular expression argument.
            let at = self.pos();
            let (args, n) = self.arguments()?;
            if let Some(Value::Re(k)) = args.first().copied().filter(|_| n >= 1) {
                v = self.string_regex(name, v, k, args.get(1).copied().unwrap_or(Value::Undef))?;
                return Ok(v);
            }
            self.restore(at);
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
                b"forEach" | b"map" | b"filter" | b"find" | b"findIndex" | b"findLast" | b"findLastIndex" | b"flatMap" | b"some" | b"every" | b"reduce" | b"reduceRight" => {
                    let (args, n) = self.arguments()?;
                    self.array_callback(id, name, &args[..n])?
                }
                b"indexOf" | b"lastIndexOf" | b"includes" | b"slice" | b"concat" | b"reverse" | b"sort" | b"pop" | b"shift" | b"fill" | b"at" | b"flat"
                | b"toString" | b"splice" | b"unshift" | b"keys" | b"values" | b"entries" => {
                    let (args, n) = self.arguments()?;
                    self.array_method(id, name, &args[..n])?
                }
                _ => return Err(Error::Unsupported),
            };
            return Ok(v);
        }
        let s = self.to_text(v)?;
        v = match name {
            b"length" => Value::Num(self.text(s).len() as f64),
            b"trim" | b"indexOf" | b"charAt" | b"slice" | b"substring" | b"split" | b"replace" | b"replaceAll"
            | b"startsWith" | b"endsWith" | b"includes" | b"repeat" | b"padStart" | b"padEnd" | b"lastIndexOf" | b"charCodeAt" | b"codePointAt" | b"at" | b"trimStart" | b"trimEnd" | b"localeCompare" => {
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
        Ok(v)
    }
    fn re_find(&self, k: u8, text: &[u8], from: usize) -> Result<Option<[Option<(usize, usize)>; CAPTURES]>, Error> {
        self.regexes[k as usize].3.find(text, from).map_err(|_| Error::Limit)
    }
    /// `match`, `search`, `replace`, `replaceAll`, `split` with a regex.
    fn string_regex(&mut self, name: &[u8], s: Value, k: u8, arg: Value) -> Result<Value, Error> {
        let text = self.text(s).to_vec();
        let global = self.regexes[k as usize].2.contains(&b'g');
        let groups = self.regexes[k as usize].3.groups;
        // All matches (global) or the first, advancing past empty matches.
        let mut found: Vec<[Option<(usize, usize)>; CAPTURES]> = Vec::new();
        let mut from = 0;
        while from <= text.len() {
            let Some(c) = self.re_find(k, &text, from)? else { break };
            let (a, b) = c[0].ok_or(Error::Limit)?;
            found.push(c);
            if !(global || name == b"split") || found.len() > ELEMS { break; }
            from = if b == a { b + 1 } else { b };
        }
        let piece = |m: &mut Self, span: Option<(usize, usize)>| -> Result<Value, Error> { match span { Some((a, b)) => m.store(&text[a..b]), None => Ok(Value::Undef) } };
        Ok(match name {
            b"search" => Value::Num(found.first().and_then(|c| c[0]).map_or(-1.0, |(a, _)| a as f64)),
            b"matchAll" => {
                // Every match with its groups (an array, not an iterator).
                if !global { return Err(Error::Unsupported); }
                let mut all: Vec<Value> = Vec::new();
                for c in &found {
                    let mut items: Vec<Value> = Vec::new();
                    for g in 0..=groups { items.push(piece(self, c[g])?); }
                    all.push(self.new_array(&items)?);
                }
                self.new_array(&all)?
            }
            b"match" => {
                if found.is_empty() { return Ok(Value::Null); }
                let mut items: Vec<Value> = Vec::new();
                if global { for c in &found { items.push(piece(self, c[0])?); } }
                else { for g in 0..=groups { items.push(piece(self, found[0][g])?); } }
                self.new_array(&items)?
            }
            b"split" => {
                let mut items: Vec<Value> = Vec::new();
                let mut last = 0;
                for c in &found {
                    let (a, b) = c[0].unwrap();
                    if b == a && (a == 0 || a == text.len()) { continue; }
                    items.push(self.store(&text[last..a])?);
                    for g in 1..=groups { items.push(piece(self, c[g])?); }
                    last = b;
                    if items.len() > ELEMS { return Err(Error::Limit); }
                }
                items.push(self.store(&text[last..])?);
                if !matches!(arg, Value::Undef) { items.truncate(self.to_number(arg) as u32 as usize); }
                self.new_array(&items)?
            }
            _ => {
                if name == b"replaceAll" && !global { return Err(Error::Unsupported); }
                let mut out: Vec<u8> = Vec::new();
                let mut last = 0;
                for c in &found {
                    let (a, b) = c[0].unwrap();
                    out.extend_from_slice(&text[last..a]);
                    match arg {
                        Value::Func(f) => {
                            let mut call = [Value::Undef; 4];
                            for g in 0..=groups.min(3) { call[g] = piece(self, c[g])?; }
                            let r = self.invoke(f, &call[..=groups.min(3)])?;
                            let t = self.to_text(r)?;
                            out.extend_from_slice(self.text(t));
                        }
                        _ => {
                            // `$&`, `$1`..`$9` and `$$` in the replacement text.
                            let t = self.to_text(arg)?;
                            let rep = self.text(t).to_vec();
                            let mut i = 0;
                            while i < rep.len() {
                                if rep[i] == b'$' && i + 1 < rep.len() {
                                    let d = rep[i + 1];
                                    if d == b'$' { out.push(b'$'); i += 2; continue; }
                                    if d == b'&' { out.extend_from_slice(&text[a..b]); i += 2; continue; }
                                    if d.is_ascii_digit() && d != b'0' && ((d - b'0') as usize) <= groups {
                                        if let Some((x, y)) = c[(d - b'0') as usize] { out.extend_from_slice(&text[x..y]); }
                                        i += 2; continue;
                                    }
                                }
                                out.push(rep[i]);
                                i += 1;
                            }
                        }
                    }
                    last = b;
                    if out.len() > ARENA { return Err(Error::Limit); }
                }
                out.extend_from_slice(&text[last..]);
                self.store(&out)?
            }
        })
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
            b"at" => { let i = self.to_number(args[0]); let i = if i < 0.0 { n as f64 + floor(i) } else { floor(i) }; if i >= 0.0 && (i as usize) < n { Value::Str((start + i as usize) as u16, 1) } else { Value::Undef } }
            b"trimStart" | b"trimEnd" => {
                let t = &self.arena[start..start + n];
                let (a, b) = if name == b"trimStart" { (t.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(n), n) } else { (0, t.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(0, |p| p + 1)) };
                Value::Str((start + a) as u16, (b - a) as u16)
            }
            b"trim" => {
                let t = &self.arena[start..start + n];
                let a = t.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(n);
                let b = t.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(a, |p| p + 1);
                Value::Str((start + a) as u16, (b - a) as u16)
            }
            b"localeCompare" => {
                // ASCII letters case-insensitively first, then lower before
                // upper (close to the default collation for plain text).
                let other = self.to_text(args[0])?;
                let key = |t: &[u8]| t.iter().map(|b| (b.to_ascii_lowercase(), b.is_ascii_uppercase())).collect::<Vec<_>>();
                let (a, b) = (key(&self.arena[start..start + n]), key(self.text(other)));
                Value::Num(match a.cmp(&b) { core::cmp::Ordering::Less => -1.0, core::cmp::Ordering::Equal => 0.0, _ => 1.0 })
            }
            b"startsWith" | b"endsWith" | b"includes" | b"lastIndexOf" => {
                let needle = self.to_text(args[0])?;
                let (h, k) = (self.text(s), self.text(needle));
                match name {
                    b"startsWith" => Value::Bool(h.starts_with(k)),
                    b"endsWith" => Value::Bool(h.ends_with(k)),
                    b"includes" => Value::Bool(k.is_empty() || h.windows(k.len()).any(|w| w == k)),
                    _ => Value::Num(if k.is_empty() { h.len() as f64 } else { h.windows(k.len()).rposition(|w| w == k).map_or(-1.0, |p| p as f64) }),
                }
            }
            b"replace" | b"replaceAll" => {
                // The replacement is text, or a function called with
                // (match, offset, whole string) for each match.
                let call = if let Value::Func(g) = args[1] { Some(g) } else { None };
                let from = self.to_text(args[0])?;
                let to = if call.is_some() { self.store(b"")? } else { self.to_text(args[1])? };
                let (h, f, t) = (self.text(s).to_vec(), self.text(from).to_vec(), self.text(to).to_vec());
                let mut out: Vec<u8> = Vec::new();
                let mut i = 0;
                let mut done = false;
                while i < h.len() {
                    if !f.is_empty() && !done && h[i..].starts_with(&f) {
                        match call {
                            Some(g) if self.live => { let r = self.invoke(g, &[from, Value::Num(i as f64), s])?; let r = self.to_text(r)?; out.extend_from_slice(self.text(r)); }
                            Some(_) => {}
                            None => out.extend_from_slice(&t),
                        }
                        i += f.len();
                        done = name == b"replace";
                    } else { out.push(h[i]); i += 1; }
                    if out.len() > ARENA { return Err(Error::Limit); }
                }
                if f.is_empty() { out = t.iter().chain(h.iter()).copied().collect(); }
                self.store(&out)?
            }
            b"repeat" => {
                let k = self.to_number(args[0]);
                if !(0.0..=1024.0).contains(&k) || k != floor(k) { return Err(Error::Unsupported); }
                let piece = self.text(s).to_vec();
                if piece.len() * k as usize > ARENA { return Err(Error::Limit); }
                let out = piece.repeat(k as usize);
                self.store(&out)?
            }
            b"padStart" | b"padEnd" => {
                let want = self.to_number(args[0]);
                let fill = if matches!(args[1], Value::Undef) { b" ".to_vec() } else { let f = self.to_text(args[1])?; self.text(f).to_vec() };
                let h = self.text(s).to_vec();
                if !(want <= 1024.0) || fill.is_empty() || want <= h.len() as f64 { return Ok(s); }
                let mut pad = Vec::new();
                while pad.len() < want as usize - h.len() { pad.push(fill[pad.len() % fill.len()]); }
                let out: Vec<u8> = if name == b"padStart" { pad.iter().chain(h.iter()).copied().collect() } else { h.iter().chain(pad.iter()).copied().collect() };
                self.store(&out)?
            }
            b"indexOf" => {
                let needle = self.to_text(args[0])?;
                let (h, k) = (self.text(s), self.text(needle));
                Value::Num(if k.is_empty() { 0.0 } else { h.windows(k.len()).position(|w| w == k).map_or(-1.0, |p| p as f64) })
            }
            b"charCodeAt" => { let i = pos(self, args[0], 0, false); if i < n { Value::Num(self.arena[start + i] as f64) } else { Value::Num(f64::NAN) } }
            b"codePointAt" => {
                // The UTF-8 scalar starting at byte `i` (a continuation byte or
                // a bad sequence gives its byte value, like charCodeAt).
                let i = pos(self, args[0], 0, false);
                if i >= n { Value::Undef } else {
                    let t = &self.arena[start + i..start + n];
                    let w = match t[0] { 0xc0..=0xdf => 2, 0xe0..=0xef => 3, 0xf0..=0xf7 => 4, _ => 1 };
                    Value::Num(t.get(..w).and_then(|c| core::str::from_utf8(c).ok()).and_then(|c| c.chars().next()).map_or(t[0] as u32, |c| c as u32) as f64)
                }
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
                let mut items: Vec<Value> = Vec::new();
                let mut count = 0;
                let mut put = |v: Value, c: &mut usize| -> Result<(), Error> { if *c == ELEMS { return Err(Error::Limit); } items.push(v); *c += 1; Ok(()) };
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
                let limit = if matches!(args[1], Value::Undef) { count } else { self.to_number(args[1]) as u32 as usize };
                self.new_array(&items[..count.min(limit)])?
            }
        })
    }
    /// `String(x)`, `Number(x)`, `parseInt(s)`, `encodeURIComponent(s)`...
    fn convert(&mut self, name: &[u8]) -> Result<Value, Error> {
        self.expect(b'(')?;
        let v = if self.tok == Tok::Punct(b')') { Value::Undef } else { self.expr()? };
        self.expect(b')')?;
        Ok(match name {
            b"String" => self.to_text(v)?,
            b"isNaN" => Value::Bool(self.to_number(v).is_nan()),
            b"Boolean" => Value::Bool(self.truthy(v)),
            b"encodeURIComponent" => {
                // UTF-8 bytes as %XX except JavaScript's unreserved marks.
                let t = self.to_text(v)?;
                let mut out = Vec::new();
                for &b in self.text(t) {
                    if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) { out.push(b); } else { out.extend_from_slice(&[b'%', b"0123456789ABCDEF"[(b >> 4) as usize], b"0123456789ABCDEF"[(b & 15) as usize]]); }
                }
                self.store(&out)?
            }
            b"decodeURIComponent" => {
                // A malformed escape or non-UTF-8 result is a URIError: refused.
                let t = self.to_text(v)?;
                let (t, mut out, mut i) = (self.text(t).to_vec(), Vec::new(), 0);
                while i < t.len() {
                    if t[i] == b'%' {
                        let hex = |c: Option<&u8>| c.and_then(|c| (*c as char).to_digit(16));
                        let (Some(h), Some(l)) = (hex(t.get(i + 1)), hex(t.get(i + 2))) else { return Err(Error::Unsupported) };
                        out.push((h * 16 + l) as u8); i += 3;
                    } else { out.push(t[i]); i += 1; }
                }
                if core::str::from_utf8(&out).is_err() { return Err(Error::Unsupported); }
                self.store(&out)?
            }
            b"parseFloat" => {
                // The longest leading decimal number, like JavaScript.
                let t = self.to_text(v)?;
                let t = self.text(t);
                let t = &t[t.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(t.len())..];
                let mut best = f64::NAN;
                for end in 1..=t.len().min(64) {
                    if let Some(x) = core::str::from_utf8(&t[..end]).ok().filter(|s| !s.ends_with(['e', 'E', '+', '-', '.']) || *s == "." ).and_then(|s| s.parse::<f64>().ok()) { if s_is_plain(&t[..end]) { best = x; } }
                }
                Value::Num(best)
            }
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
    /// The getter and setter of an accessor property `key` of object `id`.
    fn accessor(&self, id: u8, key: &[u8]) -> Option<(Option<u8>, Option<u8>)> {
        if self.accessors.is_empty() || self.find_prop(id, key).is_some() { return None; }
        self.accessors.iter().find(|a| a.0 == id && a.1 == key).map(|a| (a.2, a.3))
    }
    fn add_accessor(&mut self, id: u8, key: &[u8], f: u8, setter: bool) -> Result<(), Error> {
        let k = match self.accessors.iter().position(|a| a.0 == id && a.1 == key) {
            Some(k) => k,
            None => { if self.accessors.len() == 64 { return Err(Error::Limit); } self.accessors.push((id, key.to_vec(), None, None)); self.accessors.len() - 1 }
        };
        if setter { self.accessors[k].3 = Some(f); } else { self.accessors[k].2 = Some(f); }
        Ok(())
    }
    /// Reads `o.key` running a getter with `this` = `o`, if one is defined.
    fn read_prop(&mut self, id: u8, key: &[u8]) -> Result<Value, Error> {
        match self.accessor(id, key) {
            Some((Some(g), _)) if self.live => { self.this_next = Some(Value::Obj(id)); self.invoke(g, &[]) }
            Some(_) => Ok(Value::Undef),
            None => Ok(self.get_prop(id, key)),
        }
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
        self.made_by[self.nobjects] = 0;
        self.nobjects += 1;
        Ok((self.nobjects - 1) as u8)
    }
    /// `new C(args)`: built-ins, a `class`, or a function used as a constructor.
    fn construct(&mut self, class: &[u8], args: &[Value]) -> Result<Value, Error> {
        match class {
            b"Array" => {
                if let [Value::Num(n)] = args { if !(0.0..=ELEMS as f64).contains(n) || *n != floor(*n) { return Err(Error::Limit); } let items: Vec<Value> = (0..*n as usize).map(|_| Value::Undef).collect(); return self.new_array(&items); }
                return self.new_array(args);
            }
            b"Object" => return Ok(Value::Obj(self.new_object()?)),
            b"Error" | b"TypeError" | b"RangeError" | b"SyntaxError" => {
                let id = self.new_object()?;
                self.made_by[id as usize] = 64;
                let name = self.store(class)?;
                self.set_prop(id, b"name", name)?;
                let message = match args.first() { Some(&m) => self.to_text(m)?, None => self.store(b"")? };
                self.set_prop(id, b"message", message)?;
                return Ok(Value::Obj(id));
            }
            b"Date" => return self.date_new(args),
            b"Promise" => {
                // The executor runs now with resolve/reject; a throw rejects.
                let p = self.promise_new()?;
                let Some(&Value::Func(exec)) = args.first() else { return Err(Error::Unsupported) };
                let (res, rej) = (self.resolver(p, false)?, self.resolver(p, true)?);
                self.call_settling(exec, &[res, rej], |m, outcome| match outcome { Ok(_) => Ok(()), Err(e) => m.settle(p, 2, e) })?;
                return Ok(Value::Obj(p));
            }
            b"Set" | b"Map" => return self.collection_new(class == b"Map", args.first().copied()),
            _ => {}
        }
        let id = self.new_object()?;
        if let Some(k) = self.classes.iter().position(|c| c.0 == class) {
            self.made_by[id as usize] = 1 + k as u16;
            self.class_methods(k, id)?;
            let r = self.class_init(k, Value::Obj(id), args)?;
            return Ok(if matches!(r, Value::Obj(_)) { r } else { Value::Obj(id) });
        }
        let f = self.funcs[..self.nfuncs].iter().position(|(n, l, _)| *l != 0 && &n[..*l as usize] == class)
            .or_else(|| match self.lookup(class) { Some(Value::Func(i)) => Some(i as usize), _ => None }).ok_or(Error::Unsupported)?;
        self.made_by[id as usize] = 256 + self.real_func(f as u8) as u16;
        self.this_next = Some(Value::Obj(id));
        let r = self.invoke(f as u8, args)?;
        Ok(if matches!(r, Value::Obj(_)) { r } else { Value::Obj(id) })
    }
    /// `++`/`--` on a variable in an expression: the new value (prefix) or
    /// the old one (postfix), as a number.
    fn step_name(&mut self, name: &[u8], up: bool, prefix: bool) -> Result<Value, Error> {
        let old = self.lookup(name).or((self.skim || !self.live).then_some(Value::Undef)).ok_or(Error::Unsupported)?;
        let x = self.to_number(old);
        let new = if up { x + 1.0 } else { x - 1.0 };
        if self.live { self.assign(name, Value::Num(new), false)?; }
        Ok(Value::Num(if prefix { new } else { x }))
    }
    /// `v instanceof C` for a class (or a parent), a constructor function,
    /// `Array`, `Object` and the Error types.
    fn instance_of(&mut self, v: Value, c: &[u8]) -> Result<bool, Error> {
        if !self.live { return Ok(false); }
        let made = if let Value::Obj(id) = v { self.made_by[id as usize] } else { 0 };
        Ok(match c {
            b"Array" => matches!(v, Value::Arr(_)),
            b"Object" => matches!(v, Value::Obj(_) | Value::Arr(_) | Value::Func(_)),
            b"Function" => matches!(v, Value::Func(_)),
            b"Error" => made == 64,
            b"Set" => made == 65,
            b"Promise" => made == 67,
            b"Map" => made == 66,
            b"TypeError" | b"RangeError" | b"SyntaxError" => made == 64 && { let Value::Obj(id) = v else { return Ok(false) }; let n = self.get_prop(id, b"name"); matches!(n, Value::Str(..)) && self.text(n) == c },
            _ => if let Some(k) = self.classes.iter().position(|x| x.0 == c) {
                let mut at = (1..=16).contains(&made).then(|| made as usize - 1);
                while let Some(m) = at { if m == k { return Ok(true); } at = self.classes[m].3.map(|p| p as usize); }
                false
            } else if let Some(f) = self.funcs[..self.nfuncs].iter().position(|(n, l, _)| *l != 0 && &n[..*l as usize] == c).or_else(|| match self.lookup(c) { Some(Value::Func(i)) => Some(i as usize), _ => None }) {
                made == 256 + self.real_func(f as u8) as u16
            } else { return Err(Error::Unsupported) },
        })
    }
    /// Methods of class `k` and its ancestors (nearest wins) onto `id`.
    fn class_methods(&mut self, k: usize, id: u8) -> Result<(), Error> {
        if let Some(p) = self.classes[k].3 { self.class_methods(p as usize, id)?; }
        for (name, f) in self.classes[k].2.clone() { self.set_prop(id, name, Value::Func(f))?; }
        for (name, f, setter) in self.classes[k].5.clone() { self.add_accessor(id, name, f, setter)?; }
        Ok(())
    }
    /// Runs the constructor of class `k` on `this`; a class without one
    /// passes its arguments to its parent's, as JavaScript does.
    fn class_init(&mut self, k: usize, this: Value, args: &[Value]) -> Result<Value, Error> {
        match (self.classes[k].1, self.classes[k].3) {
            (Some(f), _) => { self.this_next = Some(this); self.invoke(f, args) }
            (None, Some(p)) => self.class_init(p as usize, this, args),
            (None, None) => Ok(Value::Undef),
        }
    }
    /// `super(...)` and `super.m(...)` inside a class's constructor or method.
    fn super_call(&mut self) -> Result<Value, Error> {
        let parent = self.func_now.and_then(|f| self.classes.iter().position(|c| c.1 == Some(f) || c.2.iter().any(|m| m.1 == f))).and_then(|k| self.classes[k].3);
        let method = if self.tok == Tok::Punct(b'.') {
            self.next()?;
            let Tok::Ident(m) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            Some(m)
        } else { None };
        let mut args = [Value::Undef; PARAMS];
        let n = self.argument_list(&mut args)?;
        if !self.live { return Ok(Value::Undef); }
        let mut k = parent.ok_or(Error::Syntax)? as usize;
        let this = self.this;
        let Some(m) = method else { self.class_init(k, this, &args[..n])?; return Ok(Value::Undef) };
        loop {
            if let Some(&(_, f)) = self.classes[k].2.iter().find(|c| c.0 == m) { self.this_next = Some(this); return self.invoke(f, &args[..n]); }
            k = self.classes[k].3.ok_or(Error::Unsupported)? as usize;
        }
    }
    /// At `(` of `Array.from(...)`: is there a second (map function) argument?
    fn from_with_map(&mut self) -> Result<bool, Error> {
        let at = self.pos();
        self.expect(b'(')?;
        let mut depth = 0;
        let mut second = false;
        loop {
            match self.tok {
                Tok::Punct(b'(' | b'[' | b'{') => depth += 1,
                Tok::Punct(b')' | b']' | b'}') if depth == 0 => break,
                Tok::Punct(b')' | b']' | b'}') => depth -= 1,
                Tok::Punct(b',') if depth == 0 => { second = true; break; }
                Tok::End => break,
                _ => {}
            }
            self.next()?;
        }
        self.restore(at);
        Ok(second)
    }
    fn promise_new(&mut self) -> Result<u8, Error> {
        if self.promises.len() == 64 { return Err(Error::Limit); }
        let id = self.new_object()?;
        self.made_by[id as usize] = 67;
        self.promises.push((id, 0, Value::Undef, Vec::new()));
        Ok(id)
    }
    fn resolver(&mut self, p: u8, reject: bool) -> Result<Value, Error> {
        if self.resolvers.len() == (BUILTIN_FUNCS - RESOLVERS) as usize { return Err(Error::Limit); }
        self.resolvers.push((p, reject));
        Ok(Value::Func(RESOLVERS + (self.resolvers.len() - 1) as u8))
    }
    fn promise_index(&self, p: u8) -> Result<usize, Error> { self.promises.iter().position(|e| e.0 == p).ok_or(Error::Unsupported) }
    /// Resolving with another promise follows it; anything else fulfils.
    fn resolve(&mut self, p: u8, v: Value) -> Result<(), Error> {
        if let Value::Obj(q) = v {
            if self.made_by[q as usize] == 67 {
                if q == p { return self.settle(p, 2, Value::Undef); }
                return self.react(q, Reaction { fulfilled: None, rejected: None, child: Some(p), finally: false, slot: None });
            }
        }
        self.settle(p, 1, v)
    }
    /// The first settlement wins; reactions waiting on it become jobs.
    fn settle(&mut self, p: u8, state: u8, v: Value) -> Result<(), Error> {
        let k = self.promise_index(p)?;
        if self.promises[k].1 != 0 { return Ok(()); }
        self.promises[k].1 = state;
        self.promises[k].2 = v;
        let waiting = core::mem::take(&mut self.promises[k].3);
        for r in waiting { self.queue(r, state, v)?; }
        Ok(())
    }
    fn queue(&mut self, r: Reaction, state: u8, v: Value) -> Result<(), Error> {
        if self.jobs.len() >= 256 { return Err(Error::Limit); }
        self.jobs.push((r, state, v));
        Ok(())
    }
    /// Runs `r` when `p` settles (now, as a job, if it already has).
    fn react(&mut self, p: u8, r: Reaction) -> Result<(), Error> {
        let k = self.promise_index(p)?;
        match self.promises[k].1 { 0 => { self.promises[k].3.push(r); Ok(()) } state => { let v = self.promises[k].2; self.queue(r, state, v) } }
    }
    /// `p.then(f, g)`, `p.catch(g)`, `p.finally(h)`: a new promise.
    #[inline(never)]
    fn promise_then(&mut self, p: u8, name: &[u8]) -> Result<Value, Error> {
        let (args, n) = self.arguments()?;
        if !self.live { return Ok(Value::Undef); }
        let func = |i: usize| match args.get(i).copied().filter(|_| i < n) { Some(Value::Func(f)) => Some(f), _ => None };
        let child = self.promise_new()?;
        let r = match name {
            b"then" => Reaction { fulfilled: func(0), rejected: func(1), child: Some(child), finally: false, slot: None },
            b"catch" => Reaction { fulfilled: None, rejected: func(0), child: Some(child), finally: false, slot: None },
            _ => Reaction { fulfilled: func(0), rejected: func(0), child: Some(child), finally: true, slot: None },
        };
        self.react(p, r)?;
        Ok(Value::Obj(child))
    }
    /// Calls `f`, turning a throw into `Err(value)` for `done` (other errors
    /// still refuse the script).
    fn call_settling(&mut self, f: u8, args: &[Value], done: impl FnOnce(&mut Self, Result<Value, Value>) -> Result<(), Error>) -> Result<(), Error> {
        let saved = (self.depth, self.loops, self.live, self.nvars, self.block_base);
        match self.invoke(f, args) {
            Ok(v) => done(self, Ok(v)),
            Err(Error::Thrown) => {
                (self.depth, self.loops, self.live, self.nvars, self.block_base) = saved;
                (self.jump, self.jump_label, self.ret) = (None, None, None);
                let e = self.thrown.take().unwrap_or(Value::Undef);
                done(self, Err(e))
            }
            Err(e) => Err(e),
        }
    }
    /// One microtask: a reaction to a settled promise.
    fn run_job(&mut self, r: Reaction, state: u8, v: Value) -> Result<(), Error> {
        self.steps += 1;
        if self.steps > STEPS { return Err(Error::Limit); }
        if let Some((g, i)) = r.slot {
            let (p, results, _) = self.gathers[g as usize];
            if state == 2 { return self.settle(p, 2, v); }
            let start = self.arrays[results as usize].0 as usize;
            self.elems[start + i as usize] = v;
            self.gathers[g as usize].2 -= 1;
            if self.gathers[g as usize].2 == 0 { self.settle(p, 1, Value::Arr(results))?; }
            return Ok(());
        }
        let handler = if state == 1 { r.fulfilled } else { r.rejected };
        let child = r.child;
        match handler {
            None => match child { Some(c) => if state == 1 { self.resolve(c, v) } else { self.settle(c, 2, v) }, None => Ok(()) },
            Some(f) => {
                let args: &[Value] = if r.finally { &[] } else { core::slice::from_ref(&v) };
                let args = args.to_vec();
                self.call_settling(f, &args, |m, outcome| match (child, outcome) {
                    (None, _) => Ok(()),
                    (Some(c), Err(e)) => m.settle(c, 2, e),
                    (Some(c), Ok(_)) if r.finally => if state == 1 { m.resolve(c, v) } else { m.settle(c, 2, v) },
                    (Some(c), Ok(out)) => m.resolve(c, out),
                })
            }
        }
    }
    /// After the script: run promise reactions in order until none remain.
    fn drain_jobs(&mut self) -> Result<(), Error> {
        let mut i = 0;
        while i < self.jobs.len() {
            let (r, state, v) = self.jobs[i];
            self.run_job(r, state, v)?;
            i += 1;
        }
        Ok(())
    }
    /// `new Set(iterable)` / `new Map([[k, v], ...])`.
    fn collection_new(&mut self, map: bool, from: Option<Value>) -> Result<Value, Error> {
        let id = self.new_object()?;
        self.made_by[id as usize] = if map { 66 } else { 65 };
        let Value::Arr(keys) = self.new_array(&[])? else { return Err(Error::Limit) };
        let vals = if map { let Value::Arr(v) = self.new_array(&[])? else { return Err(Error::Limit) }; v } else { keys };
        if self.collections.len() == 32 { return Err(Error::Limit); }
        self.collections.push((id, keys, vals));
        if let Some(src) = from.filter(|v| !matches!(v, Value::Undef | Value::Null)) {
            for item in self.iterate(src)? {
                if map {
                    let Value::Arr(pair) = item else { return Err(Error::Unsupported) };
                    let (ps, pl, _) = self.arrays[pair as usize];
                    let k = if pl > 0 { self.elems[ps as usize] } else { Value::Undef };
                    let v = if pl > 1 { self.elems[ps as usize + 1] } else { Value::Undef };
                    self.collection_put(id, k, v)?;
                } else { self.collection_put(id, item, item)?; }
            }
        }
        Ok(Value::Obj(id))
    }
    fn collection(&self, id: u8) -> Result<(u8, u8), Error> {
        self.collections.iter().find(|c| c.0 == id).map(|c| (c.1, c.2)).ok_or(Error::Unsupported)
    }
    /// Position of `key` (SameValueZero: NaN equals NaN).
    fn collection_find(&mut self, keys: u8, key: Value) -> Result<Option<usize>, Error> {
        let (start, len, _) = self.arrays[keys as usize];
        for k in 0..len as usize {
            let e = self.elems[start as usize + k];
            if let (Value::Num(a), Value::Num(b)) = (e, key) { if a.is_nan() && b.is_nan() { return Ok(Some(k)); } }
            if matches!(self.apply(b'E', e, key)?, Value::Bool(true)) { return Ok(Some(k)); }
        }
        Ok(None)
    }
    fn collection_put(&mut self, id: u8, key: Value, value: Value) -> Result<(), Error> {
        let (keys, vals) = self.collection(id)?;
        match self.collection_find(keys, key)? {
            Some(k) => { if keys != vals { let s = self.arrays[vals as usize].0 as usize; self.elems[s + k] = value; } }
            None => { self.push(keys, key)?; if keys != vals { self.push(vals, value)?; } }
        }
        Ok(())
    }
    /// The `i`th item when iterating: a Set's value, a Map's `[key, value]`.
    fn collection_item(&mut self, id: u8, i: usize) -> Result<Option<Value>, Error> {
        let (keys, vals) = self.collection(id)?;
        let (ks, len, _) = self.arrays[keys as usize];
        if i >= len as usize { return Ok(None); }
        let k = self.elems[ks as usize + i];
        if keys == vals { return Ok(Some(k)); }
        let v = self.elems[self.arrays[vals as usize].0 as usize + i];
        Ok(Some(self.new_array(&[k, v])?))
    }
    /// The values `...v` / `Array.from(v)` / `for...of` see.
    fn iterate(&mut self, v: Value) -> Result<Vec<Value>, Error> {
        Ok(match v {
            Value::Arr(id) => { let (s0, l0, _) = self.arrays[id as usize]; self.elems[s0 as usize..s0 as usize + l0 as usize].to_vec() }
            Value::Str(s0, n0) => (0..n0).map(|k| Value::Str(s0 + k, 1)).collect(),
            Value::Obj(id) if matches!(self.made_by[id as usize], 65 | 66) => {
                let mut out = Vec::new();
                while let Some(item) = self.collection_item(id, out.len())? { out.push(item); }
                out
            }
            _ if !self.live => Vec::new(),
            _ => return Err(Error::Unsupported),
        })
    }
    /// `size` and the methods of a Set or Map.
    #[inline(never)]
    fn collection_member(&mut self, id: u8, name: &[u8]) -> Result<Value, Error> {
        let (keys, vals) = self.collection(id)?;
        let map = keys != vals;
        if name == b"size" { return Ok(Value::Num(self.arrays[keys as usize].1 as f64)); }
        let (args, n) = self.arguments()?;
        let a0 = if n > 0 { args[0] } else { Value::Undef };
        Ok(match name {
            b"add" if !map => { self.collection_put(id, a0, a0)?; Value::Obj(id) }
            b"set" if map => { self.collection_put(id, a0, if n > 1 { args[1] } else { Value::Undef })?; Value::Obj(id) }
            b"get" if map => match self.collection_find(keys, a0)? { Some(k) => self.elems[self.arrays[vals as usize].0 as usize + k], None => Value::Undef },
            b"has" => Value::Bool(self.collection_find(keys, a0)?.is_some()),
            b"delete" => match self.collection_find(keys, a0)? {
                Some(k) => {
                    for arr in if map { [keys, vals].to_vec() } else { [keys].to_vec() } {
                        let (s0, l0, _) = self.arrays[arr as usize];
                        let mut items = self.elems[s0 as usize..s0 as usize + l0 as usize].to_vec();
                        items.remove(k);
                        self.set_items(arr, &items)?;
                    }
                    Value::Bool(true)
                }
                None => Value::Bool(false),
            },
            b"clear" => { self.set_items(keys, &[])?; if map { self.set_items(vals, &[])?; } Value::Undef }
            b"keys" => { let (s0, l0, _) = self.arrays[keys as usize]; let items = self.elems[s0 as usize..s0 as usize + l0 as usize].to_vec(); self.new_array(&items)? }
            b"values" => { let (s0, l0, _) = self.arrays[vals as usize]; let items = self.elems[s0 as usize..s0 as usize + l0 as usize].to_vec(); self.new_array(&items)? }
            b"entries" => { let items = self.iterate(Value::Obj(id))?; let items = if map { items } else { let mut pairs = Vec::new(); for v in items { pairs.push(self.new_array(&[v, v])?); } pairs }; self.new_array(&items)? }
            b"forEach" => {
                let Value::Func(f) = a0 else { return Err(Error::Unsupported) };
                let mut i = 0;
                while let Some(item) = { let (ks, len, _) = self.arrays[keys as usize]; (i < len as usize).then(|| self.elems[ks as usize + i]) } {
                    self.steps += 1;
                    if self.steps > STEPS { return Err(Error::Limit); }
                    let value = if map { self.elems[self.arrays[vals as usize].0 as usize + i] } else { item };
                    self.invoke(f, &[value, item, Value::Obj(id)])?;
                    i += 1;
                }
                Value::Undef
            }
            _ => return Err(Error::Unsupported),
        })
    }
    /// `x.toPrecision(p)`: `p` significant digits, exponential when the
    /// exponent is below -6 or at least `p` (as JavaScript).
    fn precision(&mut self, x: f64, p: usize) -> Result<Value, Error> {
        if !x.is_finite() { return self.number_text(x); }
        if x == 0.0 { return self.fixed(0.0, p - 1); }
        let a = if x < 0.0 { -x } else { x };
        let mut e: i32 = 0;
        let mut m = a;
        while m >= 10.0 { m /= 10.0; e += 1; }
        while m < 1.0 { m *= 10.0; e -= 1; }
        // Rounding to p digits can carry into a new digit (9.99 -> 10.0).
        let scale = pow(10.0, (p as i32 - 1) as f64);
        if floor(m * scale + 0.5) >= 10.0 * scale { e += 1; m /= 10.0; }
        if e < -6 || e >= p as i32 {
            let digits = self.fixed(m, p - 1)?;
            let mut t = if x < 0.0 { b"-".to_vec() } else { Vec::new() };
            t.extend_from_slice(self.text(digits));
            let _ = write!(Text(&mut t), "e{}{}", if e < 0 { '-' } else { '+' }, e.abs());
            return self.store(&t);
        }
        self.fixed(x, (p as i32 - 1 - e) as usize)
    }
    /// `x.toLocaleString()` in the en-US form: grouped thousands, at most
    /// three fraction digits.
    fn locale_number(&mut self, x: f64) -> Result<Value, Error> {
        if x.is_nan() { return self.store(b"NaN"); }
        if x.is_infinite() { return self.store(if x < 0.0 { "-∞".as_bytes() } else { "∞".as_bytes() }); }
        let f = self.fixed(x, 3)?;
        let mut t = self.text(f).to_vec();
        while t.last() == Some(&b'0') { t.pop(); }
        if t.last() == Some(&b'.') { t.pop(); }
        let neg = t.first() == Some(&b'-');
        let body = if neg { &t[1..] } else { &t[..] };
        let int_len = body.iter().position(|c| *c == b'.').unwrap_or(body.len());
        let mut out = if neg { b"-".to_vec() } else { Vec::new() };
        for (i, &c) in body.iter().enumerate() {
            if i > 0 && i < int_len && (int_len - i) % 3 == 0 { out.push(b','); }
            out.push(c);
        }
        if out == b"-0" { out.remove(0); }
        self.store(&out)
    }
    /// `new Date(...)` from given components, a time value or an ISO string.
    /// There is no `new Date()` / `Date.now()`: a script holds no clock.
    fn date_new(&mut self, args: &[Value]) -> Result<Value, Error> {
        let ms = match args {
            [] => return Err(Error::Unsupported),
            [Value::Num(t)] => *t,
            [s @ Value::Str(..)] => { let t = self.text(*s).to_vec(); parse_iso(&t).unwrap_or(f64::NAN) }
            parts => {
                let mut v = [0.0f64, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
                for (k, &p) in parts.iter().take(7).enumerate() { v[k] = self.to_number(p); }
                date_ms(v)
            }
        };
        let id = self.new_object()?;
        self.set_prop(id, b"__date", Value::Num(ms))?;
        Ok(Value::Obj(id))
    }
    fn date_method(&mut self, ms: f64, name: &[u8]) -> Result<Value, Error> {
        if ms.is_nan() { return if name == b"toISOString" { Err(Error::Unsupported) } else { Ok(Value::Num(f64::NAN)) }; }
        let days = floor(ms / 86_400_000.0);
        let rem = ms - days * 86_400_000.0;
        let (y, m, d) = civil(days as i64);
        let (h, mi, sec, milli) = ((rem / 3_600_000.0) as i64, (rem / 60_000.0) as i64 % 60, (rem / 1000.0) as i64 % 60, rem as i64 % 1000);
        let weekday = ((days as i64 % 7) + 11) % 7; // 1970-01-01 was a Thursday
        Ok(match name {
            b"getTime" | b"valueOf" => Value::Num(ms),
            b"getFullYear" | b"getUTCFullYear" => Value::Num(y as f64),
            b"getMonth" | b"getUTCMonth" => Value::Num((m - 1) as f64),
            b"getDate" | b"getUTCDate" => Value::Num(d as f64),
            b"getDay" | b"getUTCDay" => Value::Num(weekday as f64),
            b"getHours" | b"getUTCHours" => Value::Num(h as f64),
            b"getMinutes" | b"getUTCMinutes" => Value::Num(mi as f64),
            b"getSeconds" | b"getUTCSeconds" => Value::Num(sec as f64),
            b"getMilliseconds" | b"getUTCMilliseconds" => Value::Num(milli as f64),
            b"toISOString" | b"toJSON" => {
                let mut t = Vec::new();
                let _ = write!(Text(&mut t), "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, h, mi, sec, milli);
                self.store(&t)?
            }
            b"toDateString" => {
                const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                let mut t = Vec::new();
                let _ = write!(Text(&mut t), "{} {} {:02} {:04}", DAYS[weekday as usize], MONTHS[(m - 1) as usize], d, y);
                self.store(&t)?
            }
            _ => return Err(Error::Unsupported),
        })
    }
    /// One JSON value from `t` at `at` (objects, arrays, strings, numbers,
    /// true/false/null), depth-bounded.
    fn json_value(&mut self, t: &[u8], at: &mut usize, depth: u8) -> Result<Value, Error> {
        if depth > 16 { return Err(Error::Limit); }
        json_ws(t, at);
        let c = *t.get(*at).ok_or(Error::Syntax)?;
        match c {
            b'{' => {
                *at += 1;
                let id = self.new_object()?;
                json_ws(t, at);
                if t.get(*at) == Some(&b'}') { *at += 1; return Ok(Value::Obj(id)); }
                loop {
                    json_ws(t, at);
                    let key = self.json_value(t, at, depth + 1)?;
                    let Value::Str(..) = key else { return Err(Error::Syntax) };
                    let k = self.text(key).to_vec();
                    json_ws(t, at);
                    if t.get(*at) != Some(&b':') { return Err(Error::Syntax); }
                    *at += 1;
                    let v = self.json_value(t, at, depth + 1)?;
                    self.set_prop(id, &k, v)?;
                    json_ws(t, at);
                    match t.get(*at) { Some(b',') => *at += 1, Some(b'}') => { *at += 1; return Ok(Value::Obj(id)); } _ => return Err(Error::Syntax) }
                }
            }
            b'[' => {
                *at += 1;
                let mut items: Vec<Value> = Vec::new();
                json_ws(t, at);
                if t.get(*at) == Some(&b']') { *at += 1; return self.new_array(&items); }
                loop {
                    items.push(self.json_value(t, at, depth + 1)?);
                    if items.len() > ELEMS { return Err(Error::Limit); }
                    json_ws(t, at);
                    match t.get(*at) { Some(b',') => *at += 1, Some(b']') => { *at += 1; return self.new_array(&items); } _ => return Err(Error::Syntax) }
                }
            }
            b'"' => {
                *at += 1;
                let mut out: Vec<u8> = Vec::new();
                loop {
                    let b = *t.get(*at).ok_or(Error::Syntax)?;
                    *at += 1;
                    match b {
                        b'"' => break,
                        b'\\' => {
                            let e = *t.get(*at).ok_or(Error::Syntax)?;
                            *at += 1;
                            match e {
                                b'"' | b'\\' | b'/' => out.push(e), b'b' => out.push(8), b'f' => out.push(12), b'n' => out.push(b'\n'), b'r' => out.push(b'\r'), b't' => out.push(b'\t'),
                                b'u' => {
                                    let h = t.get(*at..*at + 4).ok_or(Error::Syntax)?;
                                    let mut code = 0u32;
                                    for &d in h { code = code * 16 + (d as char).to_digit(16).ok_or(Error::Syntax)?; }
                                    *at += 4;
                                    let mut buf = [0u8; 4];
                                    out.extend_from_slice(char::from_u32(code).unwrap_or('\u{fffd}').encode_utf8(&mut buf).as_bytes());
                                }
                                _ => return Err(Error::Syntax),
                            }
                        }
                        0..=31 => return Err(Error::Syntax),
                        _ => out.push(b),
                    }
                    if out.len() > ARENA { return Err(Error::Limit); }
                }
                self.store(&out)
            }
            b't' if t[*at..].starts_with(b"true") => { *at += 4; Ok(Value::Bool(true)) }
            b'f' if t[*at..].starts_with(b"false") => { *at += 5; Ok(Value::Bool(false)) }
            b'n' if t[*at..].starts_with(b"null") => { *at += 4; Ok(Value::Null) }
            b'-' | b'0'..=b'9' => {
                let start = *at;
                while t.get(*at).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E')) { *at += 1; }
                let s = core::str::from_utf8(&t[start..*at]).map_err(|_| Error::Syntax)?;
                if !s_is_plain(s.as_bytes()) { return Err(Error::Syntax); }
                Ok(Value::Num(s.parse::<f64>().map_err(|_| Error::Syntax)?))
            }
            _ => Err(Error::Syntax),
        }
    }
    /// JSON text of a value; undefined members are left out like JavaScript.
    fn json(&mut self, v: Value, into: &mut Vec<u8>) -> Result<(), Error> { self.json_with(v, into, &Layout { indent: Vec::new(), keys: None }, 0) }
    /// A new line and the indent for `level` (pretty printing only).
    fn json_break(into: &mut Vec<u8>, layout: &Layout, level: usize) {
        if layout.indent.is_empty() { return; }
        into.push(b'\n');
        for _ in 0..level { into.extend_from_slice(&layout.indent); }
    }
    fn json_with(&mut self, v: Value, into: &mut Vec<u8>, layout: &Layout, level: usize) -> Result<(), Error> {
        if self.too_deep() { return Err(Error::Limit); }
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
                    Self::json_break(into, layout, level + 1);
                    let e = self.elems[start as usize + k];
                    if matches!(e, Value::Undef | Value::Func(_)) { into.extend_from_slice(b"null"); } else { self.json_with(e, into, layout, level + 1)?; }
                }
                if len > 0 { Self::json_break(into, layout, level); }
                into.push(b']');
            }
            Value::Obj(id) => {
                into.push(b'{');
                let (start, len, _) = self.objects[id as usize];
                let mut first = true;
                for k in start as usize..start as usize + len as usize {
                    let (ns, nn, value) = self.props[k];
                    // Like JavaScript: undefined and function members are left out,
                    // and a replacer list keeps only the keys it names.
                    if matches!(value, Value::Undef | Value::Func(_)) { continue; }
                    if let Some(keys) = &layout.keys { if !keys.iter().any(|k| k[..] == self.arena[ns as usize..ns as usize + nn as usize]) { continue; } }
                    if !first { into.push(b','); }
                    first = false;
                    Self::json_break(into, layout, level + 1);
                    self.quote(Value::Str(ns, nn), into);
                    into.push(b':');
                    if !layout.indent.is_empty() { into.push(b' '); }
                    self.json_with(value, into, layout, level + 1)?;
                }
                if !first { Self::json_break(into, layout, level); }
                into.push(b'}');
            }
            Value::Undef | Value::Func(_) => into.extend_from_slice(b"null"),
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
    /// Items up to `close` (`)` or `]`), with `...array` spread, into `out`.
    fn items(&mut self, close: u8, out: &mut [Value]) -> Result<usize, Error> {
        let mut n = 0;
        while self.tok != Tok::Punct(close) {
            if self.tok == Tok::Punct(b'S') {
                self.next()?;
                let v = self.expr()?;
                let items = self.iterate(v)?;
                if n + items.len() > out.len() { return Err(Error::Limit); }
                out[n..n + items.len()].copy_from_slice(&items);
                n += items.len();
            } else {
                if n == out.len() { return Err(Error::Limit); }
                out[n] = self.expr()?;
                n += 1;
            }
            if self.tok != Tok::Punct(b',') { break; }
            self.next()?;
        }
        self.expect(close)?;
        Ok(n)
    }
    /// `( a, b, ... )`, at most four values.
    fn arguments(&mut self) -> Result<([Value; PARAMS], usize), Error> {
        self.expect(b'(')?;
        let mut args = [Value::Undef; PARAMS];
        let n = self.items(b')', &mut args)?;
        Ok((args, n))
    }
    fn array_method(&mut self, id: u8, name: &[u8], args: &[Value]) -> Result<Value, Error> {
        let (start, len, _) = self.arrays[id as usize];
        let (start, len) = (start as usize, len as usize);
        let arg = |i: usize| args.get(i).copied().unwrap_or(Value::Undef);
        let index = |m: &Self, v: Value, dflt: usize| -> usize {
            if matches!(v, Value::Undef) { return dflt; }
            let x = m.to_number(v); let x = if x.is_nan() { 0.0 } else { floor(x) };
            let x = if x < 0.0 { len as f64 + x } else { x };
            if x < 0.0 { 0 } else if x > len as f64 { len } else { x as usize }
        };
        Ok(match name {
            b"lastIndexOf" => {
                let mut found = -1.0;
                for k in (0..len).rev() { if matches!(self.apply(b'E', self.elems[start + k], arg(0))?, Value::Bool(true)) { found = k as f64; break; } }
                Value::Num(found)
            }
            b"toString" => self.join(id, b",")?,
            b"keys" => { let items: Vec<Value> = (0..len).map(|k| Value::Num(k as f64)).collect(); self.new_array(&items)? }
            b"values" => { let items = self.elems[start..start + len].to_vec(); self.new_array(&items)? }
            b"entries" => {
                let mut items = Vec::new();
                for k in 0..len { let e = self.elems[start + k]; items.push(self.new_array(&[Value::Num(k as f64), e])?); }
                self.new_array(&items)?
            }
            b"splice" | b"unshift" => {
                // Rebuild the contents; the array keeps its identity.
                let (a, removed) = if name == b"unshift" { (0, 0) } else {
                    let a = index(self, arg(0), len);
                    let k = if args.len() < 2 { len - a } else { let d = self.to_number(arg(1)); if d.is_nan() || d < 0.0 { 0 } else { (floor(d) as usize).min(len - a) } };
                    (a, k)
                };
                let insert = if name == b"unshift" { args } else { args.get(2..).unwrap_or(&[]) };
                let old: Vec<Value> = self.elems[start..start + len].to_vec();
                if !self.live { return Ok(if name == b"unshift" { Value::Num(len as f64) } else { Value::Undef }); }
                let mut items: Vec<Value> = old[..a].to_vec();
                items.extend_from_slice(insert);
                items.extend_from_slice(&old[a + removed..]);
                if items.len() > ELEMS { return Err(Error::Limit); }
                self.set_items(id, &items)?;
                if name == b"unshift" { Value::Num(items.len() as f64) } else { self.new_array(&old[a..a + removed])? }
            }
            b"indexOf" | b"includes" => {
                let mut found = None;
                for k in 0..len { if matches!(self.apply(b'E', self.elems[start + k], arg(0))?, Value::Bool(true)) { found = Some(k); break; } }
                if name == b"includes" { Value::Bool(found.is_some()) } else { Value::Num(found.map_or(-1.0, |k| k as f64)) }
            }
            b"slice" => {
                let (a, b) = (index(self, arg(0), 0), index(self, arg(1), len));
                let items: Vec<Value> = if a < b { self.elems[start + a..start + b].to_vec() } else { Vec::new() };
                self.new_array(&items)?
            }
            b"concat" => {
                let mut items: Vec<Value> = self.elems[start..start + len].to_vec();
                for &a in args {
                    if let Value::Arr(other) = a { let (s2, l2, _) = self.arrays[other as usize]; items.extend_from_slice(&self.elems[s2 as usize..s2 as usize + l2 as usize]); } else { items.push(a); }
                    if items.len() > ELEMS { return Err(Error::Limit); }
                }
                self.new_array(&items)?
            }
            b"reverse" => { if self.live { self.elems[start..start + len].reverse(); } Value::Arr(id) }
            b"fill" => { if self.live { let (a, b) = (index(self, arg(1), 0), index(self, arg(2), len)); for k in a..b.max(a) { self.elems[start + k] = arg(0); } } Value::Arr(id) }
            b"at" => { let i = self.to_number(arg(0)); let i = if i < 0.0 { len as f64 + floor(i) } else { floor(i) }; if i >= 0.0 && (i as usize) < len { self.elems[start + i as usize] } else { Value::Undef } }
            b"flat" => {
                let mut items: Vec<Value> = Vec::new();
                for k in 0..len {
                    match self.elems[start + k] { Value::Arr(inner) => { let (s2, l2, _) = self.arrays[inner as usize]; items.extend_from_slice(&self.elems[s2 as usize..s2 as usize + l2 as usize]); } e => items.push(e) }
                    if items.len() > ELEMS { return Err(Error::Limit); }
                }
                self.new_array(&items)?
            }
            b"sort" => {
                let cmp = match args.first() { None => None, Some(Value::Func(f)) => Some(*f), _ => return Err(Error::Unsupported) };
                if let (Some(f), true) = (cmp, self.live) {
                    // Insertion sort with the script's comparator (bounded by steps).
                    for i in 1..len {
                        let mut j = i;
                        while j > 0 {
                            let r = self.invoke(f, &[self.elems[start + j - 1], self.elems[start + j]])?;
                            if self.to_number(r) > 0.0 { self.elems.swap(start + j - 1, start + j); j -= 1; } else { break; }
                        }
                    }
                } else if self.live {
                    // Default JavaScript order: by string form, undefined last.
                    let mut keyed: Vec<(Vec<u8>, bool, Value)> = Vec::new();
                    for k in 0..len {
                        let e = self.elems[start + k];
                        let undefined = matches!(e, Value::Undef);
                        let t = if undefined { Vec::new() } else { let t = self.to_text(e)?; self.text(t).to_vec() };
                        keyed.push((t, undefined, e));
                    }
                    for i in 1..keyed.len() { let mut j = i; while j > 0 && (keyed[j].1, &keyed[j].0) < (keyed[j - 1].1, &keyed[j - 1].0) { keyed.swap(j, j - 1); j -= 1; } }
                    for (k, (_, _, e)) in keyed.into_iter().enumerate() { self.elems[start + k] = e; }
                }
                Value::Arr(id)
            }
            _ => {
                // pop / shift: remove and return one end element.
                if len == 0 { return Ok(Value::Undef); }
                let e = if name == b"pop" { self.elems[start + len - 1] } else { self.elems[start] };
                if self.live {
                    if name == b"shift" { self.elems.copy_within(start + 1..start + len, start); }
                    let (s0, l0, c0) = self.arrays[id as usize];
                    self.arrays[id as usize] = (s0, l0 - 1, c0);
                }
                e
            }
        })
    }
    fn array_callback(&mut self, id: u8, name: &[u8], args: &[Value]) -> Result<Value, Error> {
        let Some(Value::Func(f)) = args.first().copied() else { return Err(Error::Unsupported) };
        if !self.live { return Ok(Value::Undef); }
        let (_, len, _) = self.arrays[id as usize];
        let mut acc = args.get(1).copied();
        let mut kept: Vec<Value> = Vec::new();
        if name == b"reduceRight" {
            let mut acc = args.get(1).copied();
            for k in (0..len as usize).rev() {
                let (start, now, _) = self.arrays[id as usize];
                if k >= now as usize { continue; }
                let e = self.elems[start as usize + k];
                self.steps += 1;
                if self.steps > STEPS { return Err(Error::Limit); }
                acc = Some(match acc { None => e, Some(a) => self.invoke(f, &[a, e, Value::Num(k as f64), Value::Arr(id)])? });
            }
            return acc.ok_or(Error::Unsupported);
        }
        if matches!(name, b"findLast" | b"findLastIndex") {
            for k in (0..len as usize).rev() {
                let (start, now, _) = self.arrays[id as usize];
                let e = if k < now as usize { self.elems[start as usize + k] } else { Value::Undef };
                self.steps += 1;
                if self.steps > STEPS { return Err(Error::Limit); }
                let r = self.invoke(f, &[e, Value::Num(k as f64), Value::Arr(id)])?;
                if self.truthy(r) { return Ok(if name == b"findLast" { e } else { Value::Num(k as f64) }); }
            }
            return Ok(if name == b"findLast" { Value::Undef } else { Value::Num(-1.0) });
        }
        for k in 0..len as usize {
            // JavaScript visits the length present at the start; the array may
            // have moved or shrunk, so re-read where it lives now.
            let (start, now, _) = self.arrays[id as usize];
            if k >= now as usize { break; }
            let e = self.elems[start as usize + k];
            self.steps += 1;
            if self.steps > STEPS { return Err(Error::Limit); }
            if name == b"reduce" {
                acc = Some(match acc { None => e, Some(a) => self.invoke(f, &[a, e, Value::Num(k as f64), Value::Arr(id)])? });
                continue;
            }
            let r = self.invoke(f, &[e, Value::Num(k as f64), Value::Arr(id)])?;
            let yes = self.truthy(r);
            match name {
                b"map" => kept.push(r),
                b"flatMap" => if let Value::Arr(inner) = r { let (s2, l2, _) = self.arrays[inner as usize]; kept.extend_from_slice(&self.elems[s2 as usize..s2 as usize + l2 as usize]); } else { kept.push(r) },
                b"filter" => if yes { kept.push(e) },
                b"find" => if yes { return Ok(e) },
                b"findIndex" => if yes { return Ok(Value::Num(k as f64)) },
                b"some" => if yes { return Ok(Value::Bool(true)) },
                b"every" => if !yes { return Ok(Value::Bool(false)) },
                _ => {}
            }
            if kept.len() > ELEMS { return Err(Error::Limit); }
        }
        Ok(match name {
            b"map" | b"filter" | b"flatMap" => self.new_array(&kept)?,
            b"find" => Value::Undef,
            b"findIndex" => Value::Num(-1.0),
            b"some" => Value::Bool(false),
            b"every" => Value::Bool(true),
            b"reduce" => acc.ok_or(Error::Unsupported)?,
            _ => Value::Undef,
        })
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
    /// Replaces an array's contents in place, moving it when it outgrows its room.
    fn set_items(&mut self, id: u8, items: &[Value]) -> Result<(), Error> {
        let (mut start, _, mut cap) = self.arrays[id as usize];
        if items.len() > cap as usize {
            let cap2 = items.len().max(cap as usize * 2);
            let at = self.elems.len();
            if at + cap2 > ELEMS { return Err(Error::Limit); }
            self.elems.resize(at + cap2, Value::Undef);
            (start, cap) = (at as u16, cap2 as u16);
        }
        self.elems[start as usize..start as usize + items.len()].copy_from_slice(items);
        self.arrays[id as usize] = (start, items.len() as u16, cap);
        Ok(())
    }
    fn join(&mut self, id: u8, sep: &[u8]) -> Result<Value, Error> {
        if self.too_deep() { return Err(Error::Limit); }
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
        let mut values = [Value::Undef; 32];
        let n = self.items(b')', &mut values)?;
        let mut args = [0.0f64; 32];
        for k in 0..n { args[k] = self.to_number(values[k]); }
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
    /// `{a: 1, 'b': x, c, [k]: v, m() {...}, ...o}` (kept out of `atom`, whose frame recurses).
    #[inline(never)]
    fn object_literal(&mut self) -> Result<Value, Error> {
        self.next()?;
        let id = self.new_object()?;
        while self.tok != Tok::Punct(b'}') {
            if self.tok == Tok::Punct(b'S') {
                // `...o` copies o's own properties (null/undefined: none).
                self.next()?;
                let from = self.expr()?;
                match from {
                    Value::Obj(src) if self.live => {
                        let (start, len, _) = self.objects[src as usize];
                        for k in start as usize..start as usize + len as usize {
                            let (ns, nn, value) = self.props[k];
                            let key = self.arena[ns as usize..ns as usize + nn as usize].to_vec();
                            self.set_prop(id, &key, value)?;
                        }
                    }
                    Value::Obj(_) | Value::Null | Value::Undef => {}
                    _ if !self.live => {}
                    _ => return Err(Error::Unsupported),
                }
            } else {
                // `name: v`, `'name': v`, `1: v`, `[expr]: v`, `name` (shorthand)
                // or `name(...) { ... }` (a method).
                let (key, name): (Vec<u8>, Option<&'a [u8]>) = match self.tok {
                    Tok::Ident(k) => { self.next()?; (k.to_vec(), Some(k)) }
                    Tok::Str(k) => { self.next()?; (k.to_vec(), None) }
                    Tok::Num(x) => { self.next()?; let t = self.number_text(x)?; (self.text(t).to_vec(), None) }
                    Tok::Punct(b'[') => { self.next()?; let k = self.expr()?; self.expect(b']')?; let t = self.to_text(k)?; (self.text(t).to_vec(), None) }
                    _ => return Err(Error::Syntax),
                };
                if let (Some(kw @ (b"get" | b"set")), Tok::Ident(n) | Tok::Str(n)) = (name, self.tok) {
                    // `get name() {...}` / `set name(v) {...}`.
                    self.next()?;
                    let at = self.pos();
                    self.expect(b'(')?;
                    self.skip_params()?;
                    self.expect(b')')?;
                    if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
                    self.skim_body()?;
                    let f = self.define(b"", at)?;
                    self.add_accessor(id, n, f, kw == b"set")?;
                    if self.tok != Tok::Punct(b',') { break; }
                    self.next()?;
                    continue;
                }
                let v = match (self.tok, name) {
                    (Tok::Punct(b'('), _) => {
                        let at = self.pos();
                        self.next()?;
                        self.skip_params()?;
                        self.expect(b')')?;
                        if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
                        self.skim_body()?;
                        Value::Func(self.define(b"", at)?)
                    }
                    (Tok::Punct(b',' | b'}'), Some(n)) => self.lookup(n)
                        .or_else(|| self.funcs[..self.nfuncs].iter().position(|(f, l, _)| *l != 0 && &f[..*l as usize] == n).map(|i| Value::Func(i as u8)))
                        .or((self.skim || !self.live).then_some(Value::Undef)).ok_or(Error::Unsupported)?,
                    _ => { self.expect(b':')?; self.expr()? }
                };
                self.set_prop(id, &key, v)?;
            }
            if self.tok != Tok::Punct(b',') { break; }
            self.next()?;
        }
        self.expect(b'}')?;
        Ok(Value::Obj(id))
    }
    /// `[a, b, ...xs]`.
    #[inline(never)]
    fn array_literal(&mut self) -> Result<Value, Error> {
        self.next()?;
        // On the heap: 32 values would be 512 bytes of the Browser stack.
        let mut items = Vec::new();
        items.try_reserve_exact(32).map_err(|_| Error::Limit)?;
        items.resize(32, Value::Undef);
        let n = self.items(b']', &mut items)?;
        self.new_array(&items[..n])
    }
    /// `new C(...)`.
    #[inline(never)]
    fn new_expression(&mut self) -> Result<Value, Error> {
        self.next()?;
        let Tok::Ident(class) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        let mut args = [Value::Undef; PARAMS];
        let n = if self.tok == Tok::Punct(b'(') { self.argument_list(&mut args)? } else { 0 };
        if !self.live { return Ok(Value::Undef); }
        self.construct(class, &args[..n])
    }
    /// `Date.UTC(...)`.
    #[inline(never)]
    fn date_static(&mut self) -> Result<Value, Error> {
        self.next()?;
        let Tok::Ident(f) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        let (args, n) = self.arguments()?;
        match f {
            b"UTC" => { let mut v = [0.0f64, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]; for k in 0..n { v[k] = self.to_number(args[k]); } Ok(Value::Num(date_ms(v))) }
            // Date.now() would need a clock: refused.
            _ => Err(Error::Unsupported),
        }
    }
    /// `JSON.*`, `Object.*`, `Array.*`, `Number.*`.
    #[inline(never)]
    fn statics(&mut self, name: &[u8]) -> Result<Value, Error> {
        self.expect(b'.')?;
        let Tok::Ident(f) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        if (name, f) == (&b"Array"[..], &b"of"[..]) { let (args, n) = self.arguments()?; return self.new_array(&args[..n]); }
        if (name, f) == (&b"JSON"[..], &b"stringify"[..]) {
            // `JSON.stringify(v, replacer, space)`: a key list (a function
            // replacer is refused) and an indent of up to 10 spaces or text.
            let (args, n) = self.arguments()?;
            let keys = match args[1] {
                Value::Arr(id) if n > 1 => { let (st, len, _) = self.arrays[id as usize]; let mut keys = Vec::new(); for k in 0..len as usize { let t = self.to_text(self.elems[st as usize + k])?; keys.push(self.text(t).to_vec()); } Some(keys) }
                Value::Undef | Value::Null => None,
                _ if n < 2 => None,
                _ => return Err(Error::Unsupported),
            };
            let indent = match args[2] {
                Value::Num(x) => { let k = if x.is_nan() { 0.0 } else { x.clamp(0.0, 10.0) }; alloc::vec![b' '; k as usize] }
                s @ Value::Str(..) => { let t = self.text(s); t[..t.len().min(10)].to_vec() }
                _ => Vec::new(),
            };
            let mut j = Vec::new();
            self.json_with(args[0], &mut j, &Layout { indent, keys }, 0)?;
            return self.store(&j);
        }
        if (name, f) == (&b"Array"[..], &b"from"[..]) && self.from_with_map()? {
            // `Array.from(x, fn)`: each item mapped through fn(item, index).
            let (args, _) = self.arguments()?;
            if !self.live { return Ok(Value::Undef); }
            let Value::Func(g) = args[1] else { return Err(Error::Unsupported) };
            let items = match args[0] {
                Value::Obj(id) if !matches!(self.made_by[id as usize], 65 | 66) => { let n = self.to_number(self.get_prop(id, b"length")); if !(0.0..=ELEMS as f64).contains(&n) { return Err(Error::Limit); } (0..n as usize).map(|_| Value::Undef).collect() }
                v => self.iterate(v)?,
            };
            let mut out = Vec::new();
            for (k, item) in items.into_iter().enumerate() { self.steps += 1; if self.steps > STEPS { return Err(Error::Limit); } out.push(self.invoke(g, &[item, Value::Num(k as f64)])?); }
            return self.new_array(&out);
        }
        if name == b"Number" && self.tok != Tok::Punct(b'(') {
            return Ok(Value::Num(match f {
                b"MAX_SAFE_INTEGER" => 9007199254740991.0, b"MIN_SAFE_INTEGER" => -9007199254740991.0,
                b"EPSILON" => f64::EPSILON, b"MAX_VALUE" => f64::MAX, b"MIN_VALUE" => 5e-324,
                b"POSITIVE_INFINITY" => f64::INFINITY, b"NEGATIVE_INFINITY" => f64::NEG_INFINITY, b"NaN" => f64::NAN,
                _ => return Err(Error::Unsupported),
            }));
        }
        if name == b"Promise" {
            let (args, _) = self.arguments()?;
            if !self.live { return Ok(Value::Undef); }
            return match f {
                b"resolve" => { if let Value::Obj(id) = args[0] { if self.made_by[id as usize] == 67 { return Ok(args[0]); } } let p = self.promise_new()?; self.resolve(p, args[0])?; Ok(Value::Obj(p)) }
                b"reject" => { let p = self.promise_new()?; self.settle(p, 2, args[0])?; Ok(Value::Obj(p)) }
                b"all" => {
                    let items = self.iterate(args[0])?;
                    let p = self.promise_new()?;
                    let results = self.new_array(&alloc::vec![Value::Undef; items.len()])?;
                    let Value::Arr(results) = results else { return Err(Error::Limit) };
                    if items.is_empty() { self.settle(p, 1, Value::Arr(results))?; return Ok(Value::Obj(p)); }
                    if self.gathers.len() == 32 { return Err(Error::Limit); }
                    self.gathers.push((p, results, items.len()));
                    let g = (self.gathers.len() - 1) as u8;
                    for (k, item) in items.into_iter().enumerate() {
                        let q = match item { Value::Obj(id) if self.made_by[id as usize] == 67 => id, v => { let q = self.promise_new()?; self.settle(q, 1, v)?; q } };
                        self.react(q, Reaction { fulfilled: None, rejected: None, child: None, finally: false, slot: Some((g, k as u16)) })?;
                    }
                    Ok(Value::Obj(p))
                }
                _ => Err(Error::Unsupported),
            };
        }
        if (name, f) == (&b"Object"[..], &b"hasOwn"[..]) {
            let (args, _) = self.arguments()?;
            let Value::Obj(id) = args[0] else { return Ok(Value::Bool(false)) };
            let k = self.to_text(args[1])?;
            let key = self.text(k).to_vec();
            return Ok(Value::Bool(self.find_prop(id, &key).is_some() || self.accessor(id, &key).is_some()));
        }
        if (name, f) == (&b"Object"[..], &b"assign"[..]) {
            // Copies own properties of each source onto the target.
            let (args, n) = self.arguments()?;
            if !self.live { return Ok(Value::Undef); }
            let Some(&Value::Obj(to)) = args[..n].first() else { return Err(Error::Unsupported) };
            for &src in &args[1..n] {
                let Value::Obj(from) = src else { if matches!(src, Value::Null | Value::Undef) { continue; } return Err(Error::Unsupported) };
                let (start, len, _) = self.objects[from as usize];
                for k in start as usize..start as usize + len as usize {
                    let (ns, nn, value) = self.props[k];
                    let key = self.arena[ns as usize..ns as usize + nn as usize].to_vec();
                    self.set_prop(to, &key, value)?;
                }
            }
            return Ok(Value::Obj(to));
        }
        self.expect(b'(')?;
        let v = self.expr()?;
        self.expect(b')')?;
        let num = |m: &Self| if let Value::Num(x) = v { Some(x) } else { let _ = m; None };
        match (name, f, v) {
            (b"Array", b"isArray", v) => Ok(Value::Bool(matches!(v, Value::Arr(_)))),
            (b"Array", b"from", v) => {
                // From an array (copy), a string (characters) or {length: n}.
                let items: Vec<Value> = match v {
                    Value::Arr(id) => { let (s0, l0, _) = self.arrays[id as usize]; self.elems[s0 as usize..s0 as usize + l0 as usize].to_vec() }
                    Value::Str(s0, n0) => (0..n0).map(|k| Value::Str(s0 + k, 1)).collect(),
                    Value::Obj(id) if matches!(self.made_by[id as usize], 65 | 66) => self.iterate(v)?,
                    Value::Obj(id) => { let n = self.to_number(self.get_prop(id, b"length")); if !(0.0..=ELEMS as f64).contains(&n) { return Err(Error::Limit); } (0..n as usize).map(|_| Value::Undef).collect() }
                    _ => return Err(Error::Unsupported),
                };
                if items.len() > ELEMS { return Err(Error::Limit); }
                self.new_array(&items)
            }
            (b"Number", b"isInteger", _) => Ok(Value::Bool(num(self).is_some_and(|x| x.is_finite() && x == floor(x)))),
            (b"Number", b"isFinite", _) => Ok(Value::Bool(num(self).is_some_and(f64::is_finite))),
            (b"Number", b"isNaN", _) => Ok(Value::Bool(num(self).is_some_and(f64::is_nan))),
            (b"Object", b"freeze", v) => { if let (Value::Obj(id), true) = (v, self.live) { if !self.frozen.contains(&id) { self.frozen.push(id); } } Ok(v) }
            (b"Object", b"isFrozen", v) => Ok(Value::Bool(match v { Value::Obj(id) => self.frozen.contains(&id), _ => true })),
            (b"Object", b"fromEntries", Value::Arr(list)) => {
                let id = self.new_object()?;
                let (start, len, _) = self.arrays[list as usize];
                for k in 0..len as usize {
                    let Value::Arr(pair) = self.elems[start as usize + k] else { return Err(Error::Unsupported) };
                    let (ps, pl, _) = self.arrays[pair as usize];
                    let key = if pl > 0 { self.elems[ps as usize] } else { Value::Undef };
                    let value = if pl > 1 { self.elems[ps as usize + 1] } else { Value::Undef };
                    let key = self.to_text(key)?;
                    let key = self.text(key).to_vec();
                    self.set_prop(id, &key, value)?;
                }
                Ok(Value::Obj(id))
            }
            (b"Object", b"values" | b"entries", Value::Obj(id)) => {
                let (start, len, _) = self.objects[id as usize];
                let mut items = Vec::new();
                for k in 0..len as usize {
                    let (ns, nn, value) = self.props[start as usize + k];
                    items.push(if f == b"values" { value } else { self.new_array(&[Value::Str(ns, nn), value])? });
                }
                self.new_array(&items)
            }
            (b"JSON", b"stringify", v) => { let mut j = Vec::new(); self.json(v, &mut j)?; self.store(&j) }
            (b"JSON", b"parse", v) => {
                if !self.live { return Ok(Value::Undef); }
                let t = self.to_text(v)?;
                let text = self.text(t).to_vec();
                let mut at = 0;
                let parsed = self.json_value(&text, &mut at, 0).and_then(|v| { json_ws(&text, &mut at); if at == text.len() { Ok(v) } else { Err(Error::Syntax) } });
                match parsed {
                    Ok(v) => Ok(v),
                    // Malformed JSON throws, as in JavaScript (catchable).
                    Err(Error::Syntax) => { let m = self.store(b"SyntaxError: bad JSON")?; self.thrown = Some(m); Err(Error::Thrown) }
                    Err(e) => Err(e),
                }
            }
            (b"Object", b"keys", Value::Obj(id)) => {
                let (start, len, _) = self.objects[id as usize];
                let keys: Vec<Value> = (start as usize..start as usize + len as usize).map(|k| { let (ns, nn, _) = self.props[k]; Value::Str(ns, nn) }).collect();
                self.new_array(&keys)
            }
            _ => Err(Error::Unsupported),
        }
    }
    /// `String.fromCharCode(...)`.
    #[inline(never)]
    fn string_static(&mut self) -> Result<Value, Error> {
        self.next()?;
        if self.tok != Tok::Ident(b"fromCharCode") { return Err(Error::Unsupported); }
        self.next()?;
        let (args, n) = self.arguments()?;
        if !self.live { return Ok(Value::Undef); }
        let mut text: Vec<u8> = Vec::new();
        for &a in &args[..n] {
            let c = self.to_number(a);
            let ch = if c.is_finite() && c >= 0.0 { char::from_u32(c as u32 & 0xffff) } else { None }.ok_or(Error::Unsupported)?;
            let mut b = [0u8; 4];
            text.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
        }
        self.store(&text)
    }
    /// `C.m(...)`: a static method of class C (or a parent).
    #[inline(never)]
    fn class_static(&mut self, name: &[u8]) -> Result<Value, Error> {
        // `C.m(...)`: a static method of class C (or a parent).
        self.next()?;
        let Tok::Ident(m) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        let mut args = [Value::Undef; PARAMS];
        let n = self.argument_list(&mut args)?;
        if !self.live { return Ok(Value::Undef); }
        let mut k = self.classes.iter().position(|c| c.0 == name);
        while let Some(c) = k {
            if let Some(&(_, f)) = self.classes[c].4.iter().find(|s| s.0 == m) { return self.invoke(f, &args[..n]); }
            k = self.classes[c].3.map(|p| p as usize);
        }
        Err(Error::Unsupported)
    }
    fn atom(&mut self) -> Result<Value, Error> {
        // Arrow functions need one token of lookahead (or a scan to `)`).
        let arrow = match self.tok {
            Tok::Punct(b'(') => self.arrow_ahead()?,
            Tok::Ident(_) => { let at = self.pos(); self.next()?; let a = self.tok == Tok::Punct(b'F'); self.restore(at); a }
            _ => false,
        };
        match self.tok {
            Tok::Num(x) => { self.next()?; Ok(Value::Num(x)) }
            Tok::Str(raw) => { self.next()?; self.literal(raw) }
            Tok::Regex(pattern, flags) => {
                let at = pattern.as_ptr() as usize - self.src.as_ptr() as usize;
                self.next()?;
                if let Some(k) = self.regexes.iter().position(|r| r.0 == at) { return Ok(Value::Re(k as u8)); }
                if self.regexes.len() == 16 { return Err(Error::Limit); }
                let re = Regex::new(pattern, flags).map_err(|e| if e == crate::regex::Error::Limit { Error::Limit } else { Error::Syntax })?;
                self.regexes.push((at, pattern, flags, re));
                self.re_last.push(0);
                Ok(Value::Re((self.regexes.len() - 1) as u8))
            }
            Tok::Template(raw) => { self.next()?; self.template(raw) }
            Tok::Punct(b'(') if arrow => {
                let at = self.pos();
                while self.tok != Tok::Punct(b'F') { self.next()?; }
                self.next()?;
                self.skim_body()?;
                Ok(Value::Func(self.define(b"", at)?))
            }
            Tok::Punct(b'(') => { self.next()?; let v = self.expr()?; self.expect(b')')?; Ok(v) }
            Tok::Punct(b'{') => self.object_literal(),
            Tok::Punct(b'[') => self.array_literal(),
            Tok::Ident(b"new") => self.new_expression(),
            Tok::Ident(b"function") => {
                self.next()?;
                // A named function expression can call itself by its name.
                let name = if let Tok::Ident(n) = self.tok { self.next()?; n } else { b"" };
                let at = self.pos();
                self.expect(b'(')?;
                self.skip_params()?;
                self.expect(b')')?;
                if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
                self.skim_body()?;
                Ok(Value::Func(self.define(name, at)?))
            }
            Tok::Ident(_) if arrow => {
                let at = self.pos();
                self.next()?;
                self.next()?;
                self.skim_body()?;
                Ok(Value::Func(self.define(b"", at)?))
            }
            Tok::Ident(name) => {
                self.next()?;
                match name {
                    b"NaN" => Ok(Value::Num(f64::NAN)),
                    b"null" => Ok(Value::Null),
                    b"this" => Ok(self.this),
                    b"super" => self.super_call(),
                    b"undefined" => Ok(Value::Undef),
                    b"Infinity" => Ok(Value::Num(f64::INFINITY)),
                    b"true" => Ok(Value::Bool(true)),
                    b"false" => Ok(Value::Bool(false)),
                    b"Math" => self.math(),
                    // `document.write(...)` as an expression (arrow bodies); its value is undefined.
                    b"document" if self.tok == Tok::Punct(b'.') => {
                        self.next()?;
                        let line = match self.tok { Tok::Ident(b"write") => false, Tok::Ident(b"writeln") => true, _ => return Err(Error::Unsupported) };
                        self.next()?;
                        let mut args = [Value::Undef; PARAMS];
                        let n = self.argument_list(&mut args)?;
                        if self.live {
                            for &v in &args[..n] { self.emit(v)?; }
                            if line { let nl = self.store(b"\n")?; self.emit(nl)?; }
                        }
                        Ok(Value::Undef)
                    }
                    b"Date" if self.tok == Tok::Punct(b'.') => self.date_static(),
                    b"JSON" | b"Object" | b"Array" | b"Number" | b"Promise" if self.tok == Tok::Punct(b'.') => self.statics(name),
                    b"String" if self.tok == Tok::Punct(b'.') => self.string_static(),
                    b"String" | b"Number" | b"parseInt" | b"parseFloat" | b"isNaN" | b"encodeURIComponent" | b"decodeURIComponent" | b"Boolean" if self.tok == Tok::Punct(b'(') => self.convert(name),
                    b"String" | b"Number" | b"Boolean" if self.lookup(name).is_none() => Ok(Value::Func(BUILTIN_FUNCS + match name { b"String" => 0, b"Number" => 1, _ => 2 })),
                    _ if self.tok == Tok::Punct(b'(') => self.call(name),
                    // `n++` / `n--` in an expression: the old value (a `++` on
                    // the next line starts a new statement instead).
                    _ if matches!(self.tok, Tok::Punct(b'I' | b'D')) && !self.nl => { let up = self.tok == Tok::Punct(b'I'); self.next()?; self.step_name(name, up, false) }
                    _ if self.tok == Tok::Punct(b'.') && self.lookup(name).is_none() && self.classes.iter().any(|c| c.0 == name) => self.class_static(name),
                    _ => self.lookup(name)
                        .or_else(|| self.funcs[..self.nfuncs].iter().position(|(n, l, _)| *l != 0 && &n[..*l as usize] == name).map(|i| Value::Func(i as u8)))
                        // Code that is only checked (not run) may name bindings that
                        // were never made, e.g. a loop's `let` on its last test.
                        .or((self.skim || !self.live).then_some(Value::Undef)).ok_or(Error::Unsupported),
                }
            }
            _ => Err(Error::Syntax),
        }
    }
    /// Call a declared function: arguments bind to its parameters for the
    /// call (shadowing, then restored); names it declares end with it.
    /// `name(args)`: a declared function, or a variable holding one.
    fn call(&mut self, name: &[u8]) -> Result<Value, Error> {
        let f = self.funcs[..self.nfuncs].iter().position(|(n, l, _)| *l != 0 && &n[..*l as usize] == name)
            .or_else(|| match self.lookup(name) { Some(Value::Func(i)) => Some(i as usize), _ => None });
        if f.is_none() && !self.skim { return Err(Error::Unsupported); }
        let mut args = [Value::Undef; PARAMS];
        let n = self.argument_list(&mut args)?;
        let (Some(f), true) = (f, self.live) else { return Ok(Value::Undef) };
        self.invoke(f as u8, &args[..n])
    }
    fn argument_list(&mut self, args: &mut [Value; PARAMS]) -> Result<usize, Error> {
        self.expect(b'(')?;
        self.items(b')', args)
    }
    /// A value leaving a call by `return`: function values defined inside the
    /// call (directly, or as members of a returned object or array) become
    /// closures over one copy of the call's variables, shared between them
    /// and kept across their calls. Without a running call, or with no
    /// function inside, the value is unchanged.
    fn escape(&mut self, v: Value) -> Result<Value, Error> {
        let Some(now) = self.func_now else { return Ok(v) };
        let start = self.funcs[now as usize].2.at;
        let inner = |m: &Self, x: Value| matches!(x, Value::Func(f) if (f as usize) < FUNCS && f != now && m.funcs[f as usize].2.at > start && m.funcs[f as usize].2.at < m.at);
        let mut env = None;
        let mut close = |m: &mut Self, x: Value| -> Result<Value, Error> {
            let Value::Func(f) = x else { return Ok(x) };
            if !inner(m, x) { return Ok(x); }
            let e = match env { Some(e) => e, None => {
                if m.envs.len() == ENVS { return Err(Error::Limit); }
                let mut vars = Vec::new();
                vars.try_reserve_exact(m.nvars - m.frame_base).map_err(|_| Error::Limit)?;
                vars.extend_from_slice(&m.vars[m.frame_base..m.nvars]);
                m.envs.push(vars);
                let e = (m.envs.len() - 1) as u8; env = Some(e); e
            } };
            if m.closures.len() == CLOSURES { return Err(Error::Limit); }
            m.closures.push((f, e));
            Ok(Value::Func((FUNCS + m.closures.len() - 1) as u8))
        };
        match v {
            Value::Func(_) => close(self, v),
            Value::Obj(id) => {
                let (start, len, _) = self.objects[id as usize];
                for k in start as usize..start as usize + len as usize { let x = self.props[k].2; self.props[k].2 = close(self, x)?; }
                Ok(v)
            }
            Value::Arr(id) => {
                let (start, len, _) = self.arrays[id as usize];
                for k in start as usize..start as usize + len as usize { let x = self.elems[k]; self.elems[k] = close(self, x)?; }
                Ok(v)
            }
            _ => Ok(v),
        }
    }
    /// The declared function behind a function value (closure or not).
    fn real_func(&self, f: u8) -> u8 { if f as usize >= FUNCS { self.closures[f as usize - FUNCS].0 } else { f } }
    /// Run function `f` with `args`. Parameters bind for the call (shadowing,
    /// then restored); names it declares end with it. Arrow functions with
    /// an expression body return that expression.
    fn invoke(&mut self, f: u8, args: &[Value]) -> Result<Value, Error> {
        let receiver = self.this_next.take().unwrap_or(Value::Undef);
        if (RESOLVERS..BUILTIN_FUNCS).contains(&f) {
            let (p, reject) = self.resolvers[(f - RESOLVERS) as usize];
            let v = args.first().copied().unwrap_or(Value::Undef);
            if reject { self.settle(p, 2, v)?; } else { self.resolve(p, v)?; }
            return Ok(Value::Undef);
        }
        if f >= BUILTIN_FUNCS {
            let v = args.first().copied().unwrap_or(Value::Undef);
            return Ok(match f - BUILTIN_FUNCS {
                0 => if args.is_empty() { self.store(b"")? } else { self.to_text(v)? },
                1 => Value::Num(if args.is_empty() { 0.0 } else { self.to_number(v) }),
                _ => Value::Bool(self.truthy(v)),
            });
        }
        if self.too_deep() { return Err(Error::Limit); }
        self.depth += 1;
        let back = self.pos();
        let base = self.nvars;
        // A closure starts with its captured variables (written back after).
        let (f, env) = if f as usize >= FUNCS { let (g, e) = self.closures[f as usize - FUNCS]; (g, Some(e as usize)) } else { (f, None) };
        if let Some(e) = env {
            for j in 0..self.envs[e].len() { let (n, l, v) = self.envs[e][j]; self.declare(&n[..l as usize], v, 0)?; }
        }
        let frame = core::mem::replace(&mut self.frame_base, base);
        self.restore(self.funcs[f as usize].2);
        let parens = self.tok == Tok::Punct(b'(');
        if parens { self.next()?; }
        let mut i = 0;
        loop {
            // `...rest` collects the remaining arguments; `p = d` defaults
            // an `undefined` argument (the default runs only then).
            let rest = parens && self.tok == Tok::Punct(b'S');
            if rest { self.next()?; }
            if parens && !rest && matches!(self.tok, Tok::Punct(b'[' | b'{')) {
                // `({x, y = 2})` / `([k, v])`: the argument is destructured.
                if i == PARAMS { return Err(Error::Limit); }
                let v = args.get(i).copied().unwrap_or(Value::Undef);
                self.pattern(Some(v))?;
                i += 1;
                if self.tok != Tok::Punct(b',') { break; }
                self.next()?;
                continue;
            }
            let Tok::Ident(param) = self.tok else { if rest { return Err(Error::Syntax); } break };
            if i == PARAMS { return Err(Error::Limit); }
            self.next()?;
            let mut v = args.get(i).copied().unwrap_or(Value::Undef);
            if rest { v = self.new_array(args.get(i..).unwrap_or(&[]))?; }
            else if parens && self.tok == Tok::Punct(b'=') {
                self.next()?;
                let missing = matches!(v, Value::Undef);
                let d = self.only_if(missing, |m| m.expr())?;
                if missing { v = d; }
            }
            // Parameters are new bindings of this call.
            self.declare(param, v, 0)?;
            i += 1;
            if rest || !parens || self.tok != Tok::Punct(b',') { break; }
            self.next()?;
        }
        if parens { self.expect(b')')?; }
        if self.tok == Tok::Punct(b'F') { self.next()?; }
        let outer = self.ret.take();
        let this = core::mem::replace(&mut self.this, receiver);
        let loops = core::mem::replace(&mut self.loops, 0);
        let live = core::mem::replace(&mut self.live, true);
        let now = self.func_now.replace(f);
        let body = if self.tok == Tok::Punct(b'{') { self.statement(true).map(|_| self.ret.take().unwrap_or(Value::Undef)) } else { self.expr().and_then(|v| self.escape(v)) };
        if let Some(e) = env { for j in 0..self.envs[e].len() { self.envs[e][j].2 = self.vars[base + j].2; } }
        self.frame_base = frame;
        self.func_now = now;
        self.live = live;
        self.loops = loops;
        self.this = this;
        self.ret = outer;
        self.nvars = base;
        self.restore(back);
        self.depth -= 1;
        body
    }
    /// Record a function (declaration, expression or arrow) whose parameter
    /// list starts at `at`; the same source position is one function.
    fn define(&mut self, name: &[u8], at: Pos<'a>) -> Result<u8, Error> {
        if let Some(i) = self.funcs[..self.nfuncs].iter().position(|(_, _, p)| p.at == at.at) { return Ok(i as u8); }
        if self.nfuncs == FUNCS || name.len() > NAME { return Err(Error::Limit); }
        let mut n = [0; NAME];
        n[..name.len()].copy_from_slice(name);
        self.funcs[self.nfuncs] = (n, name.len() as u8, at);
        self.nfuncs += 1;
        Ok((self.nfuncs - 1) as u8)
    }
    /// Check a body without running it (parameters may be any names).
    fn skim_body(&mut self) -> Result<(), Error> {
        let (skim, live) = (self.skim, self.live);
        self.skim = true;
        self.live = false;
        let body = if self.tok == Tok::Punct(b'{') { self.statement(false) } else { self.expr().map(|_| ()) };
        self.skim = skim;
        self.live = live;
        body
    }
    /// Skips a declared parameter list (`a, b = expr, ...rest`) up to `)`;
    /// defaults are only checked here.
    fn skip_params(&mut self) -> Result<(), Error> {
        loop {
            if self.tok == Tok::Punct(b'S') {
                self.next()?;
                let Tok::Ident(_) = self.tok else { return Err(Error::Syntax) };
                return self.next();
            }
            if matches!(self.tok, Tok::Punct(b'[' | b'{')) { self.pattern(None)?; }
            else {
                let Tok::Ident(_) = self.tok else { return Ok(()) };
                self.next()?;
            }
            if self.tok == Tok::Punct(b'=') { self.next()?; self.only_if(false, |m| m.expr())?; }
            if self.tok != Tok::Punct(b',') { return Ok(()); }
            self.next()?;
        }
    }
    /// After `(`: is this a parenthesised arrow parameter list?
    fn arrow_ahead(&mut self) -> Result<bool, Error> {
        let at = self.pos();
        let mut depth = 1;
        while depth > 0 {
            self.next()?;
            match self.tok { Tok::Punct(b'(') => depth += 1, Tok::Punct(b')') => depth -= 1, Tok::End => break, _ => {} }
        }
        let arrow = if depth == 0 { self.next()?; self.tok == Tok::Punct(b'F') } else { false };
        self.restore(at);
        Ok(arrow)
    }
    /// The innermost binding of `name` (later declarations shadow earlier).
    fn slot(&self, name: &[u8]) -> Option<usize> {
        self.vars[..self.nvars].iter().rposition(|(n, l, _)| &n[..*l as usize] == name)
    }
    fn lookup(&self, name: &[u8]) -> Option<Value> {
        self.slot(name).map(|k| self.vars[k].2)
    }
    fn assign(&mut self, name: &[u8], v: Value, declare: bool) -> Result<(), Error> {
        if let Some(k) = self.slot(name) {
            // Assigning to a `const` is a TypeError in JavaScript: refused.
            if self.kinds[k] == 2 && !declare { return Err(Error::Unsupported); }
            self.vars[k].2 = v;
            return Ok(());
        }
        if !declare { return Err(Error::Unsupported); }
        self.declare(name, v, 0)
    }
    /// `var name [= v]`: function scoped. A binding of this call (or, at the
    /// top level, a global) is reused and keeps its value when none is given;
    /// otherwise the name is new here, shadowing any outer one.
    fn bind_var(&mut self, name: &[u8], v: Option<Value>) -> Result<(), Error> {
        match self.slot(name).filter(|&k| k >= self.frame_base) {
            Some(k) if self.kinds[k] == 2 => Err(Error::Syntax),
            Some(k) => { if let Some(v) = v { self.vars[k].2 = v; } Ok(()) }
            None => self.declare(name, v.unwrap_or(Value::Undef), 0),
        }
    }
    /// A new binding; `let`/`const` (kind 1/2) shadow outer names.
    fn declare(&mut self, name: &[u8], v: Value, kind: u8) -> Result<(), Error> {
        if self.nvars == VARS || name.len() > NAME { return Err(Error::Limit); }
        let mut n = [0; NAME];
        n[..name.len()].copy_from_slice(name);
        self.vars[self.nvars] = (n, name.len() as u8, v);
        self.kinds[self.nvars] = kind;
        self.nvars += 1;
        Ok(())
    }
    /// Leaving a block: its `let`/`const` bindings end; `var`s stay.
    fn end_block(&mut self, since: usize) {
        let mut keep = since;
        for k in since..self.nvars {
            if self.kinds[k] == 0 { self.vars[keep] = self.vars[k]; self.kinds[keep] = 0; keep += 1; }
        }
        self.nvars = keep.max(since.min(self.nvars));
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
    fn too_deep(&self) -> bool { self.depth >= MAX_DEPTH || stack_low() }
    fn statement(&mut self, run: bool) -> Result<(), Error> {
        if self.too_deep() { return Err(Error::Limit); }
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
    /// `var [a, b] = x` / `var {a, b} = o`. (Statement kinds kept out of the recursive `statement_inner` frame.)
    #[inline(never)]
    fn destructure_statement(&mut self, run: bool) -> Result<(), Error> {
        // `var [a, , b = 2, ...rest] = array` or `var {x, y: alias = 1, ...rest} = object`.
        self.next()?;
        let at = self.pos();
        self.pattern(None)?;
        self.expect(b'=')?;
        let v = self.expr()?;
        if run {
            let end = self.pos();
            self.restore(at);
            self.pattern(Some(v))?;
            self.restore(end);
        }
        self.end()
    }
    /// A destructuring pattern at `[` or `{`: checks it, and with a value binds
    /// each name (defaults run only for an undefined item).
    #[inline(never)]
    fn pattern(&mut self, v: Option<Value>) -> Result<(), Error> {
        let close = if self.tok == Tok::Punct(b'[') { b']' } else if self.tok == Tok::Punct(b'{') { b'}' } else { return Err(Error::Syntax) };
        self.next()?;
        let mut seen: Vec<&'a [u8]> = Vec::new();
        let mut index = 0;
        while self.tok != Tok::Punct(close) {
            if close == b']' && self.tok == Tok::Punct(b',') { self.next()?; index += 1; continue; }
            let rest = self.tok == Tok::Punct(b'S');
            if rest { self.next()?; }
            let Tok::Ident(key) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            let mut name = key;
            let mut nested = None;
            if close == b'}' && !rest && self.tok == Tok::Punct(b':') {
                self.next()?;
                if matches!(self.tok, Tok::Punct(b'[' | b'{')) {
                    // `{a: {b}}` / `{a: [x, y]}`: a pattern for the member.
                    nested = Some(self.pos());
                    self.pattern(None)?;
                } else {
                    let Tok::Ident(alias) = self.tok else { return Err(Error::Syntax) };
                    self.next()?;
                    name = alias;
                }
            }
            if seen.len() == 8 || name.len() > NAME { return Err(Error::Limit); }
            let mut item = match (close, v, rest) {
                (_, None, _) => Value::Undef,
                (b']', Some(Value::Arr(id)), true) => { let (start, len, _) = self.arrays[id as usize]; let tail = self.elems[start as usize + index.min(len as usize)..start as usize + len as usize].to_vec(); self.new_array(&tail)? }
                (b']', Some(a @ Value::Arr(_)), false) => self.index(a, index as f64)?,
                (b'}', Some(Value::Obj(src)), true) => {
                    let id = self.new_object()?;
                    let (start, len, _) = self.objects[src as usize];
                    for k in start as usize..start as usize + len as usize {
                        let (ns, nn, value) = self.props[k];
                        let pk = self.arena[ns as usize..ns as usize + nn as usize].to_vec();
                        if seen.iter().any(|e| *e == &pk[..]) { continue; }
                        self.set_prop(id, &pk, value)?;
                    }
                    Value::Obj(id)
                }
                (b'}', Some(Value::Obj(id)), false) => self.get_prop(id, key),
                _ => return Err(Error::Unsupported),
            };
            if !rest && self.tok == Tok::Punct(b'=') {
                self.next()?;
                let missing = v.is_some() && matches!(item, Value::Undef);
                let d = self.only_if(missing, |m| m.expr())?;
                if missing { item = d; }
            }
            if let (Some(p), Some(_)) = (nested, v) {
                let back = self.pos();
                self.restore(p);
                self.pattern(Some(item))?;
                self.restore(back);
            } else if v.is_some() { self.bind_var(name, Some(item))?; }
            seen.push(key);
            index += 1;
            if rest || self.tok != Tok::Punct(b',') { break; }
            self.next()?;
        }
        self.expect(close)
    }
    /// `var|let|const a = 1, b;`
    #[inline(never)]
    fn declaration(&mut self, run: bool, keyword: &[u8]) -> Result<(), Error> {
        let kind = match keyword { b"var" => 0, b"let" => 1, _ => 2 };
        // `var a = 1, b;`: several names; only `const` needs a value.
        loop {
            self.next()?;
            let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
            if matches!(name, b"var" | b"let" | b"const" | b"document" | b"if" | b"else" | b"while" | b"for" | b"function" | b"return" | b"break" | b"continue" | b"JSON" | b"Object" | b"null" | b"undefined" | b"typeof" | b"do" | b"switch" | b"case" | b"default" | b"in" | b"delete" | b"Array" | b"this" | b"throw" | b"try" | b"catch" | b"finally" | b"new" | b"class" | b"Date" | b"Math" | b"true" | b"false") { return Err(Error::Syntax); }
            self.next()?;
            let v = if self.tok == Tok::Punct(b'=') { self.next()?; Some(self.expr()?) } else if kind == 2 { return Err(Error::Syntax) } else { None };
            if run {
                // `var` reuses an existing binding (and keeps its value when
                // none is given); `let`/`const` make a new one unless the
                // name is already bound in this same block scope.
                if kind == 0 { self.bind_var(name, v)?; }
                else if self.slot(name).is_some_and(|k| k >= self.block_base && self.kinds[k] != 0) { return Err(Error::Syntax); }
                else { self.declare(name, v.unwrap_or(Value::Undef), kind)?; }
            }
            if self.tok != Tok::Punct(b',') { break; }
        }
        self.end()
    }
    /// `document.write(...)` / `document.writeln(...)`.
    #[inline(never)]
    fn write_statement(&mut self, run: bool) -> Result<(), Error> {
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
    /// `class Name [extends Parent] { ... }`.
    #[inline(never)]
    fn class_statement(&mut self, run: bool) -> Result<(), Error> {
        self.next()?;
        let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        let parent = if self.tok == Tok::Ident(b"extends") {
            self.next()?;
            let Tok::Ident(p) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            // Only a class declared earlier can be extended.
            if run { Some(self.classes.iter().position(|c| c.0 == p).ok_or(Error::Unsupported)? as u8) } else { None }
        } else { None };
        self.expect(b'{')?;
        let (mut ctor, mut methods) = (None, Vec::new());
        let (mut statics, mut accessors) = (Vec::new(), Vec::new());
        while self.tok != Tok::Punct(b'}') {
            let Tok::Ident(mut m) = self.tok else { return Err(Error::Syntax) };
            self.next()?;
            // `static name(...)`, `get name()`, `set name(v)` (methods may
            // also be named `static`, `get` or `set`).
            let fixed = m == b"static" && matches!(self.tok, Tok::Ident(_));
            if fixed { let Tok::Ident(n) = self.tok else { return Err(Error::Syntax) }; m = n; self.next()?; }
            let access = !fixed && matches!(m, b"get" | b"set") && matches!(self.tok, Tok::Ident(_));
            let setter = m == b"set";
            if access { let Tok::Ident(n) = self.tok else { return Err(Error::Syntax) }; m = n; self.next()?; }
            let at = self.pos();
            self.expect(b'(')?;
            self.skip_params()?;
            self.expect(b')')?;
            if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
            self.skim_body()?;
            if run {
                let f = self.define(b"", at)?;
                if access { accessors.push((m, f, setter)); } else if fixed { statics.push((m, f)); } else if m == b"constructor" { ctor = Some(f); } else { methods.push((m, f)); }
            }
        }
        self.next()?;
        if run {
            if self.classes.len() == 16 { return Err(Error::Limit); }
            self.classes.push((name, ctor, methods, parent, statics, accessors));
        }
        Ok(())
    }
    /// `delete o.key` / `delete o[k]`.
    #[inline(never)]
    fn delete_statement(&mut self, run: bool) -> Result<(), Error> {
        self.next()?;
        let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        self.expect(b'.')?;
        let Tok::Ident(key) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        if run {
            let Some(Value::Obj(id)) = self.lookup(name) else { return Err(Error::Unsupported) };
            if let Some(k) = self.find_prop(id, key) {
                let (start, len, cap) = self.objects[id as usize];
                self.props.copy_within(k + 1..start as usize + len as usize, k);
                self.objects[id as usize] = (start, len - 1, cap);
            }
        }
        self.end()
    }
    /// `throw value`.
    #[inline(never)]
    fn throw_statement(&mut self, run: bool) -> Result<(), Error> {
        self.next()?;
        let v = self.expr()?;
        if run { self.thrown = Some(v); return Err(Error::Thrown); }
        self.end()
    }
    /// `try { } catch (e) { } finally { }`.
    #[inline(never)]
    fn try_statement(&mut self, run: bool) -> Result<(), Error> {
        self.next()?;
        if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
        let at = self.pos();
        let saved = (self.depth, self.loops, self.live, self.nvars, self.block_base);
        let caught = match self.statement(run) {
            Ok(()) => None,
            Err(Error::Thrown) => {
                // Unwind to the try block and pass over it without running it.
                (self.depth, self.loops, self.live, self.nvars, self.block_base) = saved;
                (self.jump, self.jump_label) = (None, None);
                self.restore(at);
                self.statement(false)?;
                self.thrown.take()
            }
            Err(e) => return Err(e),
        };
        if self.tok == Tok::Ident(b"catch") {
            self.next()?;
            let since = self.nvars;
            let mut name: Option<&'a [u8]> = None;
            if self.tok == Tok::Punct(b'(') {
                self.next()?;
                let Tok::Ident(n) = self.tok else { return Err(Error::Syntax) };
                name = Some(n);
                self.next()?;
                self.expect(b')')?;
            }
            if let (Some(n), Some(v)) = (name, caught) { self.declare(n, v, 1)?; }
            self.statement(run && caught.is_some())?;
            self.end_block(since);
        } else if caught.is_some() && self.tok != Tok::Ident(b"finally") {
            return Err(Error::Syntax);
        }
        if self.tok == Tok::Ident(b"finally") {
            self.next()?;
            self.statement(run)?;
        }
        Ok(())
    }
    /// `do body while (cond)`.
    #[inline(never)]
    fn do_statement(&mut self, run: bool) -> Result<(), Error> {
        let label = self.label_next.take();
        self.next()?;
        let top = self.pos();
        self.loops += 1;
        let mut go = run && self.ret.is_none();
        loop {
            self.restore(top);
            self.statement(go)?;
            let jump = if go { self.loop_jump(label) } else { None };
            if self.tok != Tok::Ident(b"while") { return Err(Error::Syntax); }
            self.next()?;
            self.expect(b'(')?;
            let c = self.expr()?;
            self.expect(b')')?;
            let again = go && jump != Some(true) && self.ret.is_none() && self.truthy(c);
            // JavaScript inserts the `;` after `do ... while (...)` itself.
            if !again { self.loops -= 1; if self.tok == Tok::Punct(b';') { self.next()?; } return Ok(()); }
            go = again;
        }
    }
    /// `switch (x) { case a: ... default: ... }`.
    #[inline(never)]
    fn switch_statement(&mut self, run: bool) -> Result<(), Error> {
        self.next()?;
        self.expect(b'(')?;
        let d = self.expr()?;
        self.expect(b')')?;
        self.expect(b'{')?;
        self.loops += 1;
        let mut matched = false;
        while self.tok != Tok::Punct(b'}') {
            match self.tok {
                Tok::Ident(b"case") => {
                    self.next()?;
                    let v = self.expr()?;
                    self.expect(b':')?;
                    if !matched { matched = matches!(self.apply(b'E', d, v)?, Value::Bool(true)); }
                }
                Tok::Ident(b"default") => { self.next()?; self.expect(b':')?; matched = true; }
                Tok::End => return Err(Error::Syntax),
                _ => {
                    // Fall through from the first match until `break`.
                    let live = run && matched && self.jump.is_none();
                    self.statement(live)?;
                }
            }
        }
        self.next()?;
        self.loops -= 1;
        // `break` ends the switch; `continue` belongs to an outer loop.
        if self.jump == Some(true) && self.jump_label.is_none() { self.jump = None; }
        Ok(())
    }
    /// `while (cond) body`.
    #[inline(never)]
    fn while_statement(&mut self, run: bool) -> Result<(), Error> {
        let label = self.label_next.take();
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
            if !go || self.loop_jump(label) == Some(true) { self.loops -= 1; return Ok(()); }
        }
    }
    fn statement_inner(&mut self, run: bool) -> Result<(), Error> {
        let destructure = matches!(self.tok, Tok::Ident(b"var" | b"let" | b"const")) && matches!(self.peek_after()?, Tok::Punct(b'[' | b'{'));
        let labeled = matches!(self.tok, Tok::Ident(n) if !matches!(n, b"default" | b"case" | b"else")) && self.peek_after()? == Tok::Punct(b':');
        match self.tok {
            Tok::Punct(b';') => self.next(),
            Tok::Punct(b'{') => {
                self.next()?;
                let since = self.nvars;
                let base = core::mem::replace(&mut self.block_base, since);
                while self.tok != Tok::Punct(b'}') {
                    if self.tok == Tok::End { return Err(Error::Syntax); }
                    self.statement(run)?;
                }
                self.end_block(since);
                self.block_base = base;
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
            Tok::Ident(b"var" | b"let" | b"const") if destructure => self.destructure_statement(run),
            Tok::Ident(keyword @ (b"var" | b"let" | b"const")) => self.declaration(run, keyword),
            Tok::Ident(b"document") => self.write_statement(run),
            Tok::Ident(b"class") => self.class_statement(run),
            Tok::Ident(b"function") => {
                self.next()?;
                let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
                self.next()?;
                let at = self.pos();
                self.expect(b'(')?;
                self.skip_params()?;
                self.expect(b')')?;
                if self.tok != Tok::Punct(b'{') { return Err(Error::Syntax); }
                self.skim_body()?;
                if run { self.define(name, at)?; }
                Ok(())
            }
            Tok::Ident(b"return") => {
                self.next()?;
                let v = if matches!(self.tok, Tok::Punct(b';' | b'}') | Tok::End) || self.nl { Value::Undef } else { self.expr()? };
                if run { let v = self.escape(v)?; self.ret = Some(v); }
                self.end()
            }
            Tok::Ident(kind @ (b"break" | b"continue")) => {
                self.next()?;
                // `break label` / `continue label` (on the same line).
                let label = match self.tok { Tok::Ident(l) if !self.nl => { self.next()?; Some(l) } _ => None };
                if (label.is_none() && self.loops == 0) || (label.is_some() && self.labels == 0) { return Err(Error::Syntax); }
                if run { self.jump = Some(kind == b"break"); self.jump_label = label; }
                self.end()
            }
            Tok::Ident(b"delete") => self.delete_statement(run),
            Tok::Ident(b"throw") => self.throw_statement(run),
            Tok::Ident(b"try") => self.try_statement(run),
            Tok::Ident(b"do") => self.do_statement(run),
            Tok::Ident(b"switch") => self.switch_statement(run),
            Tok::Ident(b"while") => self.while_statement(run),
            Tok::Ident(b"for") => {
                self.next()?;
                self.expect(b'(')?;
                let since = self.nvars;
                let base = core::mem::replace(&mut self.block_base, since);
                let r = self.for_loop(run);
                self.end_block(since);
                self.block_base = base;
                r
            }
            Tok::Ident(name) if labeled => {
                // `label: statement`; a labelled loop also takes `continue label`.
                self.next()?;
                self.next()?;
                if matches!(self.tok, Tok::Ident(b"for" | b"while" | b"do")) { self.label_next = Some(name); }
                self.labels += 1;
                let r = self.statement(run);
                self.labels -= 1;
                self.label_next = None;
                if self.jump_label == Some(name) { (self.jump, self.jump_label) = (None, None); }
                r
            }
            Tok::Ident(name) if !matches!(name, b"else" | b"new" | b"void" | b"typeof") => {
                // `f(...)` as a statement calls for its effects.
                let at = self.pos();
                self.next()?;
                // `o.a[b] = v` assigns; `o.f(...)`, `f(...)`, `xs[0](...)` call.
                let call = !self.assigns_ahead()? && matches!(self.tok, Tok::Punct(b'(' | b'.' | b'['));
                self.restore(at);
                if call { self.expr()?; } else { self.assignment(run)?; }
                self.end()
            }
            Tok::Punct(op @ (b'I' | b'D')) => {
                // `++x` / `--x` as a statement.
                self.next()?;
                let Tok::Ident(name) = self.tok else { return Err(Error::Unsupported) };
                self.next()?;
                if run { let old = self.lookup(name).ok_or(Error::Unsupported)?; self.assign(name, Value::Num(self.to_number(old) + if op == b'I' { 1.0 } else { -1.0 }), false)?; }
                self.end()
            }
            Tok::Punct(b'[' | b'(') | Tok::Num(_) | Tok::Str(_) | Tok::Template(_) | Tok::Ident(b"new" | b"void" | b"typeof") => { self.expr()?; self.end() }
            _ => Err(Error::Unsupported),
        }
    }
    /// After a loop body: the pending jump for this loop (`Some(true)` also
    /// for a labelled jump meant for an outer statement, which stays pending
    /// so the outer one sees it).
    fn loop_jump(&mut self, label: Option<&'a [u8]>) -> Option<bool> {
        match self.jump_label {
            Some(l) if Some(l) != label => self.jump.map(|_| true),
            _ => { self.jump_label = None; self.jump.take() }
        }
    }
    /// `for (init; cond; update) body` after the opening parenthesis.
    fn for_loop(&mut self, run: bool) -> Result<(), Error> {
        let label = self.label_next.take();
        // `for ([var|let|const] name of|in expr)`?
        let at = self.pos();
        let kind = match self.tok { Tok::Ident(b"var") => Some(0), Tok::Ident(b"let") => Some(1), Tok::Ident(b"const") => Some(2), _ => None };
        if kind.is_some() { self.next()?; }
        if let Tok::Ident(name) = self.tok {
            self.next()?;
            if let Tok::Ident(each @ (b"of" | b"in")) = self.tok { return self.for_each(run, kind.unwrap_or(0), name, None, each == b"of", label); }
        } else if kind.is_some() && matches!(self.tok, Tok::Punct(b'[' | b'{')) {
            // `for (const [k, v] of entries)`: a pattern bound per item.
            let pat = self.pos();
            self.pattern(None)?;
            if let Tok::Ident(each @ (b"of" | b"in")) = self.tok { return self.for_each(run, 0, b"", Some(pat), each == b"of", label); }
        }
        self.restore(at);
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
            if !go || self.loop_jump(label) == Some(true) { self.loops -= 1; return Ok(()); }
            self.restore(update);
            if self.tok != Tok::Punct(b')') { self.assignment(true)?; }
        }
    }
    /// `for (name of xs)` (array elements, string characters) and
    /// `for (name in o)` (object keys, array indices as strings), after the
    /// keyword. Items are read live, by position, as each iteration starts.
    fn for_each(&mut self, run: bool, kind: u8, name: &'a [u8], pat: Option<Pos<'a>>, of: bool, label: Option<&'a [u8]>) -> Result<(), Error> {
        self.next()?;
        let coll = self.expr()?;
        self.expect(b')')?;
        let body = self.pos();
        self.loops += 1;
        let mut i = 0usize;
        loop {
            self.restore(body);
            let item = if !run || self.ret.is_some() { None } else {
                match (coll, of) {
                    (Value::Arr(id), true) => { let (start, len, _) = self.arrays[id as usize]; (i < len as usize).then(|| self.elems[start as usize + i]) }
                    (Value::Obj(id), true) if matches!(self.made_by[id as usize], 65 | 66) => self.collection_item(id, i)?,
                    (Value::Str(_, n), true) => if i < n as usize { Some(self.index(coll, i as f64)?) } else { None },
                    (Value::Obj(id), false) => { let (start, len, _) = self.objects[id as usize]; if i < len as usize { let (ns, nn, _) = self.props[start as usize + i]; Some(Value::Str(ns, nn)) } else { None } }
                    (Value::Arr(id), false) => if i < self.arrays[id as usize].1 as usize { Some(self.number_text(i as f64)?) } else { None },
                    (Value::Null | Value::Undef, false) => None,
                    _ => return Err(Error::Unsupported),
                }
            };
            let go = item.is_some();
            if let (Some(v), Some(p)) = (item, pat) {
                let back = self.pos();
                self.restore(p);
                self.pattern(Some(v))?;
                self.restore(back);
            } else if let Some(v) = item {
                // `let`/`const`: one binding in the loop's block, set per item.
                match self.slot(name).filter(|&k| kind == 0 || k >= self.block_base) {
                    Some(k) => self.vars[k].2 = v,
                    None if kind == 0 => self.assign(name, v, true)?,
                    None => self.declare(name, v, kind)?,
                }
            }
            self.statement(go)?;
            if !go || self.loop_jump(label) == Some(true) { self.loops -= 1; return Ok(()); }
            i += 1;
        }
    }
    /// `name = expr`, `name += expr`, `name -= expr`, `name++`, `name--`.
    fn assignment(&mut self, run: bool) -> Result<Value, Error> {
        let Tok::Ident(name) = self.tok else { return Err(Error::Syntax) };
        self.next()?;
        let found = if name == b"this" { Some(self.this) } else { self.lookup(name) };
        let Some(old) = found.or((self.skim || !run).then_some(Value::Undef)) else { return Err(Error::Unsupported) };
        if !matches!(self.tok, Tok::Punct(b'.' | b'[')) {
            let v = self.update(old)?;
            if run { self.assign(name, v, false)?; }
            return Ok(v);
        }
        // A member chain `a.b[c].d op= v`: every key but the last is read.
        let mut base = old;
        loop {
            let key = if self.tok == Tok::Punct(b'.') {
                self.next()?;
                let Tok::Ident(k) = self.tok else { return Err(Error::Syntax) };
                self.next()?;
                Key::Name(k)
            } else {
                self.next()?;
                let i = self.expr()?;
                self.expect(b']')?;
                Key::Value(i)
            };
            let current = if run { self.get_member(base, key)? } else { Value::Undef };
            if matches!(self.tok, Tok::Punct(b'.' | b'[')) { base = current; continue; }
            let v = self.update(current)?;
            if run { self.set_member(base, key, v)?; }
            return Ok(v);
        }
    }
    fn key_text(&mut self, key: Key<'a>) -> Result<Vec<u8>, Error> {
        Ok(match key { Key::Name(k) => k.to_vec(), Key::Value(v) => { let t = self.to_text(v)?; self.text(t).to_vec() } })
    }
    /// `base.key` / `base[key]` as an assignment target reads it.
    fn get_member(&mut self, base: Value, key: Key<'a>) -> Result<Value, Error> {
        match (base, key) {
            (Value::Obj(id), k) => { let k = self.key_text(k)?; self.read_prop(id, &k) }
            (Value::Arr(id), Key::Name(b"length")) => Ok(Value::Num(self.arrays[id as usize].1 as f64)),
            (Value::Arr(_) | Value::Str(..), Key::Value(i)) => { let i = self.to_number(i); self.index(base, i) }
            _ => Err(Error::Unsupported),
        }
    }
    /// Stores into an object property, an array element (one past the end
    /// appends) or an array's `length` (shrinking only).
    fn set_member(&mut self, base: Value, key: Key<'a>, v: Value) -> Result<(), Error> {
        match (base, key) {
            // Assignments to a frozen object are ignored (sloppy mode).
            (Value::Obj(id), _) if self.frozen.contains(&id) => Ok(()),
            (Value::Obj(id), k) => {
                let k = self.key_text(k)?;
                match self.accessor(id, &k) {
                    // A setter runs with `this` = the object; without one the
                    // assignment is ignored, as in sloppy-mode JavaScript.
                    Some((_, Some(f))) => { self.this_next = Some(Value::Obj(id)); self.invoke(f, &[v]).map(|_| ()) }
                    Some((_, None)) => Ok(()),
                    None => self.set_prop(id, &k, v),
                }
            }
            (Value::Arr(id), Key::Name(b"length")) => {
                let (start, len, cap) = self.arrays[id as usize];
                let n = self.to_number(v);
                if !(n >= 0.0 && n == floor(n) && n <= len as f64) { return Err(Error::Unsupported); }
                self.arrays[id as usize] = (start, n as u16, cap);
                Ok(())
            }
            (Value::Arr(id), Key::Value(i)) => {
                let i = self.to_number(i);
                let (start, len, _) = self.arrays[id as usize];
                if !(i >= 0.0 && i == floor(i) && i < 65536.0) { return Err(Error::Unsupported); }
                if (i as u16) < len { self.elems[start as usize + i as usize] = v; Ok(()) }
                else if i as u16 == len { self.push(id, v) }
                else { Err(Error::Unsupported) }
            }
            _ => Err(Error::Unsupported),
        }
    }
    /// After a name: does a member chain (`.a[b].c`) lead to an assignment?
    fn assigns_ahead(&mut self) -> Result<bool, Error> { self.assigns_ahead_with(true) }
    fn assigns_ahead_with(&mut self, steps: bool) -> Result<bool, Error> {
        let at = self.pos();
        loop {
            match self.tok {
                Tok::Punct(b'.') => { self.next()?; if !matches!(self.tok, Tok::Ident(_)) { break; } self.next()?; }
                Tok::Punct(b'[') => {
                    let mut depth = 0;
                    loop {
                        match self.tok { Tok::Punct(b'[') => depth += 1, Tok::Punct(b']') => depth -= 1, Tok::End => break, _ => {} }
                        self.next()?;
                        if depth == 0 { break; }
                    }
                }
                _ => break,
            }
        }
        let assigns = matches!(self.tok, Tok::Punct(b'=' | b'P' | b'M' | b'T' | b'Q' | b'R' | b'W' | b'v' | b'w' | b'x' | b'y' | b'z' | b'q'))
            || (steps && matches!(self.tok, Tok::Punct(b'I' | b'D')));
        self.restore(at);
        Ok(assigns)
    }
    /// The new value for `= e`, `+= e`, `-= e`, `++` or `--` applied to `old`.
    fn update(&mut self, old: Value) -> Result<Value, Error> {
        Ok(match self.tok {
            Tok::Punct(b'=') => { self.next()?; self.expr()? }
            Tok::Punct(op @ (b'P' | b'M' | b'T' | b'Q' | b'R' | b'W' | b'v' | b'w' | b'x')) => {
                self.next()?;
                let r = self.expr()?;
                self.apply(match op { b'P' => b'+', b'M' => b'-', b'T' => b'*', b'Q' => b'/', b'W' => b'X', b'v' => b'o', b'w' => b'a', b'x' => b'^', _ => b'%' }, old, r)?
            }
            Tok::Punct(op @ (b'y' | b'z' | b'q')) => {
                // `||= &&= ??=` evaluate the right side only when it is used.
                self.next()?;
                let keep = match op { b'y' => self.truthy(old), b'z' => !self.truthy(old), _ => !matches!(old, Value::Undef | Value::Null) };
                let r = self.only_if(!keep, |m| m.expr())?;
                if keep { old } else { r }
            }
            Tok::Punct(op @ (b'I' | b'D')) => { self.next()?; Value::Num(self.to_number(old) + if op == b'I' { 1.0 } else { -1.0 }) }
            _ => return Err(Error::Syntax),
        })
    }
    /// The token after the current one, without consuming anything.
    fn peek_after(&mut self) -> Result<Tok<'a>, Error> {
        let at = self.pos();
        self.next()?;
        let t = self.tok;
        self.restore(at);
        Ok(t)
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
struct Text<'v>(&'v mut Vec<u8>);
impl Write for Text<'_> { fn write_str(&mut self, s: &str) -> core::fmt::Result { self.0.extend_from_slice(s.as_bytes()); Ok(()) } }

/// Days since 1970-01-01 for a proleptic Gregorian date (month 1-12).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
/// (year, month 1-12, day) for days since 1970-01-01.
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}
/// Milliseconds for (year, month 0-11, day, h, m, s, ms), UTC; months and
/// days roll over like JavaScript's Date.
fn date_ms(v: [f64; 7]) -> f64 {
    if v.iter().any(|x| !x.is_finite()) { return f64::NAN; }
    let (mut y, mut mo) = (floor(v[0]) as i64, floor(v[1]) as i64);
    y += mo.div_euclid(12);
    mo = mo.rem_euclid(12);
    let days = days_from_civil(y, mo + 1, 1) as f64 + floor(v[2]) - 1.0;
    days * 86_400_000.0 + floor(v[3]) * 3_600_000.0 + floor(v[4]) * 60_000.0 + floor(v[5]) * 1000.0 + floor(v[6])
}
/// `YYYY-MM-DD` with optional `THH:MM[:SS[.mmm]]` and `Z` (UTC).
fn parse_iso(t: &[u8]) -> Option<f64> {
    let num = |a: usize, n: usize| -> Option<f64> { let s = t.get(a..a + n)?; if !s.iter().all(u8::is_ascii_digit) { return None; } core::str::from_utf8(s).ok()?.parse().ok() };
    if t.len() < 10 || t[4] != b'-' || t[7] != b'-' { return None; }
    let mut v = [num(0, 4)?, num(5, 2)? - 1.0, num(8, 2)?, 0.0, 0.0, 0.0, 0.0];
    let mut at = 10;
    if t.get(at) == Some(&b'T') {
        v[3] = num(at + 1, 2)?; v[4] = num(at + 4, 2)?; at += 6;
        if t.get(at) == Some(&b':') { v[5] = num(at + 1, 2)?; at += 3; }
        if t.get(at) == Some(&b'.') { v[6] = num(at + 1, 3)?; at += 4; }
        if t.get(at) == Some(&b'Z') { at += 1; }
    }
    if at != t.len() || v[1] > 11.0 || v[2] < 1.0 || v[2] > 31.0 { return None; }
    Some(date_ms(v))
}

fn json_ws(t: &[u8], at: &mut usize) { while t.get(*at).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r')) { *at += 1; } }

/// Digits, one dot, optional sign and exponent: what parseFloat accepts
/// (Rust's parser also takes "inf"/"nan", which JavaScript does not).
fn s_is_plain(t: &[u8]) -> bool {
    t.iter().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) && t.iter().any(u8::is_ascii_digit)
}
/// The closing backtick of a template starting at `i`: skips `\x` escapes
/// and `${...}` substitutions (with their own strings, braces and nested
/// templates).
fn template_end(s: &[u8], mut i: usize) -> Option<usize> {
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'`' => return Some(i),
            b'$' if s.get(i + 1) == Some(&b'{') => i = substitution_end(s, i + 2)?,
            _ => i += 1,
        }
    }
    None
}
/// Just past the `}` closing a `${` whose body starts at `i`.
fn substitution_end(s: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 1;
    while i < s.len() {
        match s[i] {
            b'{' => depth += 1,
            b'}' => { depth -= 1; if depth == 0 { return Some(i + 1); } }
            q @ (b'\'' | b'"') => {
                i += 1;
                while *s.get(i)? != q { if s[i] == b'\\' { i += 1; } i += 1; }
            }
            b'`' => i = template_end(s, i + 1)?,
            _ => {}
        }
        i += 1;
    }
    None
}
/// JavaScript ToInt32 (for the bitwise operators).
fn int32(x: f64) -> i32 {
    if !x.is_finite() { return 0; }
    let t = if x < 0.0 { -floor(-x) } else { floor(x) };
    let m = t - floor(t / 4294967296.0) * 4294967296.0;
    m as u32 as i32
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
    // The Browser heap is 64 KiB and shared with TLS and the page: refuse a
    // script unless room for its working set (and some slack) exists now.
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(HEADROOM).map_err(|_| Error::Limit)?;
    drop(probe);
    let (mut arena, mut out) = (Vec::new(), Vec::new());
    arena.try_reserve_exact(ARENA).map_err(|_| Error::Limit)?;
    out.try_reserve_exact(MAX_OUTPUT).map_err(|_| Error::Limit)?;
    let mut elems = Vec::new();
    elems.try_reserve_exact(ELEMS).map_err(|_| Error::Limit)?;
    let mut props = Vec::new();
    props.try_reserve_exact(PROPS).map_err(|_| Error::Limit)?;
    // Fixed tables live on the heap too: the Browser stack is for nesting.
    fn table<T: Clone>(n: usize, v: T) -> Result<Vec<T>, Error> { let mut t = Vec::new(); t.try_reserve_exact(n).map_err(|_| Error::Limit)?; t.resize(n, v); Ok(t) }
    let mut m = Machine { src: source, at: 0, tok: Tok::End, nl: false, arena, vars: table(VARS, ([0; NAME], 0, Value::Num(0.0)))?, kinds: table(VARS, 0)?, block_base: 0, nvars: 0, out, depth: 0, steps: 0,
        funcs: table(FUNCS, ([0; NAME], 0, Pos { at: 0, tok: Tok::End, nl: false }))?, nfuncs: 0, skim: false, live: true, ret: None, jump: None, jump_label: None, label_next: None, labels: 0, this: Value::Undef, this_next: None, thrown: None, regexes: Vec::new(), re_last: Vec::new(), frozen: Vec::new(), classes: Vec::new(), accessors: Vec::new(), promises: Vec::new(), jobs: Vec::new(), resolvers: Vec::new(), gathers: Vec::new(), collections: Vec::new(), made_by: table(OBJECTS, 0)?, func_now: None, frame_base: 0, closures: Vec::new(), envs: Vec::new(), loops: 0,
        elems, arrays: table(ARRAYS, (0, 0, 0))?, narrays: 0, props, objects: table(OBJECTS, (0, 0, 0))?, nobjects: 0 };
    m.next()?;
    while m.tok != Tok::End { m.statement(true)?; }
    m.drain_jobs()?;
    Ok(m.out)
}
