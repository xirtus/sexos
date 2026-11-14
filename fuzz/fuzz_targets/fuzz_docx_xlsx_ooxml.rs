// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_archive::{no_progress, Archive, EntryKind, IoError, Sink};

struct MemorySink {
    buf: Vec<u8>,
    limit: usize,
}

impl MemorySink {
    fn new(limit: usize) -> Self {
        Self { buf: Vec::new(), limit }
    }
}

impl Sink for MemorySink {
    fn write_all(&mut self, data: &[u8]) -> Result<(), IoError> {
        if self.buf.len().saturating_add(data.len()) > self.limit {
            return Err(IoError("ooxml extract size exceeded"));
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    // 1. If data is a ZIP archive, inspect OOXML structure
    if let Ok(archive) = Archive::open(data) {
        let entries = archive.entries().to_vec();
        for (i, entry) in entries.iter().enumerate().take(64) {
            if entry.kind != EntryKind::File || !entry.safe {
                continue;
            }
            let is_ooxml_part = entry.path.ends_with(".xml")
                || entry.path.ends_with(".rels")
                || entry.path == "[Content_Types].xml"
                || entry.path.starts_with("word/")
                || entry.path.starts_with("xl/");

            if is_ooxml_part {
                let mut sink = MemorySink::new(128 * 1024);
                if archive.extract(i, &mut sink, &mut no_progress).is_ok() {
                    if let Ok(text) = core::str::from_utf8(&sink.buf) {
                        // Exercise XML/HTML document parsing on extracted OOXML XML document
                        let _ = sex_html::parse::parse(text);
                    }
                }
            }
        }
    }

    // 2. Also treat raw bytes as potential raw OOXML XML fragment (document.xml, sharedStrings.xml, etc.)
    if let Ok(text) = core::str::from_utf8(data) {
        if text.starts_with("<?xml") || text.starts_with("<w:") || text.starts_with("<worksheet") || text.starts_with("<sst") {
            let _ = sex_html::parse::parse(text);
        }
    }
});
