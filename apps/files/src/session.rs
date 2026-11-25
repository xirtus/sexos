// SPDX-License-Identifier: GPL-3.0-only
//! Folder tabs kept in the granted root, staged before replacement.
use alloc::vec;
use alloc::vec::Vec;
use loom_core::session::{Session, Tab};
use sex_app::{files, Window};

use crate::{at, disk_root, join, shown, App, FolderTab};

pub const FILE: &[u8] = b".loom-tabs";
pub const PREVIOUS: &[u8] = b".loom-tabs.prev";
const STAGE: &[u8] = b".loom-tabs.part";
const MAX_BYTES: u64 = 6 + 12 * (10 + 4096);

fn read_session(window: &mut Window, path: &[u8]) -> Option<Session> {
    let st = files::stat(window, path).ok()?;
    if st.dir || st.size > MAX_BYTES { return None; }
    let mut raw = vec![0; st.size as usize];
    let n = files::read(window, path, &mut raw).ok()?;
    (n == raw.len()).then(|| Session::decode(&raw)).flatten()
}

impl App {
    fn session_file(&self, name: &[u8]) -> Vec<u8> { at(&join(&self.root, name)) }

    pub(crate) fn session_save(&mut self, window: &mut Window) {
        self.remember_tab();
        let state = Session {
            active: self.active_tab,
            tabs: self.tabs.iter().map(|t| Tab {
                path: t.cwd.clone(), cursor: t.cursor.min(u32::MAX as usize) as u32,
                top: t.top.min(u32::MAX as usize) as u32,
            }).collect(),
        };
        let Some(bytes) = state.encode() else { return; };
        let stage = self.session_file(STAGE);
        let target = self.session_file(FILE);
        let previous = self.session_file(PREVIOUS);
        let result = (|| {
            files::write(window, &stage, &bytes)?;
            files::sync(window)?;
            let had_target = files::stat(window, &target).is_ok();
            if had_target {
                // FAT refuses rename over an existing file. Keep the old complete
                // session until the new one is in place, including after a crash.
                if files::stat(window, &previous).is_ok() { files::remove(window, &previous)?; }
                files::rename(window, &target, &previous)?;
                files::sync(window)?;
            }
            if let Err(e) = files::rename(window, &stage, &target) {
                if had_target { let _ = files::rename(window, &previous, &target); }
                return Err(e);
            }
            files::sync(window)?;
            if files::stat(window, &previous).is_ok() && files::remove(window, &previous).is_ok() {
                let _ = files::sync(window);
            }
            Ok(())
        })();
        match result {
            Ok(()) => sex_pdx::serial_record!("[loom.session.saved] count={} active={}", state.tabs.len(), state.active + 1),
            Err(e) => sex_pdx::serial_record!("[loom.session.save-failed] code={e}"),
        }
    }

    pub(crate) fn session_load(&mut self, window: &mut Window) {
        let target = self.session_file(FILE);
        let previous = self.session_file(PREVIOUS);
        let Some(state) = read_session(window, &target).or_else(|| read_session(window, &previous)) else { return };
        let mut tabs = Vec::new();
        let mut active = None;
        for (i, tab) in state.tabs.iter().enumerate() {
            let inside = self.root == b"/"
                || tab.path == self.root
                || tab.path.strip_prefix(&self.root).is_some_and(|tail| tail.starts_with(b"/"));
            if !inside || !files::stat(window, &at(&tab.path)).is_ok_and(|st| st.dir) { continue; }
            if i == state.active { active = Some(tabs.len()); }
            tabs.push(FolderTab { cwd: tab.path.clone(), cursor: tab.cursor as usize, top: tab.top as usize });
        }
        if tabs.is_empty() { return; }
        self.tabs = tabs;
        self.active_tab = active.unwrap_or(0);
        let tab = &self.tabs[self.active_tab];
        self.cwd = tab.cwd.clone();
        self.cursor = tab.cursor;
        self.top = tab.top;
        self.load(window);
        let disk = disk_root(&self.cwd);
        self.watching = (disk != b"/" && self.status.is_empty()).then_some(disk);
        sex_pdx::serial_record!("[loom.session.loaded] count={} active={} dir={}", self.tabs.len(), self.active_tab + 1, shown(&self.cwd));
    }
}
