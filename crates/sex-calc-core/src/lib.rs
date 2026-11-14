// SPDX-License-Identifier: GPL-3.0-only
//! The calculator's arithmetic, apart from its window so the host can test it.
//!
//! An expression is read by recursive descent, lowest precedence first:
//!
//! ```text
//! sum     = product (('+' | '-') product)*
//! product = unary (('*' | '/' | '%') unary | implicit unary)*   "2pi", "3(4+1)", "2sin(30)"
//! unary   = ('-' | '+') unary | power                           so -2^2 is -4
//! power   = postfix ('^' unary)?                                right to left, 2^-1 works
//! postfix = primary '!'*
//! primary = number | '(' sum ')' | function '(' sum ')' | pi | e | ans | mem
//! ```
//!
//! `%` is the remainder (sign of the dividend). Trigonometry follows the angle mode:
//! in degrees the inverse functions answer in degrees too, and whole multiples of 90
//! give exact answers (sin 180 is 0, tan 90 is refused) rather than rounding noise.
//! Nothing here allocates; a result becomes text with [`format`].
#![no_std]

use core::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Angle { Radians, Degrees }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Nothing to work out.
    Empty,
    /// A character or arrangement the grammar does not allow.
    Syntax,
    /// A `(` without its `)`, or the other way round.
    Unbalanced,
    DivideByZero,
    /// Outside a function's domain: sqrt(-1), ln 0, asin 2, tan 90°, (-1)!.
    Domain,
    /// A name that is neither a function nor a constant.
    Unknown,
    /// The answer is too large to hold (or not a number at all).
    Overflow,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Error::Empty => "Nothing to calculate",
            Error::Syntax => "Syntax error",
            Error::Unbalanced => "Unbalanced parentheses",
            Error::DivideByZero => "Division by zero",
            Error::Domain => "Outside the function's domain",
            Error::Unknown => "Unknown name",
            Error::Overflow => "Too large",
        }
    }
}

/// What an expression may refer to besides itself.
#[derive(Clone, Copy, Debug)]
pub struct Context { pub angle: Angle, pub ans: f64, pub memory: f64 }

impl Default for Context {
    fn default() -> Self { Context { angle: Angle::Radians, ans: 0.0, memory: 0.0 } }
}

/// Works `expr` out. Parentheses are checked first, so a missing one is reported as
/// such wherever the parse would otherwise have stumbled.
pub fn eval(expr: &str, ctx: &Context) -> Result<f64, Error> {
    let bytes = expr.as_bytes();
    let mut depth = 0i32;
    for &b in bytes {
        depth += (b == b'(') as i32 - (b == b')') as i32;
        if depth < 0 { return Err(Error::Unbalanced); }
    }
    if depth != 0 { return Err(Error::Unbalanced); }
    let mut p = Parser { s: bytes, at: 0, ctx };
    p.skip();
    if p.at == bytes.len() { return Err(Error::Empty); }
    let value = p.sum()?;
    p.skip();
    if p.at != bytes.len() { return Err(Error::Syntax); }
    finite(value)
}

fn finite(v: f64) -> Result<f64, Error> { if v.is_finite() { Ok(v) } else { Err(Error::Overflow) } }

struct Parser<'a> { s: &'a [u8], at: usize, ctx: &'a Context }

