// SPDX-License-Identifier: GPL-3.0-only
//! `sextime <unix-seconds>`: a calendar calculator, not a clock. It converts
//! only the number typed by the user, so it needs no clock source, route or
//! capability, and its output claims nothing about the current time.
use sex_time::{Gregorian, LongCount, SexTimestamp};

pub const LINE_MAX: usize = 48;
pub struct Lines {
    pub rows: [[u8; LINE_MAX]; 3],
    pub lens: [usize; 3],
}

struct Out<'a> { buf: &'a mut [u8; LINE_MAX], len: usize }
impl Out<'_> {
    fn bytes(&mut self, value: &[u8]) { for &b in value { if self.len < LINE_MAX { self.buf[self.len] = b; self.len += 1; } } }
    fn signed(&mut self, value: i64) {
        if value < 0 { self.bytes(b"-"); }
        self.unsigned(value.unsigned_abs(), 1);
    }
    fn unsigned(&mut self, mut value: u64, width: usize) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        loop { digits[n] = b'0' + (value % 10) as u8; n += 1; value /= 10; if value == 0 && n >= width { break; } }
        while n > 0 { n -= 1; self.bytes(&[digits[n]]); }
    }
}

/// Parse an optionally signed decimal Unix second count; nothing else.
pub fn parse(args: &[u8]) -> Option<i64> {
    let (negative, digits) = match args.split_first()? { (b'-', rest) => (true, rest), _ => (false, args) };
    if digits.is_empty() || digits.len() > 19 || !digits.iter().all(u8::is_ascii_digit) { return None; }
    let mut value: i64 = 0;
    for &d in digits { value = value.checked_mul(10)?.checked_add((d - b'0') as i64)?; }
    if negative { value.checked_neg() } else { Some(value) }
}

pub fn render(unix: i64) -> Option<Lines> {
    let ts = SexTimestamp::from_unix(unix, 0)?;
    let civil = Gregorian::from_timestamp(ts)?;
    let mut lines = Lines { rows: [[b' '; LINE_MAX]; 3], lens: [0; 3] };
    let [first, second, third] = &mut lines.rows;
    let mut a = Out { buf: first, len: 0 };
    a.bytes(b"SexTime ");
    a.signed(ts.seconds());
    a.bytes(b" s");
    let mut b = Out { buf: second, len: 0 };
    b.bytes(b"UTC ");
    b.signed(civil.year);
    for (sep, value) in [(b"-", civil.month), (b"-", civil.day), (b"T", civil.hour), (b":", civil.minute), (b":", civil.second)] {
        b.bytes(sep);
        b.unsigned(value as u64, 2);
    }
    b.bytes(b"Z");
    let mut c = Out { buf: third, len: 0 };
    c.bytes(b"Long Count ");
    match LongCount::from_timestamp(ts) {
        Some(lc) => {
            c.unsigned(lc.baktun, 1);
            for place in [lc.katun, lc.tun, lc.winal, lc.kin] { c.bytes(b"."); c.unsigned(place as u64, 1); }
            c.bytes(b" (GMT 584283)");
        }
        None => c.bytes(b"before 0.0.0.0.0"),
    }
    lines.lens = [a.len, b.len, c.len];
    Some(lines)
}
