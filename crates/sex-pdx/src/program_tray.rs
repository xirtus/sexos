// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Program tray items and notification actions (slice S2), like
//! StatusNotifierItem and the actions and inline replies of desktop
//! notifications elsewhere.
//!
//! A program adds a small item to the panel, left of the status icons: a
//! label of up to three characters on a coloured tile, a tooltip and a menu of
//! up to eight rows. A click on the item opens its menu, or, for an item with
//! no menu, tells the program it was activated; a menu row chosen is told to
//! the program by its index.
//!
//! A notification stays Bell's (`sex_pdx::notify`: mute and per-program
//! policy apply as before). Just before posting it, a program may give the
//! Shell up to three action buttons and an inline reply field for it, under a
//! tag of its own (a conversation, a message, ...). The Shell pairs them with
//! the program's next notification Bell forwards, shows a card with the
//! buttons and the field, and tells the program the action chosen, the text
//! replied, or that the card went away unanswered.
//!
//! Wires (every text in 16-byte pieces, printable ASCII; anything else is '?'):
//! - `OP_TRAY_ITEM`, program -> Shell: `ItemMsg` (begin, text, commit, remove).
//! - `OP_NOTIFY_ACTIONS`, program -> Shell: `ActionsMsg` (begin, text, commit).
//! - `OP_TRAY_EVENT`, Shell -> program: `Event` (reply text in pieces).
//! - `OP_TRAY_VIEW`, Shell -> Display: what to draw (`View`, by sections).
//!
//! Self-contained (no other module) so host tests and the Shell's host test
//! can compile it alone. The Shell keeps the items and the cards and handles
//! clicks and keys; Display draws them with the layouts here, so a click
//! lands on what is drawn.

/// Program -> Shell: add, change or remove a tray item.
pub const OP_TRAY_ITEM: u64 = 0x2EA;
/// Shell -> program: an item activated, a menu row chosen, a notification answered.
pub const OP_TRAY_EVENT: u64 = 0x2EB;
/// Program -> Shell: the buttons and reply field for the program's next notification.
pub const OP_NOTIFY_ACTIONS: u64 = 0x2EC;
/// Shell -> Display: the tray items, the open item menu, a tooltip and the card.
pub const OP_TRAY_VIEW: u64 = 0x2ED;

/// Items the Shell keeps for all programs together.
pub const MAX_ITEMS: usize = 8;
/// Items one program may have (ids 0 and 1).
pub const ITEMS_PER_PROGRAM: u8 = 2;
pub const LABEL_BYTES: usize = 3;
pub const TIP_BYTES: usize = 32;
pub const MAX_ROWS: usize = 8;
pub const ROW_BYTES: usize = 16;
pub const MAX_ACTIONS: usize = 3;
pub const ACTION_BYTES: usize = 16;
/// Longest inline reply, in bytes.
pub const REPLY_MAX: usize = 96;
/// Notification text shown on a card (`notify::TEXT_MAX`).
pub const CARD_TEXT: usize = 128;
pub const FROM_BYTES: usize = 16;
pub const PIECE: usize = 16;

/// Printable ASCII and newlines, or '?'.
pub fn clean(b: u8) -> u8 { if b == b'\n' || (32..=126).contains(&b) { b } else { b'?' } }

/// Up to `N` bytes of text, filled piece by piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line<const N: usize> { pub bytes: [u8; N], pub len: u8 }
impl<const N: usize> Line<N> {
    pub const EMPTY: Self = Self { bytes: [0; N], len: 0 };
    pub fn new(text: &[u8]) -> Self {
        let mut l = Self::EMPTY;
        for &b in text.iter().take(N) { l.bytes[l.len as usize] = clean(b); l.len += 1; }
        l
    }
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..(self.len as usize).min(N)] }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    /// Piece `part` holding `n` bytes; false when it does not fit.
    pub fn put(&mut self, part: u8, n: u8, a: u64, b: u64) -> bool {
        let at = part as usize * PIECE;
        if n as usize > PIECE || at + n as usize > N { return false; }
        let raw = piece_bytes(a, b);
        for i in 0..n as usize { self.bytes[at + i] = clean(raw[i]); }
        self.len = self.len.max((at + n as usize) as u8);
        true
    }
    /// The pieces: (part, bytes in it, a, b). An empty line has none.
    pub fn pieces(&self) -> impl Iterator<Item = (u8, u8, u64, u64)> + '_ {
        let t = self.as_bytes();
        (0..t.len().div_ceil(PIECE)).map(move |p| {
            let s = &t[p * PIECE..((p + 1) * PIECE).min(t.len())];
            let (a, b) = piece_words(s);
            (p as u8, s.len() as u8, a, b)
        })
    }
    /// Adds one byte at the end; false when full.
    pub fn push(&mut self, c: u8) -> bool {
        if self.len as usize >= N { return false; }
        self.bytes[self.len as usize] = clean(c);
        self.len += 1;
        true
    }
    pub fn pop(&mut self) -> bool { if self.len == 0 { false } else { self.len -= 1; true } }
    pub fn clear(&mut self) { self.len = 0; }
}

