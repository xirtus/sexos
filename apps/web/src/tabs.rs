// SPDX-License-Identifier: GPL-3.0-only
//! Tabs: several pages in one window, each with its own tree, script realm, history
//! and frames. The shown tab's page lives in [`Browser`]'s fields; the others wait in
//! [`Tab`]s, swapped in when shown. A tab in the background keeps its state and its
//! scripts' heap, but its timers and media pause until it shows again.

use super::{clock, Browser, Doc, Frame};
use alloc::string::String;
use alloc::vec::Vec;
use sex_html::layout::Page;
use sex_js::Document;
use sex_web::sex_html::{self, Host};

/// Most tabs one window holds.
pub const MAX_TABS: usize = 24;

pub struct Tab {
    page: Page,
    doc: Doc,
    sheets: Vec<String>,
    timer: Option<u64>,
    timeline: sex_html::Timeline,
    animating: bool,
    canvases: sex_web::canvas::Canvases,
    media: Vec<super::media::Media>,
    media_loads: Vec<super::MediaLoad>,
    pub url: String,
    history: Vec<String>,
    forward: Vec<String>,
    scroll: i32,
    focus: Option<usize>,
    hover_css: bool,
    import_map: Vec<(String, String)>,
    script_scroll: i32,
    view: (i32, i32),
    frames: Vec<Frame>,
    images: Vec<(String, alloc::rc::Rc<sex_web::Image>)>,
    svgs: Vec<(String, alloc::rc::Rc<str>, (i32, i32))>,
    moving: Vec<sex_web::Moving>,
    frames_shown: u64,
    find: Option<String>,
    find_at: usize,
    marks: Vec<[i32; 4]>,
    pub title: String,
    laid_zoom: u32,
}

impl Tab {
    /// An empty tab, before its first page.
    pub fn blank(b: &Browser) -> Tab {
        let view = b.view;
        Tab {
            page: sex_html::render("", view.0, view.1, &b.fonts as &dyn Host),
            doc: Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", view, clock)),
            sheets: Vec::new(), timer: None, timeline: Default::default(), animating: false,
            canvases: sex_web::canvas::Canvases::new(), media: Vec::new(), media_loads: Vec::new(),
            url: String::new(), history: Vec::new(), forward: Vec::new(), scroll: 0, focus: None, hover_css: false,
            import_map: Vec::new(), script_scroll: 0, view, frames: Vec::new(), images: Vec::new(), svgs: Vec::new(),
            moving: Vec::new(), frames_shown: 0, find: None, find_at: 0, marks: Vec::new(), title: String::new(), laid_zoom: sex_html::style::zoom(),
        }
    }
}

impl Browser {
    /// Swaps tab slot `i` with the shown page.
    fn swap_tab(&mut self, i: usize) {
        use core::mem::swap;
        let t = &mut self.tabs[i];
        swap(&mut self.page, &mut t.page); swap(&mut self.doc, &mut t.doc); swap(&mut self.sheets, &mut t.sheets);
        swap(&mut self.timer, &mut t.timer); swap(&mut self.timeline, &mut t.timeline); swap(&mut self.animating, &mut t.animating);
        swap(&mut self.canvases, &mut t.canvases); swap(&mut self.media, &mut t.media); swap(&mut self.media_loads, &mut t.media_loads);
        swap(&mut self.url, &mut t.url); swap(&mut self.history, &mut t.history); swap(&mut self.forward, &mut t.forward);
        swap(&mut self.scroll, &mut t.scroll); swap(&mut self.focus, &mut t.focus); swap(&mut self.hover_css, &mut t.hover_css);
        swap(&mut self.import_map, &mut t.import_map); swap(&mut self.script_scroll, &mut t.script_scroll); swap(&mut self.view, &mut t.view);
        swap(&mut self.frames, &mut t.frames);
        swap(&mut self.fonts.images, &mut t.images); swap(&mut self.fonts.svgs, &mut t.svgs); swap(&mut self.fonts.moving, &mut t.moving);
        swap(&mut self.fonts.frames_shown, &mut t.frames_shown);
        swap(&mut self.find, &mut t.find); swap(&mut self.find_at, &mut t.find_at); swap(&mut self.marks, &mut t.marks);
        swap(&mut self.laid_zoom, &mut t.laid_zoom);
    }

