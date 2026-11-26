// SPDX-License-Identifier: GPL-3.0-only
//! Helium-style ultra-clean single-row compact browser chrome for Kaleidoscope (`apps/web`),
//! harmonized with SexOS's Candy Lavender aesthetic.
//!
//! Layout (single sleek 38px top bar by default; optional 26px toggleable bookmarks bar below):
//!   [ ← ][ → ][ ↻ ]  ( 🔒 url / search ... ★ )  [ ● Tab 1 × ] [ ● Tab 2 × ] [ + ]   [ 🛡 On ][ ▤ ][ ⋮ ]
//!
//! Pure integer layout, hit-testing, omnibox URL/search resolution, bookmark bar
//! helpers, tab strip state helpers, context menu geometry, and pixel painting.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_html::layout::{FontKey, Host, Page};

use crate::{column_below, paint, text_run, Fonts, Target};

// ── Layout metrics ───────────────────────────────────────────────────────────

/// Height of the compact Helium-style single-row top bar (matches canonical 30px viewport origin).
pub const TOP_BAR_H: i32 = 30;
/// Height of the optional toggleable bookmarks bar beneath the top bar.
pub const BOOKMARKS_H: i32 = 26;
/// Default height of the browser chrome above the page viewport (single-row compact bar).
pub const BAR: i32 = TOP_BAR_H;

/// Effective chrome height above the page viewport given whether the bookmarks bar is shown.
#[inline]
pub const fn chrome_height(show_bookmarks_bar: bool) -> i32 {
    if show_bookmarks_bar {
        TOP_BAR_H + BOOKMARKS_H
    } else {
        TOP_BAR_H
    }
}

/// The vertical tab strip's width, when vertical tabs are enabled.
pub const SIDE: i32 = 200;
/// Height of one tab in the vertical strip.
pub const SIDE_ROW: i32 = 30;
/// Height of one omnibox suggestion row.
pub const SUGGESTION_ROW: i32 = 24;
/// Width of the floating context menu.
pub const CONTEXT_MENU_W: i32 = 204;
/// Height of one row in the floating context menu.
pub const CONTEXT_MENU_ROW: i32 = 26;
/// Extra vertical padding for a separator in the context menu.
pub const CONTEXT_MENU_SEP: i32 = 7;

// ── Candy Lavender palette (ARGB) ────────────────────────────────────────────

const BG_TOP_BAR: u32 = 0xFF1C_1730;
const BG_TAB_ACTIVE: u32 = 0xFF32_2A52;
const BG_TAB_ACTIVE_BORDER: u32 = 0xFF5A_4B94;
const BG_TAB_INACTIVE: u32 = 0xFF22_1C3A;
const ACCENT_LAVENDER: u32 = 0xFFA7_8BFA;
const ACCENT_VIOLET: u32 = 0xFF7C_5CFF;
const ACCENT_BERRY: u32 = 0xFFD6_336C;
const ACCENT_MINT: u32 = 0xFF34_D399;
const ACCENT_HONEY: u32 = 0xFFFB_BF24;
const ACCENT_SKY: u32 = 0xFF60_A5FA;
const BG_BOOKMARKS: u32 = 0xFF19_142B;
const BORDER_SUBTLE: u32 = 0xFF38_2E59;
const BTN_FILL: u32 = 0xFF2A_2245;
const BTN_ACTIVE: u32 = 0xFF3F_3466;
const BTN_DISABLED: u32 = 0xFF21_1B38;
const OMNIBOX_FILL: u32 = 0xFF14_1024;
const OMNIBOX_BORDER: u32 = 0xFF3E_3366;
const PILL_FILL: u32 = 0xFF2A_2245;
const TEXT_PRIMARY: u32 = 0xFFF3_EFFF;
const TEXT_SECONDARY: u32 = 0xFFB8_AED6;
const TEXT_MUTED: u32 = 0xFF6B_628A;
const MENU_BG: u32 = 0xF226_1F40;
const MENU_BORDER: u32 = 0xFF7C_64C4;
const MENU_HOVER: u32 = 0xFF46_3878;
const MENU_SHADOW: u32 = 0x990D_0A18;

const FAVICON_DOTS: [u32; 6] = [
    ACCENT_LAVENDER,
    ACCENT_SKY,
    ACCENT_MINT,
    ACCENT_BERRY,
    ACCENT_HONEY,
    0xFF2D_D4BF,
];

// ── Tab groups & slots ───────────────────────────────────────────────────────

/// A group of tabs that sit together in the strip under a coloured label.
#[derive(Clone, Debug, PartialEq)]
pub struct TabGroup {
    pub colour: u32,
    pub name: String,
    /// Folded: only the label shows, with how many tabs it holds.
    pub folded: bool,
}

/// The colours new groups take in turn: candy-harmonized accents.
pub const GROUP_COLOURS: [u32; 8] = [
    0xFFA7_8BFA,
    0xFFD6_336C,
    0xFF60_A5FA,
    0xFF34_D399,
    0xFFFB_923C,
    0xFFFB_BF24,
    0xFF2D_D4BF,
    0xFFF4_72B6,
];

/// One place in the tab strip: a group's label, or a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Group(usize),
    Tab(usize),
}

/// The strip's places, in order: each group's label before its first tab, and the
/// tabs of a folded group left out. A group's tabs sit together.
pub fn strip_slots(group_of: &[Option<usize>], groups: &[TabGroup]) -> Vec<Slot> {
    let mut out = Vec::new();
    let mut last = None;
    for (i, g) in group_of.iter().enumerate() {
        let g = g.filter(|g| *g < groups.len());
        if g.is_some() && g != last {
            out.push(Slot::Group(g.unwrap()));
        }
        last = g;
        if !g.is_some_and(|g| groups[g].folded) {
            out.push(Slot::Tab(i));
        }
    }
    out
}

/// How many tabs group `g` holds.
fn group_count(group_of: &[Option<usize>], g: usize) -> usize {
    group_of.iter().filter(|x| **x == Some(g)).count()
}

/// A new group's name: its tab's site, without `www.` or the ending (`example` for
/// news.example.com, `bbc` for bbc.co.uk); an address by number stays whole; nothing for built-in pages.
pub fn group_name(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else {
        return String::new();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if host.starts_with('[') {
        host
    } else {
        host.split(':').next().unwrap_or("")
    };
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') || host.starts_with('[') {
        return String::from(host);
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    let two = n >= 3 && labels[n - 1].len() == 2 && labels[n - 2].len() <= 3;
    String::from(match n {
        0 => "",
        1 => labels[0],
        _ if two => labels[n - 3],
        _ => labels[n - 2],
    })
}

fn group_chip(groups: &[TabGroup], g: usize) -> i32 {
    let Some(grp) = groups.get(g) else {
        return 24;
    };
    let name = grp.name.chars().count().min(12) as i32;
    let w = if name == 0 { 24 } else { 22 + name * 7 };
    if grp.folded && name > 0 {
        w + 26
    } else {
        w
    }
}

// ── Bookmarks bar model ──────────────────────────────────────────────────────

/// One clickable pill on the bookmarks bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkItem {
    pub title: String,
    pub url: String,
}

impl BookmarkItem {
    pub fn new(title: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
        }
    }
}

