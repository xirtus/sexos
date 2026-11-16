// SPDX-License-Identifier: GPL-3.0-only
//! Program tray items and notification actions (slice S2), the pure part:
//! the items programs keep in the panel, the menu one opens, the cards with
//! buttons and a reply field that actionable notifications show, the reply
//! typed into one, and what to tell each program. Cleared for a program when
//! its run ends. The glue (`tray_items_glue.rs`) feeds it the programs'
//! messages, Bell's notifications, clicks, pointer moves and keys, sends
//! `sex_pdx::program_tray::View` sections to Display and the events to the
//! programs.
//!
//! Host-tested by tests/tray_items.rs against a `sex_pdx` built from only
//! `program_tray`.

use sex_pdx::program_tray::{self as pt, ActionsMsg, CardHit, CardLayout, CardView, Event, Hit, ItemMsg, Line, MenuLayout, MenuView, Rect,
    ReplyView, Section, TipView, View};

/// How long an action list waits for its notification from Bell.
pub const PAIR_MS: u64 = 3_000;
/// How long a card shows when nothing is typed into it.
pub const CARD_MS: u64 = 30_000;
/// Cards waiting behind the one shown.
pub const CARD_QUEUE: usize = 4;
pub const OUTBOX: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub pd: u32,
    pub task: u32,
    pub id: u8,
    pub label: [u8; pt::LABEL_BYTES],
    pub label_len: u8,
    pub colour: u32,
    pub tip: Line<{ pt::TIP_BYTES }>,
    pub rows: u8,
    pub text: [Line<{ pt::ROW_BYTES }>; pt::MAX_ROWS],
    /// Shown: committed since its last begin.
    pub ready: bool,
    /// Older items sit nearer the status icons.
    pub order: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    pd: u32,
    task: u32,
    tag: u32,
    actions: u8,
    reply: bool,
    labels: [Line<{ pt::ACTION_BYTES }>; pt::MAX_ACTIONS],
    placeholder: Line<{ pt::ACTION_BYTES }>,
    /// When committed; None while it is still being sent.
    at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Card {
    pub pd: u32,
    pub task: u32,
    pub tag: u32,
    pub view: CardView,
    /// When it goes away unanswered; 0 until it shows.
    pub until: u64,
}

/// Something to tell a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Event { pd: u32, task: u32, event: Event },
    Reply { pd: u32, task: u32, tag: u32, text: Line<{ pt::REPLY_MAX }> },
}
impl Out {
    pub fn pd(&self) -> u32 { match *self { Out::Event { pd, .. } | Out::Reply { pd, .. } => pd } }
    pub fn task(&self) -> u32 { match *self { Out::Event { task, .. } | Out::Reply { task, .. } => task } }
}

/// Where things go on screen, from the Shell's bar and desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub width: usize,
    pub height: usize,
    pub bar: usize,
    pub scale: usize,
    /// The status icons' left edge and the workspace dots' right edge.
    pub right: usize,
    pub left: usize,
    /// The items' top and size, and the gap between them.
    pub y: usize,
    pub size: usize,
    pub gap: usize,
}

/// What a press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Not on anything here: the click goes on.
    Pass,
    /// Taken, with nothing more to say.
    Taken,
    MenuOpened { pd: u32, id: u8, rows: u8, panel: Rect, row_h: usize, pad: usize },
    MenuClosed,
    Chose { pd: u32, id: u8, row: u8 },
    Activated { pd: u32, id: u8 },
    CardAction { pd: u32, tag: u32, action: u8 },
    CardClosed { pd: u32, tag: u32 },
    ReplyFocused { pd: u32, tag: u32 },
    ReplySent { pd: u32, tag: u32, len: usize },
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key { Pass, Taken, Typed, Sent { pd: u32, tag: u32, len: usize }, Left, MenuClosed }

pub struct Tray {
    items: [Option<Item>; pt::MAX_ITEMS],
    next_order: u32,
    pending: [Option<Pending>; 4],
    /// Index 0 shows; the rest wait.
    cards: [Option<Card>; CARD_QUEUE],
    /// The open menu's item (pd, id) and the row under the pointer.
    menu: Option<(u32, u8, Option<u8>)>,
    /// The item under the pointer, for its tooltip.
    hover: Option<(u32, u8)>,
    reply: ReplyView,
    shift: [bool; 2],
    outbox: [Option<Out>; OUTBOX],
    /// Sections Display has not been sent since they changed.
    dirty: u8,
}

