//! Alt+Tab window switcher: the Shell chooses, Display draws.
//!
//! While Alt is held the Shell sends `OP_WINDOW_SWITCHER` with the windows of
//! the workspace in most-recently-used order and the one selected. Display
//! draws a row of cards over the desktop, each a live thumbnail of the window
//! scaled from its own pixels with the window's title under it, the way
//! KDE's and GNOME's switchers do. A count of zero hides the switcher.
//!
//! Wire: arg0 and arg1 carry four 16-bit surface ids each (entry 0 in the low
//! bits of arg0), arg2 = count (bits 0-3) | selected (bits 8-11).

/// Shell -> Display.
pub const OP_WINDOW_SWITCHER: u64 = 0x2E1;
/// Windows one switcher shows.
pub const MAX_ENTRIES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub entries: [u16; MAX_ENTRIES],
    pub count: u8,
    pub selected: u8,
}

impl State {
    pub const HIDDEN: Self = Self { entries: [0; MAX_ENTRIES], count: 0, selected: 0 };

    /// None when an id does not fit the wire or there are too many windows.
    pub fn new(ids: &[u64], selected: usize) -> Option<Self> {
        if ids.len() > MAX_ENTRIES || (!ids.is_empty() && selected >= ids.len()) { return None; }
        let mut entries = [0u16; MAX_ENTRIES];
        for (slot, &id) in entries.iter_mut().zip(ids) {
            if id == 0 || id > u16::MAX as u64 { return None; }
            *slot = id as u16;
        }
        Some(Self { entries, count: ids.len() as u8, selected: selected as u8 })
    }

    pub fn visible(&self) -> bool { self.count != 0 }

    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries[..self.count as usize].iter().map(|&id| id as u64)
    }

    pub fn wire(&self) -> [u64; 3] {
        let mut words = [0u64; 2];
        for (i, &id) in self.entries.iter().enumerate() {
            words[i / 4] |= (id as u64) << ((i % 4) * 16);
        }
        [words[0], words[1], self.count as u64 | (self.selected as u64) << 8]
    }

    pub fn decode(arg0: u64, arg1: u64, arg2: u64) -> Option<Self> {
        if arg2 >> 12 != 0 || arg2 & 0xF0 != 0 { return None; }
        let count = (arg2 & 0xF) as usize;
        let selected = ((arg2 >> 8) & 0xF) as usize;
        if count > MAX_ENTRIES || (count != 0 && selected >= count) || (count == 0 && selected != 0) { return None; }
        let mut entries = [0u16; MAX_ENTRIES];
        for (i, slot) in entries.iter_mut().enumerate() {
            let word = if i < 4 { arg0 } else { arg1 };
            *slot = (word >> ((i % 4) * 16)) as u16;
            let used = i < count;
            if used == (*slot == 0) { return None; }
        }
        Some(Self { entries, count: count as u8, selected: selected as u8 })
    }
}

/// Where the switcher and its cards go on a `width` x `height` screen, in
/// pixels, for `count` cards at interface scale `scale`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The panel behind the cards.
    pub panel: (usize, usize, usize, usize),
    pub card_w: usize,
    pub thumb_h: usize,
    pub label_h: usize,
    pub gap: usize,
    pub count: usize,
}

impl Layout {
    pub fn new(width: usize, height: usize, count: usize, scale: u32) -> Option<Self> {
        if count == 0 || count > MAX_ENTRIES { return None; }
        let s = scale.max(1) as usize;
        let gap = 12 * s;
        let label_h = 22 * s;
        let room = width.saturating_sub(4 * gap);
        let card_w = (room.saturating_sub(gap * (count - 1)) / count).min(220 * s);
        // Thumbnails keep the screen's own shape.
        let thumb_h = card_w * height / width.max(1);
        if card_w < 32 || thumb_h < 16 { return None; }
        let panel_w = count * card_w + (count + 1) * gap;
        let panel_h = thumb_h + label_h + 3 * gap;
        if panel_w > width || panel_h > height { return None; }
        Some(Self { panel: ((width - panel_w) / 2, (height - panel_h) / 2, panel_w, panel_h), card_w, thumb_h, label_h, gap, count })
    }

    /// Card `i` (thumbnail plus its title) in screen pixels.
    pub fn card(&self, i: usize) -> (usize, usize, usize, usize) {
        let (px, py, _, _) = self.panel;
        (px + self.gap + i * (self.card_w + self.gap), py + self.gap, self.card_w, self.thumb_h + self.gap + self.label_h)
    }

