// SPDX-License-Identifier: GPL-3.0-only
//! `calc <expression>`: exact decimal arithmetic on what the user typed.
//!
//! `ans` is the previous successful result in this Spindle session.
//! `**` raises to a whole power up to 64; `sqrt(x)` is the square root;
//! `0x..`/`0b..` whole numbers (and `<expr> in hex|bin|oct` to show a whole result in that base); `abs min max round floor ceil` work on the exact values; `pi` and `e`
//! are 9-place constants (results using them are marked `~`); `exp ln log sin
//! cos tan` and fractional powers (`2**0.5`) are computed at 18 places with
//! integer arithmetic and shown to 9 (`~`).
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
pub enum Error { Syntax, DivideByZero, Overflow, TooDeep, NoAnswer, Exponent, Domain, Unknown, NotWhole }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value { pub scaled: i128, pub inexact: bool }

struct Parser<'a> { s: &'a [u8], at: usize, depth: u8, inexact: bool, ans: Option<i128>, vars: &'a [([u8; 8], u8, i128)] }

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
        if e % SCALE != 0 {
            // A fractional power of a positive base: exp(e * ln(base)).
            if base <= 0 { return Err(Error::Domain); }
            self.inexact = true;
            let l = ln_w(base)?;
            return exp_w(l.checked_mul(e).ok_or(Error::Overflow)? / SCALE);
        }
        if (e / SCALE).abs() > 64 { return Err(Error::Exponent); }
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
            Some(b'e' | b'l' | b's' | b'c' | b't') if [&b"exp("[..], b"ln(", b"log(", b"sin(", b"cos(", b"tan("].iter().any(|f| self.s[self.at..].starts_with(f)) => {
                let name_len = self.s[self.at..].iter().position(|&c| c == b'(').unwrap_or(0);
                let name = &self.s[self.at..self.at + name_len];
                self.at += name_len;
                let v = self.atom()?;
                // Exact only at the trivial points (exp 0, ln 1, sin/tan 0, cos 0).
                let trivial = match name { b"ln" | b"log" => v == SCALE, _ => v == 0 };
                if !trivial { self.inexact = true; }
                match name {
                    b"exp" => exp_w(v.checked_mul(SCALE).ok_or(Error::Overflow)?),
                    b"ln" => Ok(to_scale(ln_w(v)?)),
                    b"log" => Ok(to_scale(ln_w(v)? * W / LN10_W)),
                    b"sin" => Ok(to_scale(sin_cos_w(v)?.0)),
                    b"cos" => Ok(to_scale(sin_cos_w(v)?.1)),
                    _ => {
                        let (s, c) = sin_cos_w(v)?;
                        if c.abs() < 1_000 { return Err(Error::Domain); }
                        Ok(to_scale(s.checked_mul(W).ok_or(Error::Overflow)? / c))
                    }
                }
            }
            Some(b'p' | b'e') if [&b"pi"[..], b"e"].iter().any(|k| self.s[self.at..].starts_with(k) && !self.s.get(self.at + k.len()).is_some_and(u8::is_ascii_alphanumeric)) => {
                // Constants to 9 places; any use makes the answer approximate.
                let pi = self.s[self.at..].starts_with(b"pi");
                self.at += if pi { 2 } else { 1 };
                self.inexact = true;
                Ok(if pi { 3_141_592_654 } else { 2_718_281_828 })
            }
            Some(b'a') if self.s[self.at..].starts_with(b"ans") && !self.s.get(self.at + 3).is_some_and(u8::is_ascii_alphanumeric) => {
                self.at += 3;
                self.ans.ok_or(Error::NoAnswer)
            }
            Some(c) if c.is_ascii_alphabetic() => {
                // A variable set earlier with `name = expr`.
                let start = self.at;
                while self.s.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric()) { self.at += 1; }
                let name = &self.s[start..self.at];
                self.vars.iter().find(|(n, l, _)| &n[..*l as usize] == name).map(|v| v.2).ok_or(Error::Unknown)
            }
            _ => Err(Error::Syntax),
        }
    }
    fn number(&mut self) -> Result<i128, Error> {
        // 0x... hexadecimal and 0b... binary whole numbers.
        if let Some(radix) = match self.s.get(self.at..self.at + 2) { Some(b"0x") | Some(b"0X") => Some(16), Some(b"0b") | Some(b"0B") => Some(2), _ => None } {
            self.at += 2;
            let (mut v, mut n) = (0i128, 0);
            while let Some(d) = self.s.get(self.at).and_then(|&c| (c as char).to_digit(radix)) {
                v = v.checked_mul(radix as i128).and_then(|v| v.checked_add(d as i128)).ok_or(Error::Overflow)?;
                self.at += 1; n += 1;
            }
            if n == 0 { return Err(Error::Syntax); }
            return v.checked_mul(SCALE).ok_or(Error::Overflow);
        }
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

// Transcendental functions work at 18 decimal places (W) and round to the
// 9 shown; no floats are used.
const W: i128 = 1_000_000_000_000_000_000;
const LN2_W: i128 = 693_147_180_559_945_309;
const LN10_W: i128 = 2_302_585_092_994_045_684;
const PI_W: i128 = 3_141_592_653_589_793_238;
/// A W-scaled value rounded (half away from zero) to the 9-place scale.
fn to_scale(w: i128) -> i128 { let h = (W / SCALE) / 2; if w < 0 { -((-w + h) / (W / SCALE)) } else { (w + h) / (W / SCALE) } }
/// e^x for a W-scaled x, as a 9-place value.
fn exp_w(x: i128) -> Result<i128, Error> {
    if x > 88 * W { return Err(Error::Overflow); }
    if x < -30 * W { return Ok(0); }
    // x = k ln2 + r with |r| <= ln2/2, then a Taylor series for e^r.
    let k = (if x < 0 { x - LN2_W / 2 } else { x + LN2_W / 2 }) / LN2_W;
    let r = x - k * LN2_W;
    let (mut term, mut sum, mut i) = (W, W, 1);
    while term != 0 && i < 60 { term = term * r / W / i; sum += term; i += 1; }
    let v = to_scale(sum);
    if k >= 0 { v.checked_mul(1i128 << k).ok_or(Error::Overflow) } else { Ok(if -k >= 127 { 0 } else { (v + (1 << (-k - 1))) >> -k }) }
}
/// ln(x) for a positive 9-place x, W-scaled.
fn ln_w(x: i128) -> Result<i128, Error> {
    if x <= 0 { return Err(Error::Domain); }
    // x = m 2^k with m in [1, 2): ln m = 2 atanh((m - 1) / (m + 1)).
    let mut k = 0i128;
    let mut m = x;
    while m >= 2 * SCALE { m >>= 1; k += 1; }
    while m < SCALE { m <<= 1; k -= 1; }
    let m = m * (W / SCALE);
    let z = (m - W) * W / (m + W);
    let z2 = z * z / W;
    let (mut term, mut sum, mut i) = (z, 0i128, 1);
    while term != 0 && i < 200 { sum += term / i; term = term * z2 / W; i += 2; }
    Ok(k * LN2_W + 2 * sum)
}
/// sin and cos of a 9-place x (radians), W-scaled.
fn sin_cos_w(x: i128) -> Result<(i128, i128), Error> {
    if x.abs() > 1_000_000 * SCALE { return Err(Error::Overflow); }
    let two_pi = 2 * PI_W;
    let mut a = (x * (W / SCALE)) % two_pi;
    if a > PI_W { a -= two_pi; } else if a < -PI_W { a += two_pi; }
    let a2 = a * a / W;
    let (mut s, mut st, mut c, mut ct, mut i) = (a, a, W, W, 1i128);
    while (st != 0 || ct != 0) && i < 60 {
        st = -st * a2 / W / ((2 * i) * (2 * i + 1));
        ct = -ct * a2 / W / ((2 * i - 1) * (2 * i));
        s += st; c += ct; i += 1;
    }
    Ok((s, c))
}

pub fn eval(input: &[u8]) -> Result<Value, Error> { eval_with(input, None) }
/// Evaluate with `ans` bound to a previous result (scaled), if any.
pub fn eval_with(input: &[u8], ans: Option<i128>) -> Result<Value, Error> {
    let mut env = Env { ans, vars: [([0; 8], 0, 0); VARS], n: 0 };
    eval_env(input, &mut env).map(|(v, _)| v)
}

pub const VARS: usize = 8;
/// A Spindle session's calculator state: `ans` and up to 8 named values.
pub struct Env { pub ans: Option<i128>, vars: [([u8; 8], u8, i128); VARS], n: usize }
impl Env { pub const fn new() -> Self { Self { ans: None, vars: [([0; 8], 0, 0); VARS], n: 0 } } }

/// Evaluate `expr` or `name = expr` (a 1-8 letter name, not a built-in).
/// Returns the value and the name assigned, if any; `ans` is updated.
pub fn eval_env<'a>(input: &'a [u8], env: &mut Env) -> Result<(Value, Option<&'a [u8]>), Error> {
    if input.len() > 200 { return Err(Error::Syntax); }
    let trimmed = { let a = input.iter().position(|c| *c != b' ').unwrap_or(input.len()); &input[a..] };
    let name_len = trimmed.iter().position(|c| !c.is_ascii_alphanumeric()).unwrap_or(trimmed.len());
    let after = &trimmed[name_len..];
    let after_eq = after.iter().position(|c| *c != b' ').map(|k| &after[k..]);
    let (name, expr) = match after_eq {
        Some(rest) if name_len > 0 && trimmed[0].is_ascii_alphabetic() && rest.first() == Some(&b'=') => (Some(&trimmed[..name_len]), &rest[1..]),
        _ => (None, input),
    };
    if let Some(n) = name {
        if n.len() > 8 || matches!(n, b"ans" | b"pi" | b"e" | b"sqrt" | b"abs" | b"min" | b"max" | b"round" | b"floor" | b"ceil") { return Err(Error::Syntax); }
    }
    let v = eval_raw(expr, env.ans, &env.vars[..env.n])?;
    if let Some(n) = name {
        let slot = env.vars[..env.n].iter().position(|(m, l, _)| &m[..*l as usize] == n);
        let k = match slot { Some(k) => k, None => { if env.n == VARS { return Err(Error::Overflow); } env.n += 1; env.n - 1 } };
        let mut m = [0u8; 8];
        m[..n.len()].copy_from_slice(n);
        env.vars[k] = (m, n.len() as u8, v.scaled);
    }
    env.ans = Some(v.scaled);
    Ok((v, name))
}