/// Default quick-access bookmarks shown in the toggleable bookmarks bar.
pub fn default_bookmarks() -> Vec<BookmarkItem> {
    alloc::vec![
        BookmarkItem::new("Home", "about:welcome"),
        BookmarkItem::new("Wikipedia", "https://en.wikipedia.org/"),
        BookmarkItem::new("DuckDuckGo", "https://html.duckduckgo.com/html/"),
        BookmarkItem::new("Hacker News", "https://news.ycombinator.com/"),
        BookmarkItem::new("YouTube", "https://www.youtube.com/"),
    ]
}

/// Combines default bookmarks with user-starred `(url, title)` pairs, deduplicated by URL.
pub fn merge_bookmarks(defaults: &[BookmarkItem], user: &[(String, String)]) -> Vec<BookmarkItem> {
    let mut out: Vec<BookmarkItem> = defaults.to_vec();
    for (url, title) in user {
        if url.is_empty() || out.iter().any(|b| b.url == *url) {
            continue;
        }
        let label = if !title.trim().is_empty() {
            String::from(title.trim())
        } else {
            let bare = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.");
            String::from(bare.split('/').next().unwrap_or(url))
        };
        out.push(BookmarkItem::new(label, url.clone()));
    }
    out
}

/// Host-testable bookmark list helper supporting add, remove, and toggle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookmarkStore {
    pub items: Vec<(String, String)>,
}

impl BookmarkStore {
    pub fn contains(&self, url: &str) -> bool {
        self.items.iter().any(|(u, _)| u == url)
    }

    pub fn add(&mut self, url: &str, title: &str) -> bool {
        if url.is_empty() || self.contains(url) {
            return false;
        }
        self.items
            .push((String::from(url), String::from(title.trim())));
        true
    }

    pub fn remove(&mut self, url: &str) -> bool {
        if let Some(i) = self.items.iter().position(|(u, _)| u == url) {
            self.items.remove(i);
            true
        } else {
            false
        }
    }

    /// Toggles bookmark state for `url`. Returns `true` if now bookmarked.
    pub fn toggle(&mut self, url: &str, title: &str) -> bool {
        if self.remove(url) {
            false
        } else {
            self.add(url, title)
        }
    }
}

/// Computes `(x, w)` positions of visible pills in the bookmarks bar.
pub fn bookmark_pills(width: i32, items: &[BookmarkItem]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut x = 8;
    let max_x = width - 8;
    for item in items {
        let chars = item.title.chars().count().min(16) as i32;
        let w = (24 + chars * 7).clamp(44, 148);
        if x + w > max_x {
            break;
        }
        out.push((x, w));
        x += w + 6;
    }
    out
}

/// Index of the bookmark pill under window point `(x, y)`, if any (only when bookmarks bar is shown).
pub fn bookmark_hit_at(width: i32, items: &[BookmarkItem], x: i32, y: i32) -> Option<usize> {
    if y < TOP_BAR_H || y >= TOP_BAR_H + BOOKMARKS_H || x < 0 || x >= width {
        return None;
    }
    bookmark_pills(width, items)
        .iter()
        .position(|&(px, pw)| x >= px && x < px + pw)
}

// ── Unified Omnibox URL vs Search resolution ─────────────────────────────────

/// Encodes a search query string for use in a URL query parameter.
pub fn encode_query(q: &str) -> String {
    let mut out = String::new();
    for b in q.bytes() {
        match b {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Returns `true` if `s` looks like a direct host/domain/IP rather than a search query.
fn looks_like_host(s: &str) -> bool {
    if s.is_empty() || s.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let host_part = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_part.split(':').next().unwrap_or("");
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if host.starts_with('[') && host.ends_with(']') {
        return true;
    }
    if let Some((before, after)) = host.split_once('.') {
        if !before.is_empty() && !after.is_empty() && !host.ends_with('.') && !host.contains("..") {
            let last = host.rsplit('.').next().unwrap_or("");
            return !last.is_empty() && last.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        }
    }
    false
}

/// Resolves user omnibox input into a navigable URL.
///
/// - Preserves explicit schemes (`https://`, `http://`, `about:`, `ext:`, `@`, `data:`, `javascript:`).
/// - Uses `http://` for `localhost` and raw IPv4 addresses.
/// - Prepends `https://` for domain-like inputs (e.g. `example.com`, `en.wikipedia.org/wiki/Rust`).
/// - Routes natural-language search queries (words without a dot, or containing spaces) to
///   DuckDuckGo HTML search (`https://html.duckduckgo.com/html/?q=...`).
pub fn resolve_omnibox_input(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("://")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("ext:")
        || trimmed.starts_with('@')
        || trimmed.starts_with("data:")
        || trimmed.starts_with("javascript:")
    {
        return String::from(trimmed);
    }
    if looks_like_host(trimmed) {
        let host_part = trimmed.split(['/', '?', '#']).next().unwrap_or("");
        let host = host_part.split(':').next().unwrap_or("");
        let is_local = host.eq_ignore_ascii_case("localhost")
            || host.bytes().all(|b| b.is_ascii_digit() || b == b'.');
        let scheme = if is_local { "http://" } else { "https://" };
        return format!("{scheme}{trimmed}");
    }
    format!("https://html.duckduckgo.com/html/?q={}", encode_query(trimmed))
}

// ── Single-Row Helium Top Bar layout & hit-testing ───────────────────────────

/// Hit result for a pointer event in the inline tab region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    Group(usize),
    NewTab,
}

/// Hit result for a pointer event in the single-row Helium top bar (non-tab controls).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarHit {
    Back,
    Forward,
    Reload,
    Omnibox,
    BookmarkStar,
    Shields,
    BookmarksToggle,
    Menu,
}

/// Rectangles `[x, y, w, h]` of every control in the single-row Helium top bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolbarLayout {
    pub back: [i32; 4],
    pub forward: [i32; 4],
    pub reload: [i32; 4],
    pub omnibox: [i32; 4],
    pub star: [i32; 4],
    /// Horizontal span `[x, y, w, h]` reserved for inline tabs + `+` button.
    pub tabs_zone: [i32; 4],
    pub shields: [i32; 4],
    pub bookmarks_toggle: [i32; 4],
    pub menu: [i32; 4],
}