pub fn piece_words(s: &[u8]) -> (u64, u64) {
    let mut b = [0u8; PIECE];
    let n = s.len().min(PIECE);
    b[..n].copy_from_slice(&s[..n]);
    (u64::from_le_bytes(b[..8].try_into().unwrap()), u64::from_le_bytes(b[8..].try_into().unwrap()))
}
pub fn piece_bytes(a: u64, b: u64) -> [u8; PIECE] {
    let mut raw = [0u8; PIECE];
    raw[..8].copy_from_slice(&a.to_le_bytes());
    raw[8..].copy_from_slice(&b.to_le_bytes());
    raw
}

/// The icon's label (up to three characters) and tile colour 0x00RRGGBB.
pub fn icon_word(label: &[u8], colour: u32) -> u64 {
    let mut w = 0u64;
    for (i, &b) in label.iter().take(LABEL_BYTES).enumerate() { w |= (clean(b) as u64) << (8 * i); }
    w | ((colour & 0x00FF_FFFF) as u64) << 24
}
pub fn icon_decode(w: u64) -> ([u8; LABEL_BYTES], u8, u32) {
    let mut l = [0u8; LABEL_BYTES];
    let mut n = 0;
    for i in 0..LABEL_BYTES {
        let b = (w >> (8 * i)) as u8;
        if b == 0 { break; }
        l[i] = clean(b); n += 1;
    }
    (l, n as u8, (w >> 24) as u32 & 0x00FF_FFFF)
}

// ── Program -> Shell: tray items ────────────────────────────────────────────

/// Field numbers in `ItemMsg::Text`: the tooltip, then each menu row.
pub const FIELD_TIP: u8 = 0;
pub const fn field_row(row: u8) -> u8 { 1 + row }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemMsg {
    /// Starts item `id` again: its label, colour and menu row count. The item
    /// is hidden until `Commit`.
    Begin { id: u8, label: [u8; LABEL_BYTES], label_len: u8, colour: u32, rows: u8 },
    /// A piece of the tooltip (`FIELD_TIP`) or of a row (`field_row`).
    Text { id: u8, field: u8, part: u8, len: u8, a: u64, b: u64 },
    /// Shows the item as begun and filled.
    Commit { id: u8 },
    Remove { id: u8 },
}

const K_BEGIN: u64 = 1;
const K_TEXT: u64 = 2;
const K_COMMIT: u64 = 3;
const K_REMOVE: u64 = 4;

fn head(kind: u64, id: u8, field: u8, part: u8, len: u8) -> u64 {
    kind | (id as u64) << 8 | (field as u64) << 16 | (part as u64) << 24 | (len as u64) << 32
}

impl ItemMsg {
    pub fn wire(&self) -> [u64; 3] {
        match *self {
            ItemMsg::Begin { id, label, label_len, colour, rows } =>
                [head(K_BEGIN, id, rows, 0, 0), icon_word(&label[..(label_len as usize).min(LABEL_BYTES)], colour), 0],
            ItemMsg::Text { id, field, part, len, a, b } => [head(K_TEXT, id, field, part, len), a, b],
            ItemMsg::Commit { id } => [head(K_COMMIT, id, 0, 0, 0), 0, 0],
            ItemMsg::Remove { id } => [head(K_REMOVE, id, 0, 0, 0), 0, 0],
        }
    }
    pub fn decode(a0: u64, a1: u64, a2: u64) -> Option<Self> {
        let (kind, id, field, part, len) = (a0 & 0xFF, (a0 >> 8) as u8, (a0 >> 16) as u8, (a0 >> 24) as u8, (a0 >> 32) as u8);
        if id >= ITEMS_PER_PROGRAM || a0 >> 40 != 0 { return None; }
        match kind {
            K_BEGIN => {
                if field as usize > MAX_ROWS || part != 0 || len != 0 || a1 >> 48 != 0 || a2 != 0 { return None; }
                let (label, label_len, colour) = icon_decode(a1);
                Some(ItemMsg::Begin { id, label, label_len, colour, rows: field })
            }
            K_TEXT => {
                let cap = if field == FIELD_TIP { TIP_BYTES } else if (field as usize) <= MAX_ROWS { ROW_BYTES } else { return None };
                if len == 0 || len as usize > PIECE || part as usize * PIECE + len as usize > cap { return None; }
                Some(ItemMsg::Text { id, field, part, len, a: a1, b: a2 })
            }
            K_COMMIT if field == 0 && part == 0 && len == 0 && a1 == 0 && a2 == 0 => Some(ItemMsg::Commit { id }),
            K_REMOVE if field == 0 && part == 0 && len == 0 && a1 == 0 && a2 == 0 => Some(ItemMsg::Remove { id }),
            _ => None,
        }
    }
}

