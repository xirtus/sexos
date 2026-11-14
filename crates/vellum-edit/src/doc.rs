//! Reading a PDF file: the cross-reference chain, object streams, and repair.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::cos::{Dict, ObjId, Object};
use crate::parse::{find, is_white, rfind, Parser};
use crate::{filters, Error};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Entry {
    Free,
    Offset(usize, u16),
    Compressed(u32, u32),
}

/// How the newest cross-reference section of the file is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefStyle {
    /// A classic `xref` table and `trailer` dictionary.
    Table,
    /// A cross-reference stream (PDF 1.5).
    Stream,
    /// The cross-reference data was broken and rebuilt by scanning; a save appends a full table.
    Repaired,
}

struct ObjStm {
    data: Vec<u8>,
    first: usize,
    offsets: Vec<(u32, usize)>,
}

pub(crate) struct Document {
    pub data: Vec<u8>,
    pub xref: BTreeMap<u32, Entry>,
    pub trailer: Dict,
    pub style: XrefStyle,
    /// Offset of the newest cross-reference section (`startxref`), for `/Prev`.
    pub startxref: usize,
    pub size: u32,
    objstms: RefCell<HashMap<u32, Rc<ObjStm>>>,
}

impl Document {
    pub fn load(data: Vec<u8>) -> Result<Document, Error> {
        if find(&data[..data.len().min(1024)], b"%PDF-").is_none() {
            return Err(Error::NotPdf);
        }
        let mut doc = Document {
            data,
            xref: BTreeMap::new(),
            trailer: Dict::new(),
            style: XrefStyle::Table,
            startxref: 0,
            size: 0,
            objstms: RefCell::new(HashMap::new()),
        };
        if doc.read_xref_chain().is_err() || doc.trailer.get(b"Root").is_none() || !doc.root_ok() {
            doc.repair()?;
        }
        if doc.trailer.get(b"Encrypt").is_some() {
            return Err(Error::Encrypted);
        }
        let max = doc.xref.keys().next_back().copied().unwrap_or(0) + 1;
        let declared = doc.trailer.get(b"Size").and_then(Object::as_int).unwrap_or(0).max(0) as u32;
        doc.size = declared.max(max);
        Ok(doc)
    }

    fn root_ok(&self) -> bool {
        self.trailer.get(b"Root").and_then(Object::as_ref).and_then(|r| self.get(r)).and_then(|o| o.as_dict().cloned()).is_some()
    }

    fn read_xref_chain(&mut self) -> Result<(), Error> {
        let tail = self.data.len().saturating_sub(2048);
        let at = rfind(&self.data[tail..], b"startxref").ok_or(Error::Malformed("no startxref"))? + tail;
        let mut p = Parser::new(&self.data, at + 9);
        let start = p.uint().ok_or(Error::Malformed("bad startxref"))? as usize;
        self.startxref = start;
        let mut next = Some(start);
        let mut seen = HashSet::new();
        let mut first = true;
        while let Some(off) = next.take() {
            if !seen.insert(off) || off >= self.data.len() { break; }
            let (entries, trailer, is_stream) = self.read_section(off)?;
            if first {
                self.style = if is_stream { XrefStyle::Stream } else { XrefStyle::Table };
                self.trailer = trailer.clone();
                first = false;
            }
            for (k, v) in entries { self.xref.entry(k).or_insert(v); }
            next = trailer.get(b"Prev").and_then(Object::as_int).map(|v| v as usize);
        }
        Ok(())
    }