    /// The card under screen point (x, y), if any.
    pub fn card_at(&self, x: usize, y: usize) -> Option<usize> {
        (0..self.count).find(|&i| {
            let (cx, cy, cw, ch) = self.card(i);
            x >= cx && x < cx + cw && y >= cy && y < cy + ch
        })
    }
}

/// Most-recently-used window order: the focused window first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recent<const N: usize> {
    ids: [u64; N],
    len: usize,
}

impl<const N: usize> Recent<N> {
    pub const fn new() -> Self { Self { ids: [0; N], len: 0 } }

    /// `id` was focused: it moves to the front.
    pub fn touch(&mut self, id: u64) {
        if id == 0 || N == 0 { return; }
        let at = self.ids[..self.len].iter().position(|&x| x == id).unwrap_or_else(|| {
            if self.len < N { self.len += 1; }
            self.len - 1
        });
        self.ids.copy_within(0..at, 1);
        self.ids[0] = id;
    }

    pub fn forget(&mut self, id: u64) {
        if let Some(at) = self.ids[..self.len].iter().position(|&x| x == id) {
            self.ids.copy_within(at + 1..self.len, at);
            self.len -= 1;
            self.ids[self.len] = 0;
        }
    }

    pub fn as_slice(&self) -> &[u64] { &self.ids[..self.len] }

    /// The windows in `present`, most recent first; ones never focused go last
    /// in the order `present` gives them. Returns how many were written.
    pub fn order(&self, present: &[u64], out: &mut [u64]) -> usize {
        let mut n = 0;
        let recent = self.as_slice().iter().filter(|id| present.contains(id));
        let rest = present.iter().filter(|id| !self.as_slice().contains(id));
        for &id in recent.chain(rest) {
            if n == out.len() { break; }
            if !out[..n].contains(&id) { out[n] = id; n += 1; }
        }
        n
    }
}

/// One Alt+Tab session: opened by the first Tab, stepped by more Tabs (Shift
/// goes back), ended by letting go of Alt (keep) or Escape (go back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    ids: [u64; MAX_ENTRIES],
    count: usize,
    selected: usize,
    /// The window focused when the switcher opened; Escape returns to it.
    origin: u64,
    open: bool,
}

impl Session {
    pub const fn new() -> Self { Self { ids: [0; MAX_ENTRIES], count: 0, selected: 0, origin: 0, open: false } }
    pub fn is_open(&self) -> bool { self.open }

    /// Opens with `ordered` (most recent first) while `current` has focus and
    /// selects the next one, or steps the open switcher by `dir`. Returns the
    /// window to show now.
    pub fn step(&mut self, ordered: &[u64], current: u64, dir: i8) -> Option<u64> {
        if !self.open {
            let n = ordered.len().min(MAX_ENTRIES);
            if n == 0 { return None; }
            self.ids[..n].copy_from_slice(&ordered[..n]);
            self.count = n;
            self.open = true;
            self.origin = current;
            // Focus outside the list (a panel): the first step lands on the most recent.
            self.selected = if ordered[0] == current || dir < 0 { 0 } else { n - 1 };
        }
        let n = self.count as isize;
        self.selected = (self.selected as isize + if dir < 0 { -1 } else { 1 }).rem_euclid(n) as usize;
        Some(self.ids[self.selected])
    }

    /// Picks card `i` (a pointer click).
    pub fn select(&mut self, i: usize) -> Option<u64> {
        if !self.open || i >= self.count { return None; }
        self.selected = i;
        Some(self.ids[i])
    }

    /// A window went away while the switcher was open.
    pub fn forget(&mut self, id: u64) {
        if let Some(at) = self.ids[..self.count].iter().position(|&x| x == id) {
            self.ids.copy_within(at + 1..self.count, at);
            self.count -= 1;
            if self.selected > at || self.selected >= self.count.max(1) { self.selected = self.selected.saturating_sub(1); }
            if self.count == 0 { self.open = false; }
        }
    }

    /// Ends the session. `keep`: the selected window; otherwise the one that
    /// was focused when it opened.
    pub fn end(&mut self, keep: bool) -> Option<u64> {
        if !self.open { return None; }
        self.open = false;
        Some(if keep { self.ids[self.selected] } else { self.origin })
    }

    pub fn state(&self) -> State {
        if !self.open { return State::HIDDEN; }
        State::new(&self.ids[..self.count], self.selected).unwrap_or(State::HIDDEN)
    }
}
