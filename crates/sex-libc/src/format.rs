// SPDX-License-Identifier: GPL-3.0-only
//! The `printf` engine: C format strings into a byte sink.
//!
//! Supports flags `-+ 0#`, width and precision (also `*`), the length modifiers
//! `hh h l ll z j t L q`, and the conversions `d i u o x X c s p f F e E g G a %`.
//! `%n` is refused. Arguments come through [`Args`] so host tests can drive the
//! engine without a C `va_list`.

/// Where formatted bytes go. Returns the total that *would* have been written.
pub trait Sink {
    fn put(&mut self, bytes: &[u8]);
}

/// A C argument list, read one argument at a time in the format's order.
pub trait Args {
    fn int(&mut self) -> i64;
    fn uint(&mut self) -> u64;
    fn float(&mut self) -> f64;
    fn ptr(&mut self) -> *const u8;
}

#[derive(Clone, Copy, PartialEq)]
enum Size {
    Int,
    Char,
    Short,
    Long,
}

struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// Formats `fmt` with `args` into `out`; returns the number of bytes produced.
pub fn format(fmt: &[u8], args: &mut dyn Args, out: &mut dyn Sink) -> usize {
    let mut counter = Counter { inner: out, n: 0 };
    let out = &mut counter;
    let mut i = 0;
    while i < fmt.len() {
        let start = i;
        while i < fmt.len() && fmt[i] != b'%' {
            i += 1;
        }
        out.put(&fmt[start..i]);
        if i >= fmt.len() {
            break;
        }
        i += 1;
        let mut s = Spec { left: false, plus: false, space: false, alt: false, zero: false, width: 0, precision: None };
        while i < fmt.len() {
            match fmt[i] {
                b'-' => s.left = true,
                b'+' => s.plus = true,
                b' ' => s.space = true,
                b'#' => s.alt = true,
                b'0' => s.zero = true,
                _ => break,
            }
            i += 1;
        }
        if i < fmt.len() && fmt[i] == b'*' {
            // A C `int`: the upper half of the slot may hold anything.
            let w = args.int() as i32;
            if w < 0 {
                s.left = true;
            }
            s.width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                s.width = s.width * 10 + (fmt[i] - b'0') as usize;
                i += 1;
            }
        }
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            if i < fmt.len() && fmt[i] == b'*' {
                let p = args.int() as i32;
                s.precision = if p < 0 { None } else { Some(p as usize) };
                i += 1;
            } else {
                let mut p = 0;
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    p = p * 10 + (fmt[i] - b'0') as usize;
                    i += 1;
                }
                s.precision = Some(p);
            }
        }
        let mut size = Size::Int;
        while i < fmt.len() {
            match fmt[i] {
                b'h' => size = if size == Size::Short { Size::Char } else { Size::Short },
                b'l' | b'z' | b'j' | b't' | b'L' | b'q' => size = Size::Long,
                _ => break,
            }
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else { break };
        i += 1;
        match conv {
            b'%' => out.put(b"%"),
            b'd' | b'i' => {
                let v = args.int();
                let v = match size {
                    Size::Char => v as i8 as i64,
                    Size::Short => v as i16 as i64,
                    Size::Int => v as i32 as i64,
                    Size::Long => v,
                };
                let sign: &[u8] = if v < 0 { b"-" } else if s.plus { b"+" } else if s.space { b" " } else { b"" };
                integer(out, &s, sign, v.unsigned_abs(), 10, false, b"");
            }
            b'u' | b'o' | b'x' | b'X' => {
                let v = args.uint();
                let v = match size {
                    Size::Char => v as u8 as u64,
                    Size::Short => v as u16 as u64,
                    Size::Int => v as u32 as u64,
                    Size::Long => v,
                };
                let (base, upper) = match conv { b'o' => (8, false), b'x' => (16, false), b'X' => (16, true), _ => (10, false) };
                let prefix: &[u8] = match (s.alt && v != 0, conv) { (true, b'x') => b"0x", (true, b'X') => b"0X", (true, b'o') => b"0", _ => b"" };
                integer(out, &s, b"", v, base, upper, prefix);
            }
            b'p' => {
                let v = args.ptr() as u64;
                let s = Spec { alt: true, ..s };
                integer(out, &s, b"", v, 16, false, b"0x");
            }
            b'c' => {
                let c = [args.int() as u8];
                pad(out, &s, &c, b"", false);
            }
            b's' => {
                let p = args.ptr();
                let text: &[u8] = if p.is_null() {
                    b"(null)"
                } else {
                    let max = s.precision.unwrap_or(usize::MAX);
                    let mut n = 0;
                    while n < max && unsafe { *p.add(n) } != 0 {
                        n += 1;
                    }
                    unsafe { core::slice::from_raw_parts(p, n) }
                };
                let s = Spec { zero: false, ..s };
                pad(out, &s, text, b"", false);
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                let v = args.float();
                float(out, &s, v, conv);
            }
            b'n' => {
                // Writing through a format argument is refused.
                let _ = args.ptr();
            }
            other => out.put(&[b'%', other]),
        }
    }
    out.n
}