/// The calls that set item `id` (begin, tooltip and row pieces, commit).
pub fn item_calls(id: u8, label: &[u8], colour: u32, tip: &[u8], rows: &[&[u8]], mut send: impl FnMut([u64; 3]) -> bool) -> bool {
    let mut l = [0u8; LABEL_BYTES];
    let n = label.len().min(LABEL_BYTES);
    l[..n].copy_from_slice(&label[..n]);
    let rows = &rows[..rows.len().min(MAX_ROWS)];
    if !send(ItemMsg::Begin { id, label: l, label_len: n as u8, colour, rows: rows.len() as u8 }.wire()) { return false; }
    let tip = Line::<TIP_BYTES>::new(tip);
    for (part, len, a, b) in tip.pieces() {
        if !send(ItemMsg::Text { id, field: FIELD_TIP, part, len, a, b }.wire()) { return false; }
    }
    for (r, row) in rows.iter().enumerate() {
        let row = Line::<ROW_BYTES>::new(row);
        for (part, len, a, b) in row.pieces() {
            if !send(ItemMsg::Text { id, field: field_row(r as u8), part, len, a, b }.wire()) { return false; }
        }
    }
    send(ItemMsg::Commit { id }.wire())
}

// ── Program -> Shell: notification actions ──────────────────────────────────

/// Field numbers in `ActionsMsg::Text`: the buttons 0..3, then the reply hint.
pub const FIELD_PLACEHOLDER: u8 = MAX_ACTIONS as u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionsMsg {
    /// `actions` buttons (0..=3), an inline reply field or not, and the
    /// program's own tag, told back with the answer.
    Begin { tag: u32, actions: u8, reply: bool },
    Text { field: u8, part: u8, len: u8, a: u64, b: u64 },
    Commit,
}

impl ActionsMsg {
    pub fn wire(&self) -> [u64; 3] {
        match *self {
            ActionsMsg::Begin { tag, actions, reply } => [head(K_BEGIN, 0, actions, reply as u8, 0), tag as u64, 0],
            ActionsMsg::Text { field, part, len, a, b } => [head(K_TEXT, 0, field, part, len), a, b],
            ActionsMsg::Commit => [head(K_COMMIT, 0, 0, 0, 0), 0, 0],
        }
    }
    pub fn decode(a0: u64, a1: u64, a2: u64) -> Option<Self> {
        let (kind, id, field, part, len) = (a0 & 0xFF, (a0 >> 8) as u8, (a0 >> 16) as u8, (a0 >> 24) as u8, (a0 >> 32) as u8);
        if id != 0 || a0 >> 40 != 0 { return None; }
        match kind {
            K_BEGIN if field as usize <= MAX_ACTIONS && part <= 1 && len == 0 && a1 >> 32 == 0 && a2 == 0 =>
                Some(ActionsMsg::Begin { tag: a1 as u32, actions: field, reply: part == 1 }),
            K_TEXT if field <= FIELD_PLACEHOLDER && part == 0 && len != 0 && len as usize <= ACTION_BYTES =>
                Some(ActionsMsg::Text { field, part, len, a: a1, b: a2 }),
            K_COMMIT if field == 0 && part == 0 && len == 0 && a1 == 0 && a2 == 0 => Some(ActionsMsg::Commit),
            _ => None,
        }
    }
}

