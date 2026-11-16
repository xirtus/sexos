// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Desktop scripting (slice S4): the `silk` command, like `hyprctl`. Spindle
//! (or a program a Spindle terminal started attached to it) asks the Shell to
//! list, focus, move, float or close windows, to set a workspace's layout, and
//! for the desktop's recent events or a live feed of them.
//!
//! Wires:
//! - `OP_SILK_CTL`, Spindle -> Shell: one [`Request`] in three words. The
//!   Shell owes no reply word (the caller's reply slot stays free) and the
//!   caller never waits for one.
//! - `OP_SILK_OUT`, Shell -> Spindle: the answer as text lines, each sent as
//!   pieces of sixteen bytes in plain calls the Shell does not wait on. The
//!   answer's last piece carries `END` and whether the request was done. A
//!   live event feed (`Follow`) sends one line per event and ends when its
//!   count is reached or the caller sends `Unfollow`.
//!
//! Lines are printable ASCII, at most [`LINE_MAX`] bytes, as `key=value`
//! words a script can split on spaces. Workspaces are numbered from 1 as the
//! panel shows them (the Shell's scene 0 is workspace 1).
//!
//! Self-contained (no other module) so host tests compile it alone.

/// Spindle -> Shell: a request.
pub const OP_SILK_CTL: u64 = 0x2EE;
/// Shell -> Spindle: a piece of the answer.
pub const OP_SILK_OUT: u64 = 0x2EF;

/// The longest line of an answer.
pub const LINE_MAX: usize = 104;
pub const PIECE: usize = 16;
/// Workspaces the panel shows.
pub const WORKSPACES: u8 = 9;
/// Layout names, by `wm::Layout::index`.
pub const LAYOUTS: [&str; 4] = ["master", "dwindle", "monocle", "grid"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Every window: frame, surface, workspace, geometry, state, tabs, title.
    List,
    /// Show the window's workspace and focus it.
    Focus { frame: u32 },
    /// Send the window to a workspace (1-based); the view stays where it is.
    Move { frame: u32, workspace: u8 },
    /// A workspace's layout, by index into [`LAYOUTS`].
    Layout { workspace: u8, layout: u8 },
    /// Floating on or off.
    Float { frame: u32 },
    /// Ask the window's program to close, as its close button does.
    Close { frame: u32 },
    /// The events kept after sequence number `since` (0: all kept).
    Events { since: u32 },
    /// Events as they happen, until `count` were sent (0: until `Unfollow`).
    Follow { count: u16 },
    /// Stop a feed.
    Unfollow,
}

const K_LIST: u64 = 1;
const K_FOCUS: u64 = 2;
const K_MOVE: u64 = 3;
const K_LAYOUT: u64 = 4;
const K_FLOAT: u64 = 5;
const K_CLOSE: u64 = 6;
const K_EVENTS: u64 = 7;
const K_FOLLOW: u64 = 8;
const K_UNFOLLOW: u64 = 9;

impl Request {
    /// `tag` names the answer, so a late answer to an earlier request is told apart.
    pub fn wire(self, tag: u16) -> [u64; 3] {
        let (kind, a, b) = match self {
            Request::List => (K_LIST, 0, 0),
            Request::Focus { frame } => (K_FOCUS, frame as u64, 0),
            Request::Move { frame, workspace } => (K_MOVE, frame as u64, workspace as u64),
            Request::Layout { workspace, layout } => (K_LAYOUT, workspace as u64, layout as u64),
            Request::Float { frame } => (K_FLOAT, frame as u64, 0),
            Request::Close { frame } => (K_CLOSE, frame as u64, 0),
            Request::Events { since } => (K_EVENTS, since as u64, 0),
            Request::Follow { count } => (K_FOLLOW, count as u64, 0),
            Request::Unfollow => (K_UNFOLLOW, 0, 0),
        };
        [kind | (tag as u64) << 8, a, b]
    }