    /// Reads one section (table, possibly hybrid, or stream) at `off`.
    fn read_section(&self, off: usize) -> Result<(BTreeMap<u32, Entry>, Dict, bool), Error> {
        let mut p = Parser::new(&self.data, off);
        if p.expect_word(b"xref") {
            let mut entries = BTreeMap::new();
            loop {
                let save = p.pos;
                let (Some(first), Some(count)) = (p.uint(), p.uint()) else { p.pos = save; break };
                for i in 0..count {
                    p.skip_ws();
                    let line = self.data.get(p.pos..p.pos + 18).ok_or(Error::Malformed("short xref"))?;
                    let num = |s: &[u8]| std::str::from_utf8(s).ok().and_then(|s| s.trim().parse::<u64>().ok());
                    let (o, g) = (num(&line[0..10]), num(&line[11..16]));
                    let kind = line[17];
                    let (Some(o), Some(g)) = (o, g) else { return Err(Error::Malformed("bad xref entry")) };
                    let id = (first + i) as u32;
                    let e = if kind == b'n' { Entry::Offset(o as usize, g as u16) } else { Entry::Free };
                    // Object 0 and offset 0 entries are free in practice.
                    let e = if id == 0 || (kind == b'n' && o == 0) { Entry::Free } else { e };
                    entries.insert(id, e);
                    p.pos += 18;
                }
            }
            if !p.expect_word(b"trailer") { return Err(Error::Malformed("no trailer")); }
            let Some(Object::Dict(trailer)) = p.object(&|_| None, 0) else { return Err(Error::Malformed("bad trailer")) };
            if let Some(stm) = trailer.get(b"XRefStm").and_then(Object::as_int) {
                if let Ok((more, _, _)) = self.read_section(stm as usize) {
                    for (k, v) in more {
                        match entries.get(&k) { Some(Entry::Free) | None => { entries.insert(k, v); } _ => {} }
                    }
                }
            }
            return Ok((entries, trailer, false));
        }
        let len = |r: ObjId| self.length_hint(r);
        let mut p = Parser::new(&self.data, off);
        let Some((_, Object::Stream(s))) = p.indirect(&len) else { return Err(Error::Malformed("bad xref offset")) };
        if s.dict.get(b"Type").and_then(Object::as_name) != Some(b"XRef") {
            return Err(Error::Malformed("not an xref stream"));
        }
        let data = filters::decode(&s)?;
        let w: Vec<usize> = s.dict.get(b"W").and_then(Object::as_array).ok_or(Error::Malformed("no /W"))?
            .iter().map(|o| o.as_int().unwrap_or(0).max(0) as usize).collect();
        if w.len() < 3 || w.iter().any(|&x| x > 8) { return Err(Error::Malformed("bad /W")); }
        let size = s.dict.get(b"Size").and_then(Object::as_int).unwrap_or(0);
        let index: Vec<i64> = match s.dict.get(b"Index").and_then(Object::as_array) {
            Some(a) => a.iter().filter_map(Object::as_int).collect(),
            None => vec![0, size],
        };
        let row = w[0] + w[1] + w[2];
        let mut entries = BTreeMap::new();
        let mut pos = 0;
        let field = |bytes: &[u8]| bytes.iter().fold(0u64, |acc, &b| acc << 8 | b as u64);
        for pair in index.chunks(2) {
            let (first, count) = (pair[0].max(0) as u32, pair.get(1).copied().unwrap_or(0).max(0) as u32);
            for i in 0..count {
                let Some(r) = data.get(pos..pos + row) else { break };
                pos += row;
                let kind = if w[0] == 0 { 1 } else { field(&r[..w[0]]) };
                let f2 = field(&r[w[0]..w[0] + w[1]]);
                let f3 = field(&r[w[0] + w[1]..]);
                let e = match kind {
                    0 => Entry::Free,
                    1 => Entry::Offset(f2 as usize, f3 as u16),
                    2 => Entry::Compressed(f2 as u32, f3 as u32),
                    _ => continue,
                };
                entries.insert(first + i, e);
            }
        }
        Ok((entries, s.dict, true))
    }

    /// Resolves an indirect `/Length` (only plain integers stored uncompressed or in streams).
    fn length_hint(&self, r: ObjId) -> Option<i64> {
        match self.xref.get(&r.0)? {
            Entry::Offset(off, _) => {
                let mut p = Parser::new(&self.data, *off);
                p.uint()?; p.uint()?;
                if !p.expect_word(b"obj") { return None; }
                p.uint().map(|v| v as i64)
            }
            Entry::Compressed(..) => self.get(r)?.as_int(),
            Entry::Free => None,
        }
    }

    /// Loads an object from the file.
    pub fn get(&self, id: ObjId) -> Option<Object> {
        match *self.xref.get(&id.0)? {
            Entry::Free => None,
            Entry::Offset(off, _) => {
                let len = |r: ObjId| self.length_hint(r);
                let mut p = Parser::new(&self.data, off);
                let (got, obj) = p.indirect(&len)?;
                (got.0 == id.0).then_some(obj)
            }
            Entry::Compressed(stm, idx) => {
                let os = self.objstm(stm)?;
                let &(num, rel) = os.offsets.get(idx as usize).filter(|(n, _)| *n == id.0)
                    .or_else(|| os.offsets.iter().find(|(n, _)| *n == id.0))?;
                let _ = num;
                let mut p = Parser::new(&os.data, os.first + rel);
                p.object(&|_| None, 0)
            }
        }
    }

    pub fn generation(&self, num: u32) -> u16 {
        match self.xref.get(&num) { Some(Entry::Offset(_, g)) => *g, _ => 0 }
    }