fn rect_contains(r: [i32; 4], x: i32, y: i32) -> bool {
    r[2] > 0 && r[3] > 0 && x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// Default shields pill width for a given `(up, blocked)` state.
pub fn shields_pill_width(shields: Option<(bool, u32)>) -> i32 {
    match shields {
        Some((false, _)) => 48,
        Some((true, n)) if n > 999 => 62,
        Some((true, n)) if n >= 100 => 54,
        Some((true, n)) if n > 0 => 48,
        _ => 48,
    }
}

/// Computes single-row Helium top bar rectangles for window `width`, `shields_w`, and `vertical_tabs`.
pub fn toolbar_layout(width: i32, shields_w: i32, vertical_tabs: bool) -> ToolbarLayout {
    let y = 3;
    let h = TOP_BAR_H - 6; // 24
    let btn_w = 26;
    let gap = 3;

    // Left: Back, Forward, Reload
    let mut x = 8;
    let back = [x, y, btn_w, h];
    x += btn_w + gap;
    let forward = [x, y, btn_w, h];
    x += btn_w + gap;
    let reload = [x, y, btn_w, h];
    let omni_left = x + btn_w + 8;

    // Far right: Shields, Bookmarks Bar toggle, Menu (⋮)
    let menu_x = (width - 8 - btn_w).max(omni_left);
    let menu = [menu_x, y, btn_w, h];

    let bm_x = (menu_x - 4 - btn_w).max(omni_left);
    let bookmarks_toggle = [bm_x, y, btn_w, h];

    let sw = shields_w.clamp(42, 80);
    let shields_x = (bm_x - 4 - sw).max(omni_left);
    let shields = [shields_x, y, sw, h];

    let center_avail = (shields_x - 8 - omni_left).max(0);

    // Inline rounded pill Omnibox (~260-340px wide when tabs sit inline; full width when vertical tabs are on)
    let omni_w = if vertical_tabs {
        center_avail
    } else if center_avail >= 420 {
        ((width * 28) / 100).clamp(260, 340).min(center_avail - 120)
    } else if center_avail >= 220 {
        (center_avail / 2).clamp(140, 260)
    } else {
        center_avail
    };
    let omnibox = [omni_left, y, omni_w, h];

    let star_w = 24;
    let star = if omni_w >= 80 {
        [omni_left + omni_w - star_w - 3, y + 2, star_w, h - 4]
    } else {
        [0, 0, 0, 0]
    };

    // Inline tabs zone immediately to the right of the omnibox on the SAME row
    let tabs_left = omni_left + omni_w + 8;
    let tabs_w = if vertical_tabs {
        0
    } else {
        (shields_x - 8 - tabs_left).max(0)
    };
    let tabs_zone = [tabs_left, y, tabs_w, h];

    ToolbarLayout {
        back,
        forward,
        reload,
        omnibox,
        star,
        tabs_zone,
        shields,
        bookmarks_toggle,
        menu,
    }
}

/// Computes `(chip_rects, new_tab_button)` inside `tabs_zone` (`[x, y, w, h]`) on the single-row top bar.
pub fn tab_strip_layout(
    width: i32,
    shields_w: i32,
    slots: &[Slot],
    groups: &[TabGroup],
) -> (Vec<(i32, i32)>, (i32, i32)) {
    let tl = toolbar_layout(width, shields_w, false);
    let [zx, _, zw, _] = tl.tabs_zone;
    let new_btn_w = 26;
    if zw < new_btn_w || slots.is_empty() {
        return (Vec::new(), (zx, if zw >= new_btn_w { new_btn_w } else { 0 }));
    }
    let tabs = slots
        .iter()
        .filter(|s| matches!(s, Slot::Tab(_)))
        .count() as i32;
    let labels: i32 = slots
        .iter()
        .map(|s| match s {
            Slot::Group(g) => group_chip(groups, *g) + 4,
            Slot::Tab(_) => 0,
        })
        .sum();
    let avail = (zw - new_btn_w - 4 - labels).max(0);
    let tab_w = (avail / tabs.max(1)).clamp(44, 176);
    let mut x = zx;
    let mut chips = Vec::with_capacity(slots.len());
    for s in slots {
        let cw = match s {
            Slot::Group(g) => group_chip(groups, *g),
            Slot::Tab(_) => tab_w,
        };
        if x + cw + new_btn_w + 4 > zx + zw {
            break;
        }
        chips.push((x, cw));
        x += cw + 4;
    }
    let plus_x = x.min((zx + zw - new_btn_w).max(zx));
    (chips, (plus_x, new_btn_w))
}

/// Legacy bar chip layout (shows with two tabs or more, or any group).
pub fn tab_chips(width: i32, slots: &[Slot], groups: &[TabGroup]) -> Vec<(i32, i32)> {
    let tabs = slots.iter().filter(|s| matches!(s, Slot::Tab(_))).count() as i32;
    let labels: i32 = slots
        .iter()
        .map(|s| if let Slot::Group(g) = s { group_chip(groups, *g) + 2 } else { 0 })
        .sum();
    if tabs + (labels > 0) as i32 * 2 < 2 {
        return Vec::new();
    }
    let room = (width / 2 - 8 - labels).max(0);
    let w = (room / tabs.max(1)).clamp(28, 140);
    let mut x = 6;
    slots
        .iter()
        .map(|s| {
            let cw = if let Slot::Group(g) = s { group_chip(groups, *g) } else { w };
            let at = x;
            x += cw + 2;
            (at, cw)
        })
        .take_while(|(x, w)| x + w <= width - 60)
        .collect()
}

/// Close button `[x, y, w, h]` inside an inline tab chip `(cx, cw)`, if wide enough.
pub fn tab_close_rect(cx: i32, cw: i32) -> Option<[i32; 4]> {
    if cw < 54 {
        return None;
    }
    Some([cx + cw - 19, 12, 14, 14])
}

/// Hit-tests window point `(x, y)` in the inline tab zone of the single-row Helium top bar.
pub fn tab_hit_at(
    width: i32,
    shields_w: i32,
    slots: &[Slot],
    groups: &[TabGroup],
    x: i32,
    y: i32,
) -> Option<TabHit> {
    if y < 0 || y >= TOP_BAR_H || x < 0 || x >= width {
        return None;
    }
    let (chips, (plus_x, plus_w)) = tab_strip_layout(width, shields_w, slots, groups);
    if plus_w > 0 && x >= plus_x && x < plus_x + plus_w && y >= 6 && y < TOP_BAR_H - 6 {
        return Some(TabHit::NewTab);
    }
    for (slot, &(cx, cw)) in slots.iter().zip(chips.iter()) {
        if x >= cx && x < cx + cw && y >= 5 && y < TOP_BAR_H - 5 {
            return match *slot {
                Slot::Group(g) => Some(TabHit::Group(g)),
                Slot::Tab(i) => {
                    if let Some([bx, by, bw, bh]) = tab_close_rect(cx, cw) {
                        if x >= bx && x < bx + bw && y >= by && y < by + bh {
                            return Some(TabHit::Close(i));
                        }
                    }
                    Some(TabHit::Tab(i))
                }
            };
        }
    }
    None
}

/// Hit-tests window point `(x, y)` against the non-tab controls in the single-row Helium top bar.
pub fn toolbar_hit_at(
    width: i32,
    shields_w: i32,
    vertical_tabs: bool,
    x: i32,
    y: i32,
) -> Option<ToolbarHit> {
    if y < 0 || y >= TOP_BAR_H || x < 0 || x >= width {
        return None;
    }
    let l = toolbar_layout(width, shields_w, vertical_tabs);
    if rect_contains(l.back, x, y) {
        return Some(ToolbarHit::Back);
    }
    if rect_contains(l.forward, x, y) {
        return Some(ToolbarHit::Forward);
    }
    if rect_contains(l.reload, x, y) {
        return Some(ToolbarHit::Reload);
    }
    if rect_contains(l.star, x, y) {
        return Some(ToolbarHit::BookmarkStar);
    }
    if rect_contains(l.omnibox, x, y) {
        return Some(ToolbarHit::Omnibox);
    }
    if rect_contains(l.shields, x, y) {
        return Some(ToolbarHit::Shields);
    }
    if rect_contains(l.bookmarks_toggle, x, y) {
        return Some(ToolbarHit::BookmarksToggle);
    }
    if rect_contains(l.menu, x, y) {
        return Some(ToolbarHit::Menu);
    }
    None
}

/// The row in the vertical strip `side` wide under window point (`x`, `y`), of `n` rows.
pub fn side_row_at(side: i32, bar_h: i32, n: usize, x: i32, y: i32) -> Option<usize> {
    if side <= 0 || x < 0 || x >= side || y < bar_h + 6 {
        return None;
    }
    let i = ((y - bar_h - 6) / SIDE_ROW) as usize;
    (i < n).then_some(i)
}

/// The suggestion row under (`x`, `y`) in window coordinates, if any.
pub fn suggestion_at(width: i32, count: usize, vertical_tabs: bool, x: i32, y: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let top = TOP_BAR_H;
    let l = toolbar_layout(width, 48, vertical_tabs);
    let left = l.omnibox[0].max(6);
    let right = (l.omnibox[0] + l.omnibox[2]).min(width - 6).max(left + 140);
    if x < left || x >= right || y < top {
        return None;
    }
    let i = ((y - top) / SUGGESTION_ROW) as usize;
    (i < count).then_some(i)
}

/// Pure, host-testable tab strip state model mirroring `apps/web/src/tabs.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStripState {
    pub tabs: Vec<String>,
    pub active: usize,
}