/// The calls for a notification's buttons and reply field.
pub fn actions_calls(tag: u32, actions: &[&[u8]], reply: Option<&[u8]>, mut send: impl FnMut([u64; 3]) -> bool) -> bool {
    let actions = &actions[..actions.len().min(MAX_ACTIONS)];
    if !send(ActionsMsg::Begin { tag, actions: actions.len() as u8, reply: reply.is_some() }.wire()) { return false; }
    for (i, label) in actions.iter().enumerate().map(|(i, l)| (i as u8, *l)).chain(reply.map(|r| (FIELD_PLACEHOLDER, r))) {
        let line = Line::<ACTION_BYTES>::new(label);
        let first = line.pieces().next();
        if let Some((part, len, a, b)) = first {
            if !send(ActionsMsg::Text { field: i, part, len, a, b }.wire()) { return false; }
        }
    }
    send(ActionsMsg::Commit.wire())
}

// ── Shell -> program ────────────────────────────────────────────────────────

/// Why a card went away unanswered.
pub const GONE_CLOSED: u8 = 1;
pub const GONE_EXPIRED: u8 = 2;
pub const GONE_REPLACED: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// An item with no menu was clicked.
    Activated { item: u8 },
    /// A row of an item's menu was chosen.
    Menu { item: u8, row: u8 },
    /// A button of the notification tagged `tag` was pressed.
    Action { tag: u32, action: u8 },
    /// A piece of the reply typed into that notification's field.
    Reply { tag: u32, part: u8, total: u8, a: u64, b: u64 },
    /// The notification went away unanswered (`GONE_*`).
    Gone { tag: u32, why: u8 },
}

const E_ACTIVATED: u64 = 1;
const E_MENU: u64 = 2;
const E_ACTION: u64 = 3;
const E_REPLY: u64 = 4;
const E_GONE: u64 = 5;

fn ev(kind: u64, x: u8, y: u8, tag: u32) -> u64 { kind | (x as u64) << 8 | (y as u64) << 16 | (tag as u64) << 32 }

impl Event {
    pub fn wire(&self) -> [u64; 3] {
        match *self {
            Event::Activated { item } => [ev(E_ACTIVATED, item, 0, 0), 0, 0],
            Event::Menu { item, row } => [ev(E_MENU, item, row, 0), 0, 0],
            Event::Action { tag, action } => [ev(E_ACTION, action, 0, tag), 0, 0],
            Event::Reply { tag, part, total, a, b } => [ev(E_REPLY, part, total, tag), a, b],
            Event::Gone { tag, why } => [ev(E_GONE, why, 0, tag), 0, 0],
        }
    }
    pub fn decode(a0: u64, a1: u64, a2: u64) -> Option<Self> {
        let (kind, x, y, tag) = (a0 & 0xFF, (a0 >> 8) as u8, (a0 >> 16) as u8, (a0 >> 32) as u32);
        if a0 >> 24 & 0xFF != 0 { return None; }
        let plain = a1 == 0 && a2 == 0;
        match kind {
            E_ACTIVATED if x < ITEMS_PER_PROGRAM && y == 0 && tag == 0 && plain => Some(Event::Activated { item: x }),
            E_MENU if x < ITEMS_PER_PROGRAM && (y as usize) < MAX_ROWS && tag == 0 && plain => Some(Event::Menu { item: x, row: y }),
            E_ACTION if (x as usize) < MAX_ACTIONS && y == 0 && plain => Some(Event::Action { tag, action: x }),
            E_REPLY if (y as usize) <= REPLY_MAX && (x as usize) * PIECE < (y as usize).max(1) => Some(Event::Reply { tag, part: x, total: y, a: a1, b: a2 }),
            E_GONE if (GONE_CLOSED..=GONE_REPLACED).contains(&x) && y == 0 && plain => Some(Event::Gone { tag, why: x }),
            _ => None,
        }
    }
}

/// A reply as events: one piece per 16 bytes, at least one (an empty reply).
pub fn reply_events(tag: u32, text: &[u8]) -> impl Iterator<Item = Event> + '_ {
    let text = &text[..text.len().min(REPLY_MAX)];
    let total = text.len() as u8;
    (0..text.len().div_ceil(PIECE).max(1)).map(move |p| {
        let (a, b) = piece_words(&text[(p * PIECE).min(text.len())..((p + 1) * PIECE).min(text.len())]);
        Event::Reply { tag, part: p as u8, total, a, b }
    })
}

