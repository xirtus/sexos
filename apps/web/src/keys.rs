// SPDX-License-Identifier: GPL-3.0-only
//! The browser's own Ctrl shortcuts, which pages don't get, and find in page.
//!
//! Ctrl+T new tab, Ctrl+W close it, Ctrl+PageDown / Ctrl+PageUp next and previous
//! tab, Ctrl+1..8 that tab and Ctrl+9 the last (Tab itself is the desktop's),
//! Ctrl+L the address, Ctrl+D bookmark, Ctrl+F find, Ctrl+= / Ctrl+- / Ctrl+0 zoom,
//! Ctrl+R reload, Ctrl+Shift+Backspace forget this site's cookies and storage.

use super::{nav, Browser, Nav};
use sex_web::sex_html;
use alloc::string::String;

/// Zoom steps, in percent, as other browsers use.
const ZOOMS: &[u32] = &[25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500];

impl Browser {
    /// True when `key` (with Ctrl) was one of the browser's.
    pub(super) fn shortcut(&mut self, key: sex_pdx::app_key::Key) -> bool {
        match key.code {
            0x14 => self.new_tab(None),
            0x11 => self.close_tab(),
            0x51 => { let n = (self.active + 1) % self.tabs.len(); self.show_tab(n); }
            0x49 => { let n = (self.active + self.tabs.len() - 1) % self.tabs.len(); self.show_tab(n); }
            0x02..=0x09 => self.show_tab((key.code - 0x02) as usize),
            0x0A => self.show_tab(self.tabs.len() - 1),
            0x26 => { self.editing = Some(self.url.clone()); self.suggestions.clear(); self.chosen = None; self.draw(); }
            0x20 => {
                let title = self.page.title.clone();
                let url = self.url.clone();
                let on = self.profile.toggle_bookmark(&url, &title);
                sex_pdx::serial_record!("[web.bookmark] on={} count={} url={}", on as u8, self.profile.bookmarks.len(), url);
                self.draw();
            }
            0x21 => { self.finding = true; if self.find.is_none() { self.find = Some(String::new()); } self.draw(); }
            0x0D | 0x0C | 0x0B => {
                let now = sex_html::style::zoom();
                let next = match key.code {
                    0x0D => ZOOMS.iter().copied().find(|z| *z > now).unwrap_or(now),
                    0x0C => ZOOMS.iter().rev().copied().find(|z| *z < now).unwrap_or(now),
                    _ => 100,
                };
                self.set_zoom(next);
            }
            0x13 => { let u = self.url.clone(); self.open(&u, false); }
            0x0E if key.shift => {
                let url = self.url.clone();
                self.profile.clear_site(&mut self.jar.borrow_mut(), &url);
                sex_pdx::serial_record!("[web.site.cleared] url={}", url);
            }
            _ => return false,
        }
        true
    }

    /// Lays every tab's pages out again at `percent` zoom, keeping the reading place.
    fn set_zoom(&mut self, percent: u32) {
        let before = sex_html::style::zoom();
        if percent == before { return; }
        sex_html::style::set_zoom(percent);
        let at = if self.page.height > 0 { self.scroll as i64 * 10_000 / self.page.height as i64 } else { 0 };
        self.relayout();
        self.scroll = ((at * self.page.height as i64 / 10_000) as i32).clamp(0, self.max_scroll());
        self.full_redraw = true;
        self.refind();
        sex_pdx::serial_record!("[web.zoom] percent={} height={} items={}", percent, self.page.height, self.page.items.len());
        self.draw();
    }

    /// Finds the search again (the page changed or the text did).
    pub(super) fn refind(&mut self) {
        let Some(q) = self.find.clone() else { return };
        self.marks = sex_web::find_marks(&self.page, &mut self.fonts, &q);
        if self.find_at >= self.marks.len() { self.find_at = 0; }
    }

    /// Scrolls the current match into view.
    fn show_match(&mut self) {
        if let Some(m) = self.marks.get(self.find_at) {
            let view = self.view.1;
            if m[1] < self.scroll || m[1] + m[3] > self.scroll + view { self.scroll = (m[1] - view / 3).clamp(0, self.max_scroll()); }
        }
    }

    /// A key while the find bar has them: typing searches, Enter (Shift+Enter) goes to
    /// the next (previous) match, Escape closes the bar.
    pub(super) fn find_key(&mut self, key: sex_pdx::app_key::Key) {
        let Some(q) = self.find.as_mut() else { self.finding = false; return };
        match key.code {
            0x01 => { self.find = None; self.finding = false; self.marks.clear(); self.draw(); return; }
            0x1C => {
                let n = self.marks.len();
                if n > 0 { self.find_at = if key.shift { (self.find_at + n - 1) % n } else { (self.find_at + 1) % n }; }
            }
            0x0E => { q.pop(); self.find_at = 0; self.refind(); }
            _ => match (nav(key), sex_app::typed(key)) {
                (Some(Nav::Up | Nav::Down), _) => {}
                (_, Some(c)) => { q.push(c as char); self.find_at = 0; self.refind(); }
                _ => return,
            },
        }
        self.show_match();
        sex_pdx::serial_record!("[web.find] query={} matches={} at={}", self.find.as_deref().unwrap_or(""), self.marks.len(), self.find_at);
        self.draw();
    }

    /// Saves the profile and ends the program.
    pub(super) fn quit(&mut self) -> ! {
        self.profile.touched(&self.jar.borrow());
        self.profile.save(&mut self.window, &self.jar.borrow(), super::wall(), true);
        sex_pdx::serial_record!("[web.closed] target={} task={}", self.window.target, self.window.task);
        sex_pdx::task_finish(0)
    }
}
