// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Format a bounded diagnostic completely before one transport invocation. A record
//! longer than the buffer is cut at a character boundary and ends in `...`, so a long
//! address or message still shows its start rather than nothing.
use core::fmt::{self, Write};

const CAPACITY: usize = 256;
/// Room kept for the `...` and newline of a cut record.
const TAIL: &str = "...\n";

struct Record { bytes: [u8; CAPACITY], length: usize, cut: bool }
impl Write for Record {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.cut { return Ok(()); }
        let room = CAPACITY - self.length;
        if text.len() <= room {
            self.bytes[self.length..self.length + text.len()].copy_from_slice(text.as_bytes());
            self.length += text.len();
            return Ok(());
        }
        // Keep what fits before the tail, ending on a whole character.
        let limit = CAPACITY - TAIL.len();
        let mut take = limit.saturating_sub(self.length).min(text.len());
        while take > 0 && !text.is_char_boundary(take) { take -= 1; }
        if self.length > limit {
            // Already past the limit: drop back to a character boundary before it.
            let mut keep = limit;
            while keep > 0 && (self.bytes[keep] & 0xC0) == 0x80 { keep -= 1; }
            self.length = keep;
            take = 0;
        }
        self.bytes[self.length..self.length + take].copy_from_slice(&text.as_bytes()[..take]);
        self.length += take;
        self.cut = true;
        Ok(())
    }
}

pub fn emit(arguments: fmt::Arguments<'_>, transport: impl FnOnce(&str)) {
    let mut record = Record { bytes: [0; CAPACITY], length: 0, cut: false };
    let _ = fmt::write(&mut record, arguments);
    if !record.cut && record.length < CAPACITY {
        record.bytes[record.length] = b'\n';
        record.length += 1;
    } else {
        // Cut (or exactly full with no room for the newline): end in the tail.
        let limit = CAPACITY - TAIL.len();
        if record.length > limit {
            let mut keep = limit;
            while keep > 0 && (record.bytes[keep] & 0xC0) == 0x80 { keep -= 1; }
            record.length = keep;
        }
        record.bytes[record.length..record.length + TAIL.len()].copy_from_slice(TAIL.as_bytes());
        record.length += TAIL.len();
    }
    transport(core::str::from_utf8(&record.bytes[..record.length]).expect("SERIAL_RECORD_UTF8"));
}