    fn objstm(&self, num: u32) -> Option<Rc<ObjStm>> {
        if let Some(o) = self.objstms.borrow().get(&num) { return Some(o.clone()); }
        let Entry::Offset(..) = self.xref.get(&num)? else { return None };
        let Object::Stream(s) = self.get(ObjId(num, 0))? else { return None };
        let data = filters::decode(&s).ok()?;
        let n = s.dict.get(b"N").and_then(Object::as_int)? as usize;
        let first = s.dict.get(b"First").and_then(Object::as_int)? as usize;
        let mut p = Parser::new(&data, 0);
        let mut offsets = Vec::with_capacity(n);
        for _ in 0..n {
            let (Some(o), Some(r)) = (p.uint(), p.uint()) else { break };
            offsets.push((o as u32, r as usize));
        }
        let os = Rc::new(ObjStm { data, first, offsets });
        self.objstms.borrow_mut().insert(num, os.clone());
        Some(os)
    }

    /// Rebuilds the cross-reference map by scanning for `n g obj`.
    fn repair(&mut self) -> Result<(), Error> {
        self.style = XrefStyle::Repaired;
        self.xref.clear();
        self.objstms.borrow_mut().clear();
        let data = &self.data;
        let mut found: Vec<(u32, u16, usize)> = Vec::new();
        let mut i = 0;
        while let Some(rel) = find(&data[i..], b"obj") {
            let at = i + rel;
            i = at + 3;
            if at >= 3 && &data[at - 3..at] == b"end" { continue; }
            if data.get(at + 3).is_some_and(|&b| !is_white(b) && !crate::parse::is_delim(b)) { continue; }
            // Walk back over `num ws gen ws`.
            let mut j = at;
            let back_ws = |j: &mut usize| { let s = *j; while *j > 0 && is_white(data[*j - 1]) { *j -= 1; } s != *j };
            let back_num = |j: &mut usize| { let e = *j; while *j > 0 && data[*j - 1].is_ascii_digit() { *j -= 1; } (*j < e).then(|| std::str::from_utf8(&data[*j..e]).ok()?.parse::<u64>().ok()).flatten() };
            if !back_ws(&mut j) { continue; }
            let Some(gen) = back_num(&mut j) else { continue };
            if !back_ws(&mut j) { continue; }
            let Some(num) = back_num(&mut j) else { continue };
            if j > 0 && !is_white(data[j - 1]) && !crate::parse::is_delim(data[j - 1]) { continue; }
            if num == 0 || num > u32::MAX as u64 { continue; }
            found.push((num as u32, gen as u16, j));
        }
        for (num, gen, off) in found { self.xref.insert(num, Entry::Offset(off, gen)); }
        // Objects inside object streams.
        let ids: Vec<u32> = self.xref.keys().copied().collect();
        let mut trailer: Option<Dict> = None;
        for num in ids {
            if let Some(Object::Stream(s)) = self.get(ObjId(num, 0)) {
                match s.dict.get(b"Type").and_then(Object::as_name) {
                    Some(b"ObjStm") => {
                        if let Some(os) = self.objstm(num) {
                            for (k, (n, _)) in os.offsets.iter().enumerate() {
                                self.xref.entry(*n).or_insert(Entry::Compressed(num, k as u32));
                            }
                        }
                    }
                    Some(b"XRef") if s.dict.get(b"Root").is_some() => trailer = Some(s.dict.clone()),
                    _ => {}
                }
            }
        }
        // The last `trailer` dictionary with a /Root wins over xref streams.
        let mut at = 0;
        while let Some(rel) = find(&self.data[at..], b"trailer") {
            let mut p = Parser::new(&self.data, at + rel + 7);
            if let Some(Object::Dict(d)) = p.object(&|_| None, 0) {
                if d.get(b"Root").is_some() { trailer = Some(d); }
            }
            at += rel + 7;
        }
        let mut trailer = trailer.unwrap_or_default();
        for k in [&b"Prev"[..], b"XRefStm", b"Type", b"W", b"Index", b"Length", b"Filter", b"DecodeParms"] { trailer.remove(k); }
        if !self.root_ok_in(&trailer) {
            let catalog = self.xref.keys().copied().find(|&n| {
                self.get(ObjId(n, 0)).and_then(|o| o.as_dict().and_then(|d| d.get(b"Type")).and_then(Object::as_name).map(|t| t == b"Catalog")).unwrap_or(false)
            });
            match catalog {
                Some(n) => trailer.set(b"Root", ObjId(n, self.generation(n))),
                None => return Err(Error::Malformed("no document catalog")),
            }
        }
        self.trailer = trailer;
        // A repaired file has no trustworthy previous section.
        self.startxref = 0;
        Ok(())
    }

    fn root_ok_in(&self, trailer: &Dict) -> bool {
        trailer.get(b"Root").and_then(Object::as_ref).and_then(|r| self.get(r)).is_some_and(|o| o.as_dict().is_some())
    }
}