impl TabStripState {
    pub fn new(initial_title: impl Into<String>) -> Self {
        Self {
            tabs: alloc::vec![initial_title.into()],
            active: 0,
        }
    }

    pub fn open_tab(&mut self, title: impl Into<String>) -> usize {
        let at = (self.active + 1).min(self.tabs.len());
        self.tabs.insert(at, title.into());
        self.active = at;
        at
    }

    pub fn switch_to(&mut self, index: usize) -> bool {
        if index < self.tabs.len() {
            self.active = index;
            true
        } else {
            false
        }
    }

    /// Closes tab `index`. Returns `false` if it was the last tab (window should close).
    pub fn close_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return true;
        }
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > index {
            self.active -= 1;
        }
        true
    }
}

// ── Floating Context Menu model & hit-testing ────────────────────────────────

/// Action triggered by an item in the floating context / browser menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextAction {
    Back,
    Forward,
    Reload,
    NewTab,
    ToggleBookmark,
    ToggleBookmarksBar,
    CopyUrl,
    SelectAll,
    ToggleShields,
}

/// One row in the floating frosted lavender context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenuItem {
    pub action: ContextAction,
    pub label: String,
    pub enabled: bool,
    pub separator_before: bool,
}

/// State of an open floating context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenu {
    pub x: i32,
    pub y: i32,
    pub items: Vec<ContextMenuItem>,
    pub hovered: Option<usize>,
}

/// Builds the browser's floating context menu, clamped inside `(win_w, win_h)`.
pub fn build_context_menu(
    x: i32,
    y: i32,
    win_w: i32,
    win_h: i32,
    can_back: bool,
    can_forward: bool,
    bookmarked: bool,
    show_bookmarks_bar: bool,
    shields_up: bool,
) -> ContextMenu {
    let items = alloc::vec![
        ContextMenuItem {
            action: ContextAction::Back,
            label: String::from("Back"),
            enabled: can_back,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::Forward,
            label: String::from("Forward"),
            enabled: can_forward,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::Reload,
            label: String::from("Reload"),
            enabled: true,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::NewTab,
            label: String::from("New Tab"),
            enabled: true,
            separator_before: true,
        },
        ContextMenuItem {
            action: ContextAction::ToggleBookmark,
            label: String::from(if bookmarked {
                "Remove Bookmark"
            } else {
                "Bookmark Page"
            }),
            enabled: true,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::ToggleBookmarksBar,
            label: String::from(if show_bookmarks_bar {
                "Hide Bookmarks Bar"
            } else {
                "Show Bookmarks Bar"
            }),
            enabled: true,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::CopyUrl,
            label: String::from("Copy URL"),
            enabled: true,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::SelectAll,
            label: String::from("Select All"),
            enabled: true,
            separator_before: false,
        },
        ContextMenuItem {
            action: ContextAction::ToggleShields,
            label: String::from(if shields_up {
                "Shields: On (Toggle)"
            } else {
                "Shields: Off (Toggle)"
            }),
            enabled: true,
            separator_before: true,
        },
    ];
    let mut menu = ContextMenu {
        x,
        y,
        items,
        hovered: None,
    };
    let [_, _, mw, mh] = context_menu_rect(&menu);
    menu.x = x.clamp(4, (win_w - mw - 4).max(4));
    menu.y = y.clamp(4, (win_h - mh - 4).max(4));
    menu
}

/// Outer rectangle `[x, y, w, h]` of `menu`.
pub fn context_menu_rect(menu: &ContextMenu) -> [i32; 4] {
    let seps = menu.items.iter().filter(|i| i.separator_before).count() as i32;
    let h = 10 + menu.items.len() as i32 * CONTEXT_MENU_ROW + seps * CONTEXT_MENU_SEP;
    [menu.x, menu.y, CONTEXT_MENU_W, h]
}

/// Returns `true` if window point `(x, y)` lies inside `menu`'s bounding box.
pub fn context_menu_contains(menu: &ContextMenu, x: i32, y: i32) -> bool {
    rect_contains(context_menu_rect(menu), x, y)
}

/// Returns the index of the enabled item under `(x, y)` in `menu`, if any.
pub fn context_menu_hit_at(menu: &ContextMenu, x: i32, y: i32) -> Option<usize> {
    if !context_menu_contains(menu, x, y) {
        return None;
    }
    let mut cy = menu.y + 5;
    for (idx, item) in menu.items.iter().enumerate() {
        if item.separator_before {
            cy += CONTEXT_MENU_SEP;
        }
        if y >= cy && y < cy + CONTEXT_MENU_ROW {
            return item.enabled.then_some(idx);
        }
        cy += CONTEXT_MENU_ROW;
    }
    None
}