impl Tray {
    pub const fn new() -> Self {
        Self { items: [None; pt::MAX_ITEMS], next_order: 0, pending: [None; 4], cards: [None; CARD_QUEUE], menu: None, hover: None,
            reply: ReplyView { active: false, text: Line::EMPTY }, shift: [false; 2], outbox: [None; OUTBOX], dirty: 0 }
    }

    fn mark(&mut self, s: Section) { self.dirty |= s.bit(); }
    /// Sections to send, as `Section::bit`s.
    pub fn dirty(&self) -> u8 { self.dirty }
    pub fn sent(&mut self, s: Section) { self.dirty &= !s.bit(); }
    pub fn mark_all(&mut self) { for s in Section::ALL { self.mark(s); } }

    pub fn items(&self) -> impl Iterator<Item = &Item> { self.items.iter().flatten() }
    pub fn card(&self) -> Option<&Card> { self.cards[0].as_ref() }
    pub fn queued(&self) -> usize { self.cards.iter().flatten().count() }
    pub fn menu_open(&self) -> bool { self.menu.is_some() }
    pub fn reply_active(&self) -> bool { self.reply.active && self.cards[0].is_some() }
    pub fn reply_text(&self) -> &[u8] { self.reply.text.as_bytes() }

    fn find(&self, pd: u32, id: u8) -> Option<usize> { self.items.iter().position(|i| i.is_some_and(|i| i.pd == pd && i.id == id)) }

    /// A program's `OP_TRAY_ITEM`. False when refused (no room, or a piece
    /// for an item not being begun).
    pub fn item_msg(&mut self, pd: u32, task: u32, msg: ItemMsg) -> bool {
        match msg {
            ItemMsg::Begin { id, label, label_len, colour, rows } => {
                let at = match self.find(pd, id) {
                    Some(i) => i,
                    None => match self.items.iter().position(|i| i.is_none()) { Some(i) => i, None => return false },
                };
                let order = match self.items[at] { Some(old) if old.task == task => old.order, _ => { self.next_order += 1; self.next_order } };
                if self.menu.is_some_and(|m| m.0 == pd && m.1 == id) { self.menu = None; self.mark(Section::Menu); }
                self.items[at] = Some(Item { pd, task, id, label, label_len, colour, tip: Line::EMPTY, rows: rows.min(pt::MAX_ROWS as u8),
                    text: [Line::EMPTY; pt::MAX_ROWS], ready: false, order });
                self.mark(Section::Icons);
                true
            }
            ItemMsg::Text { id, field, part, len, a, b } => {
                let Some(i) = self.find(pd, id) else { return false };
                let Some(item) = self.items[i].as_mut().filter(|i| i.task == task && !i.ready) else { return false };
                if field == pt::FIELD_TIP { return item.tip.put(part, len, a, b); }
                let row = (field - 1) as usize;
                row < item.rows as usize && item.text[row].put(part, len, a, b)
            }
            ItemMsg::Commit { id } => {
                let Some(i) = self.find(pd, id) else { return false };
                let Some(item) = self.items[i].as_mut().filter(|i| i.task == task) else { return false };
                item.ready = true;
                self.mark(Section::Icons);
                true
            }
            ItemMsg::Remove { id } => {
                let Some(i) = self.find(pd, id) else { return false };
                if self.items[i].is_some_and(|i| i.task != task) { return false; }
                self.items[i] = None;
                self.forget_item(pd, id);
                true
            }
        }
    }

    fn forget_item(&mut self, pd: u32, id: u8) {
        if self.menu.is_some_and(|m| m.0 == pd && m.1 == id) { self.menu = None; self.mark(Section::Menu); }
        if self.hover == Some((pd, id)) { self.hover = None; self.mark(Section::Tip); }
        self.mark(Section::Icons);
    }

    /// A program's `OP_NOTIFY_ACTIONS`. False when refused.
    pub fn actions_msg(&mut self, pd: u32, task: u32, msg: ActionsMsg, now: u64) -> bool {
        let slot = self.pending.iter().position(|p| p.is_some_and(|p| p.pd == pd));
        match msg {
            ActionsMsg::Begin { tag, actions, reply } => {
                let Some(i) = slot.or_else(|| self.pending.iter().position(|p| p.is_none()))
                    .or_else(|| (0..self.pending.len()).min_by_key(|&i| self.pending[i].map_or(0, |p| p.at.unwrap_or(0)))) else { return false };
                self.pending[i] = Some(Pending { pd, task, tag, actions: actions.min(pt::MAX_ACTIONS as u8), reply,
                    labels: [Line::EMPTY; pt::MAX_ACTIONS], placeholder: Line::EMPTY, at: None });
                true
            }
            ActionsMsg::Text { field, part, len, a, b } => {
                let Some(p) = slot.and_then(|i| self.pending[i].as_mut()).filter(|p| p.task == task && p.at.is_none()) else { return false };
                if field == pt::FIELD_PLACEHOLDER { p.reply && p.placeholder.put(part, len, a, b) }
                else { field < p.actions && p.labels[field as usize].put(part, len, a, b) }
            }
            ActionsMsg::Commit => {
                let Some(p) = slot.and_then(|i| self.pending[i].as_mut()).filter(|p| p.task == task && p.at.is_none()) else { return false };
                p.at = Some(now);
                true
            }
        }
    }

