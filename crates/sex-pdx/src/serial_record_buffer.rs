// SPDX-License-Identifier: GPL-3.0-only
//! Format a bounded diagnostic completely before one transport invocation.
use core::fmt::{self, Write};

struct Record { bytes: [u8; 256], length: usize }
impl Write for Record {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.length.checked_add(text.len()).ok_or(fmt::Error)?;
        if end > self.bytes.len() { return Err(fmt::Error); }
        self.bytes[self.length..end].copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}

pub fn emit(arguments: fmt::Arguments<'_>, transport: impl FnOnce(&str)) {
    let mut record = Record { bytes: [0; 256], length: 0 };
    if fmt::write(&mut record, arguments).and_then(|_| record.write_str("\n")).is_err() {
        transport("[serial.record.overflow] panic diagnostic-too-long\n");
    } else {
        transport(core::str::from_utf8(&record.bytes[..record.length]).expect("SERIAL_RECORD_UTF8"));
    }
}