struct Counter<'a> {
    inner: &'a mut dyn Sink,
    n: usize,
}

impl Sink for Counter<'_> {
    fn put(&mut self, bytes: &[u8]) {
        self.n += bytes.len();
        self.inner.put(bytes);
    }
}

fn integer(out: &mut dyn Sink, s: &Spec, sign: &[u8], v: u64, base: u64, upper: bool, prefix: &[u8]) {
    let mut digits = [0u8; 64];
    let mut n = 0;
    let mut x = v;
    while x > 0 {
        let d = (x % base) as u8;
        digits[63 - n] = if d < 10 { b'0' + d } else if upper { b'A' + d - 10 } else { b'a' + d - 10 };
        x /= base;
        n += 1;
    }
    let precision = s.precision.unwrap_or(1);
    let zeros = precision.saturating_sub(n);
    let body_len = zeros + n;
    let mut body = [b'0'; 128];
    let body_len = body_len.min(128);
    body[body_len - n..body_len].copy_from_slice(&digits[64 - n..]);
    let mut head = [0u8; 4];
    head[..sign.len()].copy_from_slice(sign);
    head[sign.len()..sign.len() + prefix.len()].copy_from_slice(prefix);
    let head = &head[..sign.len() + prefix.len()];
    // C ignores the 0 flag when a precision is given.
    pad(out, s, &body[..body_len], head, s.zero && s.precision.is_none());
}

/// Writes `head` (sign, prefix) and `body` padded to the width.
fn pad(out: &mut dyn Sink, s: &Spec, body: &[u8], head: &[u8], zero: bool) {
    let len = head.len() + body.len();
    let fill = s.width.saturating_sub(len);
    let spaces = [b' '; 32];
    let zeros = [b'0'; 32];
    let repeat = |out: &mut dyn Sink, what: &[u8; 32], mut n: usize| {
        while n > 0 {
            let k = n.min(32);
            out.put(&what[..k]);
            n -= k;
        }
    };
    if s.left {
        out.put(head);
        out.put(body);
        repeat(out, &spaces, fill);
    } else if zero {
        out.put(head);
        repeat(out, &zeros, fill);
        out.put(body);
    } else {
        repeat(out, &spaces, fill);
        out.put(head);
        out.put(body);
    }
}

/// `%f %e %g`: correct to about 17 significant digits, which is what FFmpeg
/// prints (frame rates, durations). Not bit-exact with glibc in the last digit.
fn float(out: &mut dyn Sink, s: &Spec, v: f64, conv: u8) {
    let upper = conv.is_ascii_uppercase();
    let sign: &[u8] = if v.is_sign_negative() && !v.is_nan() { b"-" } else if s.plus { b"+" } else if s.space { b" " } else { b"" };
    let a = libm::fabs(v);
    if !a.is_finite() {
        let text: &[u8] = match (a.is_nan(), upper) { (true, false) => b"nan", (true, true) => b"NAN", (false, false) => b"inf", (false, true) => b"INF" };
        pad(out, &Spec { zero: false, ..*s }, text, sign, false);
        return;
    }
    let mut buf = [0u8; 400];
    let mut w = Buf { b: &mut buf, n: 0 };
    let precision = s.precision.unwrap_or(6);
    match conv.to_ascii_lowercase() {
        b'f' => fixed(&mut w, a, precision.min(60), s.alt),
        b'e' => exponent(&mut w, a, precision.min(60), s.alt, upper),
        b'a' => exponent(&mut w, a, precision.min(60), s.alt, upper),
        _ => {
            // %g: exponent form when the exponent is < -4 or >= precision.
            let p = if precision == 0 { 1 } else { precision.min(60) };
            let e = if a == 0.0 { 0 } else { decimal_exponent(round_sig(a, p)) };
            if e < -4 || e >= p as i32 {
                exponent(&mut w, a, p - 1, s.alt, upper);
            } else {
                fixed(&mut w, a, (p as i32 - 1 - e).max(0) as usize, s.alt);
            }
            if !s.alt {
                w.trim_zeros();
            }
        }
    }
    let n = w.n;
    pad(out, s, &buf[..n], sign, s.zero);
}