    /// A notification Bell forwarded from `sender`. When that program gave
    /// actions for it, a card is made: Some((tag, place in the queue)).
    pub fn notification(&mut self, sender: u32, text: &[u8], from: &[u8], now: u64) -> Option<(u32, usize)> {
        let i = self.pending.iter().position(|p| p.is_some_and(|p| p.pd == sender && p.at.is_some_and(|t| now.saturating_sub(t) <= PAIR_MS)))?;
        let p = self.pending[i].take()?;
        let view = CardView { actions: p.actions, reply: p.reply, from: Line::new(from), text: Line::new(text), labels: p.labels,
            placeholder: if p.reply && p.placeholder.is_empty() { Line::new(b"Reply") } else { p.placeholder } };
        let card = Card { pd: p.pd, task: p.task, tag: p.tag, view, until: 0 };
        let at = match self.cards.iter().position(|c| c.is_none()) {
            Some(at) => at,
            None => {
                // The oldest waiting card makes room; the one showing stays.
                if let Some(old) = self.cards[1] { self.tell(old.pd, old.task, Event::Gone { tag: old.tag, why: pt::GONE_REPLACED }); }
                self.cards.copy_within(2.., 1);
                CARD_QUEUE - 1
            }
        };
        self.cards[at] = Some(card);
        if at == 0 { self.show_next(now); }
        Some((p.tag, at))
    }

    fn show_next(&mut self, now: u64) {
        if let Some(c) = self.cards[0].as_mut() { c.until = now + CARD_MS; }
        self.reply = ReplyView { active: false, text: Line::EMPTY };
        self.mark(Section::Card);
        self.mark(Section::Reply);
    }

    /// The card showing goes; the next one shows.
    fn advance(&mut self, now: u64) {
        self.cards.copy_within(1.., 0);
        self.cards[CARD_QUEUE - 1] = None;
        self.show_next(now);
    }

    fn tell(&mut self, pd: u32, task: u32, event: Event) { self.queue(Out::Event { pd, task, event }); }
    fn queue(&mut self, out: Out) {
        // A full outbox loses the newest: programs that do not read their
        // events must not hold the Shell's memory.
        if let Some(slot) = self.outbox.iter_mut().find(|o| o.is_none()) { *slot = Some(out); }
    }
    /// The next thing to tell a program, kept until `told`.
    pub fn next_out(&self) -> Option<Out> { self.outbox[0] }
    pub fn told(&mut self) {
        self.outbox.copy_within(1.., 0);
        self.outbox[OUTBOX - 1] = None;
    }
    pub fn outbox_len(&self) -> usize { self.outbox.iter().flatten().count() }

    /// Time passing: action lists Bell never forwarded are forgotten, and a
    /// card nobody answers goes away. True when a card went.
    pub fn tick(&mut self, now: u64) -> Option<(u32, u32)> {
        for p in self.pending.iter_mut() {
            if p.is_some_and(|p| p.at.is_some_and(|t| now.saturating_sub(t) > PAIR_MS)) { *p = None; }
        }
        let c = self.cards[0]?;
        if c.until == 0 { self.show_next(now); return None; }
        if now < c.until || self.reply.active || !self.reply.text.is_empty() { return None; }
        self.tell(c.pd, c.task, Event::Gone { tag: c.tag, why: pt::GONE_EXPIRED });
        self.advance(now);
        Some((c.pd, c.tag))
    }

