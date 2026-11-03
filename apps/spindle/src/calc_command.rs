// SPDX-License-Identifier: GPL-3.0-only
//! `calc <expression>`: exact decimal arithmetic on what the user typed.
//!
//! `ans` is the previous successful result in this Spindle session.
//! `**` raises to a whole power up to 64; `sqrt(x)` is the square root;
//! `abs min max round floor ceil` work on the exact values.
//! Numbers are fixed point with 9 decimal places in an i128, so `0.1+0.2`
//! is exactly `0.3`. Operators: `+ - * / %`, unary minus and parentheses.
//! Parsing is recursive descent with a depth bound; every operation is
//! checked, and overflow, division by zero or malformed input is an error,
//! never a wrapped or rounded-away answer (division rounds toward zero at
//! the 9th decimal and says so with `~`).

const SCALE: i128 = 1_000_000_000;
const MAX_DEPTH: u8 = 16;
pub const LINE_MAX: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Syntax, DivideByZero, Overflow, TooDeep, NoAnswer, Exponent, Domain }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value { pub scaled: i128, pub inexact: bool }

struct Parser<'a> { s: &'a [u8], at: usize, depth: u8, inexact: bool, ans: Option<i128> }

impl Parser<'_> {
    fn skip(&mut self) { while self.at < self.s.len() && self.s[self.at] == b' ' { self.at += 1; } }
    fn peek(&mut self) -> Option<u8> { self.skip(); self.s.get(self.at).copied() }
    fn expr(&mut self) -> Result<i128, Error> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.at += 1;
            let r = self.term()?;
            v = if op == b'+' { v.checked_add(r) } else { v.checked_sub(r) }.ok_or(Error::Overflow)?;
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<i128, Error> {
        let mut v = self.unary()?;
        while let Some(op @ (b'*' | b'/' | b'%')) = self.peek() {
            if self.s[self.at..].starts_with(b"**") { break; }
            self.at += 1;
            let r = self.unary()?;
            v = match op {
                b'*' => self.mul(v, r)?,
                b'/' => {
                    if r == 0 { return Err(Error::DivideByZero); }
                    let n = v.checked_mul(SCALE).ok_or(Error::Overflow)?;
                    if n % r != 0 { self.inexact = true; }
                    n / r
                }
                _ => { if r == 0 { return Err(Error::DivideByZero); } v % r }
            };
        }
        Ok(v)
    }
    fn unary(&mut self) -> Result<i128, Error> {
        if self.peek() == Some(b'-') { self.at += 1; return self.guard(|p| p.unary())?.checked_neg().ok_or(Error::Overflow); }
        if self.peek() == Some(b'+') { self.at += 1; return self.guard(|p| p.unary()); }
        self.power()
    }
    /// `a ** n` for a whole exponent |n| <= 64 (right-associative, binds
    /// tighter than unary minus: `-2**2` is -4).
    fn power(&mut self) -> Result<i128, Error> {
        let base = self.atom()?;
        if !(self.peek() == Some(b'*') && self.s[self.at..].starts_with(b"**")) { return Ok(base); }
        self.at += 2;
        let e = self.guard(|p| p.unary())?;
        if e % SCALE != 0 || (e / SCALE).abs() > 64 { return Err(Error::Exponent); }
        let n = e / SCALE;
        let mut v = SCALE;
        for _ in 0..n.abs() { v = self.mul(v, base)?; }
        if n < 0 {
            if v == 0 { return Err(Error::DivideByZero); }
            let num = SCALE.checked_mul(SCALE).ok_or(Error::Overflow)?;
            if num % v != 0 { self.inexact = true; }
            v = num / v;
        }
        Ok(v)
    }
    fn mul(&mut self, v: i128, r: i128) -> Result<i128, Error> {
        let p = v.checked_mul(r).ok_or(Error::Overflow)?;
        if p % SCALE != 0 { self.inexact = true; }
        Ok(p / SCALE)
    }
    fn guard(&mut self, f: impl FnOnce(&mut Self) -> Result<i128, Error>) -> Result<i128, Error> {
        if self.depth >= MAX_DEPTH { return Err(Error::TooDeep); }
        self.depth += 1; let r = f(self); self.depth -= 1; r
    }
    fn atom(&mut self) -> Result<i128, Error> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                let v = self.guard(|p| p.expr())?;
                if self.peek() != Some(b')') { return Err(Error::Syntax); }
                self.at += 1; Ok(v)
            }
            Some(b'0'..=b'9' | b'.') => self.number(),
            Some(b'a' | b'm' | b'r' | b'f' | b'c') if [&b"abs("[..], b"min(", b"max(", b"round(", b"floor(", b"ceil("].iter().any(|f| self.s[self.at..].starts_with(f)) => {
                let name_len = self.s[self.at..].iter().position(|&c| c == b'(').unwrap_or(0);
                let name = &self.s[self.at..self.at + name_len];
                self.at += name_len + 1;
                let mut vals = [0i128; 8];
                let mut n = 0;
                loop {
                    if n == vals.len() { return Err(Error::Syntax); }
                    vals[n] = self.guard(|p| p.expr())?;
                    n += 1;
                    match self.peek() { Some(b',') => self.at += 1, Some(b')') => { self.at += 1; break } _ => return Err(Error::Syntax) }
                }
                let one = || if n == 1 { Ok(vals[0]) } else { Err(Error::Syntax) };
                Ok(match name {
                    b"abs" => one()?.checked_abs().ok_or(Error::Overflow)?,
                    b"min" => *vals[..n].iter().min().unwrap(),
                    b"max" => *vals[..n].iter().max().unwrap(),
                    // Whole numbers: round half away from zero, floor, ceil.
                    b"round" => { let v = one()?; let r = (v.abs() + SCALE / 2) / SCALE * SCALE; if v < 0 { -r } else { r } }
                    b"floor" => { let v = one()?; v.div_euclid(SCALE) * SCALE }
                    _ => { let v = one()?; -((-v).div_euclid(SCALE) * SCALE) }
                })
            }
            Some(b's') if self.s[self.at..].starts_with(b"sqrt(") => {
                self.at += 4;
                let v = self.atom()?;
                if v < 0 { return Err(Error::Domain); }
                // sqrt(v / S) * S == sqrt(v * S), rounded down at the 9th place.
                let n = (v as u128).checked_mul(SCALE as u128).ok_or(Error::Overflow)?;
                // Integer Newton from above (no floats in Spindle).
                let mut r: u128 = if n == 0 { 0 } else { 1u128 << ((128 - n.leading_zeros() + 1) / 2) };
                while r > 0 { let next = (r + n / r) / 2; if next >= r { break; } r = next; }
                if r * r != n { self.inexact = true; }
                Ok(r as i128)
            }
            Some(b'a') if self.s[self.at..].starts_with(b"ans") && !self.s.get(self.at + 3).is_some_and(u8::is_ascii_alphanumeric) => {
                self.at += 3;
                self.ans.ok_or(Error::NoAnswer)
            }
            _ => Err(Error::Syntax),
        }
    }
    fn number(&mut self) -> Result<i128, Error> {
        let (mut whole, mut frac, mut places, mut digits, mut dot) = (0i128, 0i128, 0u32, 0, false);
        while let Some(&c) = self.s.get(self.at) {
            match c {
                b'0'..=b'9' if dot => { if places < 9 { frac = frac * 10 + (c - b'0') as i128; places += 1; } else { self.inexact |= c != b'0'; } }
                b'0'..=b'9' => whole = whole.checked_mul(10).and_then(|w| w.checked_add((c - b'0') as i128)).ok_or(Error::Overflow)?,
                b'.' if !dot => dot = true,
                _ => break,
            }
            if c != b'.' { digits += 1; }
            self.at += 1;
        }
        if digits == 0 { return Err(Error::Syntax); }
        whole.checked_mul(SCALE).and_then(|w| w.checked_add(frac * 10i128.pow(9 - places))).ok_or(Error::Overflow)
    }
}

