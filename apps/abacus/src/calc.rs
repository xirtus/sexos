// SPDX-License-Identifier: GPL-3.0-only
//! Expressions: numbers, + − × ÷ % ^, brackets, unary minus, constants (pi, e, ans)
//! and functions (sqrt, sin, cos, tan, asin, acos, atan, ln, log, abs, round, floor,
//! ceil). Angles in degrees or radians.
use alloc::string::String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Angle { Degrees, Radians }

#[derive(Clone, Debug, PartialEq)]
pub enum Error { Syntax(usize), DivideByZero, Domain, Unknown(String) }

struct Parser<'a> { s: &'a [u8], at: usize, ans: f64, angle: Angle }

/// The value of `text`; `ans` is the last answer.
pub fn eval(text: &str, ans: f64, angle: Angle) -> Result<f64, Error> {
    if text.len()>512 || text.bytes().filter(|c|*c==b'(').count()>64 {return Err(Error::Syntax(0));}
    // Accept the display's own symbols.
    let mut norm = String::with_capacity(text.len());
    for c in text.chars() {
        match c { '×' | '·' => norm.push('*'), '÷' => norm.push('/'), '−' => norm.push('-'), 'π' => norm.push_str("pi"), '√' => norm.push_str("sqrt"), ',' => norm.push('.'), c => norm.push(c) }
    }
    let mut p = Parser { s: norm.as_bytes(), at: 0, ans, angle };
    let v = p.sum()?;
    p.space();
    if p.at != p.s.len() { return Err(Error::Syntax(p.at)); }
    if !v.is_finite() { return Err(Error::Domain); }
    Ok(v)
}

impl<'a> Parser<'a> {
    fn space(&mut self) { while self.at < self.s.len() && self.s[self.at] == b' ' { self.at += 1; } }
    fn peek(&mut self) -> Option<u8> { self.space(); self.s.get(self.at).copied() }
    fn eat(&mut self, c: u8) -> bool { if self.peek() == Some(c) { self.at += 1; true } else { false } }

    fn sum(&mut self) -> Result<f64, Error> {
        let mut v = self.product()?;
        loop {
            if self.eat(b'+') { v += self.product()?; } else if self.eat(b'-') { v -= self.product()?; } else { return Ok(v); }
        }
    }
    fn product(&mut self) -> Result<f64, Error> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') { v *= self.unary()?; }
            else if self.eat(b'/') { let d = self.unary()?; if d == 0.0 { return Err(Error::DivideByZero); } v /= d; }
            else if self.peek() == Some(b'(') || self.peek().map_or(false, |c| c.is_ascii_alphabetic()) { v *= self.unary()?; } // 2pi, 3(4)
            else { return Ok(v); }
        }
    }
    fn unary(&mut self) -> Result<f64, Error> {
        if self.eat(b'-') { return Ok(-self.unary()?); }
        if self.eat(b'+') { return self.unary(); }
        self.power()
    }
    fn power(&mut self) -> Result<f64, Error> {
        let mut base = self.postfix()?;
        if self.eat(b'^') { let e = self.unary()?; base = libm::pow(base, e); }
        Ok(base)
    }
    fn postfix(&mut self) -> Result<f64, Error> {
        let mut v = self.atom()?;
        loop {
            if self.eat(b'%') { v /= 100.0; }
            else if self.eat(b'!') {
                if v < 0.0 || v != libm::floor(v) || v > 170.0 { return Err(Error::Domain); }
                v = (1..=v as u32).fold(1.0, |a, n| a * n as f64);
            } else { return Ok(v); }
        }
    }
    fn atom(&mut self) -> Result<f64, Error> {
        let start = { self.space(); self.at };
        match self.peek() {
            Some(b'(') => { self.at += 1; let v = self.sum()?; if !self.eat(b')') { return Err(Error::Syntax(self.at)); } Ok(v) }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                while self.at < self.s.len() && (self.s[self.at].is_ascii_digit() || self.s[self.at] == b'.') { self.at += 1; }
                if self.at < self.s.len() && (self.s[self.at] == b'e' || self.s[self.at] == b'E') && self.s.get(self.at + 1).map_or(false, |c| c.is_ascii_digit() || *c == b'-' || *c == b'+') {
                    self.at += 2;
                    while self.at < self.s.len() && self.s[self.at].is_ascii_digit() { self.at += 1; }
                }
                let t = core::str::from_utf8(&self.s[start..self.at]).unwrap();
                t.parse::<f64>().map_err(|_| Error::Syntax(start))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                while self.at < self.s.len() && self.s[self.at].is_ascii_alphabetic() { self.at += 1; }
                if matches!(&self.s[start..self.at],b"log"|b"exp") && self.s.get(self.at)==Some(&b'2') {self.at+=1;}
                let name = core::str::from_utf8(&self.s[start..self.at]).unwrap();
                let to_rad = |x: f64, a: Angle| if a == Angle::Degrees { x * core::f64::consts::PI / 180.0 } else { x };
                let from_rad = |x: f64, a: Angle| if a == Angle::Degrees { x * 180.0 / core::f64::consts::PI } else { x };
                match name {
                    "pi" => return Ok(core::f64::consts::PI),
                    "e" => return Ok(core::f64::consts::E),
                    "ans" => return Ok(self.ans),
                    _ => {}
                }
                let x = self.power()?;
                let a = self.angle;
                Ok(match name {
                    "sqrt" => { if x < 0.0 { return Err(Error::Domain); } libm::sqrt(x) }
                    "sinh" => libm::sinh(x),
                    "cosh" => libm::cosh(x),
                    "tanh" => libm::tanh(x),
                    "exp" => libm::exp(x),
                    "exp2" => libm::exp2(x),
                    "log2" | "logtwo" => {if x<=0.0{return Err(Error::Domain);}libm::log2(x)},
                    "cbrt" => libm::cbrt(x),
                    "trunc" => libm::trunc(x),
                    "sin" => libm::sin(to_rad(x, a)),
                    "cos" => libm::cos(to_rad(x, a)),
                    "tan" => libm::tan(to_rad(x, a)),
                    "asin" => from_rad(libm::asin(x), a),
                    "acos" => from_rad(libm::acos(x), a),
                    "atan" => from_rad(libm::atan(x), a),
                    "ln" => { if x <= 0.0 { return Err(Error::Domain); } libm::log(x) }
                    "log" => { if x <= 0.0 { return Err(Error::Domain); } libm::log10(x) }
                    "abs" => libm::fabs(x),
                    "round" => libm::round(x),
                    "floor" => libm::floor(x),
                    "ceil" => libm::ceil(x),
                    _ => return Err(Error::Unknown(name.into())),
                })
            }
            _ => Err(Error::Syntax(start)),
        }
    }
}