    /// Forgets everything of programs whose run is over (`alive(pd, task)` false).
    /// Returns how many items and cards went.
    pub fn retain(&mut self, alive: impl Fn(u32, u32) -> bool, now: u64) -> usize {
        let mut gone = 0;
        for i in 0..self.items.len() {
            if let Some(item) = self.items[i] {
                if !alive(item.pd, item.task) { self.items[i] = None; self.forget_item(item.pd, item.id); gone += 1; }
            }
        }
        for p in self.pending.iter_mut() { if p.is_some_and(|p| !alive(p.pd, p.task)) { *p = None; } }
        let showing = self.cards[0];
        let mut kept = [None; CARD_QUEUE];
        let mut n = 0;
        for c in self.cards.iter().flatten() {
            if alive(c.pd, c.task) { kept[n] = Some(*c); n += 1; } else { gone += 1; }
        }
        self.cards = kept;
        if self.cards[0] != showing { self.show_next(now); }
        self.outbox = { let mut o = [None; OUTBOX]; let mut k = 0; for e in self.outbox.iter().flatten() { if alive(e.pd(), e.task()) { o[k] = Some(*e); k += 1; } } o };
        gone
    }

    /// Items in the panel, nearest the status icons first: (index, place).
    pub fn placed(&self, s: &Screen) -> ([(usize, Rect); pt::MAX_ITEMS], usize) {
        let mut order = [(0usize, 0u32); pt::MAX_ITEMS];
        let mut n = 0;
        for (i, item) in self.items.iter().enumerate() {
            if let Some(item) = item.filter(|i| i.ready) { order[n] = (i, item.order); n += 1; }
        }
        order[..n].sort_unstable_by_key(|o| o.1);
        let mut out = [(0, (0, 0, 0, 0)); pt::MAX_ITEMS];
        let mut k = 0;
        for (slot, (i, _)) in order[..n].iter().enumerate() {
            let Some(r) = pt::icon_rect(slot, s.right, s.left, s.y, s.size, s.gap) else { break };
            out[k] = (*i, r); k += 1;
        }
        (out, k)
    }

    fn icon_at(&self, s: &Screen, x: usize, y: usize) -> Option<(usize, Rect)> {
        let (placed, n) = self.placed(s);
        placed[..n].iter().copied().find(|(_, r)| pt::inside(*r, x, y))
    }

    fn menu_layout(&self, s: &Screen) -> Option<(MenuLayout, Item)> {
        let (pd, id, _) = self.menu?;
        let item = self.items[self.find(pd, id)?]?;
        let (placed, n) = self.placed(s);
        let (_, r) = placed[..n].iter().copied().find(|(i, _)| self.items[*i].is_some_and(|i| i.pd == pd && i.id == id))?;
        Some((MenuLayout::new(r.0, s.bar + 2 * s.scale.max(1), item.rows as usize, s.width, s.height, s.scale)?, item))
    }

    fn card_layout(&self, s: &Screen) -> Option<CardLayout> {
        let c = self.cards[0]?;
        CardLayout::new(s.width, s.bar, s.scale, c.view.actions as usize, c.view.reply)
    }

    /// A left press at (x, y), before windows and the panel see it.
    pub fn press(&mut self, s: &Screen, x: usize, y: usize, now: u64) -> Press {
        if self.menu.is_some() {
            let Some((l, item)) = self.menu_layout(s) else { self.menu = None; self.mark(Section::Menu); return Press::MenuClosed };
            match l.hit(x, y) {
                Hit::Row(r) => {
                    self.menu = None; self.mark(Section::Menu);
                    self.tell(item.pd, item.task, Event::Menu { item: item.id, row: r as u8 });
                    return Press::Chose { pd: item.pd, id: item.id, row: r as u8 };
                }
                Hit::Inside => return Press::Taken,
                Hit::Outside => { self.menu = None; self.mark(Section::Menu); return Press::MenuClosed; }
            }
        }
        if let (Some(c), Some(l)) = (self.cards[0], self.card_layout(s)) {
            match l.hit(x, y) {
                CardHit::Close => {
                    self.tell(c.pd, c.task, Event::Gone { tag: c.tag, why: pt::GONE_CLOSED });
                    self.advance(now);
                    return Press::CardClosed { pd: c.pd, tag: c.tag };
                }
                CardHit::Action(a) => {
                    self.tell(c.pd, c.task, Event::Action { tag: c.tag, action: a });
                    self.advance(now);
                    return Press::CardAction { pd: c.pd, tag: c.tag, action: a };
                }
                CardHit::Field => {
                    if !self.reply.active { self.reply.active = true; self.mark(Section::Reply); }
                    return Press::ReplyFocused { pd: c.pd, tag: c.tag };
                }
                CardHit::Send => return match self.submit(now) { Some((pd, tag, len)) => Press::ReplySent { pd, tag, len }, None => Press::Taken },
                CardHit::Inside => return Press::Taken,
                CardHit::Outside => if self.reply.active { self.reply.active = false; self.mark(Section::Reply); },
            }
        }
        let Some((i, r)) = self.icon_at(s, x, y) else { return Press::Pass };
        let Some(item) = self.items[i] else { return Press::Pass };
        if item.rows == 0 {
            self.tell(item.pd, item.task, Event::Activated { item: item.id });
            return Press::Activated { pd: item.pd, id: item.id };
        }
        self.menu = Some((item.pd, item.id, None));
        self.hover = None;
        self.mark(Section::Menu);
        self.mark(Section::Tip);
        match self.menu_layout(s) {
            Some((l, _)) => Press::MenuOpened { pd: item.pd, id: item.id, rows: item.rows, panel: l.panel, row_h: l.row_h, pad: l.pad },
            None => { self.menu = None; let _ = r; Press::Taken }
        }
    }