    /// The label of each tab for the strip.
    pub(super) fn tab_labels(&self) -> Vec<String> {
        (0..self.tabs.len()).map(|i| {
            let (title, url) = if i == self.active { (self.page.title.as_str(), self.url.as_str()) } else { (self.tabs[i].title.as_str(), self.tabs[i].url.as_str()) };
            String::from(if !title.is_empty() { title } else if url.is_empty() { "New tab" } else { url.trim_start_matches("https://").trim_start_matches("http://") })
        }).collect()
    }

    /// Shows tab `i`.
    pub(super) fn show_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active || self.in_frame.is_some() { return; }
        // The tab going back pauses its sound and remembers its title.
        for m in self.media.iter_mut() { m.hush(); }
        let title = self.page.title.clone();
        let from = self.active;
        self.swap_tab(from);
        self.tabs[from].title = title;
        self.swap_tab(i);
        self.active = i;
        self.editing = None;
        self.suggestions.clear();
        self.press_link = None;
        self.full_redraw = true;
        // The window may have changed size while the tab waited.
        let want = self.surface.document(sex_web::BAR);
        if self.view != want || self.laid_zoom != sex_html::style::zoom() {
            self.view = want;
            self.doc.get().viewport = want;
            self.relayout();
            if let Some(script) = self.doc.script() { let _ = script.dispatch("resize"); }
            self.after_script(false);
        }
        self.timers_changed();
        let title: Vec<u8> = self.page.title.bytes().take(16).collect();
        self.window.title(if title.is_empty() { b"Web" } else { &title });
        sex_pdx::serial_record!("[web.tab.shown] index={} count={} url={}", i, self.tabs.len(), self.url);
        self.draw();
    }

    /// Opens a new tab after the shown one, showing `url` (or a blank page with the
    /// address bar ready for typing).
    pub(super) fn new_tab(&mut self, url: Option<&str>) {
        if self.tabs.len() >= MAX_TABS || self.in_frame.is_some() { return; }
        let at = self.active + 1;
        let blank = Tab::blank(self);
        self.tabs.insert(at, blank);
        self.show_tab(at);
        sex_pdx::serial_record!("[web.tab.opened] index={} count={}", at, self.tabs.len());
        match url {
            Some(u) => self.open(u, false),
            None => { self.open("about:blank", false); self.editing = Some(String::new()); self.draw(); }
        }
    }

    /// Closes the shown tab, showing the one after it (or before, at the end). The
    /// last tab closing closes the window.
    pub(super) fn close_tab(&mut self) {
        if self.in_frame.is_some() { return; }
        if self.tabs.len() <= 1 { self.quit(); }
        let going = self.active;
        let next = if going + 1 < self.tabs.len() { going + 1 } else { going - 1 };
        // Take the closing page's frames and downloads down with it.
        let owner = self.doc.get().owner();
        self.ws_close_page(owner);
        self.drop_frames();
        for mut l in core::mem::take(&mut self.media_loads) { l.ex.cancel(&mut self.window); }
        self.media.clear();
        self.show_tab(next);
        self.tabs.remove(going);
        if self.active > going { self.active -= 1; }
        sex_pdx::serial_record!("[web.tab.closed] index={} count={} shown={}", going, self.tabs.len(), self.active);
        self.draw();
    }

    /// The tab chip under window point (`x`, `y`), if any.
    pub(super) fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        if !(0..sex_web::BAR).contains(&y) { return None; }
        sex_web::tab_chips(self.surface.w, self.tabs.len()).iter().position(|(cx, w)| x >= *cx && x < cx + w)
    }
}