/// Puts a reply's pieces back together in the program.
#[derive(Clone, Copy, Debug)]
pub struct ReplyAssembler { text: Line<REPLY_MAX>, tag: u32, next: u8 }
impl ReplyAssembler {
    pub const fn new() -> Self { Self { text: Line::EMPTY, tag: 0, next: 0 } }
    /// The whole reply (tag, text) once its last piece is in. A piece out of
    /// order starts over; a first piece always does.
    pub fn push(&mut self, e: Event) -> Option<(u32, Line<REPLY_MAX>)> {
        let Event::Reply { tag, part, total, a, b } = e else { return None };
        if part == 0 { self.next = 0; self.tag = tag; self.text = Line::EMPTY; }
        if part != self.next || tag != self.tag { self.next = 0; return None; }
        let at = part as usize * PIECE;
        let n = (total as usize).saturating_sub(at).min(PIECE) as u8;
        if n > 0 && !self.text.put(part, n, a, b) { self.next = 0; return None; }
        if at + PIECE >= total as usize { self.next = 0; return Some((tag, self.text)); }
        self.next += 1;
        None
    }
}

// ── Shell -> Display: what to draw ──────────────────────────────────────────

/// One item in the panel, where the Shell placed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Icon { pub x: u16, pub y: u16, pub size: u16, pub label: [u8; LABEL_BYTES], pub label_len: u8, pub colour: u32 }

/// An item's open menu, anchored at (x, y).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuView { pub x: u16, pub y: u16, pub rows: u8, pub hover: Option<u8>, pub text: [Line<ROW_BYTES>; MAX_ROWS] }

/// A tooltip under an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TipView { pub x: u16, pub y: u16, pub text: Line<TIP_BYTES> }

/// A notification card with buttons and, perhaps, a reply field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardView {
    pub actions: u8,
    pub reply: bool,
    pub from: Line<FROM_BYTES>,
    /// "summary\nbody".
    pub text: Line<CARD_TEXT>,
    pub labels: [Line<ACTION_BYTES>; MAX_ACTIONS],
    pub placeholder: Line<ACTION_BYTES>,
}
impl CardView {
    pub fn summary(&self) -> &[u8] { self.text.as_bytes().split(|b| *b == b'\n').next().unwrap_or(&[]) }
    pub fn body(&self) -> &[u8] {
        let t = self.text.as_bytes();
        t.iter().position(|b| *b == b'\n').map_or(&[][..], |i| &t[i + 1..])
    }
}

/// What is typed into the card's reply field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyView { pub active: bool, pub text: Line<REPLY_MAX> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub icons: [Icon; MAX_ITEMS],
    pub icon_count: u8,
    pub menu: Option<MenuView>,
    pub tip: Option<TipView>,
    pub card: Option<CardView>,
    pub reply: ReplyView,
}
impl View {
    pub const EMPTY: Self = Self {
        icons: [Icon { x: 0, y: 0, size: 0, label: [0; LABEL_BYTES], label_len: 0, colour: 0 }; MAX_ITEMS], icon_count: 0,
        menu: None, tip: None, card: None, reply: ReplyView { active: false, text: Line::EMPTY },
    };
    pub fn icons(&self) -> &[Icon] { &self.icons[..(self.icon_count as usize).min(MAX_ITEMS)] }
}

/// A part of the view sent and shown on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section { Icons = 1, Menu = 2, Tip = 3, Card = 4, Reply = 5 }
impl Section {
    pub const ALL: [Section; 5] = [Section::Icons, Section::Menu, Section::Tip, Section::Card, Section::Reply];
    pub fn of(n: u8) -> Option<Self> { Self::ALL.iter().copied().find(|s| *s as u8 == n) }
    pub fn bit(self) -> u8 { 1 << (self as u8) }
}

const V_HEAD: u64 = 1;
const V_TEXT: u64 = 2;
const V_COMMIT: u64 = 3;
const V_ICON: u64 = 4;

/// Card fields: text, from, three labels, placeholder.
const CF_TEXT: u8 = 0;
const CF_FROM: u8 = 1;
const CF_LABEL: u8 = 2;
const CF_PLACEHOLDER: u8 = CF_LABEL + MAX_ACTIONS as u8;

fn vtext(section: Section, field: u8, part: u8, len: u8, a: u64, b: u64) -> [u64; 3] {
    [V_TEXT | (section as u64) << 8 | (field as u64) << 16 | (part as u64) << 24 | (len as u64) << 32, a, b]
}