    /// The tag and request, or None for a word no request makes. Values are
    /// checked for range here; whether the window exists is the Shell's to say.
    pub fn decode(w: [u64; 3]) -> Option<(u16, Request)> {
        if w[0] >> 24 != 0 { return None; }
        let tag = (w[0] >> 8) as u16;
        let frame = u32::try_from(w[1]).ok();
        let ws = |v: u64| (1..=WORKSPACES as u64).contains(&v).then_some(v as u8);
        let r = match w[0] & 0xFF {
            K_LIST if w[1] == 0 && w[2] == 0 => Request::List,
            K_FOCUS if w[2] == 0 => Request::Focus { frame: frame? },
            K_MOVE => Request::Move { frame: frame?, workspace: ws(w[2])? },
            K_LAYOUT if (w[2] as usize) < LAYOUTS.len() => Request::Layout { workspace: ws(w[1])?, layout: w[2] as u8 },
            K_FLOAT if w[2] == 0 => Request::Float { frame: frame? },
            K_CLOSE if w[2] == 0 => Request::Close { frame: frame? },
            K_EVENTS if w[2] == 0 => Request::Events { since: frame? },
            K_FOLLOW if w[1] <= u16::MAX as u64 && w[2] == 0 => Request::Follow { count: w[1] as u16 },
            K_UNFOLLOW if w[1] == 0 && w[2] == 0 => Request::Unfollow,
            _ => return None,
        };
        Some((tag, r))
    }

    pub fn name(self) -> &'static str {
        match self {
            Request::List => "windows",
            Request::Focus { .. } => "focus",
            Request::Move { .. } => "move",
            Request::Layout { .. } => "layout",
            Request::Float { .. } => "float",
            Request::Close { .. } => "close",
            Request::Events { .. } => "events",
            Request::Follow { .. } => "follow",
            Request::Unfollow => "unfollow",
        }
    }
}

/// How `silk` is used, one line per form.
pub const USAGE: [&str; 9] = [
    "usage: silk windows",
    "       silk focus FRAME",
    "       silk move FRAME WORKSPACE        (workspace 1-9)",
    "       silk layout WORKSPACE master|dwindle|monocle|grid",
    "       silk float FRAME",
    "       silk close FRAME",
    "       silk events [since SEQ]",
    "       silk events follow [COUNT]       (Esc stops)",
    "FRAME is the frame= number silk windows prints.",
];

fn number(w: &[u8]) -> Option<u32> {
    if w.is_empty() || w.len() > 10 || !w.iter().all(u8::is_ascii_digit) { return None; }
    let mut n = 0u64;
    for &c in w { n = n * 10 + (c - b'0') as u64; }
    u32::try_from(n).ok()
}

fn workspace(w: &[u8]) -> Result<u8, &'static str> {
    match number(w) {
        Some(n) if (1..=WORKSPACES as u32).contains(&n) => Ok(n as u8),
        _ => Err("silk: workspace is a number from 1 to 9"),
    }
}

fn frame(w: Option<&[u8]>) -> Result<u32, &'static str> {
    w.and_then(number).ok_or("silk: FRAME is the frame= number silk windows prints")
}

/// The words after `silk`, as a request; the message to print otherwise
/// (`Err("")` asks for the usage lines).
pub fn parse(args: &[u8]) -> Result<Request, &'static str> {
    let mut words = args.split(|c| *c == b' ' || *c == b'\t').filter(|w| !w.is_empty());
    let cmd = words.next().unwrap_or(b"");
    let (a, b, c) = (words.next(), words.next(), words.next());
    let r = match cmd {
        b"windows" | b"list" | b"clients" if a.is_none() => Request::List,
        b"focus" if b.is_none() => Request::Focus { frame: frame(a)? },
        b"move" if c.is_none() => Request::Move { frame: frame(a)?, workspace: workspace(b.ok_or("silk: move FRAME WORKSPACE")?)? },
        b"layout" if c.is_none() => {
            let ws = workspace(a.ok_or("silk: layout WORKSPACE NAME")?)?;
            let name = b.ok_or("silk: layout WORKSPACE master|dwindle|monocle|grid")?;
            if name.starts_with(b"plugin:") { return Err("silk: layout plugins are not in this build (docs/handoff S4)"); }
            let layout = LAYOUTS.iter().position(|l| l.as_bytes() == name).ok_or("silk: layouts are master, dwindle, monocle and grid")?;
            Request::Layout { workspace: ws, layout: layout as u8 }
        }
        b"float" if b.is_none() => Request::Float { frame: frame(a)? },
        b"close" if b.is_none() => Request::Close { frame: frame(a)? },
        b"events" => match (a, b, c) {
            (None, _, _) => Request::Events { since: 0 },
            (Some(b"since"), Some(n), None) => Request::Events { since: number(n).ok_or("silk: events since SEQ")? },
            (Some(b"follow"), None, _) => Request::Follow { count: 0 },
            (Some(b"follow"), Some(n), None) => match number(n) {
                Some(n) if n <= u16::MAX as u32 => Request::Follow { count: n as u16 },
                _ => return Err("silk: events follow COUNT (up to 65535)"),
            },
            _ => return Err(""),
        },
        _ => return Err(""),
    };
    Ok(r)
}

