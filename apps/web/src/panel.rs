// SPDX-License-Identifier: GPL-3.0-only
//! The side panel, as in Chrome, Edge and Brave: a narrow page at the right of the
//! window beside the shown tab. Ctrl+Shift+B opens it on the bookmarks (and recent
//! history), whose links open in the page beside it; an extension's `side_panel`
//! page opens there from about:extensions or `chrome.sidePanel.open()`.
//!
//! The panel's page is a tab like any other, kept in the last tab slot and left out
//! of the strip. While it is open, the shown tab and the panel are the two sides of a
//! split view (see `Browser::columns`), so a press on the panel gives it the keyboard
//! and its page lays out to the panel's width.

use super::{profile::Jarbox, tabs::Tab, Browser};
use alloc::string::String;

/// The bookmarks panel's address.
pub const BOOKMARKS: &str = "about:bookmarks";

impl Browser {
    /// How many tabs the strip shows: all but the panel's.
    pub(super) fn strip_len(&self) -> usize { self.tabs.len() - self.panel as usize }

    /// The panel's tab slot, while it is open.
    pub(super) fn panel_slot(&self) -> Option<usize> { self.panel.then(|| self.tabs.len() - 1) }

    /// Whether the panel has the keyboard.
    pub(super) fn panel_shown(&self) -> bool { self.panel_slot() == Some(self.active) }

    /// The page beside the panel: the tab it was opened beside, or the last shown.
    fn panel_main(&self) -> usize {
        match self.split { Some((l, _)) if self.panel && l < self.strip_len() => l, _ => self.active.min(self.strip_len().saturating_sub(1)) }
    }

    /// Pairs the panel with the shown tab (or keeps its pair while it has the keyboard).
    pub(super) fn repair_panel(&mut self) {
        if let Some(p) = self.panel_slot() {
            let main = if self.active == p { self.panel_main() } else { self.active };
            self.split = Some((main, p));
        }
    }

    /// Ctrl+Shift+B: the bookmarks panel, or closed again.
    pub(super) fn toggle_panel(&mut self) {
        if self.in_frame.is_some() { return; }
        let p = self.panel_slot();
        if p.is_some_and(|p| self.tabs_url(p) == BOOKMARKS) { return self.close_panel(); }
        self.panel_show(BOOKMARKS);
    }

    /// The address of tab slot `i`.
    fn tabs_url(&self, i: usize) -> &str { if i == self.active { &self.url } else { &self.tabs[i].url } }

    /// Opens the panel on `url` (or shows `url` in it), the shown tab keeping the keyboard.
    pub(super) fn panel_show(&mut self, url: &str) {
        if self.in_frame.is_some() { return; }
        // A split view of two tabs makes way for the panel.
        if !self.panel && self.split.take().is_some() { web_record!("[web.split] on=0 shown={}", self.active); }
        let back = if self.panel_shown() { self.panel_main() } else { self.active };
        if !self.panel {
            if self.tabs.len() >= super::tabs::MAX_TABS { return; }
            let mut t = Tab::blank(self);
            // The panel keeps the shown tab's cookies; a private window's stay private.
            let jarbox = if self.jarbox == Jarbox::Private { Jarbox::Private } else { Jarbox::Own };
            (t.jar, t.storage) = self.profile.store(&mut self.window, jarbox, super::wall());
            t.jarbox = jarbox;
            self.tabs.push(t);
            self.panel = true;
        }
        let p = self.tabs.len() - 1;
        self.split = Some((back, p));
        if self.split_pair().is_none() {
            // No room beside the page: the panel waits closed.
            self.tabs.pop();
            self.panel = false;
            self.split = None;
            web_record!("[web.panel] on=0 room=0");
            return;
        }
        // The page beside it lays out narrower; the panel's page opens at its width.
        self.refit();
        self.show_tab(p);
        self.open(url, false);
        let w = self.view.0;
        self.show_tab(back);
        web_record!("[web.panel] on=1 w={} url={}", w, url);
    }

    /// Closes the panel; the page beside it takes the width back.
    pub(super) fn close_panel(&mut self) {
        let Some(p) = self.panel_slot() else { return };
        let back = self.panel_main();
        // Its page's frames, sockets and sound go with it.
        if self.active != p { self.show_tab(p); }
        self.drop_shown_page();
        self.show_tab(back);
        self.tabs.pop();
        self.panel = false;
        self.split = None;
        web_record!("[web.panel] on=0 shown={}", self.active);
        self.refit();
    }

    /// A link followed in the bookmarks panel: it opens in the page beside it, which
    /// takes the keyboard back.
    pub(super) fn panel_follow(&mut self, url: &str) {
        let back = self.panel_main();
        self.show_tab(back);
        web_record!("[web.panel.follow] shown={} url={}", self.active, url);
        self.open(url, true);
    }

    /// The bookmarks panel's page: bookmarks, newest first, then recent pages.
    pub(super) fn bookmarks_page(&self) -> String {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('\'', "&#39;").replace('"', "&quot;");
        let row = |url: &str, title: &str| alloc::format!("<li><a href='{}'>{}</a></li>", esc(url), esc(if title.is_empty() { url } else { title }));
        let marks: String = self.profile.bookmarks.iter().rev().map(|(u, t)| row(u, t)).collect();
        let recent: String = self.profile.history.iter().rev().take(20).map(|v| row(&v.url, &v.title)).collect();
        alloc::format!(
            "<!doctype html><title>Bookmarks</title><style>body{{font:14px sans-serif;margin:12px;color:#1f1f1f}}h2{{font-size:13px;color:#5f6368;margin:14px 0 6px}}\
             ul{{list-style:none;padding:0;margin:0}}li{{margin:6px 0;white-space:nowrap;overflow:hidden}}a{{color:#0b57d0;text-decoration:none}}</style>\
             <h2>Bookmarks</h2><ul>{}</ul><h2>Recent</h2><ul>{}</ul>",
            if marks.is_empty() { String::from("<li>None yet. Ctrl+D bookmarks a page.</li>") } else { marks },
            if recent.is_empty() { String::from("<li>Nothing yet.</li>") } else { recent })
    }
}
