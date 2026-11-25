// SPDX-License-Identifier: GPL-3.0-only
//! The browser's own Ctrl shortcuts, which pages don't get, and find in page.
//!
//! Ctrl+T new tab, Ctrl+W close it, Ctrl+PageDown / Ctrl+PageUp next and previous
//! tab, Ctrl+1..8 that tab and Ctrl+9 the last (Tab itself is the desktop's),
//! Ctrl+L the address, Ctrl+D bookmark, Ctrl+F find, Ctrl+= / Ctrl+- / Ctrl+0 zoom,
//! Ctrl+R reload, Ctrl+J downloads, Ctrl+Shift+Backspace forget this site's cookies and storage,
//! Ctrl+Shift+S Shields down (or up) for this site, Ctrl+Shift+R reader view,
//! Ctrl+Shift+P a private tab, Ctrl+Shift+1..4 a tab in a container, Ctrl+Shift+,
//! tabs down the side (vertical tabs) or back across the bar, Ctrl+Shift+\\ split view,
//! Ctrl+Shift+G the tab into a new group or out of its group (a click on a group's
//! label folds it), Ctrl+Shift+B the side panel, Ctrl+Shift+E site settings,
//! Ctrl+Shift+K asks Chastity to fill a visible HTTPS login form; Ctrl+Shift+U asks it to save one.
//! Ctrl+Alt+J clears download history in the panel. Ctrl+Shift+/ opens Kaleidoscope's keyboard reference.
//! On site settings, Ctrl+Alt+C clears only cookies and Ctrl+Alt+S clears only local storage.

use super::{nav, profile::Jarbox, Browser, Nav};
use sex_web::sex_html;
use alloc::string::String;

/// Zoom steps, in percent, as other browsers use.
const ZOOMS: &[u32] = &[25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500];