impl Parser<'_> {
    fn skip(&mut self) { while self.s.get(self.at).is_some_and(|b| b.is_ascii_whitespace()) { self.at += 1; } }
    fn peek(&mut self) -> Option<u8> { self.skip(); self.s.get(self.at).copied() }
    fn eat(&mut self, c: u8) -> bool { if self.peek() == Some(c) { self.at += 1; true } else { false } }

    fn sum(&mut self) -> Result<f64, Error> {
        let mut v = self.product()?;
        loop {
            if self.eat(b'+') { v += self.product()?; }
            else if self.eat(b'-') { v -= self.product()?; }
            else { return Ok(v); }
        }
    }
    fn product(&mut self) -> Result<f64, Error> {
        let mut v = self.unary()?;
        loop {
            match self.peek() {
                Some(b'*') => { self.at += 1; v *= self.unary()?; }
                Some(b'/') => { self.at += 1; let d = self.unary()?; if d == 0.0 { return Err(Error::DivideByZero); } v /= d; }
                Some(b'%') => { self.at += 1; let d = self.unary()?; if d == 0.0 { return Err(Error::DivideByZero); } v = libm::fmod(v, d); }
                // A bracket or a name straight after a value multiplies it.
                Some(b'(') => v *= self.unary()?,
                Some(c) if c.is_ascii_alphabetic() => v *= self.unary()?,
                _ => return Ok(v),
            }
        }
    }
    fn unary(&mut self) -> Result<f64, Error> {
        if self.eat(b'-') { return Ok(-self.unary()?); }
        if self.eat(b'+') { return self.unary(); }
        self.power()
    }
    fn power(&mut self) -> Result<f64, Error> {
        let base = self.postfix()?;
        if !self.eat(b'^') { return Ok(base); }
        let exponent = self.unary()?;
        if base == 0.0 && exponent < 0.0 { return Err(Error::DivideByZero); }
        let v = libm::pow(base, exponent);
        // A negative number to a fractional power has no real answer.
        if v.is_nan() { return Err(Error::Domain); }
        finite(v)
    }
    fn postfix(&mut self) -> Result<f64, Error> {
        let mut v = self.primary()?;
        while self.eat(b'!') { v = factorial(v)?; }
        Ok(v)
    }
    fn primary(&mut self) -> Result<f64, Error> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                let v = self.sum()?;
                if !self.eat(b')') { return Err(Error::Unbalanced); }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            Some(c) if c.is_ascii_alphabetic() => {
                let start = self.at;
                while self.s.get(self.at).is_some_and(|b| b.is_ascii_alphanumeric()) { self.at += 1; }
                let name = &self.s[start..self.at];
                match name {
                    b"pi" => return Ok(core::f64::consts::PI),
                    b"e" => return Ok(core::f64::consts::E),
                    b"ans" => return Ok(self.ctx.ans),
                    b"mem" => return Ok(self.ctx.memory),
                    _ => {}
                }
                let f = FUNCTIONS.iter().find(|(n, _)| n.as_bytes() == name).ok_or(Error::Unknown)?.1;
                // A function's argument is in brackets, so `sin 30` is not read as sin·30.
                if self.peek() != Some(b'(') { return Err(Error::Syntax); }
                let x = self.primary()?;
                self.apply(f, x)
            }
            _ => Err(Error::Syntax),
        }
    }
    fn number(&mut self) -> Result<f64, Error> {
        let start = self.at;
        let digits = |p: &mut Self| { let s = p.at; while p.s.get(p.at).is_some_and(u8::is_ascii_digit) { p.at += 1; } p.at - s };
        let mut n = digits(self);
        if self.s.get(self.at) == Some(&b'.') { self.at += 1; n += digits(self); }
        if n == 0 { return Err(Error::Syntax); }
        // `2e3` is two thousand; `2e` alone is two times e, left for the caller.
        if self.s.get(self.at) == Some(&b'e') {
            let mut k = self.at + 1;
            if matches!(self.s.get(k), Some(b'+' | b'-')) { k += 1; }
            if self.s.get(k).is_some_and(u8::is_ascii_digit) { self.at = k; digits(self); }
        }
        let text = core::str::from_utf8(&self.s[start..self.at]).map_err(|_| Error::Syntax)?;
        finite(text.parse::<f64>().map_err(|_| Error::Syntax)?)
    }
    fn apply(&self, f: Function, x: f64) -> Result<f64, Error> {
        let deg = self.ctx.angle == Angle::Degrees;
        let to_rad = |v: f64| if deg { v * core::f64::consts::PI / 180.0 } else { v };
        let from_rad = |v: f64| if deg { v * 180.0 / core::f64::consts::PI } else { v };
        let v = match f {
            Function::Sin | Function::Cos | Function::Tan if deg && libm::fmod(x, 90.0) == 0.0 => {
                // Exact on the axes: the quarter turn the angle lands on.
                let quarter = libm::fmod(libm::fmod(x / 90.0, 4.0) + 4.0, 4.0) as u8;
                let (s, c) = [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][quarter as usize];
                match f { Function::Sin => s, Function::Cos => c, _ => if c == 0.0 { return Err(Error::Domain) } else { s / c } }
            }
            Function::Sin => libm::sin(to_rad(x)),
            Function::Cos => libm::cos(to_rad(x)),
            Function::Tan => libm::tan(to_rad(x)),
            Function::Asin | Function::Acos if !(-1.0..=1.0).contains(&x) => return Err(Error::Domain),
            Function::Asin => from_rad(libm::asin(x)),
            Function::Acos => from_rad(libm::acos(x)),
            Function::Atan => from_rad(libm::atan(x)),
            Function::Sqrt if x < 0.0 => return Err(Error::Domain),
            Function::Sqrt => libm::sqrt(x),
            Function::Ln | Function::Log if x <= 0.0 => return Err(Error::Domain),
            Function::Ln => libm::log(x),
            Function::Log => libm::log10(x),
            Function::Exp => libm::exp(x),
            Function::Abs => libm::fabs(x),
        };
        finite(v)
    }
}

