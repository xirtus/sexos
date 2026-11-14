// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Abacus in Voyeur: a query that is arithmetic (`2+2`, `(3+4)*2`,
//! `10/4`, `2^10`, `7%3`) answers with its value as the first result.
//!
//! Minimal on purpose: decimal fixed point with six places, + - * x / % ^,
//! parentheses and unary minus. A plain number is not a calculation. The
//! full calculator engine (units, functions, bases) is a later slice.

/// Fixed-point value: `raw / SCALE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value { pub raw: i128 }

pub const SCALE: i128 = 1_000_000;
const LIMIT: i128 = 1_000_000_000_000_000_000_000_000; // 10^24 raw = 10^18 value
const DEPTH: u8 = 16;

impl Value {
    pub const fn int(v: i64) -> Self { Self { raw: v as i128 * SCALE } }
    fn checked(raw: i128) -> Option<Self> { (raw.abs() <= LIMIT).then_some(Self { raw }) }

    /// Writes the value in plain decimal (no exponent, trailing zeros cut)
    /// and returns how many bytes it took; 0 when `out` is too small.
    pub fn format(self, out: &mut [u8]) -> usize {
        let mut tmp = [0u8; 48];
        let mut n = 0usize;
        let neg = self.raw < 0;
        let a = self.raw.unsigned_abs();
        let (mut int, mut frac) = (a / SCALE as u128, (a % SCALE as u128) as u32);
        let mut digits = [0u8; 40];
        let mut d = 0;
        loop { digits[d] = b'0' + (int % 10) as u8; d += 1; int /= 10; if int == 0 { break; } }
        if neg && (d > 1 || digits[0] != b'0' || frac != 0) { tmp[n] = b'-'; n += 1; }
        while d > 0 { d -= 1; tmp[n] = digits[d]; n += 1; }
        if frac != 0 {
            tmp[n] = b'.'; n += 1;
            let mut places = 6;
            while frac % 10 == 0 { frac /= 10; places -= 1; }
            let mut f = [0u8; 6];
            for i in (0..places).rev() { f[i] = b'0' + (frac % 10) as u8; frac /= 10; }
            tmp[n..n + places].copy_from_slice(&f[..places]); n += places;
        }
        if n > out.len() { return 0; }
        out[..n].copy_from_slice(&tmp[..n]);
        n
    }
}

struct Parser<'a> { s: &'a [u8], at: usize, ops: u32, depth: u8 }

impl<'a> Parser<'a> {
    fn skip(&mut self) { while self.at < self.s.len() && self.s[self.at] == b' ' { self.at += 1; } }
    fn peek(&mut self) -> Option<u8> { self.skip(); self.s.get(self.at).copied() }
    fn expr(&mut self) -> Option<Value> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.at += 1; self.ops += 1;
            let r = self.term()?;
            v = Value::checked(if op == b'+' { v.raw.checked_add(r.raw)? } else { v.raw.checked_sub(r.raw)? })?;
        }
        Some(v)
    }
    fn term(&mut self) -> Option<Value> {
        let mut v = self.power()?;
        while let Some(op @ (b'*' | b'x' | b'/' | b'%')) = self.peek() {
            self.at += 1; self.ops += 1;
            let r = self.power()?;
            v = match op {
                b'/' => { if r.raw == 0 { return None; } Value::checked(v.raw.checked_mul(SCALE)? / r.raw)? }
                b'%' => { if r.raw == 0 { return None; } Value::checked(v.raw % r.raw)? }
                _ => Value::checked(v.raw.checked_mul(r.raw)? / SCALE)?,
            };
        }
        Some(v)
    }
    fn power(&mut self) -> Option<Value> {
        let base = self.unary()?;
        if self.peek() != Some(b'^') { return Some(base); }
        self.at += 1; self.ops += 1;
        self.depth = self.depth.checked_add(1).filter(|d| *d <= DEPTH)?;
        let e = self.power()?;
        self.depth -= 1;
        // Whole exponents 0..=64 only.
        if e.raw % SCALE != 0 || e.raw < 0 || e.raw > 64 * SCALE { return None; }
        let mut v = Value::int(1);
        for _ in 0..e.raw / SCALE { v = Value::checked(v.raw.checked_mul(base.raw)? / SCALE)?; }
        Some(v)
    }
    fn unary(&mut self) -> Option<Value> {
        match self.peek()? {
            b'-' => {
                self.at += 1;
                self.depth = self.depth.checked_add(1).filter(|d| *d <= DEPTH)?;
                let v = self.unary()?;
                self.depth -= 1;
                Some(Value { raw: -v.raw })
            }
            b'+' => { self.at += 1; self.unary() }
            b'(' => {
                self.at += 1;
                self.depth = self.depth.checked_add(1).filter(|d| *d <= DEPTH)?;
                let v = self.expr()?;
                self.depth -= 1;
                if self.peek()? != b')' { return None; }
                self.at += 1;
                Some(v)
            }
            b'0'..=b'9' | b'.' => self.number(),
            _ => None,
        }
    }
    fn number(&mut self) -> Option<Value> {
        let mut int: i128 = 0;
        let mut frac: i128 = 0;
        let mut places = 0u32;
        let mut digits = 0;
        let mut dot = false;
        while let Some(&c) = self.s.get(self.at) {
            match c {
                b'0'..=b'9' => {
                    digits += 1;
                    if dot {
                        if places < 6 { frac = frac * 10 + (c - b'0') as i128; places += 1; }
                    } else {
                        int = int.checked_mul(10)?.checked_add((c - b'0') as i128)?;
                        if int > LIMIT / SCALE { return None; }
                    }
                }
                b'.' if !dot => dot = true,
                _ => break,
            }
            self.at += 1;
        }
        if digits == 0 { return None; }
        Value::checked(int * SCALE + frac * 10i128.pow(6 - places))
    }
}

/// The value of `query` when it is arithmetic with at least one operator;
/// a trailing `=` is allowed. None for anything else.
pub fn evaluate(query: &str) -> Option<Value> {
    let mut s = query.as_bytes();
    while let Some((&last, rest)) = s.split_last() { if last == b' ' || last == b'=' { s = rest; } else { break; } }
    if s.is_empty() || s.len() > 64 { return None; }
    // Must start like a number so "x" or "%" alone is never a calculation.
    if !s.iter().find(|&&c| c != b' ').is_some_and(|c| matches!(c, b'0'..=b'9' | b'.' | b'(' | b'-' | b'+')) { return None; }
    let mut p = Parser { s, at: 0, ops: 0, depth: 0 };
    let v = p.expr()?;
    p.skip();
    (p.at == s.len() && p.ops > 0).then_some(v)
}
