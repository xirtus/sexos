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
            return Err(IoError("addon extract limit exceeded"));
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }
}

fn unescape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        rest = &rest[idx + 1..];
        if let Some(semi) = rest.find(';') {
            let entity = &rest[..semi];
            match entity {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if entity.starts_with("#x") => {
                    if let Ok(cp) = u32::from_str_radix(&entity[2..], 16) {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                        }
                    }
                }
                _ if entity.starts_with('#') => {
                    if let Ok(cp) = entity[1..].parse::<u32>() {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                        }
                    }
                }
                _ => {
                    out.push('&');
                    out.push_str(entity);
                    out.push(';');
                }
            }
            rest = &rest[semi + 1..];
        } else {
            out.push('&');
            break;
        }
    }
    out.push_str(rest);
    out
}

fn order_char(c: u8) -> i32 {
    if c == b'~' {
        -1
    } else if c.is_ascii_alphabetic() {
        c as i32
    } else if c == 0 {
        0
    } else {
        c as i32 + 256
    }
}

fn compare_version_parts(a: &str, b: &str) -> core::cmp::Ordering {
    let mut a_bytes = a.bytes().peekable();
    let mut b_bytes = b.bytes().peekable();

    while a_bytes.peek().is_some() || b_bytes.peek().is_some() {
        let mut first_diff = 0;
        while a_bytes.peek().map_or(false, |c| !c.is_ascii_digit())
            || b_bytes.peek().map_or(false, |c| !c.is_ascii_digit())
        {
            let ca = a_bytes.next().unwrap_or(0);
            let cb = b_bytes.next().unwrap_or(0);
            if ca != cb {
                return order_char(ca).cmp(&order_char(cb));
            }
        }
        while a_bytes.peek().map_or(false, |c| *c == b'0') {
            a_bytes.next();
        }
        while b_bytes.peek().map_or(false, |c| *c == b'0') {
            b_bytes.next();
        }
        let mut a_len = 0;
        let mut b_len = 0;
        while a_bytes.peek().map_or(false, |c| c.is_ascii_digit()) {
            a_len += 1;
            let ca = a_bytes.next().unwrap();
            let cb = b_bytes.next().unwrap_or(0);
            if cb.is_ascii_digit() {
                b_len += 1;
                if first_diff == 0 && ca != cb {
                    first_diff = (ca as i32) - (cb as i32);
                }
            } else {
                return core::cmp::Ordering::Greater;
            }
        }
        while b_bytes.peek().map_or(false, |c| c.is_ascii_digit()) {
            b_len += 1;
            b_bytes.next();
        }
        if a_len != b_len {
            return a_len.cmp(&b_len);
        }
        if first_diff != 0 {
            return first_diff.cmp(&0);
        }
    }
    core::cmp::Ordering::Equal
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Text unescaping and Debian version comparison
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = unescape_xml(s);
        if let Some((v1, v2)) = s.split_once('\n') {
            let _ = compare_version_parts(v1.trim(), v2.trim());
        }
        // Exercise XML parsing on string
        if s.starts_with("<?xml") || s.contains("<addon") {
            let _ = sex_html::parse::parse(s);
        }
    }

    // 2. Kodi addon package inspection (ZIP archive)
    if let Ok(archive) = Archive::open(data) {
        let entries = archive.entries().to_vec();
        for (i, entry) in entries.iter().enumerate().take(32) {
            if entry.kind != EntryKind::File || !entry.safe {
                continue;
            }
            if entry.path.ends_with("addon.xml") {
                let mut sink = MemorySink::new(64 * 1024);
                if archive.extract(i, &mut sink, &mut no_progress).is_ok() {
                    if let Ok(xml_str) = core::str::from_utf8(&sink.buf) {
                        let _ = unescape_xml(xml_str);
                        let _ = sex_html::parse::parse(xml_str);
                    }
                }
            }
        }
    }
});
