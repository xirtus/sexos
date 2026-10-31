// SPDX-License-Identifier: GPL-3.0-only
//! Bounded civil views of one last query. Formatting grants no clock authority.
use crate::clock_probe::View;
use sex_pdx::desktop_style::{scale, ADVANCE, GLYPH_HEIGHT, INSET, LINE};
use sex_time::{Gregorian, LongCount, SexTimestamp};

pub struct Status { pub bytes: [u8; 24], pub len: usize }
fn label(value: &[u8]) -> Status {
    let mut s = Status { bytes: [b' '; 24], len: value.len() };
    s.bytes[..value.len()].copy_from_slice(value);
    s
}
fn timestamp(value: u32) -> Option<SexTimestamp> {
    let seconds = value as u64;
    if !(sex_pdx::clock_source::MIN_SECONDS..=sex_pdx::clock_source::MAX_SECONDS).contains(&seconds) {
        return None;
    }
    SexTimestamp::from_unix(i64::from(value), 0)
}
fn iso(date: Gregorian) -> Status {
    let mut out = label(b"0000-00-00T00:00:00Z");
    for (at, value, width) in [(0, date.year as u32, 4), (5, date.month as u32, 2),
        (8, date.day as u32, 2), (11, date.hour as u32, 2),
        (14, date.minute as u32, 2), (17, date.second as u32, 2)] {
        let mut value = value;
        for digit in (at..at + width).rev() {
            out.bytes[digit] = b'0' + (value % 10) as u8;
            value /= 10;
        }
    }
    out
}
/// Legacy overview status stays byte-for-byte compatible. The conversion is
/// shared with the dedicated panel rather than maintaining a second calendar.
pub fn status(view: View) -> Option<Status> {
    Some(match view {
        View::Idle => return None,
        View::Pending => label(b"checking"),
        View::Unavailable => label(b"unavailable"),
        View::Cancelled => label(b"cancelled"),
        View::Uncertain => label(b"waiting receipt"),
        View::Rollback => label(b"rollback; blocked"),
        View::Ready(value) => match timestamp(value).and_then(Gregorian::from_timestamp) {
            Some(date) => iso(date),
            None => label(b"unavailable"),
        },
    })
}

pub struct Frame { pub bytes: [u8; 512], pub len: usize, pub cols: usize, pub rows: usize }
impl Frame {
    fn row(&mut self, row: usize, text: &[u8]) {
        if row >= self.rows { return; }
        let n = text.len().min(self.cols);
        self.bytes[row * (self.cols + 1)..row * (self.cols + 1) + n].copy_from_slice(&text[..n]);
    }
}
// Every panel line fits this finite inline buffer; the longest number is a
// 10-digit SexTime second in the checked 2020..=2099 source interval.
struct Line { bytes: [u8; 40], len: usize }
impl Line {
    fn new() -> Self { Self { bytes: [0; 40], len: 0 } }
    fn add(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.bytes.len() - self.len);
        self.bytes[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }
    fn number(&mut self, mut value: u64) {
        let mut digits = [0; 20];
        let mut start = digits.len();
        loop {
            start -= 1; digits[start] = b'0' + (value % 10) as u8; value /= 10;
            if value == 0 { break; }
        }
        self.add(&digits[start..]);
    }
    fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
}
fn state(view: View, valid: bool) -> &'static [u8] {
    match view {
        View::Idle => b"No sample", View::Pending => b"Pending",
        View::Unavailable => b"Unavailable", View::Cancelled => b"Cancelled",
        View::Uncertain => b"Waiting receipt", View::Rollback => b"Rollback / blocked",
        View::Ready(_) if valid => b"Ready / enlarge tile",
        View::Ready(_) => b"Invalid snapshot",
    }
}
/// Whole padded replacement, at most 24 physical text rows and 512 bytes.
/// A usable 20-column/eight-row tile shows all provenance and calendar views.
/// Smaller tiles show a nonnumeric state instead of a clipped date or a date
/// without its snapshot/source labels. No heap, polling, cache, or clock writes.
pub fn render(w: u32, h: u32, view: View) -> Frame {
    let factor = scale(w);
    let cols = (w.saturating_sub(2 * INSET) / (ADVANCE * factor)).saturating_sub(1).min(63) as usize;
    let top = 28 + INSET;
    let rows = if cols == 0 || h < top + GLYPH_HEIGHT * factor { 0 } else {
        (1 + (h - top - GLYPH_HEIGHT * factor) / (LINE * factor)) as usize
    }.min(24).min(if cols == 0 { 0 } else { 512 / (cols + 1) });
    let mut frame = Frame { bytes: [0; 512], len: 0, cols, rows };
    if rows == 0 { return frame; }
    frame.len = rows * (cols + 1) - 1;
    for row in 0..rows {
        let at = row * (cols + 1);
        frame.bytes[at..at + cols].fill(b' ');
        if row + 1 < rows { frame.bytes[at + cols] = b'\n'; }
    }
    let sample = match view {
        View::Ready(value) => timestamp(value).and_then(|ts| {
            Some((ts, Gregorian::from_timestamp(ts)?, LongCount::from_timestamp(ts)?))
        }), _ => None,
    };
    if cols < 20 || rows < 8 {
        frame.row(0, state(view, sample.is_some()));
        frame.row(1, b"T sample  Q clear");
        frame.row(2, b"q35 CMOS / 1s");
        frame.row(3, b"Snapshot / not live");
        return frame;
    }
    let wide = cols >= 32;
    frame.row(0, if wide { b"Clock / last query snapshot" } else { b"Clock / last query" });
    if let Some((ts, date, count)) = sample {
        let date = iso(date); frame.row(1, &date.bytes[..date.len]);
        let mut seconds = Line::new(); seconds.add(b"SexTime "); seconds.number(ts.seconds() as u64); seconds.add(b" s");
        frame.row(2, seconds.as_bytes());
        let mut lc = Line::new(); lc.add(if wide { b"Long Count " } else { b"LC " });
        for (index, value) in [count.baktun, count.katun as u64, count.tun as u64, count.winal as u64, count.kin as u64].into_iter().enumerate() {
            if index != 0 { lc.add(b"."); } lc.number(value);
        }
        frame.row(3, lc.as_bytes());
    } else {
        frame.row(1, state(view, false));
        frame.row(2, b"No date snapshot");
        frame.row(3, if wide { b"SexTime / Long Count unavailable" } else { b"No calendar values" });
    }
    frame.row(4, if wide { b"UTC day / GMT 584283" } else { b"UTC day GMT584283" });
    frame.row(5, if wide { b"q35 CMOS / 1-second precision" } else { b"q35 CMOS / 1s" });
    frame.row(6, if wide { b"Snapshot only / not live" } else { b"Snapshot / not live" });
    frame.row(7, b"T sample  Q clear");
    frame
}
