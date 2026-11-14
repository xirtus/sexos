// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_archive::{no_progress, Archive, EntryKind, IoError, NullSink, Sink};

struct BoundedSink {
    written: u64,
    limit: u64,
}

impl BoundedSink {
    fn new(limit: u64) -> Self {
        Self { written: 0, limit }
    }
}

impl Sink for BoundedSink {
    fn write_all(&mut self, data: &[u8]) -> Result<(), IoError> {
        self.written = self.written.saturating_add(data.len() as u64);
        if self.written > self.limit {
            Err(IoError("bounded sink limit exceeded"))
        } else {
            Ok(())
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // 1. Try generic archive open (auto-detection: 7z, zip, tar, single gz/zst/xz)
    if let Ok(archive) = Archive::open(data) {
        let entries = archive.entries().to_vec();
        for (i, entry) in entries.iter().enumerate().take(32) {
            let _ = &entry.path;
            let _ = &entry.raw_path;
            let _ = entry.is_dir();
            let _ = entry.safe;
            let _ = entry.size;
            let _ = entry.compressed_size;
            let _ = entry.mtime;
            let _ = entry.mode;

            if entry.kind == EntryKind::File && entry.safe {
                let mut sink = BoundedSink::new(512 * 1024);
                let _ = archive.extract(i, &mut sink, &mut no_progress);
            }
        }
        let mut null_sink = NullSink;
        let _ = archive.extract(0, &mut null_sink, &mut no_progress);
    }

    // 2. Try open_named with common archive file extensions to force single-stream branches
    for name in &["archive.tar.gz", "archive.tar.zst", "archive.tar.xz", "archive.zip", "archive.7z"] {
        if let Ok(archive) = Archive::open_named(data, Some(name)) {
            let entries = archive.entries().to_vec();
            if let Some((i, _entry)) = entries.iter().enumerate().find(|(_, e)| e.kind == EntryKind::File && e.safe) {
                let mut sink = BoundedSink::new(256 * 1024);
                let _ = archive.extract(i, &mut sink, &mut no_progress);
            }
        }
    }
});