// ── Shell -> Spindle: the answer in pieces ──────────────────────────────────

/// In a piece's header: this piece ends its line.
pub const LINE_END: u8 = 1;
/// This piece ends the answer.
pub const END: u8 = 2;
/// With `END`: the request was refused or failed.
pub const FAILED: u8 = 4;

/// One piece: header (tag, bytes in it, flags) and sixteen bytes of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece { pub tag: u16, pub len: u8, pub flags: u8, pub bytes: [u8; PIECE] }

impl Piece {
    pub fn wire(&self) -> [u64; 3] {
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        a.copy_from_slice(&self.bytes[..8]);
        b.copy_from_slice(&self.bytes[8..]);
        [self.tag as u64 | (self.len as u64) << 16 | (self.flags as u64) << 24, u64::from_le_bytes(a), u64::from_le_bytes(b)]
    }
    pub fn decode(w: [u64; 3]) -> Option<Piece> {
        let (len, flags) = ((w[0] >> 16) as u8, (w[0] >> 24) as u8);
        if w[0] >> 32 != 0 || len as usize > PIECE || flags & !(LINE_END | END | FAILED) != 0 { return None; }
        if flags & FAILED != 0 && flags & END == 0 { return None; }
        let mut bytes = [0u8; PIECE];
        bytes[..8].copy_from_slice(&w[1].to_le_bytes());
        bytes[8..].copy_from_slice(&w[2].to_le_bytes());
        for c in bytes[..len as usize].iter_mut() { if !(32..=126).contains(c) { *c = b'?'; } }
        Some(Piece { tag: w[0] as u16, len, flags, bytes })
    }
    pub fn text(&self) -> &[u8] { &self.bytes[..self.len as usize] }
}

/// The pieces of one line. `last` ends the answer, `failed` with it says
/// the request was not done. A line longer than [`LINE_MAX`] is cut.
pub fn pieces(tag: u16, line: &[u8], last: bool, failed: bool) -> impl Iterator<Item = Piece> + '_ {
    let line = &line[..line.len().min(LINE_MAX)];
    let count = line.len().div_ceil(PIECE).max(1);
    (0..count).map(move |i| {
        let s = &line[(i * PIECE).min(line.len())..((i + 1) * PIECE).min(line.len())];
        let mut bytes = [0u8; PIECE];
        for (d, &c) in bytes.iter_mut().zip(s) { *d = if (32..=126).contains(&c) { c } else { b'?' }; }
        let end = i + 1 == count;
        let mut flags = if end { LINE_END } else { 0 };
        if end && last { flags |= END; if failed { flags |= FAILED; } }
        Piece { tag, len: s.len() as u8, flags, bytes }
    })
}

/// What a piece completed on the receiving side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Got { Nothing, Line, LineAndEnd { failed: bool } }

/// Puts the pieces of the expected answer back into lines.
pub struct Assembler { pub tag: u16, line: [u8; LINE_MAX], len: usize }

impl Assembler {
    pub const fn new() -> Self { Self { tag: 0, line: [0; LINE_MAX], len: 0 } }
    /// Expects the answer tagged `tag` and forgets any half line.
    pub fn expect(&mut self, tag: u16) { self.tag = tag; self.len = 0; }
    /// A piece; pieces of another answer are ignored.
    pub fn take(&mut self, p: &Piece) -> Got {
        if p.tag != self.tag { return Got::Nothing; }
        for &c in p.text() { if self.len < LINE_MAX { self.line[self.len] = c; self.len += 1; } }
        if p.flags & LINE_END == 0 { return Got::Nothing; }
        if p.flags & END != 0 { Got::LineAndEnd { failed: p.flags & FAILED != 0 } } else { Got::Line }
    }
    /// The line just completed; call once after `take` said so.
    pub fn line(&mut self) -> &[u8] { let n = core::mem::take(&mut self.len); &self.line[..n] }
}
