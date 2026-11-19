// SPDX-License-Identifier: GPL-3.0-only
//! Duplicates (Ctrl+Shift+D): files with the same contents anywhere under the folder
//! being looked at. The folders are listed and the files compared in the background,
//! a piece between keys (by size, then their ends, then everything), and the groups
//! shown largest waste first. Delete moves the selected copy to the trash.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loom_core::dupes::{File, Finder, Options, Report, Step};
use sex_app::files::{self, direct, Entry};
use sex_app::{Nav, Window};

use crate::{at, base, human, join, parent, shown, App, ESC};

/// Bytes read per piece of work.
const BUDGET: u64 = 256 * 1024;
const ROWS: usize = 18;

/// Reads through Files with no window.
pub struct Direct;
impl loom_core::ReadAt for Direct {
    fn read_at(&mut self, path: &[u8], offset: u64, buf: &mut [u8]) -> Option<usize> { direct::read_at(&at(path), offset, buf).ok() }
}

pub struct State {
    pub base: Vec<u8>,
    dirs: VecDeque<Vec<u8>>,
    files: Vec<File>,
    finder: Option<Finder>,
    /// When finished: the files and the report.
    done: Option<(Vec<File>, Report)>,
    /// One row per file of a group: (group, path).
    rows: Vec<(usize, Vec<u8>)>,
    cursor: usize,
    top: usize,
    started: u64,
}

