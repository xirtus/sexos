// SPDX-License-Identifier: GPL-3.0-only
//! Session restore, as Chrome's "Continue where you left off": the window's tabs, their
//! groups and the shown one are kept in Kaleidoscope/session.tsv as they change, and
//! Kaleidoscope started without an address opens them again. Only the shown tab loads
//! at once; the others show their titles and load when first shown. Private tabs and
//! the side panel are not kept.
//! The file's form is in [`sex_web::session`].

use super::{profile::Jarbox, tabs::Tab, Browser};
use sex_web::session::{Kept, Session};
use alloc::vec::Vec;
use sex_app::files;
use super::state_file;

/// Most a session file may be.
const MAX: usize = 64 << 10;

impl Browser {
    /// The window as it stands, without private tabs or the side panel.
    fn session(&self) -> Session {
        let mut s = Session { groups: self.groups.clone(), ..Default::default() };
        for i in 0..self.strip_len() {
            let (jarbox, url, title) = if i == self.active { (self.jarbox, &self.url, &self.page.title) } else { (self.tabs[i].jarbox, &self.tabs[i].url, &self.tabs[i].title) };
            if jarbox == Jarbox::Private || url.is_empty() || url == "about:blank" { continue; }
            if i == self.active { s.shown = s.tabs.len(); }
            let container = if let Jarbox::Container(n) = jarbox { n } else { 0 };
            s.tabs.push(Kept { group: self.tabs[i].group, container, url: url.clone(), title: title.clone() });
        }
        s
    }

    /// Keeps the window's tabs for next time, when they changed.
    pub(super) fn save_session(&mut self) {
        if !self.keep_session || self.in_frame.is_some() { return; }
        let text = self.session().text();
        if text == self.session_saved { return; }
        let _ = files::mkdir(&mut self.window, b"Kaleidoscope");
        if state_file::replace(&mut self.window, "Kaleidoscope/session.tsv", text.as_bytes()) {
            sex_pdx::serial_record!("[web.session.saved] tabs={} bytes={}", text.lines().filter(|l| l.starts_with("tab\t")).count(), text.len());
            self.session_saved = text;
        }
    }

    /// Opens the kept tabs again. False when there were none.
    pub(super) fn restore_session(&mut self) -> bool {
        let Some(text) = state_file::read(&mut self.window, "Kaleidoscope/session.tsv", MAX as u64) else { return false };
        let s = Session::parse(&text);
        if s.tabs.is_empty() { return false; }
        // Each tab waits blank with its address and title; the first is the shown slot.
        self.tabs.clear();
        let boxes: Vec<Jarbox> = s.tabs.iter().map(|k| if k.container == 0 { Jarbox::Own } else { Jarbox::Container(k.container) }).collect();
        for (k, &jarbox) in s.tabs.iter().zip(boxes.iter()) {
            let mut t = Tab::blank(self);
            (t.jar, t.storage) = self.profile.store(&mut self.window, jarbox, super::wall());
            t.jarbox = jarbox;
            t.group = k.group;
            t.url = k.url.clone();
            t.title = k.title.clone();
            t.restore = Some(k.url.clone());
            self.tabs.push(t);
        }
        self.groups = s.groups.clone();
        self.active = 0;
        let first = &s.tabs[0];
        self.jarbox = boxes[0];
        (self.jar, self.storage) = self.profile.store(&mut self.window, boxes[0], super::wall());
        self.tabs[0].restore = None;
        sex_pdx::serial_record!("[web.session.restored] tabs={} groups={} shown={}", s.tabs.len(), s.groups.len(), s.shown);
        if s.shown == 0 {
            self.open(&first.url, false);
        } else {
            // The first tab waits too: its page loads when shown.
            self.tabs[0].restore = Some(first.url.clone());
            self.url = first.url.clone();
            self.page.title = first.title.clone();
            self.show_tab(s.shown);
        }
        self.session_saved = self.session().text();
        true
    }
}

