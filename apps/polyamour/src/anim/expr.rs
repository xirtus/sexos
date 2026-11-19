// SPDX-License-Identifier: GPL-3.0-only
//! The scripted-expression evaluator drivers use: Blender's "simple
//! expression" subset of Python, evaluated without a Python interpreter.
//!
//! Numbers, variables (the driver's variables, `frame`, `pi`, `e`, `True`,
//! `False`), `+ - * / // % **`, unary `-`/`+`, comparisons (`< <= > >= ==
//! !=`, chained like Python), `and`, `or`, `not`, the conditional `a if c
//! else b`, and the functions Blender's driver namespace offers: `sin cos
//! tan asin acos atan atan2 sinh cosh tanh exp log log10 sqrt pow abs floor
//! ceil trunc round int float min max radians degrees fmod hypot clamp lerp
//! smoothstep signum`.

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    const OPS: [&str; 17] = ["**", "//", "<=", ">=", "==", "!=", "+", "-", "*", "/", "%", "<", ">", "?", ":", "&", "|"];
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < b.len() && (b[i + 1] as char).is_ascii_digit()) {
            let start = i;
            while i < b.len() && ((b[i] as char).is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let save = i;
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                if i < b.len() && (b[i] as char).is_ascii_digit() {
                    while i < b.len() && (b[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            out.push(Tok::Num(s[start..i].parse().map_err(|_| format!("bad number '{}'", &s[start..i]))?));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            out.push(Tok::Ident(s[start..i].to_string()));
            continue;
        }
        match c {
            '(' => out.push(Tok::LParen),
            ')' => out.push(Tok::RParen),
            ',' => out.push(Tok::Comma),
            _ => {
                let op = OPS.iter().find(|o| s[i..].starts_with(**o)).ok_or_else(|| format!("unexpected '{c}'"))?;
                out.push(Tok::Op(op));
                i += op.len();
                continue;
            }
        }
        i += 1;
    }
    Ok(out)
}

struct P<'a> {
    t: Vec<Tok>,
    i: usize,
    vars: &'a dyn Fn(&str) -> Option<f64>,
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is_ident(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == w)
    }
    fn is_op(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(x)) if *x == w)
    }
    /// expr := or ['if' or 'else' expr]
    fn expr(&mut self) -> Result<f64, String> {
        let a = self.or()?;
        if self.is_ident("if") {
            self.i += 1;
            let c = self.or()?;
            if !self.is_ident("else") {
                return Err("expected 'else'".into());
            }
            self.i += 1;
            let b = self.expr()?;
            return Ok(if c != 0.0 { a } else { b });
        }
        Ok(a)
    }
    fn or(&mut self) -> Result<f64, String> {
        let mut a = self.and()?;
        while self.is_ident("or") {
            self.i += 1;
            let b = self.and()?;
            a = if a != 0.0 { a } else { b };
        }
        Ok(a)
    }
    fn and(&mut self) -> Result<f64, String> {
        let mut a = self.not()?;
        while self.is_ident("and") {
            self.i += 1;
            let b = self.not()?;
            a = if a == 0.0 { a } else { b };
        }
        Ok(a)
    }
    fn not(&mut self) -> Result<f64, String> {
        if self.is_ident("not") {
            self.i += 1;
            let v = self.not()?;
            return Ok((v == 0.0) as i32 as f64);
        }
        self.cmp()
    }
    fn cmp(&mut self) -> Result<f64, String> {
        let mut a = self.sum()?;
        let mut result: Option<bool> = None;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if matches!(*o, "<" | "<=" | ">" | ">=" | "==" | "!=") => *o,
                _ => break,
            };
            self.i += 1;
            let b = self.sum()?;
            let r = match op {
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                ">=" => a >= b,
                "==" => a == b,
                _ => a != b,
            };
            result = Some(result.unwrap_or(true) && r);
            a = b;
        }
        Ok(match result {
            Some(r) => r as i32 as f64,
            None => a,
        })
    }
    fn sum(&mut self) -> Result<f64, String> {
        let mut a = self.term()?;
        loop {
            if self.is_op("+") {
                self.i += 1;
                a += self.term()?;
            } else if self.is_op("-") {
                self.i += 1;
                a -= self.term()?;
            } else {
                return Ok(a);
            }
        }
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut a = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if matches!(*o, "*" | "/" | "//" | "%") => *o,
                _ => return Ok(a),
            };
            self.i += 1;
            let b = self.unary()?;
            a = match op {
                "*" => a * b,
                "/" => {
                    if b == 0.0 {
                        return Err("division by zero".into());
                    }
                    a / b
                }
                "//" => {
                    if b == 0.0 {
                        return Err("division by zero".into());
                    }
                    (a / b).floor()
                }
                _ => {
                    if b == 0.0 {
                        return Err("modulo by zero".into());
                    }
                    a - b * (a / b).floor()
                }
            };
        }
    }
    fn unary(&mut self) -> Result<f64, String> {
        if self.is_op("-") {
            self.i += 1;
            return Ok(-self.unary()?);
        }
        if self.is_op("+") {
            self.i += 1;
            return self.unary();
        }
        self.power()
    }
    fn power(&mut self) -> Result<f64, String> {
        let a = self.atom()?;
        if self.is_op("**") {
            self.i += 1;
            let b = self.unary()?;
            return Ok(a.powf(b));
        }
        Ok(a)
    }
    fn atom(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(n)
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err("expected ')'".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(Tok::Ident(name)) => {
                self.i += 1;
                if self.peek() == Some(&Tok::LParen) {
                    self.i += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if self.peek() == Some(&Tok::Comma) {
                                self.i += 1;
                                continue;
                            }
                            break;
                        }
                    }
                    if self.peek() != Some(&Tok::RParen) {
                        return Err("expected ')'".into());
                    }
                    self.i += 1;
                    return call(&name, &args);
                }
                let short = name.strip_prefix("math.").unwrap_or(&name);
                match short {
                    "pi" => Ok(core::f64::consts::PI),
                    "e" => Ok(core::f64::consts::E),
                    "tau" => Ok(core::f64::consts::TAU),
                    "True" => Ok(1.0),
                    "False" => Ok(0.0),
                    _ => (self.vars)(&name).ok_or_else(|| format!("name '{name}' is not defined")),
                }
            }
            Some(t) => Err(format!("unexpected {t:?}")),
            None => Err("unexpected end of expression".into()),
        }
    }
}