impl Browser {
    /// True when `key` (with Ctrl) was one of the browser's.
    pub(super) fn shortcut(&mut self, key: sex_pdx::app_key::Key) -> bool {
        match key.code {
            // The reference is a built-in page, so it works without a network grant.
            0x35 if key.shift => {
                self.open("about:shortcuts", true);
                web_record!("[web.shortcuts.opened] url=about:shortcuts");
            }
            // Ctrl+T: a new tab; from a private tab it is private too, from a container
            // tab it is an ordinary one (as in Firefox).
            0x14 => { let b = if self.jarbox == Jarbox::Private { Jarbox::Private } else { Jarbox::Own }; self.new_tab(None, b) }
            // Ctrl+Shift+P: a private tab. Ctrl+Shift+1..4: a tab in that container.
            0x19 if key.shift => self.new_tab(None, Jarbox::Private),
            0x02..=0x05 if key.shift => self.new_tab(None, Jarbox::Container((key.code - 0x01) as u8)),
            // Ctrl+Shift+, (as in Edge): tabs down the side, or back across the top.
            0x33 if key.shift => { let on = !self.vertical; self.set_vertical(on) }
            // Ctrl+Shift+\: split view, the shown tab beside the next one, or one again.
            0x2B if key.shift => self.toggle_split(),
            // Ctrl+Shift+G: the tab into a new group (as Chrome's "Add tab to new group"), or out.
            0x22 if key.shift => self.toggle_group(),
            // Ctrl+Shift+B: the side panel with the bookmarks, or closed.
            0x30 if key.shift => self.toggle_panel(),
            // These actions require the browser's own site settings page. A
            // site cannot trigger them by navigating to a crafted address.
            0x2E if key.alt && !key.shift => self.clear_site_part(sex_web::site_data::Part::Cookies),
            0x1F if key.alt && !key.shift => self.clear_site_part(sex_web::site_data::Part::Storage),
            // Ctrl+Shift+E: settings for the current web site; press again to return.
            0x12 if key.shift => {
                let url = self.url.clone();
                if let Some(site) = super::site_settings::target(&url) { self.open(&site, true); }
                else if sex_fetch::Url::parse(&url).is_ok() { self.open(&super::site_settings::address(&url), true); }
            }
            0x25 if key.shift => self.request_autofill(),
            0x18 if key.shift => self.request_password_save(),
            0x11 => self.close_tab(),
            // Next and previous go round the strip's tabs (not the side panel's page).
            0x51 => { let n = (self.active + 1) % self.strip_len(); self.show_tab(n); }
            0x49 => { let n = (self.active + self.strip_len() - 1) % self.strip_len(); self.show_tab(n); }
            0x02..=0x09 => { let i = (key.code - 0x02) as usize; if i < self.strip_len() { self.show_tab(i) } }
            0x0A => self.show_tab(self.strip_len() - 1),
            0x26 => { self.editing = Some(self.url.clone()); self.suggestions.clear(); self.chosen = None; self.draw(); }
            0x20 => {
                let title = self.page.title.clone();
                let url = self.url.clone();
                let on = self.profile.toggle_bookmark(&url, &title);
                web_record!("[web.bookmark] on={} count={} url={}", on as u8, self.profile.bookmarks.len(), url);
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
            // Ctrl+Shift+R: reader view of this page, or back to the page from it.
            0x13 if key.shift => {
                let url = self.url.clone();
                match url.strip_prefix("about:reader?url=") {
                    Some(page) => { let page = super::extensions::decode(page); self.open(&page, true) }
                    None if url.starts_with("http") => {
                        // Use the DOM the person is actually reading. A second fetch
                        // can return different text or lose authenticated state.
                        let article = sex_web::reader::article(&self.doc.get().dom, &url);
                        let page = match article {
                            Some(a) => {
                                web_record!("[web.reader] words={} title={} url={}", a.words, a.title, url);
                                sex_web::reader::page(&a, &url)
                            }
                            None => {
                                web_record!("[web.reader] words=0 url={}", url);
                                super::message_page("No reader view", "This page has no article to show on its own.", &url)
                            }
                        };
                        self.reader_pending = Some((url.clone(), page));
                        self.open(&alloc::format!("about:reader?url={}", encode(&url)), true);
                    }
                    None => {}
                }
            }
            0x13 => { let u = self.url.clone(); self.open(&u, false); }
            0x24 if key.alt => self.clear_download_history(),
            0x24 => self.open("about:downloads", true),
            // Ctrl+Shift+S: Shields down for this site, or up again; the page reloads.
            0x1F if key.shift => {
                let url = super::site_settings::target(&self.url).unwrap_or_else(|| self.url.clone());
                self.shields.toggle(&mut self.window, &url);
                let shown = self.url.clone();
                self.open(&shown, false);
            }
            0x0E if key.shift => {
                let url = super::site_settings::target(&self.url).unwrap_or_else(|| self.url.clone());
                self.profile.clear_site(&mut self.jar.borrow_mut(), &mut self.storage.borrow_mut(), &url);
                self.code_store.forget(&mut self.window, &mut self.code_cache.borrow_mut(), &url);
                web_record!("[web.site.cleared] url={}", url);
                if super::site_settings::target(&self.url).is_some() { let shown = self.url.clone(); self.open(&shown, false); }
            }
            _ => return false,
        }
        true
    }

    fn clear_site_part(&mut self, part: sex_web::site_data::Part) {
        let Some(url) = super::site_settings::target(&self.url) else { return; };
        let Some((cleared, kept)) = self.profile.clear_site_part(&mut self.window,
            &mut self.jar.borrow_mut(), &mut self.storage.borrow_mut(),
            self.jarbox, &url, part, super::wall()) else { return; };
        let kind = match part { sex_web::site_data::Part::Cookies => "cookies", sex_web::site_data::Part::Storage => "storage" };
        web_record!("[web.site.part-cleared] kind={} count={} kept={} url={}", kind, cleared, kept as u8, url);
        if !kept {
            self.status = Some("Site data changed here, but Files could not confirm the save.");
            self.draw();
            return;
        }
        let shown = self.url.clone();
        self.open(&shown, false);
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
        web_record!("[web.zoom] percent={} height={} items={}", percent, self.page.height, self.page.items.len());
        self.draw();
    }

    /// Finds the search again (the page changed or the text did).
    pub(super) fn refind(&mut self) {
        let Some(q) = self.find.clone() else { return };
        self.marks = sex_web::find_marks(&self.page, &mut self.fonts, &q);
        if self.find_at >= self.matches() { self.find_at = 0; }
    }

    /// How many places the search was found (a match may have a mark on each line).
    pub(super) fn matches(&self) -> usize { self.marks.last().map_or(0, |m| m[4] as usize + 1) }

    /// Scrolls the current match into view.
    fn show_match(&mut self) {
        if let Some(m) = self.marks.iter().find(|m| m[4] as usize == self.find_at) {
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
                let n = self.matches();
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
        web_record!("[web.find] query={} matches={} at={}", self.find.as_deref().unwrap_or(""), self.matches(), self.find_at);
        self.draw();
    }

    /// Saves the profile and ends the program.
    pub(super) fn quit(&mut self) -> ! {
        self.cancel_autofill();
        self.cancel_password_save();
        self.profile.save(&mut self.window, super::wall(), true);
        self.code_store.save(&mut self.window, &mut self.code_cache.borrow_mut());
        sex_pdx::serial_record!("[web.closed] target={} task={}", self.window.target, self.window.task);
        sex_pdx::task_finish(0)
    }
}

/// `s` as a query value (what [`super::extensions::decode`] reads back).
pub(super) fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/' | b':') { out.push(b as char); } else { out.push_str(&alloc::format!("%{:02X}", b)); }
    }
    out
}
