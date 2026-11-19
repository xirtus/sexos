// SPDX-License-Identifier: GPL-3.0-only
//! Exact unsigned bit-pattern arithmetic. All arithmetic wraps to the selected
//! 8/16/32/64-bit word; shifts >= word width and division by zero are errors.
//! Precedence: |, ^, &, shifts, +/-, */%, unary ~/-/+, parentheses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Syntax,
    Zero,
    Shift,
    Width,
    TooLong,
}
pub fn mask(width: u8) -> Result<u64, Error> {
    match width {
        8 | 16 | 32 => Ok((1u64 << width) - 1),
        64 => Ok(u64::MAX),
        _ => Err(Error::Width),
    }
}
pub fn eval(text: &str, ans: u64, width: u8) -> Result<u64, Error> {
    if text.len() > 4096 {
        return Err(Error::TooLong);
    }
    let mut p = Parser {
        s: text.as_bytes(),
        at: 0,
        ans,
        mask: mask(width)?,
        width,
        depth: 0,
    };
    let v = p.expr(0)?;
    p.space();
    if p.at != p.s.len() {
        return Err(Error::Syntax);
    }
    Ok(v)
}
struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    ans: u64,
    mask: u64,
    width: u8,
    depth: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.s.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn expr(&mut self, min: u8) -> Result<u64, Error> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(Error::TooLong);
        }
        let mut left = self.atom()?;
        loop {
            self.space();
            let tail = &self.s[self.at..];
            let (op, len, prec) = if tail.starts_with(b"<<") {
                (b'l', 2, 4)
            } else if tail.starts_with(b">>") {
                (b'r', 2, 4)
            } else {
                match tail.first() {
                    Some(b'|') => (b'|', 1, 1),
                    Some(b'^') => (b'^', 1, 2),
                    Some(b'&') => (b'&', 1, 3),
                    Some(b'+') => (b'+', 1, 5),
                    Some(b'-') => (b'-', 1, 5),
                    Some(b'*') => (b'*', 1, 6),
                    Some(b'/') => (b'/', 1, 6),
                    Some(b'%') => (b'%', 1, 6),
                    _ => break,
                }
            };
            if prec < min {
                break;
            }
            self.at += len;
            let right = self.expr(prec + 1)?;
            left = match op {
                b'|' => left | right,
                b'^' => left ^ right,
                b'&' => left & right,
                b'+' => left.wrapping_add(right),
                b'-' => left.wrapping_sub(right),
                b'*' => left.wrapping_mul(right),
                b'/' => {
                    if right == 0 {
                        return Err(Error::Zero);
                    }
                    left / right
                }
                b'%' => {
                    if right == 0 {
                        return Err(Error::Zero);
                    }
                    left % right
                }
                b'l' | b'r' => {
                    if right >= self.width as u64 {
                        return Err(Error::Shift);
                    }
                    if op == b'l' {
                        left << right
                    } else {
                        left >> right
                    }
                }
                _ => unreachable!(),
            } & self.mask;
        }
        self.depth -= 1;
        Ok(left)
    }
    fn atom(&mut self) -> Result<u64, Error> {
        self.space();
        let Some(&c) = self.s.get(self.at) else {
            return Err(Error::Syntax);
        };
        if c == b'~' || c == b'-' || c == b'+' {
            self.at += 1;
            self.depth += 1;
            if self.depth > 64 {
                return Err(Error::TooLong);
            }
            let v = self.atom()?;
            self.depth -= 1;
            return Ok(match c {
                b'~' => !v,
                b'-' => 0u64.wrapping_sub(v),
                _ => v,
            } & self.mask);
        }
        if c == b'(' {
            self.at += 1;
            let v = self.expr(0)?;
            self.space();
            if self.s.get(self.at) != Some(&b')') {
                return Err(Error::Syntax);
            }
            self.at += 1;
            return Ok(v);
        }
        if self.s[self.at..].starts_with(b"ans") {
            self.at += 3;
            return Ok(self.ans & self.mask);
        }
        let mut radix = 10;
        if self.s.get(self.at) == Some(&b'0') {
            match self.s.get(self.at + 1) {
                Some(b'x') | Some(b'X') => {
                    radix = 16;
                    self.at += 2;
                }
                Some(b'b') | Some(b'B') => {
                    radix = 2;
                    self.at += 2;
                }
                Some(b'o') | Some(b'O') => {
                    radix = 8;
                    self.at += 2;
                }
                _ => {}
            }
        }
        let begin = self.at;
        let mut v = 0u64;
        let mut digit = false;
        while let Some(&c) = self.s.get(self.at) {
            if c == b'_' && digit {
                self.at += 1;
                continue;
            }
            let n = match c {
                b'0'..=b'9' => (c - b'0') as u32,
                b'a'..=b'f' => (c - b'a' + 10) as u32,
                b'A'..=b'F' => (c - b'A' + 10) as u32,
                _ => break,
            };
            if n >= radix {
                return Err(Error::Syntax);
            }
            v = v.wrapping_mul(radix as u64).wrapping_add(n as u64) & self.mask;
            digit = true;
            self.at += 1;
        }
        if self.at == begin || !digit || self.s.get(self.at.saturating_sub(1)) == Some(&b'_') {
            return Err(Error::Syntax);
        }
        Ok(v)
    }
}
pub fn format(value: u64, radix: u32) -> alloc::string::String {
    match radix {
        16 => alloc::format!("0x{value:X}"),
        2 => alloc::format!("0b{value:b}"),
        8 => alloc::format!("0o{value:o}"),
        _ => alloc::format!("{value}"),
    }
}