fn eval_raw(input: &[u8], ans: Option<i128>, vars: &[([u8; 8], u8, i128)]) -> Result<Value, Error> {
    let mut p = Parser { s: input, at: 0, depth: 0, inexact: false, ans, vars };
    let v = p.expr()?;
    if p.peek().is_some() { return Err(Error::Syntax); }
    Ok(Value { scaled: v, inexact: p.inexact })
}

/// `<expr> in hex|bin|oct`: the expression and the base to show it in.
pub fn split_radix(input: &[u8]) -> (&[u8], u32) {
    let end = input.iter().rposition(|c| *c != b' ').map_or(0, |k| k + 1);
    let t = &input[..end];
    for (suffix, radix) in [(&b" in hex"[..], 16), (b" in bin", 2), (b" in oct", 8)] {
        if t.ends_with(suffix) { return (&t[..t.len() - suffix.len()], radix); }
    }
    (input, 10)
}

/// Like `render`, in base 16, 2 or 8 (`= 0xff`); only exact whole values.
pub fn render_radix(v: Value, radix: u32, out: &mut [u8; LINE_MAX]) -> Result<usize, Error> {
    if radix == 10 { return Ok(render(v, out)); }
    if v.inexact || v.scaled % SCALE != 0 { return Err(Error::NotWhole); }
    let mut digits = [0u8; 128];
    let (mut k, mut w) = (0, (v.scaled / SCALE).unsigned_abs());
    loop { digits[k] = b"0123456789abcdef"[(w % radix as u128) as usize]; k += 1; w /= radix as u128; if w == 0 { break; } }
    let prefix: &[u8] = match radix { 16 => b"0x", 2 => b"0b", _ => b"0o" };
    let sign = (v.scaled < 0) as usize;
    if 2 + sign + prefix.len() + k > LINE_MAX { return Err(Error::Overflow); }
    out[..2].copy_from_slice(b"= ");
    let mut n = 2;
    if sign == 1 { out[n] = b'-'; n += 1; }
    out[n..n + prefix.len()].copy_from_slice(prefix); n += prefix.len();
    while k > 0 { k -= 1; out[n] = digits[k]; n += 1; }
    Ok(n)
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
        assert_eq!(show("sqrt 4"), "Unknown");
        assert_eq!(show("abs(-2.5)+max(1, 7, 3)-min(4, -1)"), "= 10.5");
        assert_eq!(show("round(2.5) + round(-2.5) + floor(-1.2) + ceil(1.2)"), "= 0");
        assert_eq!(show("round(1, 2)"), "Syntax");
        assert_eq!(show("maxx(1)"), "Unknown");
        assert_eq!(show("2*pi"), "~ 6.283185308");
        assert_eq!(show("e"), "~ 2.718281828");
        assert_eq!(show("pie"), "Unknown");
        assert_eq!(show("0xff + 0b101"), "= 260");
        assert_eq!(show("0x"), "Syntax");
        assert_eq!(show("x + 1"), "Unknown");
    }
    #[test] fn errors_are_named_not_wrapped() {
        assert_eq!(show("1/0"), "DivideByZero");
        assert_eq!(show("5%0"), "DivideByZero");
        assert_eq!(show("99999999999999999999999999999*99999999999"), "Overflow");
        assert_eq!(show(""), "Syntax");
        assert_eq!(show("2+"), "Syntax");
        assert_eq!(show("(1"), "Syntax");
        assert_eq!(show("1 2"), "Syntax");
        assert_eq!(show("abc"), "Unknown");
        assert_eq!(show("1..2"), "Syntax");
        assert_eq!(show(&("(".repeat(40) + "1" + &")".repeat(40))), "TooDeep");
        assert_eq!(show(&"-".repeat(40)), "TooDeep");
        assert_eq!(show("ans+1"), "NoAnswer");
        assert_eq!(show("2**65"), "Exponent");
        assert_eq!(show("2**65"), "Exponent");
        assert_eq!(show("10**40"), "Overflow");
        assert_eq!(show("0**-1"), "DivideByZero");
        assert_eq!(show("answer"), "Unknown");
    }
    #[test] fn named_values_persist_in_the_environment() {
        let mut env = Env::new();
        let r = |env: &mut Env, s: &str| { let mut o = [0; LINE_MAX]; match eval_env(s.as_bytes(), env) { Ok((v, _)) => { let n = render(v, &mut o); String::from_utf8(o[..n].to_vec()).unwrap() } Err(e) => format!("{e:?}") } };
        assert_eq!(r(&mut env, "r = 2"), "= 2");
        assert_eq!(r(&mut env, "area = pi * r**2"), "~ 12.566370616");
        assert_eq!(r(&mut env, "r = r + 1"), "= 3");
        assert_eq!(r(&mut env, "r * 2 + ans"), "= 9");
        assert_eq!(r(&mut env, "pi = 3"), "Syntax");
        assert_eq!(r(&mut env, "x == 1"), "Syntax");
        assert_eq!(r(&mut env, "missing"), "Unknown");
    }
    #[test] fn transcendental_functions_to_nine_places() {
        assert_eq!(show("exp(1)"), "~ 2.718281828");
        assert_eq!(show("exp(0)"), "= 1");
        assert_eq!(show("exp(-1)"), "~ 0.367879441");
        assert_eq!(show("ln(10)"), "~ 2.302585093");
        assert_eq!(show("ln(1)"), "= 0");
        assert_eq!(show("log(1000)"), "~ 3");
        assert_eq!(show("ln(0.5)"), "~ -0.693147181");
        assert_eq!(show("sin(0.523598776)"), "~ 0.5");
        assert_eq!(show("cos(pi)"), "~ -1");
        assert_eq!(show("tan(1)"), "~ 1.557407725");
        assert_eq!(show("sin(0)"), "= 0");
        assert_eq!(show("2**0.5"), "~ 1.414213562");
        assert_eq!(show("10**0.5"), "~ 3.16227766");
        assert_eq!(show("ln(0)"), "Domain");
        assert_eq!(show("(-8)**0.5"), "Domain");
        assert_eq!(show("exp(100)"), "Overflow");
    }
    #[test] fn results_show_in_other_bases() {
        let show = |s: &str| { let (e, r) = split_radix(s.as_bytes()); let mut o = [0; LINE_MAX]; render_radix(eval(e).unwrap(), r, &mut o).map(|n| String::from_utf8(o[..n].to_vec()).unwrap()) };
        assert_eq!(show("255 in hex").unwrap(), "= 0xff");
        assert_eq!(show("0xff+1 in bin ").unwrap(), "= 0b100000000");
        assert_eq!(show("-8 in oct").unwrap(), "= -0o10");
        assert_eq!(show("0 in hex").unwrap(), "= 0x0");
        assert_eq!(show("6/4").unwrap(), "= 1.5");
        assert_eq!(show("6/4 in hex"), Err(Error::NotWhole));
        assert_eq!(show("1/3*3 in hex"), Err(Error::NotWhole), "inexact");
        assert_eq!(show("2**64 in bin"), Err(Error::Overflow), "longer than a line");
        assert_eq!(split_radix(b"x in hexagon"), (&b"x in hexagon"[..], 10));
    }
    #[test] fn ans_is_the_previous_result() {
        let first = eval(b"2*(3+4)/5").unwrap();
        let next = eval_with(b"ans*2 - ans", Some(first.scaled)).unwrap();
        let mut o = [0; LINE_MAX]; let n = render(next, &mut o);
        assert_eq!(&o[..n], b"= 2.8");
        assert_eq!(eval_with(b"(ans)/3", Some(SCALE)).map(|v| v.inexact), Ok(true));
    }
}