// ── Chrome struct & rendering ────────────────────────────────────────────────

/// What the browser draws around the page: Helium single-row compact top bar
/// (nav buttons, inline omnibox pill, inline tabs + `+`, utility icons), optional
/// toggleable bookmarks bar, omnibox suggestions, find bar, and context menu.
#[derive(Default)]
pub struct Chrome<'a> {
    /// The address, or what is being typed.
    pub text: &'a str,
    pub editing: bool,
    /// Each tab's label.
    pub tabs: &'a [String],
    /// The line/dot accent for each tab (container or private colour), 0 for default.
    pub colours: &'a [u32],
    pub active: usize,
    /// Addresses offered under the omnibox while typing, and the chosen one.
    pub suggestions: &'a [String],
    pub chosen: Option<usize>,
    /// Find in page: what is searched, the current match (from 0) and how many there are.
    pub find: Option<(&'a str, usize, usize)>,
    /// Page rectangles (x, y, w, h) of the find matches, with each one's match number.
    pub marks: &'a [[i32; 5]],
    /// The page zoom in percent, shown in the omnibox when not 100.
    pub zoom: u32,
    /// The shield badge: whether Shields are up for the site, and how many requests
    /// they blocked on the page.
    pub shields: Option<(bool, u32)>,
    /// Tabs in a strip down the left this wide instead of inline tabs (0: inline Helium tabs).
    pub side: i32,
    /// The window columns (x, width) the page takes in a split view.
    pub pane: (i32, i32),
    /// Each tab's group (an index into `groups`), if it is in one.
    pub group_of: &'a [Option<usize>],
    /// Tab groups, as in Chrome: a coloured label before their tabs, which folds them away.
    pub groups: &'a [TabGroup],
    /// Whether Back / Forward buttons are enabled.
    pub can_back: bool,
    pub can_forward: bool,
    /// Whether the current page is actively loading (shows Stop icon).
    pub loading: bool,
    /// Whether the current page is bookmarked (highlights the Star button).
    pub bookmarked: bool,
    /// Whether the optional bookmarks bar strip is currently shown below the top bar.
    pub show_bookmarks_bar: bool,
    /// Clickable pills rendered on the bookmarks bar when `show_bookmarks_bar` is true.
    pub bookmarks: &'a [BookmarkItem],
    /// Optional floating frosted lavender context menu.
    pub context_menu: Option<&'a ContextMenu>,
}

/// Cuts `text` with an ellipsis to fit within pixel width `w`.
fn fit(fonts: &mut Fonts, text: &str, font: FontKey, w: i32) -> String {
    if w <= 0 {
        return String::new();
    }
    let mut shown = String::from(text);
    if fonts.text_width(&shown, font) <= w {
        return shown;
    }
    let ell_w = fonts.text_width("…", font);
    while !shown.is_empty() && fonts.text_width(&shown, font) + ell_w > w {
        shown.pop();
    }
    shown.push('…');
    shown
}

// ── Crisp pixel micro-icons ──────────────────────────────────────────────────

fn draw_chevron(t: &mut Target, cx: i32, cy: i32, dir: i32, color: u32) {
    for step in 0..5 {
        let x = cx + dir * (2 - step);
        t.fill(x, cy - step, 2, 2, color, 0);
        t.fill(x, cy + step, 2, 2, color, 0);
    }
}

fn draw_close_x(t: &mut Target, x: i32, y: i32, size: i32, color: u32) {
    for i in 0..size {
        t.fill(x + i, y + i, 1, 1, color, 0);
        t.fill(x + size - 1 - i, y + i, 1, 1, color, 0);
    }
}

fn draw_plus(t: &mut Target, cx: i32, cy: i32, color: u32) {
    t.fill(cx - 4, cy, 9, 1, color, 0);
    t.fill(cx, cy - 4, 1, 9, color, 0);
}

fn draw_reload(t: &mut Target, cx: i32, cy: i32, color: u32) {
    t.fill(cx - 4, cy - 5, 8, 2, color, 1);
    t.fill(cx - 4, cy + 4, 8, 2, color, 1);
    t.fill(cx - 5, cy - 3, 2, 7, color, 1);
    t.fill(cx + 3, cy + 1, 2, 3, color, 1);
    t.fill(cx + 2, cy - 6, 4, 2, color, 0);
    t.fill(cx + 4, cy - 4, 2, 3, color, 0);
}

fn draw_star(t: &mut Target, cx: i32, cy: i32, filled: bool, color: u32) {
    if filled {
        t.fill(cx, cy - 5, 1, 11, color, 0);
        t.fill(cx - 1, cy - 3, 3, 7, color, 0);
        t.fill(cx - 5, cy - 1, 11, 2, color, 0);
        t.fill(cx - 3, cy + 1, 7, 2, color, 0);
        t.fill(cx - 4, cy + 3, 3, 2, color, 0);
        t.fill(cx + 2, cy + 3, 3, 2, color, 0);
    } else {
        t.fill(cx, cy - 5, 1, 3, color, 0);
        t.fill(cx - 5, cy - 1, 11, 1, color, 0);
        t.fill(cx - 3, cy + 1, 2, 2, color, 0);
        t.fill(cx + 2, cy + 1, 2, 2, color, 0);
        t.fill(cx - 4, cy + 3, 2, 2, color, 0);
        t.fill(cx + 3, cy + 3, 2, 2, color, 0);
        t.fill(cx - 1, cy + 2, 3, 1, color, 0);
    }
}

fn draw_shield_icon(t: &mut Target, sx: i32, sy: i32, color: u32) {
    t.fill(sx, sy, 9, 6, color, 1);
    for row in 0..5 {
        t.fill(sx + row, sy + 6 + row, 9 - 2 * row, 1, color, 0);
    }
}

fn draw_bookmarks_bar_icon(t: &mut Target, cx: i32, cy: i32, color: u32) {
    // Crisp top-shelf + 3 pill dashes (bookmarks bar glyph)
    t.fill(cx - 6, cy - 5, 13, 2, color, 1);
    t.fill(cx - 6, cy - 1, 4, 3, color, 1);
    t.fill(cx - 1, cy - 1, 4, 3, color, 1);
    t.fill(cx + 4, cy - 1, 3, 3, color, 1);
    t.fill(cx - 6, cy + 4, 13, 1, color, 0);
}

fn draw_menu_dots_vertical(t: &mut Target, cx: i32, cy: i32, color: u32) {
    // ⋮ vertical menu dots
    t.fill(cx - 1, cy - 6, 3, 3, color, 1);
    t.fill(cx - 1, cy - 1, 3, 3, color, 1);
    t.fill(cx - 1, cy + 4, 3, 3, color, 1);
}

fn draw_lock(t: &mut Target, x: i32, y: i32, color: u32) {
    t.fill(x + 2, y, 4, 2, color, 1);
    t.fill(x + 1, y + 1, 1, 3, color, 0);
    t.fill(x + 6, y + 1, 1, 3, color, 0);
    t.fill(x, y + 4, 8, 6, color, 1);
}

