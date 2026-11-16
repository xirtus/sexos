// SPDX-License-Identifier: GPL-3.0-only
//! Desktop scripting (slice S4), the Shell's side of `silk`: what a request
//! means for the windows there are, the lines of an answer, the ring of
//! desktop events and the live feeds of them.
//!
//! Pure data with no IPC, so the host tests in `tests/silk_ctl.rs` run this
//! exact file. `silk_ctl_glue.rs` takes requests from Spindle, gives this
//! module a snapshot of the frames, carries out what it allows with the
//! Shell's own actions, and sends the lines (`sex_pdx::silk_ctl`).
//!
//! Events are found by comparing snapshots, once per main loop turn: a frame
//! that appears was opened, one that goes was closed, one whose workspace
//! changed was moved; the focused window, the shown workspace and each
//! workspace's layout are compared too. So every way of doing these (keys,
//! the pointer, the overview, `silk`) is seen the same way.

use core::fmt::Write;
use sex_pdx::silk_ctl::{Request, LAYOUTS, LINE_MAX};

/// A window as `silk windows` shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Win {
    pub frame: u32,
    /// The surface of the frame's shown tab.
    pub sid: u64,
    /// The Shell's scene (workspace - 1).
    pub scene: u8,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub floating: bool,
    pub zoomed: bool,
    pub minimized: bool,
    /// Tabs in the frame (a group has more than one) and the shown one, from 0.
    pub tabs: u8,
    pub tab: u8,
    pub title: [u8; 16],
    pub title_len: u8,
}

impl Win {
    pub const EMPTY: Win = Win { frame: 0, sid: 0, scene: 0, x: 0, y: 0, w: 0, h: 0, floating: false, zoomed: false, minimized: false,
        tabs: 1, tab: 0, title: [0; 16], title_len: 0 };
    pub fn state(&self) -> &'static str {
        if self.minimized { "hidden" } else if self.zoomed { "zoom" } else if self.floating { "float" } else { "tiled" }
    }
    pub fn title(&self) -> &[u8] { &self.title[..(self.title_len as usize).min(16)] }
}

/// One line of an answer.
pub struct Line { bytes: [u8; LINE_MAX], len: usize }
impl Line {
    pub const fn new() -> Self { Self { bytes: [0; LINE_MAX], len: 0 } }
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
    pub fn push(&mut self, s: &[u8]) {
        for &c in s {
            if self.len == LINE_MAX { return; }
            self.bytes[self.len] = if (32..=126).contains(&c) { c } else { b'?' };
            self.len += 1;
        }
    }
}
impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result { self.push(s.as_bytes()); Ok(()) }
}

/// `frame=12 sid=358 ws=3 x=0 y=48 w=640 h=752 state=tiled tab=1/1 focus=0 title=Abacus`
/// (the title last, as it may hold spaces; the tab shown counts from 1).
pub fn window_line(w: &Win, focused: bool) -> Line {
    let mut l = Line::new();
    let _ = write!(l, "frame={} sid={} ws={} x={} y={} w={} h={} state={} tab={}/{} focus={} title=",
        w.frame, w.sid, w.scene as u32 + 1, w.x, w.y, w.w, w.h, w.state(), w.tab as u32 + 1, w.tabs.max(1), focused as u8);
    if w.title_len == 0 { l.push(b"-"); } else { l.push(w.title()); }
    l
}

/// The last line of `silk windows`.
pub fn list_end(count: usize, scene: u8, focus_frame: u32) -> Line {
    let mut l = Line::new();
    let _ = write!(l, "ok windows={} ws={} focus={}", count, scene as u32 + 1, focus_frame);
    l
}

/// What the Shell does for a request, once `check` found it sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    List,
    /// Show `scene` (when it is not shown), bring the tab forward, focus `sid`.
    Focus { frame: u32, sid: u64, scene: u8 },
    /// Already there: nothing to do but say so.
    Stay { frame: u32, scene: u8 },
    Move { frame: u32, sid: u64, from: u8, to: u8 },
    Layout { scene: u8, layout: u8 },
    Float { frame: u32, sid: u64, on: bool, shown: bool },
    Close { frame: u32, sid: u64 },
    Events { since: u32 },
    Follow { count: u16 },
    Unfollow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal { NoWindow(u32), Hidden(u32) }
