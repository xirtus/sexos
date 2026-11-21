// SPDX-License-Identifier: GPL-3.0-only
//! Tabs: several pages in one window, each with its own tree, script realm, history
//! and frames. The shown tab's page lives in [`Browser`]'s fields; the others wait in
//! [`Tab`]s, swapped in when shown. A tab in the background keeps its state and its
//! scripts' heap, but its timers and media pause until it shows again.

use super::{clock, profile::Jarbox, Browser, Doc, Frame};
use alloc::string::String;
use alloc::vec::Vec;
use sex_html::layout::Page;
use sex_web::Slot;
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
    marks: Vec<[i32; 5]>,
    pub title: String,
    laid_zoom: u32,
    blocked: u32,
    pub(super) jar: alloc::rc::Rc<core::cell::RefCell<sex_fetch::Jar>>,
    pub(super) storage: alloc::rc::Rc<core::cell::RefCell<sex_js::storage::WebStorage>>,
    pub jarbox: Jarbox,
    /// The tab group this slot's tab is in. It stays with the slot, not the page,
    /// so it isn't swapped when the tab shows.
    pub group: Option<usize>,
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
            moving: Vec::new(), frames_shown: 0, find: None, find_at: 0, marks: Vec::new(), title: String::new(), laid_zoom: sex_html::style::zoom(), blocked: 0,
            jar: Default::default(), storage: Default::default(), jarbox: Jarbox::Own, group: None,
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
        swap(&mut self.blocked, &mut t.blocked);
        swap(&mut self.jar, &mut t.jar); swap(&mut self.storage, &mut t.storage); swap(&mut self.jarbox, &mut t.jarbox);
    }

    /// The colour under each tab: its box's.
    pub(super) fn tab_colours(&self) -> Vec<u32> {
        (0..self.strip_len()).map(|i| if i == self.active { self.jarbox } else { self.tabs[i].jarbox }.colour()).collect()
    }

    /// The label of each tab for the strip.
    pub(super) fn tab_labels(&self) -> Vec<String> {
        (0..self.strip_len()).map(|i| {
            let (title, url) = if i == self.active { (self.page.title.as_str(), self.url.as_str()) } else { (self.tabs[i].title.as_str(), self.tabs[i].url.as_str()) };
            String::from(if !title.is_empty() { title } else if url.is_empty() { "New tab" } else { url.trim_start_matches("https://").trim_start_matches("http://") })
        }).collect()
    }

    /// Shows tab `i`.
    pub(super) fn show_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active || self.in_frame.is_some() { return; }
        // Showing a tab of a folded group unfolds it, as in Chrome.
        if let Some(g) = self.tabs[i].group.filter(|g| self.groups.get(*g).is_some_and(|g| g.folded)) {
            self.groups[g].folded = false;
            sex_pdx::serial_record!("[web.group.folded] group={} on=0", g);
        }
        // A tab outside the split ends it, as in Edge.
        // With the side panel open, the tab shown takes its place beside it.
        if let Some(p) = self.panel_slot().filter(|p| i != *p) { self.split = Some((i, p)); }
        else if self.split.is_some_and(|(l, r)| i != l && i != r) { self.split = None; sex_pdx::serial_record!("[web.split] on=0 shown={}", i); }
        // The tab going back pauses its sound and remembers its title.
        for m in self.media.iter_mut() { m.hush(); }
        let title = self.page.title.clone();
        let from = self.active;
        self.swap_tab(from);
        self.tabs[from].title = title;
        self.swap_tab(i);
        self.active = i;
        // Its scripts compile under its own site again.
        self.code_site();
        self.editing = None;
        self.suggestions.clear();
        self.press_link = None;
        self.full_redraw = true;
        // The window may have changed size while the tab waited.
        let want = self.page_view();
        if self.view != want || self.laid_zoom != sex_html::style::zoom() {
            self.view = want;
            self.doc.get().viewport = want;
            self.relayout();
            if let Some(script) = self.doc.script() { let _ = script.dispatch("resize"); }
            self.after_script(false);
        }
        self.timers_changed();
        let title: Vec<u8> = self.page.title.bytes().take(16).collect();
        self.window.title(if title.is_empty() { b"Kaleidoscope" } else { &title });
        sex_pdx::serial_record!("[web.tab.shown] index={} count={} url={}", i, self.tabs.len(), self.url);
        self.draw();
    }

    /// Opens a new tab after the shown one, its cookies and storage from `jarbox`,
    /// showing `url` (or a blank page with the address bar ready for typing).
    pub(super) fn new_tab(&mut self, url: Option<&str>, jarbox: Jarbox) {
        if self.tabs.len() >= MAX_TABS || self.in_frame.is_some() { return; }
        // New tabs go before the side panel's slot, and show beside the panel.
        let at = (self.active + 1).min(self.strip_len());
        self.split = None;
        let mut blank = Tab::blank(self);
        (blank.jar, blank.storage) = self.profile.store(&mut self.window, jarbox, super::wall());
        blank.jarbox = jarbox;
        // A tab opened from a grouped tab joins its group, beside it.
        blank.group = self.tabs[self.active].group;
        self.tabs.insert(at, blank);
        // From the side panel, the new tab goes before the panel's slot, which moves up.
        if at <= self.active { self.active += 1; }
        if self.panel { self.split = Some((at, self.tabs.len() - 1)); }
        self.show_tab(at);
        sex_pdx::serial_record!("[web.tab.opened] index={} count={} box={}", at, self.tabs.len(), jarbox.tag());
        if let Some(g) = self.tabs[at].group { sex_pdx::serial_record!("[web.group.joined] tab={} group={}", at, g); }
        match url {
            Some(u) => self.open(u, false),
            None => { self.open("about:blank", false); self.editing = Some(String::new()); self.draw(); }
        }
    }

    /// Closes the shown tab, showing the one after it (or before, at the end). The
    /// last tab closing closes the window.
    pub(super) fn close_tab(&mut self) {
        if self.in_frame.is_some() { return; }
        // Ctrl+W in the side panel closes the panel.
        if self.panel_shown() { return self.close_panel(); }
        if self.strip_len() <= 1 { self.quit(); }
        let going = self.active;
        let next = if going + 1 < self.strip_len() { going + 1 } else { going - 1 };
        self.split = None;
        self.drop_shown_page();
        self.show_tab(next);
        self.tabs.remove(going);
        if self.active > going { self.active -= 1; }
        self.prune_groups();
        self.repair_panel();
        // The last private tab took the private box's cookies and storage with it.
        if self.jarbox != Jarbox::Private && !self.tabs.iter().enumerate().any(|(i, t)| i != self.active && t.jarbox == Jarbox::Private) { self.profile.forget_private(); }
        sex_pdx::serial_record!("[web.tab.closed] index={} count={} shown={}", going, self.tabs.len(), self.active);
        self.draw();
    }

    /// Takes the shown page's frames, sockets, sound and downloads down, as it goes.
    pub(super) fn drop_shown_page(&mut self) {
        let owner = self.doc.get().owner();
        self.ws_close_page(owner);
        self.drop_frames();
        for mut l in core::mem::take(&mut self.media_loads) { l.ex.cancel(&mut self.window); }
        self.media.clear();
    }

    /// In a split view, paints the half that waits (as it last was) and the line
    /// between the halves. Its frames and moving pictures wait until it shows.
    pub(super) fn paint_split(&mut self) {
        let Some((l, r)) = self.split_pair() else { return };
        let Some((left, right)) = self.columns() else { return };
        let (other, (ox, ow)) = if self.active == l { (r, right) } else { (l, left) };
        let gap = left.0 + left.1;
        let (w, h) = (self.surface.w, self.surface.h);
        let t = &mut self.tabs[other];
        use core::mem::swap;
        swap(&mut self.fonts.images, &mut t.images); swap(&mut self.fonts.svgs, &mut t.svgs);
        let pixels = &mut self.window.pixels()[..self.surface.pixels];
        if let Some(mut target) = sex_web::column(pixels, w, h, t.scroll, ox, ow) { sex_web::paint(&t.page, &mut self.fonts, &mut target, None); }
        swap(&mut self.fonts.images, &mut t.images); swap(&mut self.fonts.svgs, &mut t.svgs);
        for y in sex_web::BAR..h {
            let row = (y * w) as usize;
            for x in gap..(gap + super::SPLIT_GAP).min(w) { pixels[row + x as usize] = 0x0022_2C3A; }
        }
    }

    /// The chip under window point (`x`, `y`) in the bar, if any: a tab or a group's label.
    pub(super) fn tab_at(&self, x: i32, y: i32) -> Option<Slot> {
        if !(0..sex_web::BAR).contains(&y) || self.side() > 0 { return None; }
        let slots = self.slots();
        sex_web::tab_chips(self.surface.w, &slots, &self.groups).iter().position(|(cx, w)| x >= *cx && x < cx + w).map(|i| slots[i])
    }

    /// The row under window point (`x`, `y`) in the vertical strip, if any.
    pub(super) fn side_at(&self, x: i32, y: i32) -> Option<Slot> {
        let slots = self.slots();
        sex_web::side_row_at(self.side(), slots.len(), x, y).map(|i| slots[i])
    }

    /// Each tab's group.
    pub(super) fn group_of(&self) -> Vec<Option<usize>> { self.tabs[..self.strip_len()].iter().map(|t| t.group).collect() }

    /// The strip's places: group labels and the tabs not folded away.
    pub(super) fn slots(&self) -> Vec<Slot> { sex_web::strip_slots(&self.group_of(), &self.groups) }

    /// Moves tab slot `from` to `to`, the shown tab staying shown. Ends a split view.
    fn move_tab(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() { return; }
        if !self.panel && self.split.take().is_some() { sex_pdx::serial_record!("[web.split] on=0 shown={}", self.active); }
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.active = if self.active == from { to } else if from < self.active && to >= self.active { self.active - 1 } else if from > self.active && to <= self.active { self.active + 1 } else { self.active };
        self.repair_panel();
    }

    /// Ctrl+Shift+G: the shown tab into a new group, named for its site, or out of its
    /// group (to just after it).
    pub(super) fn toggle_group(&mut self) {
        if self.in_frame.is_some() || self.panel_shown() { return; }
        let at = self.active;
        match self.tabs[at].group {
            Some(g) => {
                let last = (0..self.tabs.len()).rev().find(|i| self.tabs[*i].group == Some(g)).unwrap_or(at);
                self.tabs[at].group = None;
                self.move_tab(at, last);
                sex_pdx::serial_record!("[web.group.left] group={} from={} to={}", g, at, self.active);
                self.prune_groups();
                // A split that ended gives the shown page the whole width again.
                return self.refit();
            }
            None => {
                if self.groups.len() >= MAX_GROUPS { return; }
                // The next colour no group has yet, as Chrome picks.
                let colour = sex_web::GROUP_COLOURS.iter().copied().find(|c| !self.groups.iter().any(|g| g.colour == *c)).unwrap_or(sex_web::GROUP_COLOURS[self.groups.len() % 8]);
                let name = sex_web::group_name(&self.url);
                sex_pdx::serial_record!("[web.group] group={} tab={} name={}", self.groups.len(), at, name);
                self.groups.push(sex_web::TabGroup { colour, name, folded: false });
                self.tabs[at].group = Some(self.groups.len() - 1);
            }
        }
        self.draw();
    }

    /// A click on a group's label: folds its tabs away, or shows them again. Folding
    /// the group of the shown tab shows the nearest tab outside it first.
    pub(super) fn fold_group(&mut self, g: usize) {
        if g >= self.groups.len() || self.in_frame.is_some() { return; }
        let on = !self.groups[g].folded;
        if on && self.tabs[self.active].group == Some(g) {
            let n = self.strip_len();
            let after = (self.active..n).find(|i| self.tabs[*i].group != Some(g));
            let before = (0..self.active).rev().find(|i| self.tabs[*i].group != Some(g));
            let Some(next) = after.or(before) else { return };
            self.show_tab(next);
        }
        self.groups[g].folded = on;
        sex_pdx::serial_record!("[web.group.folded] group={} on={} tabs={}", g, on as u8, self.tabs.iter().filter(|t| t.group == Some(g)).count());
        self.draw();
    }

    /// Forgets groups with no tabs left, renumbering the rest.
    fn prune_groups(&mut self) {
        let mut g = 0;
        while g < self.groups.len() {
            if self.tabs.iter().any(|t| t.group == Some(g)) { g += 1; continue; }
            self.groups.remove(g);
            for t in self.tabs.iter_mut() { if let Some(x) = t.group.as_mut().filter(|x| **x > g) { *x -= 1; } }
            sex_pdx::serial_record!("[web.group.gone] group={} left={}", g, self.groups.len());
        }
    }
}

/// Most groups one window holds.
pub const MAX_GROUPS: usize = 8;
