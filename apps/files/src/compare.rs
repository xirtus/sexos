// SPDX-License-Identifier: GPL-3.0-only
//! Compare folders (Ctrl+Shift+C): the folder being looked at against another one, by
//! contents (same size and the same SHA-256), through every folder inside. Each row of
//! the listing is then marked: same, differs, newer here or there, only here; what is
//! only in the other folder is counted on the status line.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use loom_core::compare::{compare, Entry, Method, Readers, Status};
use sex_app::files::direct;
use sex_app::Window;

use crate::{at, join, shown, App};

/// Reads paths relative to a folder.
struct Under(Vec<u8>);
impl loom_core::ReadAt for Under {
    fn read_at(&mut self, path: &[u8], offset: u64, buf: &mut [u8]) -> Option<usize> { direct::read_at(&at(&join(&self.0, path)), offset, buf).ok() }
}

/// The marks of the last comparison, for the folder it was made from.
pub struct Marks { pub here: Vec<u8>, pub rows: BTreeMap<Vec<u8>, Status> }

/// Everything under `top`, relative to it.
fn walk(top: &[u8], rel: &[u8], out: &mut Vec<Entry>, budget: &mut usize) {
    let dir = if rel.is_empty() { top.to_vec() } else { join(top, rel) };
    for e in direct::list(&at(&dir)).unwrap_or_default() {
        if e.name == crate::TRASH || e.name == crate::meta::FILE || e.name.first() == Some(&b'.') && e.name.ends_with(b".part") { continue; }
        if *budget == 0 { return; }
        *budget -= 1;
        let mut p = rel.to_vec();
        if !p.is_empty() { p.push(b'/'); }
        p.extend_from_slice(&e.name);
        if e.dir { out.push(Entry::dir(&p)); walk(top, &p, out, budget); }
        else {
            let mtime = direct::stat(&at(&join(top, &p))).map(|s| s.modified).unwrap_or(0);
            out.push(Entry::file(&p, e.size, mtime));
        }
    }
}

pub fn mark(s: Status) -> &'static str {
    match s {
        Status::Same => "same",
        Status::NewerLeft => "newer here",
        Status::NewerRight => "newer there",
        Status::Differs => "differs",
        Status::LeftOnly => "only here",
        Status::RightOnly => "only there",
        Status::Kinds => "file and folder",
        Status::Unreadable => "unreadable",
    }
}

impl App {
    /// The compare prompt was confirmed with `text`, a folder.
    pub(crate) fn compare_with(&mut self, window: &mut Window, text: &[u8]) {
        let text = text.trim_ascii();
        if text.is_empty() { self.loom.marks = None; self.status = "Compare marks cleared.".into(); return; }
        let there = if text.first() == Some(&b'/') { text.to_vec() } else { join(&self.cwd, text) };
        let here = self.cwd.clone();
        match direct::stat(&at(&there)) {
            Ok(st) if st.dir => {}
            _ => { self.status = format!("{} isn't a folder Files can see.", shown(&there)); return; }
        }
        if there == here { self.status = "That is this folder.".into(); return; }
        self.status = format!("Comparing with {}...", shown(&there));
        self.render(window);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let mut budget = 20_000usize;
        walk(&here, b"", &mut left, &mut budget);
        walk(&there, b"", &mut right, &mut budget);
        let (mut l, mut r) = (Under(here.clone()), Under(there.clone()));
        let mut readers = Readers { left: &mut l, right: &mut r };
        let rows = compare(&left, &right, Method::Content, Some(&mut readers));
        let mut count: BTreeMap<Status, usize> = BTreeMap::new();
        for row in &rows { *count.entry(row.status).or_default() += 1; }
        let n = |s: Status| count.get(&s).copied().unwrap_or(0);
        sex_pdx::serial_record!("[files.app.compare] here={} there={} rows={} same={} differs={} here_only={} there_only={}", shown(&here), shown(&there), rows.len(),
            n(Status::Same), n(Status::Differs) + n(Status::NewerLeft) + n(Status::NewerRight) + n(Status::Kinds), n(Status::LeftOnly), n(Status::RightOnly));
        for row in rows.iter().take(40) { sex_pdx::serial_record!("[files.app.compare.row] path={} status={}", shown(&row.path), mark(row.status)); }
        let top: BTreeMap<Vec<u8>, Status> = rows.iter().filter(|r| !r.path.contains(&b'/')).map(|r| (r.path.clone(), r.status)).collect();
        let there_only: Vec<String> = top.iter().filter(|(_, s)| **s == Status::RightOnly).map(|(p, _)| String::from(shown(p))).collect();
        let differ = rows.iter().filter(|r| r.status != Status::Same).count();
        self.status = if differ == 0 { format!("Everything here is the same in {}.", shown(&there)) } else {
            format!("{} of {} items differ from {}{}. Ctrl+Shift+C with an empty name clears the marks.", differ, rows.len(), shown(&there),
                if there_only.is_empty() { String::new() } else { format!("; only there: {}", there_only.join(", ")) })
        };
        self.loom.marks = Some(Marks { here, rows: top });
    }
    /// The compare mark of a row of the current folder.
    pub(crate) fn compare_badge(&self, name: &[u8]) -> String {
        match &self.loom.marks {
            Some(m) if m.here == self.cwd => m.rows.get(name).map(|s| format!("   [{}]", mark(*s))).unwrap_or_default(),
            _ => String::new(),
        }
    }
}