pub fn eval(input: &[u8]) -> Result<Value, Error> { eval_with(input, None) }
/// Evaluate with `ans` bound to a previous result (scaled), if any.
pub fn eval_with(input: &[u8], ans: Option<i128>) -> Result<Value, Error> {
    if input.len() > 200 { return Err(Error::Syntax); }
    let mut p = Parser { s: input, at: 0, depth: 0, inexact: false, ans };
    let v = p.expr()?;
    if p.peek().is_some() { return Err(Error::Syntax); }
    Ok(Value { scaled: v, inexact: p.inexact })
}

/// `= 1.5` or `~ 0.333333333`; trailing zeros trimmed.
pub fn render(v: Value, out: &mut [u8; LINE_MAX]) -> usize {
    let mut n = 0;
    let mut put = |b: u8, n: &mut usize| { if *n < LINE_MAX { out[*n] = b; *n += 1; } };
    put(if v.inexact { b'~' } else { b'=' }, &mut n); put(b' ', &mut n);
    if v.scaled < 0 { put(b'-', &mut n); }
    let a = v.scaled.unsigned_abs();
    let (whole, mut frac) = (a / SCALE as u128, a % SCALE as u128);
    let mut digits = [0u8; 40]; let mut k = 0; let mut w = whole;
    loop { digits[k] = b'0' + (w % 10) as u8; k += 1; w /= 10; if w == 0 { break; } }
    while k > 0 { k -= 1; put(digits[k], &mut n); }
    if frac != 0 {
        put(b'.', &mut n);
        let mut places = 9; while frac % 10 == 0 { frac /= 10; places -= 1; }
        let mut f = [0u8; 9]; for i in (0..places).rev() { f[i] = b'0' + (frac % 10) as u8; frac /= 10; }
        for &d in &f[..places] { put(d, &mut n); }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    fn show(s: &str) -> String { match eval(s.as_bytes()) { Ok(v) => { let mut o = [0; LINE_MAX]; let n = render(v, &mut o); String::from_utf8(o[..n].to_vec()).unwrap() } Err(e) => format!("{e:?}") } }
    #[test] fn arithmetic_is_exact_decimal() {
        assert_eq!(show("2*(3+4)/5"), "= 2.8");
        assert_eq!(show("0.1+0.2"), "= 0.3");
        assert_eq!(show(" -3 - -4 "), "= 1");
        assert_eq!(show("10 % 4"), "= 2");
        assert_eq!(show("7.5%2"), "= 1.5");
        assert_eq!(show("1/3"), "~ 0.333333333");
        assert_eq!(show("1/3*3"), "~ 0.999999999");
        assert_eq!(show("12345678901234567890"), "= 12345678901234567890");
        assert_eq!(show(".5*4"), "= 2");
        assert_eq!(show("2**10"), "= 1024");
        assert_eq!(show("-2**2"), "= -4");
        assert_eq!(show("2**3**2"), "= 512");
        assert_eq!(show("1.5**2"), "= 2.25");
        assert_eq!(show("2**-2"), "= 0.25");
        assert_eq!(show("3**-1"), "~ 0.333333333");
        assert_eq!(show("2*3**2"), "= 18");
        assert_eq!(show("sqrt(16)"), "= 4");
        assert_eq!(show("sqrt(2)"), "~ 1.414213562");
        assert_eq!(show("sqrt(2.25)+1"), "= 2.5");
        assert_eq!(show("sqrt(-1)"), "Domain");
        assert_eq!(show("sqrt 4"), "Syntax");
        assert_eq!(show("abs(-2.5)+max(1, 7, 3)-min(4, -1)"), "= 10.5");
        assert_eq!(show("round(2.5) + round(-2.5) + floor(-1.2) + ceil(1.2)"), "= 0");
        assert_eq!(show("round(1, 2)"), "Syntax");
        assert_eq!(show("maxx(1)"), "Syntax");
    }
    #[test] fn errors_are_named_not_wrapped() {
        assert_eq!(show("1/0"), "DivideByZero");
        assert_eq!(show("5%0"), "DivideByZero");
        assert_eq!(show("99999999999999999999999999999*99999999999"), "Overflow");
        assert_eq!(show(""), "Syntax");
        assert_eq!(show("2+"), "Syntax");
        assert_eq!(show("(1"), "Syntax");
        assert_eq!(show("1 2"), "Syntax");
        assert_eq!(show("abc"), "Syntax");
        assert_eq!(show("1..2"), "Syntax");
        assert_eq!(show(&("(".repeat(40) + "1" + &")".repeat(40))), "TooDeep");
        assert_eq!(show(&"-".repeat(40)), "TooDeep");
        assert_eq!(show("ans+1"), "NoAnswer");
        assert_eq!(show("2**0.5"), "Exponent");
        assert_eq!(show("2**65"), "Exponent");
        assert_eq!(show("10**40"), "Overflow");
        assert_eq!(show("0**-1"), "DivideByZero");
        assert_eq!(show("answer"), "Syntax");
    }
    #[test] fn ans_is_the_previous_result() {
        let first = eval(b"2*(3+4)/5").unwrap();
        let next = eval_with(b"ans*2 - ans", Some(first.scaled)).unwrap();
        let mut o = [0; LINE_MAX]; let n = render(next, &mut o);
        assert_eq!(&o[..n], b"= 2.8");
        assert_eq!(eval_with(b"(ans)/3", Some(SCALE)).map(|v| v.inexact), Ok(true));
    }
}