/// The calls that show `section` of `view`; stops at the first refused.
pub fn view_calls(view: &View, section: Section, mut send: impl FnMut([u64; 3]) -> bool) -> bool {
    let s = section as u64;
    let head = |a1: u64, shown: bool| [V_HEAD | s << 8, a1, shown as u64];
    match section {
        Section::Icons => {
            if !send(head(view.icon_count as u64, true)) { return false; }
            for (i, ic) in view.icons().iter().enumerate() {
                let w = V_ICON | (i as u64) << 8 | (ic.x as u64) << 16 | (ic.y as u64) << 32 | ((ic.size as u64) & 0xFFFF) << 48;
                if !send([w, icon_word(&ic.label[..(ic.label_len as usize).min(LABEL_BYTES)], ic.colour), 0]) { return false; }
            }
        }
        Section::Menu => match &view.menu {
            None => if !send(head(0, false)) { return false; },
            Some(m) => {
                let a1 = m.x as u64 | (m.y as u64) << 16 | (m.rows as u64) << 32 | (m.hover.unwrap_or(0xFF) as u64) << 40;
                if !send(head(a1, true)) { return false; }
                for (r, line) in m.text.iter().take(m.rows as usize).enumerate() {
                    for (part, len, a, b) in line.pieces() { if !send(vtext(section, r as u8, part, len, a, b)) { return false; } }
                }
            }
        },
        Section::Tip => match &view.tip {
            None => if !send(head(0, false)) { return false; },
            Some(t) => {
                if !send(head(t.x as u64 | (t.y as u64) << 16, true)) { return false; }
                for (part, len, a, b) in t.text.pieces() { if !send(vtext(section, 0, part, len, a, b)) { return false; } }
            }
        },
        Section::Card => match &view.card {
            None => if !send(head(0, false)) { return false; },
            Some(c) => {
                if !send(head(c.actions as u64 | (c.reply as u64) << 8, true)) { return false; }
                let fields = [(CF_TEXT, c.text.as_bytes()), (CF_FROM, c.from.as_bytes())];
                for (field, bytes) in fields {
                    for (p, chunk) in bytes.chunks(PIECE).enumerate() {
                        let (a, b) = piece_words(chunk);
                        if !send(vtext(section, field, p as u8, chunk.len() as u8, a, b)) { return false; }
                    }
                }
                for (i, l) in c.labels.iter().take(c.actions as usize).enumerate().map(|(i, l)| (CF_LABEL + i as u8, l)).chain(core::iter::once((CF_PLACEHOLDER, &c.placeholder))) {
                    for (part, len, a, b) in l.pieces() { if !send(vtext(section, i, part, len, a, b)) { return false; } }
                }
            }
        },
        Section::Reply => {
            if !send(head(view.reply.active as u64, true)) { return false; }
            for (part, len, a, b) in view.reply.text.pieces() { if !send(vtext(section, 0, part, len, a, b)) { return false; } }
        }
    }
    send([V_COMMIT | s << 8, 0, 0])
}