struct Buf<'a> {
    b: &'a mut [u8],
    n: usize,
}

impl Buf<'_> {
    fn push(&mut self, c: u8) {
        if self.n < self.b.len() {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
    /// Drops trailing fractional zeros (and the point) before any exponent.
    fn trim_zeros(&mut self) {
        let text = &self.b[..self.n];
        let Some(dot) = text.iter().position(|&c| c == b'.') else { return };
        let exp = text.iter().position(|&c| c == b'e' || c == b'E').unwrap_or(self.n);
        let mut end = exp;
        while end > dot + 1 && self.b[end - 1] == b'0' {
            end -= 1;
        }
        if end == dot + 1 {
            end = dot;
        }
        let tail = self.n - exp;
        self.b.copy_within(exp..self.n, end);
        self.n = end + tail;
    }
}

fn decimal_exponent(a: f64) -> i32 {
    if a == 0.0 {
        return 0;
    }
    let mut e = libm::floor(libm::log10(a)) as i32;
    // log10 can be off by one at exact powers of ten.
    if libm::pow(10.0, e as f64) > a {
        e -= 1;
    } else if libm::pow(10.0, (e + 1) as f64) <= a {
        e += 1;
    }
    e
}

fn round_sig(a: f64, digits: usize) -> f64 {
    if a == 0.0 {
        return 0.0;
    }
    let e = decimal_exponent(a);
    let scale = libm::pow(10.0, (digits as i32 - 1 - e) as f64);
    libm::round(a * scale) / scale
}

/// Digits of `a` with `precision` decimals. Exact for integers below 2^53,
/// otherwise as exact as 17 significant digits allow.
fn fixed(w: &mut Buf, a: f64, precision: usize, alt: bool) {
    let scale = libm::pow(10.0, precision as f64);
    let scaled = libm::round(a * scale);
    if scaled < 9.0e18 && precision <= 18 {
        let v = scaled as u64;
        let p10 = 10u64.pow(precision as u32);
        let (int, frac) = (v / p10, v % p10);
        digits_u64(w, int, 1);
        if precision > 0 || alt {
            w.push(b'.');
        }
        if precision > 0 {
            digits_u64(w, frac, precision);
        }
        return;
    }
    // Large values: integer part digit by digit from the top.
    let e = decimal_exponent(a).max(0);
    let mut rest = a;
    for k in (0..=e).rev() {
        let p = libm::pow(10.0, k as f64);
        let d = libm::floor(rest / p).clamp(0.0, 9.0);
        w.push(b'0' + d as u8);
        rest -= d * p;
    }
    if precision > 0 || alt {
        w.push(b'.');
    }
    for _ in 0..precision {
        rest *= 10.0;
        let d = libm::floor(rest).clamp(0.0, 9.0);
        w.push(b'0' + d as u8);
        rest -= d;
    }
}

fn digits_u64(w: &mut Buf, v: u64, min: usize) {
    let mut d = [0u8; 20];
    let mut n = 0;
    let mut x = v;
    while x > 0 || n < min {
        d[19 - n] = b'0' + (x % 10) as u8;
        x /= 10;
        n += 1;
        if n == 20 {
            break;
        }
    }
    for &c in &d[20 - n..] {
        w.push(c);
    }
}

fn exponent(w: &mut Buf, a: f64, precision: usize, alt: bool, upper: bool) {
    let mut e = decimal_exponent(a);
    let mut m = if a == 0.0 { 0.0 } else { a / libm::pow(10.0, e as f64) };
    // Rounding can carry into a new digit: 9.99 -> 10.0.
    let scale = libm::pow(10.0, precision as f64);
    if libm::round(m * scale) >= 10.0 * scale {
        m /= 10.0;
        e += 1;
    }
    fixed(w, m, precision, alt);
    w.push(if upper { b'E' } else { b'e' });
    w.push(if e < 0 { b'-' } else { b'+' });
    digits_u64(w, e.unsigned_abs() as u64, 2);
}

/// A sink into a fixed C buffer of `cap` bytes, always NUL-terminated if `cap > 0`.
pub struct CBuf {
    pub ptr: *mut u8,
    pub cap: usize,
    pub n: usize,
}

impl Sink for CBuf {
    fn put(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.n + 1 < self.cap {
                unsafe { *self.ptr.add(self.n) = b };
            }
            self.n += 1;
        }
    }
}