    /// Sends the typed reply; the card goes. None when there is nothing to send.
    fn submit(&mut self, now: u64) -> Option<(u32, u32, usize)> {
        let c = self.cards[0]?;
        if !c.view.reply || self.reply.text.is_empty() { return None; }
        let text = self.reply.text;
        self.queue(Out::Reply { pd: c.pd, task: c.task, tag: c.tag, text });
        self.advance(now);
        Some((c.pd, c.tag, text.len as usize))
    }

    /// The pointer moved: the menu row and the item under it.
    pub fn pointer(&mut self, s: &Screen, x: usize, y: usize) {
        if let Some((pd, id, hover)) = self.menu {
            let row = self.menu_layout(s).and_then(|(l, _)| match l.hit(x, y) { Hit::Row(r) => Some(r as u8), _ => None });
            if row != hover { self.menu = Some((pd, id, row)); self.mark(Section::Menu); }
            return;
        }
        let over = self.icon_at(s, x, y).and_then(|(i, _)| self.items[i]).map(|i| (i.pd, i.id));
        if over != self.hover { self.hover = over; self.mark(Section::Tip); }
    }

    /// A key, before programs see it. Typing goes to the reply field while it
    /// is active; Esc closes an open menu. Modifiers and releases pass.
    pub fn key(&mut self, code: u8, value: u64, now: u64) -> Key {
        let down = value != 0;
        match code { 0x2A => self.shift[0] = down, 0x36 => self.shift[1] = down, _ => {} }
        if matches!(code, 0x2A | 0x36 | 0x1D | 0x38 | 0x3A | 0x5B | 0x5C | 0x61 | 0x64) || !down { return Key::Pass; }
        if self.menu.is_some() && code == 0x01 { self.menu = None; self.mark(Section::Menu); return Key::MenuClosed; }
        if !self.reply_active() { return Key::Pass; }
        match code {
            0x01 => { self.reply.active = false; self.mark(Section::Reply); Key::Left }
            0x1C | 0x60 => match self.submit(now) { Some((pd, tag, len)) => Key::Sent { pd, tag, len }, None => Key::Taken },
            0x0E => { if self.reply.text.pop() { self.mark(Section::Reply); } Key::Typed }
            _ => match pt::key_char(code, self.shift[0] || self.shift[1]) {
                Some(c) => { if self.reply.text.push(c) { self.mark(Section::Reply); } Key::Typed }
                None => Key::Taken,
            },
        }
    }

    /// The whole view for Display.
    pub fn view(&self, s: &Screen) -> View {
        let mut v = View::EMPTY;
        let (placed, n) = self.placed(s);
        for (k, (i, r)) in placed[..n].iter().enumerate() {
            let Some(item) = self.items[*i] else { continue };
            v.icons[k] = pt::Icon { x: r.0 as u16, y: r.1 as u16, size: r.2 as u16, label: item.label, label_len: item.label_len, colour: item.colour };
        }
        v.icon_count = n as u8;
        if let (Some((_, _, hover)), Some((l, item))) = (self.menu, self.menu_layout(s)) {
            v.menu = Some(MenuView { x: l.panel.0 as u16, y: l.panel.1 as u16, rows: item.rows, hover, text: item.text });
        }
        if let Some((pd, id)) = self.hover.filter(|_| self.menu.is_none()) {
            let found = placed[..n].iter().find(|(i, _)| self.items[*i].is_some_and(|i| i.pd == pd && i.id == id));
            if let Some((i, r)) = found {
                let item = self.items[*i].unwrap();
                if !item.tip.is_empty() { v.tip = Some(TipView { x: r.0 as u16, y: (s.bar + 4 * s.scale.max(1)) as u16, text: item.tip }); }
            }
        }
        v.card = self.cards[0].map(|c| c.view);
        v.reply = ReplyView { active: self.reply_active(), text: self.reply.text };
        v
    }
}