/// Display's side: sections are filled in `staged` and shown on commit, so a
/// half-sent section never shows.
#[derive(Clone, Copy, Debug)]
pub struct Receiver { pub shown: View, staged: View, open: u8 }
impl Receiver {
    pub const fn new() -> Self { Self { shown: View::EMPTY, staged: View::EMPTY, open: 0 } }
    /// One call; the section committed, if this call committed one.
    pub fn push(&mut self, a0: u64, a1: u64, a2: u64) -> Option<Section> {
        let kind = a0 & 0xFF;
        if kind == V_ICON {
            if self.open & Section::Icons.bit() == 0 { return None; }
            let i = (a0 >> 8) as u8 as usize;
            if i >= (self.staged.icon_count as usize).min(MAX_ITEMS) || a2 != 0 { return None; }
            let (label, label_len, colour) = icon_decode(a1);
            self.staged.icons[i] = Icon { x: (a0 >> 16) as u16, y: (a0 >> 32) as u16, size: (a0 >> 48) as u16, label, label_len, colour };
            return None;
        }
        let section = Section::of((a0 >> 8) as u8)?;
        match kind {
            V_HEAD => {
                self.open |= section.bit();
                let shown = a2 & 1 != 0;
                match section {
                    Section::Icons => { self.staged.icons = View::EMPTY.icons; self.staged.icon_count = (a1 as usize).min(MAX_ITEMS) as u8; }
                    Section::Menu => {
                        let rows = ((a1 >> 32) as u8).min(MAX_ROWS as u8);
                        let hover = (a1 >> 40) as u8;
                        self.staged.menu = shown.then_some(MenuView { x: a1 as u16, y: (a1 >> 16) as u16, rows,
                            hover: (hover < rows).then_some(hover), text: [Line::EMPTY; MAX_ROWS] });
                    }
                    Section::Tip => { self.staged.tip = shown.then_some(TipView { x: a1 as u16, y: (a1 >> 16) as u16, text: Line::EMPTY }); }
                    Section::Card => {
                        self.staged.card = shown.then_some(CardView { actions: (a1 as u8).min(MAX_ACTIONS as u8), reply: a1 >> 8 & 1 != 0,
                            from: Line::EMPTY, text: Line::EMPTY, labels: [Line::EMPTY; MAX_ACTIONS], placeholder: Line::EMPTY });
                    }
                    Section::Reply => { self.staged.reply = ReplyView { active: a1 & 1 != 0, text: Line::EMPTY }; }
                }
                None
            }
            V_TEXT => {
                if self.open & section.bit() == 0 { return None; }
                let (field, part, len) = ((a0 >> 16) as u8, (a0 >> 24) as u8, (a0 >> 32) as u8);
                let ok = match section {
                    Section::Menu => self.staged.menu.as_mut().and_then(|m| m.text.get_mut(field as usize)).map(|l| l.put(part, len, a1, a2)),
                    Section::Tip => self.staged.tip.as_mut().filter(|_| field == 0).map(|t| t.text.put(part, len, a1, a2)),
                    Section::Card => self.staged.card.as_mut().and_then(|c| match field {
                        CF_TEXT => Some(c.text.put(part, len, a1, a2)),
                        CF_FROM => Some(c.from.put(part, len, a1, a2)),
                        CF_PLACEHOLDER => Some(c.placeholder.put(part, len, a1, a2)),
                        f if f >= CF_LABEL && f < CF_PLACEHOLDER => Some(c.labels[(f - CF_LABEL) as usize].put(part, len, a1, a2)),
                        _ => None,
                    }),
                    Section::Reply => (field == 0).then(|| self.staged.reply.text.put(part, len, a1, a2)),
                    Section::Icons => None,
                };
                if ok != Some(true) { self.open &= !section.bit(); }
                None
            }
            V_COMMIT => {
                if self.open & section.bit() == 0 { return None; }
                self.open &= !section.bit();
                match section {
                    Section::Icons => { self.shown.icons = self.staged.icons; self.shown.icon_count = self.staged.icon_count; }
                    Section::Menu => self.shown.menu = self.staged.menu,
                    Section::Tip => self.shown.tip = self.staged.tip,
                    Section::Card => self.shown.card = self.staged.card,
                    Section::Reply => self.shown.reply = self.staged.reply,
                }
                Some(section)
            }
            _ => None,
        }
    }
}

// ── Layouts both sides share ────────────────────────────────────────────────

pub type Rect = (usize, usize, usize, usize);
pub fn inside(r: Rect, x: usize, y: usize) -> bool { x >= r.0 && y >= r.1 && x - r.0 < r.2 && y - r.1 < r.3 }