fn call(name: &str, a: &[f64]) -> Result<f64, String> {
    let name = name.strip_prefix("math.").unwrap_or(name);
    let need = |n: usize| if a.len() == n { Ok(()) } else { Err(format!("{name}() takes {n} argument(s)")) };
    Ok(match name {
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" | "sqrt" | "abs" | "fabs" | "floor" | "ceil" | "trunc" | "int" | "float" | "radians" | "degrees" | "signum" | "log10" => {
            need(1)?;
            let x = a[0];
            match name {
                "sin" => x.sin(),
                "cos" => x.cos(),
                "tan" => x.tan(),
                "asin" => x.clamp(-1.0, 1.0).asin(),
                "acos" => x.clamp(-1.0, 1.0).acos(),
                "atan" => x.atan(),
                "sinh" => x.sinh(),
                "cosh" => x.cosh(),
                "tanh" => x.tanh(),
                "exp" => x.exp(),
                "sqrt" => {
                    if x < 0.0 {
                        return Err("math domain error".into());
                    }
                    x.sqrt()
                }
                "abs" | "fabs" => x.abs(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "trunc" | "int" => x.trunc(),
                "float" => x,
                "radians" => x.to_radians(),
                "degrees" => x.to_degrees(),
                "log10" => x.log10(),
                _ => {
                    if x > 0.0 {
                        1.0
                    } else if x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
            }
        }
        "log" => match a.len() {
            1 => a[0].ln(),
            2 => a[0].ln() / a[1].ln(),
            _ => return Err("log() takes 1 or 2 arguments".into()),
        },
        "round" => match a.len() {
            1 => a[0].round(),
            2 => {
                let k = 10f64.powi(a[1] as i32);
                (a[0] * k).round() / k
            }
            _ => return Err("round() takes 1 or 2 arguments".into()),
        },
        "pow" => {
            need(2)?;
            a[0].powf(a[1])
        }
        "atan2" => {
            need(2)?;
            a[0].atan2(a[1])
        }
        "fmod" => {
            need(2)?;
            a[0] % a[1]
        }
        "hypot" => {
            need(2)?;
            a[0].hypot(a[1])
        }
        "min" | "max" => {
            if a.is_empty() {
                return Err(format!("{name}() needs arguments"));
            }
            let f = if name == "min" { f64::min } else { f64::max };
            a.iter().copied().fold(a[0], f)
        }
        "clamp" => match a.len() {
            1 => a[0].clamp(0.0, 1.0),
            3 => a[0].clamp(a[1].min(a[2]), a[2].max(a[1])),
            _ => return Err("clamp() takes 1 or 3 arguments".into()),
        },
        "lerp" => {
            need(3)?;
            a[0] + (a[1] - a[0]) * a[2]
        }
        "smoothstep" => {
            need(3)?;
            let t = ((a[2] - a[0]) / (a[1] - a[0])).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
        _ => return Err(format!("unknown function {name}()")),
    })
}

/// Evaluate `src`, looking names up with `vars`.
pub fn eval(src: &str, vars: &dyn Fn(&str) -> Option<f64>) -> Result<f64, String> {
    let t = lex(src)?;
    if t.is_empty() {
        return Err("empty expression".into());
    }
    let mut p = P { t, i: 0, vars };
    let v = p.expr()?;
    if p.i != p.t.len() {
        return Err(format!("unexpected {:?}", p.t[p.i]));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        eval(s, &|n| match n {
            "var" => Some(2.0),
            "frame" => Some(10.0),
            _ => None,
        })
        .unwrap()
    }

    #[test]
    fn evaluates_python_like_expressions() {
        assert_eq!(ev("1 + 2 * 3"), 7.0);
        assert_eq!(ev("(1 + 2) * 3"), 9.0);
        assert_eq!(ev("-2 ** 2"), -4.0);
        assert_eq!(ev("2 ** 3 ** 2"), 512.0);
        assert_eq!(ev("7 // 2 + 7 % 2"), 4.0);
        assert_eq!(ev("var * frame"), 20.0);
        assert!((ev("sin(pi / 2)") - 1.0).abs() < 1e-12);
        assert_eq!(ev("max(1, var, 0.5)"), 2.0);
        assert_eq!(ev("1 if var > 1 else 0"), 1.0);
        assert_eq!(ev("0 < var < 3"), 1.0);
        assert_eq!(ev("not var"), 0.0);
        assert_eq!(ev("clamp(var * 2, 0, 3)"), 3.0);
        assert_eq!(ev("1e2 + .5"), 100.5);
        assert!(eval("nope + 1", &|_| None).is_err());
        assert!(eval("1 +", &|_| None).is_err());
        assert!(eval("1 / 0", &|_| None).is_err());
    }
}
