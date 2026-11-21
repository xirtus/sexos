// SPDX-License-Identifier: GPL-3.0-only
//! The Privacy page's program rows: what each installed program's package asks for, what
//! the person answered, and when a run last held the grant. Files sends the rows
//! (`sex_pdx::package::GRANTS`) and takes the person's change (`CONSENT`); this file only
//! reads and writes the words. Host-tested (`scripts/settings_host_gate.sh`).

/// Grant bits and words, as `sex_pkg::GRANTS` and `sex_pdx::package` have them
/// (`main.rs` checks the bits match).
pub const GRANTS: [(u32, &str); 2] = [(1, "network"), (2, "microphone")];
/// The report lines Files marks for programs (`sex_pdx::package::MACHINE`).
pub const MACHINE: u8 = 1;
pub const ROWS_MAX: usize = 24;

/// One grant of one program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row { pub program: [u8; 16], pub len: usize, pub bit: u32, pub answered: bool, pub allowed: bool, pub used: u64 }
impl Row {
    pub const EMPTY: Row = Row { program: [0; 16], len: 0, bit: 0, answered: false, allowed: false, used: 0 };
    pub fn program(&self) -> &[u8] { &self.program[..self.len] }
    pub fn word(&self) -> &'static str { GRANTS.iter().find(|g| g.0 == self.bit).map_or("?", |g| g.1) }
}

/// Rows from Files' report: one per grant asked, `PROGRAM ASKED ANSWERED ALLOWED USED`
/// lines. Lines that do not parse are skipped.
pub fn parse(report: &[u8], out: &mut [Row; ROWS_MAX]) -> usize {
    let mut n = 0;
    for line in report.split(|b| *b == b'\n') {
        let Some(line) = line.strip_prefix(&[MACHINE]) else { continue };
        let mut words = line.split(|b| *b == b' ');
        let Some(program) = words.next().filter(|p| !p.is_empty() && p.len() <= 16) else { continue };
        let mut num = || words.next().and_then(|w| core::str::from_utf8(w).ok()).and_then(|w| w.parse::<u64>().ok());
        let (Some(asked), Some(answered), Some(allowed), Some(used)) = (num(), num(), num(), num()) else { continue };
        for (bit, _) in GRANTS {
            if asked as u32 & bit == 0 || n == ROWS_MAX { continue; }
            let mut row = Row { program: [0; 16], len: program.len(), bit, answered: answered as u32 & bit != 0, allowed: allowed as u32 & bit != 0, used };
            row.program[..program.len()].copy_from_slice(program);
            out[n] = row;
            n += 1;
        }
    }
    n
}

/// The CONSENT request's `length`: this grant answered, allowed or not.
pub fn consent(row: &Row, allow: bool) -> u64 { (row.bit as u64) << 16 | if allow { row.bit as u64 } else { 0 } }

/// The default answer's name for a grant, from the `privacy.*_first_run` values.
fn default_of(bit: u32, defaults: [i32; 2]) -> &'static str {
    let v = if bit == GRANTS[0].0 { defaults[0] } else { defaults[1] };
    match v { 1 => "no", 2 => "yes", _ => "ask" }
}

struct Out<'a> { buf: &'a mut [u8], at: usize }
impl Out<'_> {
    fn byte(&mut self, b: u8) { if self.at < self.buf.len() { self.buf[self.at] = b; self.at += 1; } }
    fn put(&mut self, s: &[u8]) { for b in s { self.byte(*b); } }
    fn num(&mut self, v: u64, width: usize) {
        let mut d = [b'0'; 20];
        let (mut n, mut i) = (v, d.len());
        loop { i -= 1; d[i] = b'0' + (n % 10) as u8; n /= 10; if n == 0 { break; } }
        for _ in d.len() - i..width { self.byte(b'0'); }
        self.put(&d[i..]);
    }
    fn pad(&mut self, from: usize, to: usize) { for _ in from..to { self.byte(b' '); } }
}

/// Unix seconds as `YYYY-MM-DD HH:MM UTC`.
fn when(w: &mut Out, secs: u64) {
    let days = (secs / 86400) as i64;
    // Days to civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    w.num(year as u64, 4); w.byte(b'-'); w.num(month as u64, 2); w.byte(b'-'); w.num(day as u64, 2);
    w.byte(b' '); w.num(secs % 86400 / 3600, 2); w.byte(b':'); w.num(secs % 3600 / 60, 2); w.put(b" UTC");
}

/// One line per row, `\n` after each: program, grant, the answer, the last use.
pub fn render(rows: &[Row], defaults: [i32; 2], out: &mut [u8]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    for r in rows {
        w.put(r.program()); w.pad(r.len, 17);
        w.put(r.word().as_bytes()); w.pad(r.word().len(), 12);
        let answer: &[u8] = if !r.answered { b"not asked yet" } else if r.allowed { b"allowed" } else { b"not allowed" };
        w.put(answer); w.pad(answer.len(), 14);
        if !r.answered { w.put(b"(first run: "); w.put(default_of(r.bit, defaults).as_bytes()); w.put(b")"); }
        else if r.used == 0 { w.put(b"not used since start"); }
        else { w.put(b"last run "); when(&mut w, r.used); }
        w.byte(b'\n');
    }
    w.at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_from_the_report() {
        let report = b"\x01chello 3 1 1 1763116320\n\x01quiet 0 0 0 0\nnot for us\n\x01bad x 1 1 0\n\x01rec 2 0 0 0\n";
        let mut rows = [Row::EMPTY; ROWS_MAX];
        let n = parse(report, &mut rows);
        assert_eq!(n, 3);
        assert_eq!((rows[0].program(), rows[0].bit, rows[0].answered, rows[0].allowed, rows[0].used), (&b"chello"[..], 1, true, true, 1763116320));
        assert_eq!((rows[1].program(), rows[1].bit, rows[1].answered, rows[1].allowed), (&b"chello"[..], 2, false, false));
        assert_eq!((rows[2].program(), rows[2].word()), (&b"rec"[..], "microphone"));
        // Revoke the network: answered and not allowed; allow the microphone.
        assert_eq!(consent(&rows[0], false), 1 << 16);
        assert_eq!(consent(&rows[1], true), 2 << 16 | 2);
        let mut buf = [0u8; 1024];
        let k = render(&rows[..n], [0, 1], &mut buf);
        let text = core::str::from_utf8(&buf[..k]).unwrap();
        let lines: [&str; 3] = [text.lines().next().unwrap(), text.lines().nth(1).unwrap(), text.lines().nth(2).unwrap()];
        assert_eq!(lines[0], "chello           network     allowed       last run 2025-11-14 10:32 UTC");
        assert_eq!(lines[1], "chello           microphone  not asked yet (first run: no)");
        assert!(lines[2].starts_with("rec "));
    }

    #[test]
    fn full_and_empty() {
        let mut report = [0u8; 2048];
        let mut at = 0;
        for i in 0..20u8 {
            for b in [1, b'p', b'a' + i, b' ', b'3', b' ', b'0', b' ', b'0', b' ', b'0', b'\n'] { report[at] = b; at += 1; }
        }
        let mut rows = [Row::EMPTY; ROWS_MAX];
        assert_eq!(parse(&report[..at], &mut rows), ROWS_MAX, "stops at the table's size");
        assert_eq!(parse(b"", &mut rows), 0);
    }
}