/// Where item `i` (0 nearest the status icons) goes in the panel: right to
/// left from `right` (the status icons' left edge), each `size` square with
/// a `gap`, at `y`; None once it would cross `left` (the workspace dots).
pub fn icon_rect(i: usize, right: usize, left: usize, y: usize, size: usize, gap: usize) -> Option<Rect> {
    let x = right.checked_sub(gap + (i + 1) * (size + gap))?;
    (x >= left).then_some((x, y, size, size))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit { Row(usize), Inside, Outside }

/// An item's menu under it, kept on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuLayout { pub panel: Rect, pub rows: usize, pub row_h: usize, pub pad: usize }
impl MenuLayout {
    pub const WIDTH: usize = 200;
    pub const ROW: usize = 24;
    pub const PAD: usize = 4;
    pub fn new(x: usize, y: usize, rows: usize, width: usize, height: usize, scale: usize) -> Option<Self> {
        let s = scale.clamp(1, 3);
        let (w, row_h, pad) = (Self::WIDTH * s, Self::ROW * s, Self::PAD * s);
        let h = rows.max(1) * row_h + 2 * pad;
        if rows == 0 || width < w || height < h { return None; }
        Some(Self { panel: (x.min(width - w), y.min(height - h), w, h), rows, row_h, pad })
    }
    pub fn row(&self, i: usize) -> Rect {
        let (x, y, w, _) = self.panel;
        (x + self.pad, y + self.pad + i * self.row_h, w - 2 * self.pad, self.row_h)
    }
    pub fn hit(&self, x: usize, y: usize) -> Hit {
        if !inside(self.panel, x, y) { return Hit::Outside; }
        (0..self.rows).find(|&i| inside(self.row(i), x, y)).map_or(Hit::Inside, Hit::Row)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardHit { Close, Action(u8), Field, Send, Inside, Outside }

/// The notification card: at the right under the panel, like the quick menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardLayout {
    pub panel: Rect,
    pub close: Rect,
    /// The sender and summary line, and the body line.
    pub head: Rect,
    pub body: Rect,
    pub actions: [Rect; MAX_ACTIONS],
    pub action_count: usize,
    pub field: Option<Rect>,
    pub send: Option<Rect>,
}
impl CardLayout {
    pub const WIDTH: usize = 360;
    pub fn new(width: usize, bar: usize, scale: usize, actions: usize, reply: bool) -> Option<Self> {
        let s = scale.clamp(1, 3);
        let (w, pad, gap, line, button) = (Self::WIDTH * s, 12 * s, 8 * s, 22 * s, 28 * s);
        if width < w + 16 * s { return None; }
        let (x, y) = (width - w - 8 * s, bar + 6 * s);
        let inner = w - 2 * pad;
        let close = (x + w - pad - 18 * s, y + pad, 18 * s, 18 * s);
        let head = (x + pad, y + pad, inner - 22 * s, line);
        let body = (x + pad, head.1 + line, inner, line);
        let mut at = body.1 + line + gap;
        let n = actions.min(MAX_ACTIONS);
        let mut rects = [(0, 0, 0, 0); MAX_ACTIONS];
        if n > 0 {
            let bw = (inner - (n - 1) * gap) / n;
            for (i, r) in rects.iter_mut().take(n).enumerate() { *r = (x + pad + i * (bw + gap), at, bw, button); }
            at += button + gap;
        }
        let (field, send) = if reply {
            let sw = 72 * s;
            let f = (x + pad, at, inner - sw - gap, button);
            at += button + gap;
            (Some(f), Some((f.0 + f.2 + gap, f.1, sw, button)))
        } else { (None, None) };
        let h = at - gap + pad - y;
        Some(Self { panel: (x, y, w, h), close, head, body, actions: rects, action_count: n, field, send })
    }
    pub fn hit(&self, x: usize, y: usize) -> CardHit {
        if !inside(self.panel, x, y) { return CardHit::Outside; }
        if inside(self.close, x, y) { return CardHit::Close; }
        if let Some(i) = self.actions[..self.action_count].iter().position(|r| inside(*r, x, y)) { return CardHit::Action(i as u8); }
        if self.field.is_some_and(|r| inside(r, x, y)) { return CardHit::Field; }
        if self.send.is_some_and(|r| inside(r, x, y)) { return CardHit::Send; }
        CardHit::Inside
    }
}

/// A key typed into the reply field: the character for a US keyboard
/// scancode, with Shift.
pub fn key_char(code: u8, shift: bool) -> Option<u8> {
    const ROW1: &[u8; 12] = b"1234567890-=";
    const ROW1_S: &[u8; 12] = b"!@#$%^&*()_+";
    const TOP: &[u8; 12] = b"qwertyuiop[]";
    const TOP_S: &[u8; 12] = b"QWERTYUIOP{}";
    const HOME: &[u8; 12] = b"asdfghjkl;'`";
    const HOME_S: &[u8; 12] = b"ASDFGHJKL:\"~";
    const BOTTOM: &[u8; 10] = b"zxcvbnm,./";
    const BOTTOM_S: &[u8; 10] = b"ZXCVBNM<>?";
    let pick = |plain: &[u8], shifted: &[u8], i: usize| Some(if shift { shifted[i] } else { plain[i] });
    match code {
        0x02..=0x0D => pick(ROW1, ROW1_S, (code - 0x02) as usize),
        0x10..=0x1B => pick(TOP, TOP_S, (code - 0x10) as usize),
        0x1E..=0x29 => pick(HOME, HOME_S, (code - 0x1E) as usize),
        0x2B => Some(if shift { b'|' } else { b'\\' }),
        0x2C..=0x35 => pick(BOTTOM, BOTTOM_S, (code - 0x2C) as usize),
        0x39 => Some(b' '),
        _ => None,
    }
}