impl Refusal {
    pub fn line(self) -> Line {
        let mut l = Line::new();
        match self {
            Refusal::NoWindow(f) => { let _ = write!(l, "error no-window frame={}", f); }
            Refusal::Hidden(f) => { let _ = write!(l, "error hidden frame={} (a scratchpad window; Super+S shows it)", f); }
        }
        l
    }
    pub fn reason(self) -> &'static str { match self { Refusal::NoWindow(_) => "no_window", Refusal::Hidden(_) => "hidden" } }
}

/// What `req` means for these windows, with the Shell showing `active`.
pub fn check(req: Request, wins: &[Win], active: u8) -> Result<Act, Refusal> {
    let find = |frame: u32| wins.iter().find(|w| w.frame == frame).ok_or(Refusal::NoWindow(frame));
    Ok(match req {
        Request::List => Act::List,
        Request::Focus { frame } => {
            let w = find(frame)?;
            if w.minimized { return Err(Refusal::Hidden(frame)); }
            Act::Focus { frame, sid: w.sid, scene: w.scene }
        }
        Request::Move { frame, workspace } => {
            let w = find(frame)?;
            let to = workspace.saturating_sub(1);
            if w.scene == to { Act::Stay { frame, scene: to } } else { Act::Move { frame, sid: w.sid, from: w.scene, to } }
        }
        Request::Layout { workspace, layout } => Act::Layout { scene: workspace.saturating_sub(1), layout },
        Request::Float { frame } => {
            let w = find(frame)?;
            if w.minimized { return Err(Refusal::Hidden(frame)); }
            Act::Float { frame, sid: w.sid, on: !w.floating, shown: w.scene == active }
        }
        Request::Close { frame } => Act::Close { frame, sid: find(frame)?.sid },
        Request::Events { since } => Act::Events { since },
        Request::Follow { count } => Act::Follow { count },
        Request::Unfollow => Act::Unfollow,
    })
}

// ── Events ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Opened { frame: u32, sid: u64, scene: u8 },
    Closed { frame: u32, sid: u64 },
    Focused { frame: u32, sid: u64 },
    Moved { frame: u32, sid: u64, from: u8, to: u8 },
    Workspace { from: u8, to: u8 },
    Layout { scene: u8, layout: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event { pub seq: u32, pub kind: Kind }

impl Event {
    pub fn name(&self) -> &'static str {
        match self.kind {
            Kind::Opened { .. } => "opened",
            Kind::Closed { .. } => "closed",
            Kind::Focused { .. } => "focused",
            Kind::Moved { .. } => "moved",
            Kind::Workspace { .. } => "workspace",
            Kind::Layout { .. } => "layout",
        }
    }
    /// `event seq=7 kind=moved frame=12 sid=358 from=1 to=3` (workspaces from 1).
    pub fn line(&self) -> Line {
        let mut l = Line::new();
        let _ = write!(l, "event seq={} kind={}", self.seq, self.name());
        let ws = |s: u8| s as u32 + 1;
        let _ = match self.kind {
            Kind::Opened { frame, sid, scene } => write!(l, " frame={} sid={} ws={}", frame, sid, ws(scene)),
            Kind::Closed { frame, sid } | Kind::Focused { frame, sid } => write!(l, " frame={} sid={}", frame, sid),
            Kind::Moved { frame, sid, from, to } => write!(l, " frame={} sid={} from={} to={}", frame, sid, ws(from), ws(to)),
            Kind::Workspace { from, to } => write!(l, " from={} to={}", ws(from), ws(to)),
            Kind::Layout { scene, layout } => write!(l, " ws={} layout={}", ws(scene), LAYOUTS.get(layout as usize).copied().unwrap_or("?")),
        };
        l
    }
}

/// A frame as the event watch compares it: (frame, shown surface, scene).
pub type Seen = (u32, u64, u8);

/// The desktop as last seen and the last `R` events. `F` frames, `S` scenes.
pub struct Watch<const F: usize, const S: usize, const R: usize> {
    frames: [Option<Seen>; F],
    focus: u64,
    scene: u8,
    layouts: [u8; S],
    primed: bool,
    ring: [Option<Event>; R],
    next: u32,
}

impl<const F: usize, const S: usize, const R: usize> Watch<F, S, R> {
    pub const fn new() -> Self {
        Self { frames: [None; F], focus: 0, scene: 0, layouts: [0; S], primed: false, ring: [None; R], next: 1 }
    }
    fn push(&mut self, kind: Kind) {
        let seq = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.ring[seq as usize % R] = Some(Event { seq, kind });
    }
    /// The newest sequence number given (0 before any event).
    pub fn last(&self) -> u32 { self.next.wrapping_sub(1) }