/// A number as a person reads it: up to 12 significant digits, no trailing zeros,
/// scientific notation when very large or small.
pub fn format(v: f64) -> String {
    use core::fmt::Write;
    let mut s = String::new();
    if v == 0.0 { return "0".into(); }
    if !v.is_finite() { return "∞".into(); }
    let mag = libm::fabs(v);
    if mag >= 1e15 || mag < 1e-9 {
        let _ = write!(s, "{:.10e}", v);
        // Trim the mantissa's zeros: 1.2300000000e5 → 1.23e5
        if let Some(e) = s.find('e') {
            let (m, ex) = s.split_at(e);
            let m = m.trim_end_matches('0').trim_end_matches('.');
            return alloc::format!("{m}{ex}");
        }
        return s;
    }
    let digits = 12i32 - (libm::floor(libm::log10(mag)) as i32 + 1);
    let _ = write!(s, "{:.*}", digits.clamp(0, 12) as usize, v);
    if s.contains('.') { let t = s.trim_end_matches('0').trim_end_matches('.').len(); s.truncate(t); }
    if s == "-0" { s = "0".into(); }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(s: &str) -> f64 { eval(s, 7.0, Angle::Degrees).unwrap() }
    #[test]
    fn arithmetic_follows_precedence() {
        assert_eq!(ev("12+30"), 42.0);
        assert_eq!(ev("2+3×4"), 14.0);
        assert_eq!(ev("(2+3)*4"), 20.0);
        assert_eq!(ev("-2^2"), -4.0);
        assert_eq!(ev("2^3^2"), 512.0);
        assert_eq!(ev("50%"), 0.5);
        assert_eq!(ev("5!"), 120.0);
        assert_eq!(ev("2pi"), 2.0 * core::f64::consts::PI);
        assert_eq!(ev("ans*2"), 14.0);
        assert_eq!(ev("1.5e3"), 1500.0);
        assert!((ev("sin 30") - 0.5).abs() < 1e-12);
        assert!((ev("√16") - 4.0).abs() < 1e-12);
        assert_eq!(eval("1/0", 0.0, Angle::Radians), Err(Error::DivideByZero));
        assert_eq!(eval("sqrt(-1)", 0.0, Angle::Radians), Err(Error::Domain));
        assert!(matches!(eval("2+", 0.0, Angle::Radians), Err(Error::Syntax(_))));
        assert!(matches!(eval("foo(2)", 0.0, Angle::Radians), Err(Error::Unknown(_))));
    }
    #[test]
    fn numbers_format_for_people() {
        assert_eq!(format(42.0), "42");
        assert_eq!(format(0.1 + 0.2), "0.3");
        assert_eq!(format(1.0 / 3.0), "0.333333333333");
        assert_eq!(format(-2.5), "-2.5");
        assert_eq!(format(1e20), "1e20");
        assert_eq!(format(123456789012.0), "123456789012");
    }
}
