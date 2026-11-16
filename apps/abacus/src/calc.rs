// SPDX-License-Identifier: GPL-3.0-only
//! Expressions: numbers, + − × ÷ % ^ and `mod`, brackets, unary minus, factorials,
//! constants (pi, e, ans, mem) and functions (sqrt, sin, cos, tan, asin, acos, atan,
//! ln, log, exp, abs, round, floor, ceil). Angles in degrees or radians.
use alloc::string::String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Angle { Degrees, Radians }

#[derive(Clone, Debug, PartialEq)]
pub enum Error { Syntax(usize), DivideByZero, Domain, Unknown(String), Unbalanced, Overflow }

impl Error {
    /// What the result line says.
    pub fn message(&self) -> String {
        match self {
            Error::DivideByZero => "Can't divide by zero".into(),
            Error::Domain => "Out of range".into(),
            Error::Unknown(n) => alloc::format!("Unknown: {n}"),
            Error::Syntax(_) => "Check the sum".into(),
            Error::Unbalanced => "Brackets don't match".into(),
            Error::Overflow => "Too big".into(),
        }
    }
}

struct Parser<'a> { s: &'a [u8], at: usize, ans: f64, mem: f64, angle: Angle }

/// The value of `text`; `ans` is the last answer.
pub fn eval(text: &str, ans: f64, angle: Angle) -> Result<f64, Error> { eval_with(text, ans, 0.0, angle) }

/// The value of `text`, with `mem` the memory's value.
pub fn eval_with(text: &str, ans: f64, mem: f64, angle: Angle) -> Result<f64, Error> {
    // Accept the display's own symbols.
    let mut norm = String::with_capacity(text.len());
    for c in text.chars() {
        match c { '×' | '·' => norm.push('*'), '÷' => norm.push('/'), '−' => norm.push('-'), 'π' => norm.push_str("pi"), '√' => norm.push_str("sqrt"), ',' => norm.push('.'), c => norm.push(c) }
    }
    let mut p = Parser { s: norm.as_bytes(), at: 0, ans, mem, angle };
    let v = p.sum()?;
    p.space();
    if p.at != p.s.len() { return Err(if p.s[p.at] == b')' { Error::Unbalanced } else { Error::Syntax(p.at) }); }
    if v.is_nan() { return Err(Error::Domain); }
    if v.is_infinite() { return Err(Error::Overflow); }
    Ok(v)
}