impl CBuf {
    pub fn finish(&mut self) {
        if self.cap > 0 {
            unsafe { *self.ptr.add(self.n.min(self.cap - 1)) = 0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec::Vec;

    enum A {
        I(i64),
        F(f64),
        S(&'static [u8]),
    }
    struct List(Vec<A>, usize);
    impl Args for List {
        fn int(&mut self) -> i64 {
            self.1 += 1;
            match self.0[self.1 - 1] { A::I(v) => v, _ => panic!() }
        }
        fn uint(&mut self) -> u64 {
            self.int() as u64
        }
        fn float(&mut self) -> f64 {
            self.1 += 1;
            match self.0[self.1 - 1] { A::F(v) => v, _ => panic!() }
        }
        fn ptr(&mut self) -> *const u8 {
            self.1 += 1;
            match self.0[self.1 - 1] { A::S(s) => s.as_ptr(), A::I(v) => v as usize as *const u8, _ => panic!() }
        }
    }
    struct V(Vec<u8>);
    impl Sink for V {
        fn put(&mut self, b: &[u8]) {
            self.0.extend_from_slice(b)
        }
    }
    fn f(fmt: &str, args: Vec<A>) -> alloc::string::String {
        let mut v = V(Vec::new());
        let n = format(fmt.as_bytes(), &mut List(args, 0), &mut v);
        assert_eq!(n, v.0.len());
        alloc::string::String::from_utf8(v.0).unwrap()
    }

    #[test]
    fn integers_strings_and_padding() {
        assert_eq!(f("%d|%5d|%-5d|%05d|%+d|% d", vec![A::I(-42), A::I(42), A::I(42), A::I(-42), A::I(7), A::I(7)]), "-42|   42|42   |-0042|+7| 7");
        assert_eq!(f("%x %X %#x %o %#o %u", vec![A::I(255), A::I(255), A::I(255), A::I(8), A::I(8), A::I(-1)]), "ff FF 0xff 10 010 4294967295");
        assert_eq!(f("%lld %llu %zu %hhd %hd", vec![A::I(i64::MIN), A::I(-1), A::I(12), A::I(300), A::I(70000)]), "-9223372036854775808 18446744073709551615 12 44 4464");
        assert_eq!(f("%.3d|%8.3d|%.0d|", vec![A::I(5), A::I(-5), A::I(0)]), "005|    -005||");
        assert_eq!(f("[%s] [%5s] [%-5s] [%.2s] %c %%", vec![A::S(b"ab\0"), A::S(b"ab\0"), A::S(b"ab\0"), A::S(b"abc\0"), A::I(b'z' as i64)]), "[ab] [   ab] [ab   ] [ab] z %");
        assert_eq!(f("%*d|%-*d|%.*s", vec![A::I(4), A::I(1), A::I(3), A::I(2), A::I(1), A::S(b"xyz\0")]), "   1|2  |x");
        assert_eq!(f("%p", vec![A::I(0x1234)]), "0x1234");
        // `*` arguments are C ints, so a zero-extended -5 still means left-aligned in 5.
        assert_eq!(f("%*d|%.*d", vec![A::I(0xffff_fffb), A::I(42), A::I(0xffff_ffff), A::I(7)]), "42   |7");
    }

    #[test]
    fn floats_like_c() {
        let cases: &[(&str, f64, &str)] = &[
            ("%f", 1.5, "1.500000"), ("%.2f", 3.14159, "3.14"), ("%.0f", 2.5, "3"), ("%8.3f", -1.0, "  -1.000"),
            ("%f", 0.0, "0.000000"), ("%.1f", 0.05, "0.1"), ("%e", 12345.678, "1.234568e+04"), ("%.2E", 0.000123, "1.23E-04"),
            ("%g", 100000.0, "100000"), ("%g", 1000000.0, "1e+06"), ("%g", 0.0001, "0.0001"), ("%g", 0.00001, "1e-05"),
            ("%g", 23.976023976, "23.976"), ("%.3g", 29.97, "30"), ("%g", 0.0, "0"), ("%f", 1e20, "100000000000000000000.000000"),
            ("%f", f64::INFINITY, "inf"), ("%5.1f", f64::NAN, "  nan"), ("%e", 9.9999999, "1.000000e+01"),
        ];
        for &(fmt, v, want) in cases {
            assert_eq!(f(fmt, vec![A::F(v)]), want, "{fmt} {v}");
        }
    }

    #[test]
    fn c_buffer_truncates_and_terminates() {
        let mut b = [0xffu8; 6];
        let mut sink = CBuf { ptr: b.as_mut_ptr(), cap: b.len(), n: 0 };
        let n = format(b"hello world", &mut List(vec![], 0), &mut sink);
        sink.finish();
        assert_eq!(n, 11);
        assert_eq!(&b, b"hello\0");
    }
}