    /// Compares the desktop now with the last look and keeps what changed.
    /// The first look only learns. `focus_frame` is the frame of `focus`.
    pub fn observe(&mut self, frames: &[Seen], focus: u64, focus_frame: u32, scene: u8, layouts: &[u8]) -> usize {
        let before = self.next;
        if self.primed {
            // Closed (or became another frame's tab) first, then moved, then
            // opened. A frame showing another tab is the same window.
            for i in 0..F {
                let Some(old) = self.frames[i] else { continue };
                if !frames.iter().any(|f| f.0 == old.0) { self.push(Kind::Closed { frame: old.0, sid: old.1 }); }
            }
            for f in frames {
                match self.frames.iter().flatten().find(|o| o.0 == f.0).copied() {
                    Some(old) if old.2 != f.2 => self.push(Kind::Moved { frame: f.0, sid: f.1, from: old.2, to: f.2 }),
                    Some(_) => {}
                    None => self.push(Kind::Opened { frame: f.0, sid: f.1, scene: f.2 }),
                }
            }
            if scene != self.scene { self.push(Kind::Workspace { from: self.scene, to: scene }); }
            for (s, &l) in layouts.iter().enumerate().take(S) {
                if l != self.layouts[s] { self.push(Kind::Layout { scene: s as u8, layout: l }); }
            }
            if focus != self.focus && focus != 0 { self.push(Kind::Focused { frame: focus_frame, sid: focus }); }
        }
        self.primed = true;
        self.frames = [None; F];
        for (slot, f) in self.frames.iter_mut().zip(frames) { *slot = Some(*f); }
        self.focus = focus;
        self.scene = scene;
        for (s, &l) in layouts.iter().enumerate().take(S) { self.layouts[s] = l; }
        self.next.wrapping_sub(before) as usize
    }

    /// The kept events newer than `since`, oldest first.
    pub fn since(&self, since: u32) -> impl Iterator<Item = Event> + '_ {
        let last = self.last();
        let first = last.saturating_sub(R as u32 - 1).max(since.saturating_add(1)).max(1);
        (first..=last).filter_map(move |seq| self.ring[seq as usize % R].filter(|e| e.seq == seq))
    }
}

/// Live event feeds: who asked, under which tag, how many are left and the
/// last event sent to them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feed { pub pd: u32, pub task: u32, pub tag: u16, pub left: u16, pub endless: bool, pub after: u32 }

pub struct Feeds<const N: usize> { feeds: [Option<Feed>; N] }

impl<const N: usize> Feeds<N> {
    pub const fn new() -> Self { Self { feeds: [None; N] } }
    /// A caller's feed from now on; its earlier feed (if any) is replaced and returned.
    /// None when every place is taken by other callers.
    pub fn start(&mut self, pd: u32, task: u32, tag: u16, count: u16, now: u32) -> Result<Option<Feed>, ()> {
        let feed = Feed { pd, task, tag, left: count, endless: count == 0, after: now };
        if let Some(f) = self.feeds.iter_mut().find(|f| f.is_some_and(|f| f.pd == pd)) { return Ok(f.replace(feed)); }
        let free = self.feeds.iter_mut().find(|f| f.is_none()).ok_or(())?;
        *free = Some(feed);
        Ok(None)
    }
    pub fn stop(&mut self, pd: u32) -> Option<Feed> { self.feeds.iter_mut().find(|f| f.is_some_and(|f| f.pd == pd))?.take() }
    /// Drops feeds whose caller's run ended.
    pub fn retain(&mut self, mut alive: impl FnMut(u32, u32) -> bool) -> usize {
        let mut gone = 0;
        for f in self.feeds.iter_mut() { if f.is_some_and(|x| !alive(x.pd, x.task)) { *f = None; gone += 1; } }
        gone
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Option<Feed>> { self.feeds.iter_mut() }
    pub fn any(&self) -> bool { self.feeds.iter().any(|f| f.is_some()) }
}

impl Feed {
    /// Takes one event for this feed; true when it was the last one it wanted.
    pub fn sent(&mut self, seq: u32) -> bool {
        self.after = seq;
        if self.endless { return false; }
        self.left = self.left.saturating_sub(1);
        self.left == 0
    }
}