impl<'a> Parser<'a> {
    fn space(&mut self) { while self.at < self.s.len() && self.s[self.at] == b' ' { self.at += 1; } }
    fn peek(&mut self) -> Option<u8> { self.space(); self.s.get(self.at).copied() }
    fn eat(&mut self, c: u8) -> bool { if self.peek() == Some(c) { self.at += 1; true } else { false } }
    /// Eats the word `w` when it stands alone (`mod`, not `modx`).
    fn word(&mut self, w: &str) -> bool {
        self.space();
        let end = self.at + w.len();
        if self.s.get(self.at..end) == Some(w.as_bytes()) && !self.s.get(end).map_or(false, |c| c.is_ascii_alphabetic()) { self.at = end; true } else { false }
    }

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
            else if self.word("mod") { let d = self.unary()?; if d == 0.0 { return Err(Error::DivideByZero); } v %= d; }
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
        if self.eat(b'^') {
            let e = self.unary()?;
            if base == 0.0 && e < 0.0 { return Err(Error::DivideByZero); }
            base = libm::pow(base, e);
        }
        Ok(base)
    }
    fn postfix(&mut self) -> Result<f64, Error> {
        let mut v = self.atom()?;
        loop {
            if self.eat(b'%') { v /= 100.0; }
            else if self.eat(b'!') {
                if v < 0.0 || v != libm::floor(v) { return Err(Error::Domain); }
                if v > 170.0 { return Err(Error::Overflow); }
                v = (1..=v as u32).fold(1.0, |a, n| a * n as f64);
            } else { return Ok(v); }
        }
    }
    fn atom(&mut self) -> Result<f64, Error> {
        let start = { self.space(); self.at };
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                match self.peek() { Some(b')') => return Err(Error::Syntax(self.at)), None => return Err(Error::Unbalanced), _ => {} }
                let v = self.sum()?;
                if !self.eat(b')') { return Err(if self.peek().is_none() { Error::Unbalanced } else { Error::Syntax(self.at) }); }
                Ok(v)
            }
            Some(b')') => Err(Error::Unbalanced),
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
                let name = core::str::from_utf8(&self.s[start..self.at]).unwrap();
                let to_rad = |x: f64, a: Angle| if a == Angle::Degrees { x * core::f64::consts::PI / 180.0 } else { x };
                let from_rad = |x: f64, a: Angle| if a == Angle::Degrees { x * 180.0 / core::f64::consts::PI } else { x };
                match name {
                    "pi" => return Ok(core::f64::consts::PI),
                    "e" => return Ok(core::f64::consts::E),
                    "ans" => return Ok(self.ans),
                    "mem" => return Ok(self.mem),
                    _ => {}
                }
                const FUNCTIONS: [&str; 14] = ["sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "ln", "log", "exp", "abs", "round", "floor", "ceil"];
                if !FUNCTIONS.contains(&name) { return Err(Error::Unknown(name.into())); }
                let x = self.power()?;
                let a = self.angle;
                Ok(match name {
                    "sqrt" => { if x < 0.0 { return Err(Error::Domain); } libm::sqrt(x) }
                    "sin" | "cos" | "tan" if a == Angle::Degrees && libm::fmod(x, 90.0) == 0.0 => {
                        // Exact on the axes (sin 180 is 0, tan 90 has no value), not rounding noise.
                        let quarter = libm::fmod(libm::fmod(x / 90.0, 4.0) + 4.0, 4.0) as usize;
                        let (s, c) = [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][quarter];
                        match name { "sin" => s, "cos" => c, _ => if c == 0.0 { return Err(Error::Domain) } else { s / c } }
                    }
                    "sin" => libm::sin(to_rad(x, a)),
                    "cos" => libm::cos(to_rad(x, a)),
                    "tan" => libm::tan(to_rad(x, a)),
                    "asin" => from_rad(libm::asin(x), a),
                    "acos" => from_rad(libm::acos(x), a),
                    "atan" => from_rad(libm::atan(x), a),
                    "ln" => { if x <= 0.0 { return Err(Error::Domain); } libm::log(x) }
                    "log" => { if x <= 0.0 { return Err(Error::Domain); } libm::log10(x) }
                    "exp" => libm::exp(x),
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
        let _ = write!(s, "{:.11e}", v);
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

    /// Golden answers, carried over from the old calculator's tests (sex-calc-core):
    /// each expression and the text Abacus shows for it.
    const RADIANS: &[(&str, &str)] = &[
        ("1+2", "3"), ("2+3*4", "14"), ("(2+3)*4", "20"), ("10-4-3", "3"), ("100/10/5", "2"),
        ("7 mod 3", "1"), ("-7 mod 3", "-1"), ("2^10", "1024"), ("2^3^2", "512"), ("-2^2", "-4"),
        ("(-2)^2", "4"), ("2^-1", "0.5"), ("--3", "3"), ("+5", "5"), ("3-(-2)", "5"), ("0.1+0.2", "0.3"),
        ("1/3", "0.333333333333"), ("2/3", "0.666666666667"), (".5*4", "2"), ("1.5e3", "1500"), ("2e-3", "0.002"),
        ("2e", "5.43656365692"), ("2pi", "6.28318530718"), ("3(4+1)", "15"), ("(1+2)(3+4)", "21"),
        ("pi", "3.14159265359"), ("e", "2.71828182846"), ("sqrt(16)", "4"), ("sqrt(2)", "1.41421356237"),
        ("ln(e)", "1"), ("log(1000)", "3"), ("exp(0)", "1"), ("exp(1)", "2.71828182846"), ("abs(-3.5)", "3.5"),
        ("sin(0)", "0"), ("cos(0)", "1"), ("sin(pi/2)", "1"), ("atan(1)*4", "3.14159265359"), ("2sin(pi/6)", "1"),
        ("5!", "120"), ("3!+1", "7"), ("123456789*1000", "123456789000"), ("1/1000000", "0.000001"),
        ("2^100", "1.26765060023e30"), ("-0.5*2", "-1"), ("  1 +  2 * 3 ", "7"), ("((((1))))", "1"),
        ("sqrt(sqrt(81))", "3"), ("170!", "7.25741561531e306"), ("50%", "0.5"), ("200*15%", "30"),
    ];
    const DEGREES: &[(&str, &str)] = &[
        ("sin(30)", "0.5"), ("sin 30", "0.5"), ("sin(180)", "0"), ("cos(90)", "0"), ("cos(60)", "0.5"), ("tan(45)", "1"),
        ("sin(-90)", "-1"), ("cos(360)", "1"), ("asin(1)", "90"), ("acos(0.5)", "60"), ("atan(1)", "45"),
    ];
    #[test]
    fn golden_answers() {
        for (e, want) in RADIANS { assert_eq!(eval(e, 0.0, Angle::Radians).map(format).as_deref(), Ok(*want), "{e}"); }
        for (e, want) in DEGREES { assert_eq!(eval(e, 0.0, Angle::Degrees).map(format).as_deref(), Ok(*want), "{e}"); }
        assert_eq!(eval_with("mem*2", 0.0, 20.0, Angle::Radians), Ok(40.0));
    }
    #[test]
    fn golden_errors() {
        let r = |e: &str| eval(e, 0.0, Angle::Radians);
        for e in ["1/0", "0^-1", "5 mod 0"] { assert_eq!(r(e), Err(Error::DivideByZero), "{e}"); }
        for e in ["(1+2", "1+2)", ")(", "exp(", "(("] { assert_eq!(r(e), Err(Error::Unbalanced), "{e}"); }
        for e in ["sqrt(-1)", "ln(0)", "log(-5)", "asin(2)", "(-8)^0.5", "(-1)!", "2.5!"] { assert_eq!(r(e), Err(Error::Domain), "{e}"); }
        for e in ["171!", "10^400", "exp(1000)"] { assert_eq!(r(e), Err(Error::Overflow), "{e}"); }
        for e in ["", "1+", "*2", "2 3", "1..2", "()"] { assert!(matches!(r(e), Err(Error::Syntax(_))), "{e}"); }
        for e in ["foo(1)", "x+1"] { assert!(matches!(r(e), Err(Error::Unknown(_))), "{e}"); }
        for e in ["tan(90)", "tan(-270)"] { assert_eq!(eval(e, 0.0, Angle::Degrees), Err(Error::Domain), "{e}"); }
    }
}
