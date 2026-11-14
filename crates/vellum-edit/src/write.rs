//! The incremental writer (7.5.6): original bytes, then changed/new objects, a cross-reference
//! section in the file's own style, and a trailer with `/Prev`.

use std::collections::BTreeMap;

use crate::cos::{write_object, Dict, Object};
use crate::doc::{Document, Entry, XrefStyle};
use crate::Error;

pub(crate) fn incremental(doc: &Document, changes: &BTreeMap<u32, (u16, Object)>, next_num: u32) -> Result<Vec<u8>, Error> {
    if changes.is_empty() {
        return Ok(doc.data.clone());
    }
    let mut out = Vec::with_capacity(doc.data.len() + 4096);
    out.extend_from_slice(&doc.data);
    if !out.ends_with(b"\n") && !out.ends_with(b"\r") {
        out.push(b'\n');
    }

    let mut entries: BTreeMap<u32, Entry> = BTreeMap::new();
    if doc.style == XrefStyle::Repaired {
        // No trustworthy previous section: this one lists every object.
        entries.extend(doc.xref.iter().filter(|(_, e)| !matches!(e, Entry::Free)).map(|(k, v)| (*k, *v)));
    }
    for (&num, (gen, obj)) in changes {
        entries.insert(num, Entry::Offset(out.len(), *gen));
        out.extend_from_slice(format!("{num} {gen} obj\n").as_bytes());
        write_object(&mut out, obj);
        out.extend_from_slice(b"\nendobj\n");
    }

    let mut trailer: Dict = doc.trailer.clone();
    for k in [&b"Prev"[..], b"XRefStm", b"Type", b"W", b"Index", b"Length", b"Filter", b"DecodeParms", b"Size", b"N", b"First"] {
        trailer.remove(k);
    }
    let size = doc.size.max(next_num);
    let has_compressed = entries.values().any(|e| matches!(e, Entry::Compressed(..)));
    let prev = (doc.style != XrefStyle::Repaired).then_some(doc.startxref);
    let full = doc.style == XrefStyle::Repaired;

    let use_stream = doc.style == XrefStyle::Stream || has_compressed;
    let xref_off = out.len();
    if !use_stream {
        out.extend_from_slice(b"xref\n");
        if full { entries.insert(0, Entry::Free); }
        let keys: Vec<u32> = entries.keys().copied().collect();
        let mut i = 0;
        while i < keys.len() {
            let mut j = i + 1;
            while j < keys.len() && keys[j] == keys[j - 1] + 1 { j += 1; }
            out.extend_from_slice(format!("{} {}\n", keys[i], j - i).as_bytes());
            for k in &keys[i..j] {
                let line = match entries[k] {
                    Entry::Offset(off, gen) => format!("{off:010} {gen:05} n \n"),
                    _ => "0000000000 65535 f \n".to_string(),
                };
                out.extend_from_slice(line.as_bytes());
            }
            i = j;
        }
        trailer.set(b"Size", Object::Int(size as i64));
        if let Some(p) = prev { trailer.set(b"Prev", Object::Int(p as i64)); }
        out.extend_from_slice(b"trailer\n");
        write_object(&mut out, &Object::Dict(trailer));
        out.push(b'\n');
    } else {
        let me = size;
        entries.insert(me, Entry::Offset(xref_off, 0));
        if full { entries.insert(0, Entry::Free); }
        let max_off = entries.values().map(|e| match e { Entry::Offset(o, _) => *o as u64, Entry::Compressed(s, _) => *s as u64, Entry::Free => 0 }).max().unwrap_or(0);
        let w1 = if max_off > u32::MAX as u64 { 8 } else { 4 };
        let max3 = entries.values().map(|e| match e { Entry::Offset(_, g) => *g as u64, Entry::Compressed(_, i) => *i as u64, Entry::Free => 65535 }).max().unwrap_or(0);
        let w2 = if max3 > 0xFFFF { 4 } else { 2 };
        let mut data = Vec::new();
        let mut index = Vec::new();
        let keys: Vec<u32> = entries.keys().copied().collect();
        let mut i = 0;
        while i < keys.len() {
            let mut j = i + 1;
            while j < keys.len() && keys[j] == keys[j - 1] + 1 { j += 1; }
            index.push(Object::Int(keys[i] as i64));
            index.push(Object::Int((j - i) as i64));
            for k in &keys[i..j] {
                let (t, a, b) = match entries[k] {
                    Entry::Free => (0u8, 0u64, 65535u64),
                    Entry::Offset(o, g) => (1, o as u64, g as u64),
                    Entry::Compressed(s, ix) => (2, s as u64, ix as u64),
                };
                data.push(t);
                data.extend_from_slice(&a.to_be_bytes()[8 - w1..]);
                data.extend_from_slice(&b.to_be_bytes()[8 - w2..]);
            }
            i = j;
        }
        trailer.set(b"Type", Object::name("XRef"));
        trailer.set(b"Size", Object::Int(me as i64 + 1));
        trailer.set(b"W", Object::Array(vec![Object::Int(1), Object::Int(w1 as i64), Object::Int(w2 as i64)]));
        trailer.set(b"Index", Object::Array(index));
        if let Some(p) = prev { trailer.set(b"Prev", Object::Int(p as i64)); }
        out.extend_from_slice(format!("{me} 0 obj\n").as_bytes());
        write_object(&mut out, &Object::Stream(crate::cos::Stream { dict: trailer, data }));
        out.extend_from_slice(b"\nendobj\n");
    }
    out.extend_from_slice(format!("startxref\n{xref_off}\n%%EOF\n").as_bytes());
    Ok(out)
}