/// Paints the full browser window: page viewport, Helium-style single-row compact
/// Candy Lavender chrome, optional toggleable bookmarks bar, suggestions, find bar,
/// and floating frosted lavender context menu.
pub fn frame_with(
    page: &Page,
    fonts: &mut Fonts,
    pixels: &mut [u32],
    width: i32,
    height: i32,
    scroll: i32,
    focus: Option<usize>,
    c: &Chrome,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    let bar_h = chrome_height(c.show_bookmarks_bar);
    let side = c.side.clamp(0, width);
    let (px, pw) = if c.pane.1 > 0 {
        c.pane
    } else {
        (side, width - side)
    };

    // 1. Page viewport
    if let Some(mut t) = column_below(pixels, width, height, scroll, px, pw, bar_h) {
        paint(page, fonts, &mut t, focus);
        for m in c.marks.iter() {
            let current = c.find.is_some_and(|(_, at, _)| at as i32 == m[4]);
            let color = if current { 0x80FF_8C00 } else { 0x70FF_EB3B };
            t.fill(m[0], m[1], m[2], m[3], color, 2);
        }
    }

    let font = FontKey {
        size: 13,
        bold: false,
        italic: false,
        mono: false,
        face: 0,
    };
    let small = FontKey { size: 12, ..font };

    // 2. Vertical tab strip (when enabled via Ctrl+Shift+,)
    let slots = if c.group_of.len() == c.tabs.len() {
        strip_slots(c.group_of, c.groups)
    } else {
        (0..c.tabs.len()).map(Slot::Tab).collect()
    };
    if side > 0 && height > bar_h {
        let mut strip = Target {
            pixels,
            width: side,
            height,
            scroll: 0,
            top: bar_h,
            premul: false,
            clip: None,
            stride: width,
        };
        strip.fill(0, 0, side, height - bar_h, BG_TOP_BAR, 0);
        strip.fill(side - 1, 0, 1, height - bar_h, BORDER_SUBTLE, 0);
        for (row, slot) in slots.iter().enumerate() {
            let y = 6 + row as i32 * SIDE_ROW;
            if y + SIDE_ROW > height - bar_h {
                break;
            }
            match *slot {
                Slot::Group(g) => {
                    let grp = &c.groups[g];
                    strip.fill(10, y + 9, 9, 9, grp.colour, 4);
                    let name = if grp.name.is_empty() {
                        "Group"
                    } else {
                        grp.name.as_str()
                    };
                    let label = if grp.folded {
                        format!("{name} ({})", group_count(c.group_of, g))
                    } else {
                        String::from(name)
                    };
                    let shown = fit(fonts, &label, small, side - 40);
                    text_run(&mut strip, fonts, 26, y + 17, small, TEXT_PRIMARY, &shown);
                }
                Slot::Tab(i) => {
                    let Some(label) = c.tabs.get(i) else { continue };
                    let group = c
                        .group_of
                        .get(i)
                        .copied()
                        .flatten()
                        .and_then(|g| c.groups.get(g));
                    let inset = if group.is_some() { 12 } else { 0 };
                    if let Some(g) = group {
                        strip.fill(13, y - 4, 3, SIDE_ROW, g.colour, 0);
                    }
                    let on = i == c.active;
                    if on {
                        strip.fill(
                            6 + inset,
                            y,
                            side - 13 - inset,
                            SIDE_ROW - 4,
                            BG_TAB_ACTIVE,
                            8,
                        );
                        strip.fill(6 + inset, y + 4, 3, SIDE_ROW - 12, ACCENT_LAVENDER, 1);
                    }
                    if let Some(&col) = c.colours.get(i).filter(|col| **col != 0) {
                        strip.fill(6 + inset, y + 4, 3, SIDE_ROW - 12, col, 1);
                    }
                    let shown = fit(fonts, label, small, side - 32 - inset);
                    text_run(
                        &mut strip,
                        fonts,
                        16 + inset,
                        y + 17,
                        small,
                        if on { TEXT_PRIMARY } else { TEXT_SECONDARY },
                        &shown,
                    );
                }
            }
        }
    }

    // 3. Top Chrome Target (Single-row Helium bar + optional Bookmarks Bar)
    let mut bar = Target {
        pixels,
        width,
        height: height.min(bar_h).max(0),
        scroll: 0,
        top: 0,
        premul: false,
        clip: None,
        stride: width,
    };

    // Single-row Helium top bar surface + subtle bottom hairline
    bar.fill(0, 0, width, TOP_BAR_H, BG_TOP_BAR, 0);
    bar.fill(0, TOP_BAR_H - 1, width, 1, BORDER_SUBTLE, 0);

    if c.show_bookmarks_bar {
        bar.fill(0, TOP_BAR_H, width, BOOKMARKS_H, BG_BOOKMARKS, 0);
        bar.fill(0, bar_h - 1, width, 1, BORDER_SUBTLE, 0);
    }

    // Split-view focus indicator line
    if c.pane.1 > 0 {
        bar.fill(px, bar_h - 2, pw, 2, ACCENT_LAVENDER, 0);
    }

    // ── Paint Single-Row Helium Top Bar ──────────────────────────────────────
    let shields_w = shields_pill_width(c.shields);
    let tl = toolbar_layout(width, shields_w, side > 0);

    // 1) Left compact nav buttons: Back, Forward, Reload/Stop
    bar.fill(
        tl.back[0],
        tl.back[1],
        tl.back[2],
        tl.back[3],
        if c.can_back { BTN_FILL } else { BTN_DISABLED },
        8,
    );
    draw_chevron(
        &mut bar,
        tl.back[0] + tl.back[2] / 2,
        tl.back[1] + tl.back[3] / 2,
        -1,
        if c.can_back { TEXT_PRIMARY } else { TEXT_MUTED },
    );

    bar.fill(
        tl.forward[0],
        tl.forward[1],
        tl.forward[2],
        tl.forward[3],
        if c.can_forward { BTN_FILL } else { BTN_DISABLED },
        8,
    );
    draw_chevron(
        &mut bar,
        tl.forward[0] + tl.forward[2] / 2,
        tl.forward[1] + tl.forward[3] / 2,
        1,
        if c.can_forward { TEXT_PRIMARY } else { TEXT_MUTED },
    );

    bar.fill(tl.reload[0], tl.reload[1], tl.reload[2], tl.reload[3], BTN_FILL, 8);
    if c.loading {
        draw_close_x(
            &mut bar,
            tl.reload[0] + tl.reload[2] / 2 - 4,
            tl.reload[1] + tl.reload[3] / 2 - 4,
            8,
            ACCENT_BERRY,
        );
    } else {
        draw_reload(
            &mut bar,
            tl.reload[0] + tl.reload[2] / 2,
            tl.reload[1] + tl.reload[3] / 2,
            TEXT_PRIMARY,
        );
    }

    // 2) Inline Rounded Pill Omnibox (~260-340px)
    if tl.omnibox[2] > 16 {
        let [ox, oy, ow, oh] = tl.omnibox;
        if c.editing {
            bar.fill(ox - 1, oy - 1, ow + 2, oh + 2, ACCENT_LAVENDER, 13);
            bar.fill(ox + 1, oy + 1, (ow - 2).max(0), (oh - 2).max(0), 0xFFFF_FFFF, 11);
        } else {
            bar.fill(ox, oy, ow, oh, OMNIBOX_BORDER, 12);
            bar.fill(ox + 1, oy + 1, (ow - 2).max(0), (oh - 2).max(0), OMNIBOX_FILL, 11);
        }

        // Site-info / lock / shield icon at left of omnibox
        let mut text_left = ox + 10;
        if !c.editing && ow >= 90 {
            let icon_y = oy + (oh - 10) / 2;
            if c.text.contains("https://") {
                draw_lock(&mut bar, ox + 10, icon_y, ACCENT_MINT);
            } else if c.text.contains("about:") {
                bar.fill(ox + 10, icon_y + 1, 8, 8, ACCENT_LAVENDER, 4);
            } else {
                bar.fill(ox + 10, icon_y + 1, 8, 8, TEXT_MUTED, 4);
            }
            text_left = ox + 24;
        }

        // Inline Bookmark Star (★) at right of omnibox
        let mut right_reserve = 8;
        if tl.star[2] > 0 {
            let [sx, sy, sw, sh] = tl.star;
            if c.bookmarked {
                bar.fill(sx, sy, sw, sh, 0xFF3E_315C, 6);
            }
            draw_star(
                &mut bar,
                sx + sw / 2,
                sy + sh / 2,
                c.bookmarked,
                if c.bookmarked {
                    ACCENT_HONEY
                } else if c.editing {
                    TEXT_MUTED
                } else {
                    TEXT_SECONDARY
                },
            );
            right_reserve += sw + 4;
        }

        // Compact zoom badge inside omnibox if != 100%
        let zoom = if c.zoom != 0 && c.zoom != 100 {
            format!("{}%", c.zoom)
        } else {
            String::new()
        };
        if !zoom.is_empty() {
            let zw = fonts.text_width(&zoom, small) + 6;
            if ow - right_reserve - zw > 64 {
                let zx = ox + ow - right_reserve - zw;
                text_run(&mut bar, fonts, zx, oy + 17, small, ACCENT_LAVENDER, &zoom);
                right_reserve += zw + 4;
            }
        }

        // Omnibox URL / search text
        let room = (ox + ow - right_reserve - text_left).max(0);
        let text_color = if c.editing { 0xFF1F_1F1F } else { TEXT_PRIMARY };
        let mut shown = c.text;
        while fonts.text_width(shown, font) > room && !shown.is_empty() {
            let cut = shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i);
            shown = if c.editing {
                &shown[cut..]
            } else {
                &shown[..shown.char_indices().last().map_or(0, |(i, _)| i)]
            };
        }
        text_run(&mut bar, fonts, text_left, oy + 17, font, text_color, shown);
        if c.editing {
            let cx = text_left + fonts.text_width(shown, font) + 1;
            bar.fill(cx, oy + 5, 2, oh - 10, ACCENT_VIOLET, 0);
        }
    }

    // 3) Inline Rounded Tabs + '+' New Tab button on the SAME row
    if side == 0 && tl.tabs_zone[2] > 0 {
        let (chips, (plus_x, plus_w)) = tab_strip_layout(width, shields_w, &slots, c.groups);
        for (slot, &(cx, cw)) in slots.iter().zip(chips.iter()) {
            match *slot {
                Slot::Group(g) => {
                    let grp = &c.groups[g];
                    bar.fill(cx, 7, cw, TOP_BAR_H - 14, grp.colour, 8);
                    let n = group_count(c.group_of, g);
                    let label = match (grp.folded, grp.name.is_empty()) {
                        (true, true) => format!("{n}"),
                        (true, false) => format!("{} ({n})", grp.name),
                        _ => grp.name.clone(),
                    };
                    let label = fit(fonts, &label, small, cw - 10);
                    text_run(&mut bar, fonts, cx + 6, 23, small, 0xFFFF_FFFF, &label);
                }
                Slot::Tab(i) => {
                    let Some(text) = c.tabs.get(i) else { continue };
                    let on = i == c.active;
                    if on {
                        // Crisp lifted rounded pill for the active tab
                        bar.fill(cx, 5, cw, TOP_BAR_H - 10, BG_TAB_ACTIVE_BORDER, 10);
                        bar.fill(cx + 1, 6, (cw - 2).max(0), TOP_BAR_H - 12, BG_TAB_ACTIVE, 9);
                    } else {
                        // Subtle flat pill for inactive tabs
                        bar.fill(cx, 6, cw, TOP_BAR_H - 12, BG_TAB_INACTIVE, 9);
                    }
                    // Colored favicon dot on the left of each tab
                    let dot_col = c
                        .colours
                        .get(i)
                        .copied()
                        .filter(|&col| col != 0)
                        .unwrap_or(FAVICON_DOTS[i % FAVICON_DOTS.len()]);
                    bar.fill(cx + 9, 15, 7, 7, dot_col, 3);

                    let has_close = tab_close_rect(cx, cw);
                    let text_room = cw - 24 - if has_close.is_some() { 18 } else { 0 };
                    let label = fit(fonts, text, small, text_room);
                    text_run(
                        &mut bar,
                        fonts,
                        cx + 21,
                        23,
                        small,
                        if on { TEXT_PRIMARY } else { TEXT_SECONDARY },
                        &label,
                    );
                    if let Some([bx, by, bw, bh]) = has_close {
                        if on {
                            bar.fill(bx, by, bw, bh, 0xFF45_3B6B, 7);
                        }
                        draw_close_x(
                            &mut bar,
                            bx + 4,
                            by + 4,
                            6,
                            if on { TEXT_PRIMARY } else { TEXT_SECONDARY },
                        );
                    }
                    if let Some(g) = c
                        .group_of
                        .get(i)
                        .copied()
                        .flatten()
                        .and_then(|g| c.groups.get(g))
                    {
                        bar.fill(cx + 4, TOP_BAR_H - 4, (cw - 8).max(0), 2, g.colour, 0);
                    }
                }
            }
        }
        // '+' New Tab button right after the inline tabs
        if plus_w > 0 && plus_x + plus_w <= tl.shields[0] - 4 {
            bar.fill(plus_x, 6, plus_w, TOP_BAR_H - 12, BTN_FILL, 8);
            draw_plus(&mut bar, plus_x + plus_w / 2, TOP_BAR_H / 2, ACCENT_LAVENDER);
        }
    }

    // 4) Far right utility icons: Shields pill, Bookmarks Bar toggle, ⋮ Menu
    if tl.shields[0] + tl.shields[2] <= width {
        let (up, blocked) = c.shields.unwrap_or((true, 0));
        let pill_bg = if !up {
            0xFF3B_3456
        } else if blocked > 0 {
            ACCENT_BERRY
        } else {
            ACCENT_VIOLET
        };
        bar.fill(
            tl.shields[0],
            tl.shields[1],
            tl.shields[2],
            tl.shields[3],
            pill_bg,
            10,
        );
        draw_shield_icon(&mut bar, tl.shields[0] + 7, tl.shields[1] + 7, 0xFFFF_FFFF);
        let label = if !up {
            String::from("Off")
        } else if blocked > 999 {
            String::from("999+")
        } else if blocked > 0 {
            format!("{blocked}")
        } else {
            String::from("On")
        };
        text_run(
            &mut bar,
            fonts,
            tl.shields[0] + 20,
            tl.shields[1] + 17,
            small,
            0xFFFF_FFFF,
            &label,
        );
    }

    if tl.bookmarks_toggle[0] + tl.bookmarks_toggle[2] <= width {
        bar.fill(
            tl.bookmarks_toggle[0],
            tl.bookmarks_toggle[1],
            tl.bookmarks_toggle[2],
            tl.bookmarks_toggle[3],
            if c.show_bookmarks_bar { BTN_ACTIVE } else { BTN_FILL },
            8,
        );
        draw_bookmarks_bar_icon(
            &mut bar,
            tl.bookmarks_toggle[0] + tl.bookmarks_toggle[2] / 2,
            tl.bookmarks_toggle[1] + tl.bookmarks_toggle[3] / 2,
            if c.show_bookmarks_bar {
                ACCENT_LAVENDER
            } else {
                TEXT_SECONDARY
            },
        );
    }

    if tl.menu[0] + tl.menu[2] <= width {
        bar.fill(tl.menu[0], tl.menu[1], tl.menu[2], tl.menu[3], BTN_FILL, 8);
        draw_menu_dots_vertical(
            &mut bar,
            tl.menu[0] + tl.menu[2] / 2,
            tl.menu[1] + tl.menu[3] / 2,
            TEXT_PRIMARY,
        );
    }

    // ── Optional Toggleable Clickable Bookmarks Bar ──────────────────────────
    if c.show_bookmarks_bar {
        let bm_y = TOP_BAR_H + 3;
        let bm_h = BOOKMARKS_H - 6;
        let pills = bookmark_pills(width, c.bookmarks);
        for (idx, (item, &(bx, bw))) in c.bookmarks.iter().zip(pills.iter()).enumerate() {
            bar.fill(bx, bm_y, bw, bm_h, PILL_FILL, 8);
            let dot_color = FAVICON_DOTS[idx % FAVICON_DOTS.len()];
            bar.fill(bx + 7, bm_y + (bm_h - 6) / 2, 6, 6, dot_color, 3);
            let label = fit(fonts, &item.title, small, bw - 22);
            text_run(&mut bar, fonts, bx + 17, bm_y + 14, small, TEXT_PRIMARY, &label);
        }
    }

    // 4. Overlays (Suggestions dropdown, Find bar, Floating Context Menu)
    let mut over = Target {
        pixels: bar.pixels,
        width,
        height,
        scroll: 0,
        top: 0,
        premul: false,
        clip: None,
        stride: width,
    };

    // Omnibox Suggestions Dropdown
    if c.editing && !c.suggestions.is_empty() {
        let drop_y = TOP_BAR_H;
        let drop_x = tl.omnibox[0].max(6);
        let drop_w = tl.omnibox[2].max(220).min(width - drop_x - 6);
        let h = c.suggestions.len() as i32 * SUGGESTION_ROW + 8;
        if drop_w > 24 && drop_y + h <= height {
            over.fill(drop_x - 1, drop_y - 1, drop_w + 2, h + 2, MENU_BORDER, 10);
            over.fill(drop_x, drop_y, drop_w, h, MENU_BG, 9);
            for (i, s) in c.suggestions.iter().enumerate() {
                let y = drop_y + 4 + i as i32 * SUGGESTION_ROW;
                if c.chosen == Some(i) {
                    over.fill(drop_x + 4, y, (drop_w - 8).max(0), SUGGESTION_ROW, MENU_HOVER, 6);
                }
                let label = fit(fonts, s, font, drop_w - 24);
                text_run(&mut over, fonts, drop_x + 12, y + 17, font, TEXT_PRIMARY, &label);
            }
        }
    }

    // Find-in-page bar (bottom right)
    if let Some((query, at, count)) = c.find {
        let status = if count == 0 {
            String::from("no matches")
        } else {
            format!("{} of {}", at + 1, count)
        };
        let sw = fonts.text_width(&status, small);
        let w = (fonts.text_width(query, font) + sw + 48).clamp(180, (width - 12).max(180));
        let fh = 30;
        let (x, y) = ((width - 8 - w).max(4), (height - fh - 6).max(bar_h));
        over.fill(x - 1, y - 1, w + 2, fh + 2, MENU_BORDER, 9);
        over.fill(x, y, w, fh, BG_TOP_BAR, 8);
        let input_w = (w - 20 - sw).max(40);
        over.fill(x + 6, y + 4, input_w, fh - 8, 0xFFFF_FFFF, 6);
        let q = fit(fonts, query, font, input_w - 14);
        text_run(&mut over, fonts, x + 10, y + 19, font, 0xFF1F_1F1F, &q);
        let cx = x + 11 + fonts.text_width(&q, font);
        over.fill(cx, y + 7, 1, fh - 14, ACCENT_VIOLET, 0);
        text_run(
            &mut over,
            fonts,
            x + w - 8 - sw,
            y + 19,
            small,
            if count == 0 { 0xFFFF_8A80 } else { TEXT_PRIMARY },
            &status,
        );
    }

    // Floating Frosted Lavender Context Menu
    if let Some(menu) = c.context_menu {
        let [mx, my, mw, mh] = context_menu_rect(menu);
        over.fill(mx + 3, my + 4, mw, mh, MENU_SHADOW, 10);
        over.fill(mx - 1, my - 1, mw + 2, mh + 2, MENU_BORDER, 11);
        over.fill(mx, my, mw, mh, MENU_BG, 10);
        let mut cy = my + 5;
        for (idx, item) in menu.items.iter().enumerate() {
            if item.separator_before {
                over.fill(mx + 10, cy + 3, mw - 20, 1, BORDER_SUBTLE, 0);
                cy += CONTEXT_MENU_SEP;
            }
            if menu.hovered == Some(idx) && item.enabled {
                over.fill(mx + 4, cy, mw - 8, CONTEXT_MENU_ROW, MENU_HOVER, 6);
            }
            let color = if item.enabled { TEXT_PRIMARY } else { TEXT_MUTED };
            let label = fit(fonts, &item.label, font, mw - 24);
            text_run(&mut over, fonts, mx + 12, cy + 18, font, color, &label);
            cy += CONTEXT_MENU_ROW;
        }
    }
}