impl State {
    pub fn busy(&self) -> bool { self.done.is_none() }
    fn rows_from(&mut self) {
        self.rows.clear();
        if let Some((files, report)) = &self.done {
            for (g, group) in report.groups.iter().enumerate() {
                for &i in &group.files { self.rows.push((g, files[i].path.clone())); }
            }
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }
}

impl App {
    pub(crate) fn dupes_start(&mut self) {
        if self.found.is_some() { self.status = "Esc leaves the search results first.".into(); return; }
        let base = self.cwd.clone();
        sex_pdx::serial_record!("[files.app.dupes.start] base={}", shown(&base));
        self.loom.mode = crate::loom::Mode::Dupes(State { base: base.clone(), dirs: VecDeque::from([base]), files: Vec::new(), finder: None, done: None,
            rows: Vec::new(), cursor: 0, top: 0, started: sex_pdx::get_ticks() });
        self.last_drawn = 0;
    }
    /// One piece of the search. True when the window should be drawn again.
    pub(crate) fn dupes_step(&mut self) -> bool {
        let crate::loom::Mode::Dupes(st) = &mut self.loom.mode else { return false };
        if st.done.is_some() { return false; }
        if let Some(dir) = st.dirs.pop_front() {
            for e in direct::list(&at(&dir)).unwrap_or_default() {
                if e.name == crate::TRASH || e.name == crate::meta::FILE || e.name.first() == Some(&b'.') && e.name.ends_with(b".part") { continue; }
                let p = join(&dir, &e.name);
                if e.dir { st.dirs.push_back(p); } else { st.files.push(File::new(&p, e.size)); }
            }
        } else if st.finder.is_none() {
            st.finder = Some(Finder::new(core::mem::take(&mut st.files), Options::default()));
        } else if let Some(Step::Done) = st.finder.as_mut().map(|f| f.step(&mut Direct, BUDGET)) {
            let (files, report) = st.finder.take().unwrap().into_report();
            let ms = sex_pdx::get_ticks().saturating_sub(st.started);
            sex_pdx::serial_record!("[files.app.dupes] base={} files={} groups={} wasted={} read={} ms={}", shown(&st.base), files.len(), report.groups.len(), report.wasted, report.bytes_read, ms);
            for g in report.groups.iter().take(10) {
                let names: Vec<&str> = g.files.iter().map(|&i| shown(&files[i].path)).collect();
                sex_pdx::serial_record!("[files.app.dupes.group] size={} files={}", g.size, names.join("|"));
            }
            st.done = Some((files, report));
            st.rows_from();
            return true;
        }
        let now = sex_pdx::get_ticks();
        if now.saturating_sub(self.last_drawn) >= 200 { self.last_drawn = now; return true; }
        false
    }
    pub(crate) fn dupes_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let crate::loom::Mode::Dupes(st) = &mut self.loom.mode else { return };
        let last = st.rows.len().saturating_sub(1);
        match (key.code, sex_app::nav(key)) {
            (ESC, _) => { let busy = st.busy(); self.loom.mode = crate::loom::Mode::None; self.status = if busy { "Stopped looking for duplicates.".into() } else { String::new() }; self.load(window); }
            (_, Some(Nav::Up)) => st.cursor = st.cursor.saturating_sub(1),
            (_, Some(Nav::Down)) => st.cursor = (st.cursor + 1).min(last),
            (_, Some(Nav::Home)) => st.cursor = 0,
            (_, Some(Nav::End)) => st.cursor = last,
            (_, Some(Nav::Delete)) if !st.busy() && !st.rows.is_empty() => {
                let (g, path) = st.rows[st.cursor].clone();
                let size = st.done.as_ref().map(|(_, r)| r.groups[g].size).unwrap_or(0);
                // Trash it as Files does from its folder, then come back here.
                let (cwd, entries, cursor) = (core::mem::replace(&mut self.cwd, parent(&path)), core::mem::take(&mut self.entries), self.cursor);
                self.entries = vec![Entry { name: base(&path).to_vec(), dir: false, size }];
                self.cursor = 0;
                self.trash(window);
                let status = core::mem::take(&mut self.status);
                self.cwd = cwd; self.entries = entries; self.cursor = cursor;
                let gone = files::stat(window, &at(&path)).is_err();
                let crate::loom::Mode::Dupes(st) = &mut self.loom.mode else { return };
                if gone {
                    if let Some((files, report)) = st.done.as_mut() {
                        let group = &mut report.groups[g];
                        group.files.retain(|&i| files[i].path != path);
                        report.wasted = report.wasted.saturating_sub(size);
                        if group.files.len() < 2 { report.groups.remove(g); }
                    }
                    st.rows_from();
                }
                self.status = status;
            }
            _ => {}
        }
    }
    pub(crate) fn dupes_render(&mut self) -> Option<String> {
        let crate::loom::Mode::Dupes(st) = &mut self.loom.mode else { return None };
        let mut t = String::new();
        let Some((_, report)) = &st.done else {
            let stage = if !st.dirs.is_empty() { format!("listing folders, {} files so far", st.files.len()) } else {
                match &st.finder { Some(f) => format!("comparing contents, {} left to read", human(f.remaining)), None => "starting".into() }
            };
            t.push_str(&format!("Looking for duplicates under {}: {}.\n\nEsc stops.\n", shown(&st.base), stage));
            return Some(t);
        };
        t.push_str(&format!("Duplicates under {}: {} groups, {} could be freed.\n\n", shown(&st.base), report.groups.len(), human(report.wasted)));
        if st.rows.is_empty() { t.push_str("  No two files here have the same contents.\n"); }
        if st.cursor < st.top { st.top = st.cursor; }
        if st.cursor >= st.top + ROWS { st.top = st.cursor + 1 - ROWS; }
        let mut last_group = usize::MAX;
        for (i, (g, path)) in st.rows.iter().enumerate().skip(st.top).take(ROWS) {
            if *g != last_group {
                let grp = &report.groups[*g];
                t.push_str(&format!("{} copies of {}:\n", grp.files.len(), human(grp.size)));
                last_group = *g;
            }
            let rel = path.strip_prefix(st.base.as_slice()).map(|r| r.strip_prefix(b"/").unwrap_or(r)).unwrap_or(path);
            t.push_str(&format!("{}{}\n", if i == st.cursor { "  > " } else { "    " }, shown(rel)));
        }
        if st.rows.len() > st.top + ROWS { t.push_str(&format!("  ... {} more\n", st.rows.len() - st.top - ROWS)); }
        t.push('\n');
        Some(t)
    }
}