#[derive(Clone, Copy)]
enum Function { Sin, Cos, Tan, Asin, Acos, Atan, Sqrt, Ln, Log, Exp, Abs }
const FUNCTIONS: [(&str, Function); 11] = [
    ("sin", Function::Sin), ("cos", Function::Cos), ("tan", Function::Tan),
    ("asin", Function::Asin), ("acos", Function::Acos), ("atan", Function::Atan),
    ("sqrt", Function::Sqrt), ("ln", Function::Ln), ("log", Function::Log),
    ("exp", Function::Exp), ("abs", Function::Abs),
];

/// n! for whole n up to 170 (171! is beyond f64).
fn factorial(n: f64) -> Result<f64, Error> {
    if n < 0.0 || n != libm::floor(n) { return Err(Error::Domain); }
    if n > 170.0 { return Err(Error::Overflow); }
    Ok((2..=n as u32).fold(1.0, |acc, k| acc * k as f64))
}

/// Room for any result [`format`] writes.
pub const FORMAT_BYTES: usize = 32;

struct Buf<'a> { out: &'a mut [u8], len: usize }
impl Write for Buf<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        if end > self.out.len() { return Err(core::fmt::Error); }
        self.out[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// A result as a person reads it: twelve significant digits at most, no trailing
/// zeros, plain notation from 1e-6 up to 1e12 and scientific (`1.5e20`) outside it.
/// Rounding to twelve digits hides binary noise, so 0.1+0.2 shows as 0.3.
pub fn format(v: f64, out: &mut [u8; FORMAT_BYTES]) -> &str {
    if v == 0.0 { out[0] = b'0'; return core::str::from_utf8(&out[..1]).unwrap(); }
    if !v.is_finite() { out[..3].copy_from_slice(b"NaN"); return core::str::from_utf8(&out[..3]).unwrap(); }
    let mut sci = [0u8; FORMAT_BYTES];
    let mut b = Buf { out: &mut sci, len: 0 };
    let _ = write!(b, "{:.11e}", libm::fabs(v));
    let n = b.len;
    let text = &sci[..n];
    let e_at = text.iter().position(|c| *c == b'e').unwrap_or(n);
    let exp: i32 = core::str::from_utf8(&text[e_at + 1..]).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut digits = [0u8; 12];
    let mut d = 0;
    for &c in &text[..e_at] { if c.is_ascii_digit() && d < 12 { digits[d] = c; d += 1; } }
    while d > 1 && digits[d - 1] == b'0' { d -= 1; }
    let digits = &digits[..d];
    let mut o = Buf { out: &mut out[..], len: 0 };
    fn s(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("") }
    if v < 0.0 { let _ = o.write_str("-"); }
    if (-6..12).contains(&exp) {
        if exp >= 0 {
            let whole = exp as usize + 1;
            let _ = o.write_str(s(&digits[..whole.min(d)]));
            for _ in d..whole { let _ = o.write_str("0"); }
            if d > whole { let _ = o.write_str("."); let _ = o.write_str(s(&digits[whole..])); }
        } else {
            let _ = o.write_str("0.");
            for _ in 0..(-exp - 1) { let _ = o.write_str("0"); }
            let _ = o.write_str(s(digits));
        }
    } else {
        let _ = o.write_str(s(&digits[..1]));
        if d > 1 { let _ = o.write_str("."); let _ = o.write_str(s(&digits[1..])); }
        let _ = write!(o, "e{}", exp);
    }
    let len = o.len;
    core::str::from_utf8(&out[..len]).unwrap_or("?")
}

/// How many past answers the calculator keeps.
pub const HISTORY: usize = 20;
/// The longest expression a history entry keeps; longer ones keep their start.
pub const EXPR_MAX: usize = 96;

#[derive(Clone, Copy)]
pub struct Entry { expr: [u8; EXPR_MAX], len: u8, pub value: f64 }

impl Entry {
    pub fn expr(&self) -> &str { core::str::from_utf8(&self.expr[..self.len as usize]).unwrap_or("") }
}

/// A calculator's state: the angle mode, the last answer, the memory and the history.
pub struct Calculator {
    pub ctx: Context,
    entries: [Entry; HISTORY],
    count: usize,
    next: usize,
}

impl Default for Calculator { fn default() -> Self { Self::new() } }

impl Calculator {
    pub const fn new() -> Self {
        Calculator {
            ctx: Context { angle: Angle::Radians, ans: 0.0, memory: 0.0 },
            entries: [Entry { expr: [0; EXPR_MAX], len: 0, value: 0.0 }; HISTORY],
            count: 0,
            next: 0,
        }
    }
    /// Works `expr` out; a good answer becomes `ans` and the newest history entry.
    pub fn evaluate(&mut self, expr: &str) -> Result<f64, Error> {
        let value = eval(expr, &self.ctx)?;
        self.ctx.ans = value;
        let trimmed = expr.trim();
        // Cut on a character boundary so the entry stays valid text.
        let mut n = trimmed.len().min(EXPR_MAX);
        while !trimmed.is_char_boundary(n) { n -= 1; }
        let entry = &mut self.entries[self.next];
        entry.expr[..n].copy_from_slice(&trimmed.as_bytes()[..n]);
        entry.len = n as u8;
        entry.value = value;
        self.next = (self.next + 1) % HISTORY;
        self.count = (self.count + 1).min(HISTORY);
        Ok(value)
    }
    pub fn history_len(&self) -> usize { self.count }
    /// The `i`th newest entry: 0 is the last answer.
    pub fn history(&self, i: usize) -> Option<&Entry> {
        (i < self.count).then(|| &self.entries[(self.next + HISTORY - 1 - i) % HISTORY])
    }
    pub fn clear_history(&mut self) { self.count = 0; self.next = 0; }
    pub fn toggle_angle(&mut self) {
        self.ctx.angle = if self.ctx.angle == Angle::Radians { Angle::Degrees } else { Angle::Radians };
    }
    pub fn memory_add(&mut self, v: f64) { self.ctx.memory += v; }
    pub fn memory_subtract(&mut self, v: f64) { self.ctx.memory -= v; }
    pub fn memory_clear(&mut self) { self.ctx.memory = 0.0; }
    pub fn memory(&self) -> f64 { self.ctx.memory }
}
