// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Touchpad gestures, from frames of contacts.
//!
//! One finger moves the pointer. Two fingers scroll (content follows the fingers, as on
//! Windows and macOS touchpads) or pinch to zoom. A tap clicks: one finger left, two right,
//! three middle. Pressing a clickpad clicks left, or right with two fingers down. Three or
//! more fingers swiping sideways change workspace; up opens the overview.
use crate::touch::{Contact, Frame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Pointer motion in screen pixels.
    Move(i32, i32),
    /// Button 1 left, 2 right, 3 middle; true when pressed.
    Button(u8, bool),
    /// Wheel steps, positive up (away from the user), like a mouse wheel.
    Scroll(i8),
    SwipeLeft,
    SwipeRight,
    SwipeUp,
    SwipeDown,
    ZoomIn,
    ZoomOut,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Pointer pixels per touchpad unit, times 256.
    pub pointer_q8: i32,
    /// Touchpad units of two-finger travel per wheel step.
    pub scroll_step: i32,
    /// A touch shorter than this, that moved less than `tap_slop`, is a tap.
    pub tap_ms: u64,
    pub tap_slop: i32,
    /// Three-finger travel that changes workspace.
    pub swipe_distance: i32,
    /// Finger spread change, in percent, for one zoom step.
    pub pinch_percent: i32,
}
impl Config {
    /// Defaults scaled to the pad: crossing its width moves the pointer ~700 pixels.
    pub fn for_extent(x_max: i32) -> Self {
        let w = x_max.max(64);
        Self { pointer_q8: (700 * 256 / w).max(1), scroll_step: (w / 40).max(1), tap_ms: 180, tap_slop: (w / 60).max(1), swipe_distance: w / 5, pinch_percent: 25 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode { Undecided, Pointer, Scroll, Pinch, Swipe }

pub struct Recognizer {
    pub config: Config,
    last: Option<Frame>,
    mode: Mode,
    /// The current touch (first finger down to last finger up).
    started_ms: Option<u64>,
    most_fingers: usize,
    travel: i32,
    /// Two-finger and swipe accumulators.
    acc: (i32, i32),
    spread_base: i32,
    swipe_done: bool,
    /// Sub-pixel pointer remainder (q8).
    rem: (i32, i32),
    button: Option<u8>,
}

fn centroid(cs: &[Contact]) -> (i32, i32) {
    let n = cs.len().max(1) as i32;
    (cs.iter().map(|c| c.x).sum::<i32>() / n, cs.iter().map(|c| c.y).sum::<i32>() / n)
}
fn spread(cs: &[Contact]) -> i32 {
    if cs.len() < 2 { return 0; }
    let (dx, dy) = ((cs[0].x - cs[1].x) as i64, (cs[0].y - cs[1].y) as i64);
    isqrt((dx * dx + dy * dy) as u64) as i32
}
fn isqrt(v: u64) -> u64 {
    if v < 2 { return v; }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + v / x) / 2; }
    x
}
fn down(f: &Frame, out: &mut [Contact; 5]) -> usize {
    let mut n = 0;
    for c in f.down() { if n < out.len() { out[n] = *c; n += 1; } }
    out[..n].sort_unstable_by_key(|c| c.id);
    n
}

impl Recognizer {
    pub fn new(config: Config) -> Self {
        Self { config, last: None, mode: Mode::Undecided, started_ms: None, most_fingers: 0, travel: 0, acc: (0, 0), spread_base: 0, swipe_done: false, rem: (0, 0), button: None }
    }

    pub fn feed(&mut self, frame: &Frame, now_ms: u64, mut emit: impl FnMut(Event)) {
        let mut cur = [Contact::default(); 5];
        let n = down(frame, &mut cur);
        let cur = &cur[..n];

        // Clickpad button.
        match (frame.button, self.button) {
            (true, None) => { let b = if n >= 2 { 2 } else { 1 }; self.button = Some(b); emit(Event::Button(b, true)); }
            (false, Some(b)) => { self.button = None; emit(Event::Button(b, false)); }
            _ => {}
        }

        if n == 0 {
            if let Some(start) = self.started_ms.take() {
                let tap = now_ms.saturating_sub(start) <= self.config.tap_ms && self.travel <= self.config.tap_slop
                    && matches!(self.mode, Mode::Undecided | Mode::Pointer) && !frame.button;
                if tap {
                    let b = match self.most_fingers { 1 => 1, 2 => 2, _ => 3 };
                    emit(Event::Button(b, true));
                    emit(Event::Button(b, false));
                }
            }
            self.reset_touch();
            self.last = Some(*frame);
            return;
        }
        if self.started_ms.is_none() { self.started_ms = Some(now_ms); }

        let mut prev = [Contact::default(); 5];
        let pn = self.last.as_ref().map_or(0, |l| down(l, &mut prev));
        let prev = &prev[..pn];
        let same = pn == n && prev.iter().zip(cur).all(|(a, b)| a.id == b.id);
        if n > self.most_fingers {
            self.most_fingers = n;
            // More fingers landed: the gesture starts over with them.
            if self.mode != Mode::Swipe { self.mode = Mode::Undecided; self.acc = (0, 0); }
            self.spread_base = spread(cur);
        }
        if same {
            let (c0, c1) = (centroid(prev), centroid(cur));
            let (dx, dy) = (c1.0 - c0.0, c1.1 - c0.1);
            self.travel = self.travel.saturating_add(dx.abs() + dy.abs());
            match n {
                1 => {
                    if self.mode == Mode::Undecided && self.travel > self.config.tap_slop { self.mode = Mode::Pointer; }
                    if self.mode == Mode::Pointer || self.mode == Mode::Undecided { self.pointer(dx, dy, &mut emit); }
                }
                2 => self.two_fingers(cur, dx, dy, &mut emit),
                _ => {
                    self.acc.0 += dx; self.acc.1 += dy;
                    if !self.swipe_done {
                        let d = self.config.swipe_distance;
                        let ev = if self.acc.0 <= -d { Some(Event::SwipeLeft) } else if self.acc.0 >= d { Some(Event::SwipeRight) }
                            else if self.acc.1 <= -d { Some(Event::SwipeUp) } else if self.acc.1 >= d { Some(Event::SwipeDown) } else { None };
                        if let Some(ev) = ev { self.swipe_done = true; self.mode = Mode::Swipe; emit(ev); }
                    }
                }
            }
        }
        self.last = Some(*frame);
    }

    fn pointer(&mut self, dx: i32, dy: i32, emit: &mut impl FnMut(Event)) {
        let q = self.config.pointer_q8;
        let (x, y) = (dx * q + self.rem.0, dy * q + self.rem.1);
        let (px, py) = (x / 256, y / 256);
        self.rem = (x - px * 256, y - py * 256);
        if px != 0 || py != 0 { emit(Event::Move(px, py)); }
    }

    fn two_fingers(&mut self, cur: &[Contact], dx: i32, dy: i32, emit: &mut impl FnMut(Event)) {
        let s = spread(cur);
        if self.mode == Mode::Undecided || self.mode == Mode::Pointer {
            let change = (s - self.spread_base).abs() * 100 / self.spread_base.max(1);
            if change >= self.config.pinch_percent / 2 && self.spread_base > self.config.tap_slop * 2 { self.mode = Mode::Pinch; }
            else if self.travel > self.config.tap_slop { self.mode = Mode::Scroll; }
        }
        match self.mode {
            Mode::Scroll => {
                self.acc.1 += dy;
                let step = self.config.scroll_step;
                let steps = (self.acc.1 / step).clamp(-127, 127);
                if steps != 0 {
                    self.acc.1 -= steps * step;
                    // Fingers moving down pull the content down: the view goes up.
                    emit(Event::Scroll(steps as i8));
                }
            }
            Mode::Pinch => {
                let base = self.spread_base.max(1);
                let p = self.config.pinch_percent;
                if s * 100 >= base * (100 + p) { emit(Event::ZoomIn); self.spread_base = s; }
                else if s * 100 <= base * (100 - p) { emit(Event::ZoomOut); self.spread_base = s; }
            }
            _ => {}
        }
    }

    fn reset_touch(&mut self) {
        self.mode = Mode::Undecided;
        self.most_fingers = 0;
        self.travel = 0;
        self.acc = (0, 0);
        self.spread_base = 0;
        self.swipe_done = false;
        self.rem = (0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(cs: &[(u8, i32, i32)], button: bool) -> Frame {
        let mut f = Frame { button, ..Default::default() };
        for (i, &(id, x, y)) in cs.iter().enumerate() { f.contacts[i] = Contact { id, x, y, down: true }; }
        f.count = cs.len();
        f
    }
    fn run(r: &mut Recognizer, frames: &[(u64, Frame)]) -> Vec<Event> {
        let mut out = Vec::new();
        for (t, f) in frames { r.feed(f, *t, |e| out.push(e)); }
        out
    }
    fn rec() -> Recognizer { Recognizer::new(Config::for_extent(3000)) }

    #[test]
    fn one_finger_moves_the_pointer() {
        let mut r = rec();
        let ev = run(&mut r, &[(0, frame(&[(1, 1000, 1000)], false)), (10, frame(&[(1, 1100, 1050)], false)), (20, frame(&[(1, 1200, 1100)], false)), (300, frame(&[], false))]);
        let (sx, sy) = ev.iter().fold((0, 0), |a, e| if let Event::Move(x, y) = e { (a.0 + x, a.1 + y) } else { a });
        // 200 units at 700 px per 3000 units: about 46 px, half that down.
        assert!((44..=47).contains(&sx) && (21..=24).contains(&sy), "{} {}", sx, sy);
        assert!(!ev.iter().any(|e| matches!(e, Event::Button(..))), "a drag is not a tap: {:?}", ev);
    }

    #[test]
    fn taps_click_by_finger_count() {
        let mut r = rec();
        let ev = run(&mut r, &[(0, frame(&[(1, 1000, 1000)], false)), (60, frame(&[(1, 1003, 1001)], false)), (100, frame(&[], false))]);
        assert_eq!(ev.iter().filter(|e| matches!(e, Event::Button(..))).collect::<Vec<_>>(), vec![&Event::Button(1, true), &Event::Button(1, false)]);
        let ev = run(&mut r, &[(1000, frame(&[(1, 1000, 1000), (2, 1400, 1000)], false)), (1080, frame(&[], false))]);
        assert_eq!(ev, vec![Event::Button(2, true), Event::Button(2, false)]);
        let ev = run(&mut r, &[(2000, frame(&[(1, 900, 1000), (2, 1300, 1000), (3, 1700, 1000)], false)), (2050, frame(&[], false))]);
        assert_eq!(ev, vec![Event::Button(3, true), Event::Button(3, false)]);
        // Held too long: no tap.
        let ev = run(&mut r, &[(3000, frame(&[(1, 1000, 1000)], false)), (3600, frame(&[], false))]);
        assert!(ev.is_empty());
    }

    #[test]
    fn clickpad_press_is_left_or_right() {
        let mut r = rec();
        let ev = run(&mut r, &[(0, frame(&[(1, 1000, 1800)], true)), (300, frame(&[(1, 1000, 1800)], false)), (320, frame(&[], false))]);
        assert_eq!(ev, vec![Event::Button(1, true), Event::Button(1, false)]);
        let ev = run(&mut r, &[(1000, frame(&[(1, 1000, 1800), (2, 1400, 1800)], true)), (1300, frame(&[(1, 1000, 1800), (2, 1400, 1800)], false)), (1320, frame(&[], false))]);
        assert_eq!(ev, vec![Event::Button(2, true), Event::Button(2, false)]);
    }

    #[test]
    fn two_fingers_scroll_with_the_content() {
        let mut r = rec();
        let mut frames = vec![];
        for k in 0..10 { frames.push((k * 10, frame(&[(1, 1000, 500 + k as i32 * 40), (2, 1400, 500 + k as i32 * 40)], false))); }
        frames.push((200, frame(&[], false)));
        let ev = run(&mut r, &frames);
        let steps: i32 = ev.iter().map(|e| if let Event::Scroll(s) = e { *s as i32 } else { 0 }).sum();
        // 360 units down at 75 units per step: 4 steps up.
        assert_eq!(steps, 4, "{:?}", ev);
        assert!(!ev.iter().any(|e| matches!(e, Event::Move(..) | Event::Button(..))));
    }

    #[test]
    fn pinch_zooms() {
        let mut r = rec();
        let mut frames = vec![];
        for k in 0..12 { let d = 300 + k * 40; frames.push((k as u64 * 10, frame(&[(1, 1500 - d / 2, 1000), (2, 1500 + d / 2, 1000)], false))); }
        let ev = run(&mut r, &frames);
        assert!(ev.iter().filter(|e| **e == Event::ZoomIn).count() >= 2, "{:?}", ev);
        assert!(!ev.iter().any(|e| matches!(e, Event::Scroll(_) | Event::ZoomOut)));
        let mut frames = vec![];
        for k in 0..12 { let d = 800 - k * 40; frames.push((1000 + k as u64 * 10, frame(&[(3, 1500 - d / 2, 1000), (4, 1500 + d / 2, 1000)], false))); }
        frames.push((2000, frame(&[], false)));
        let ev = run(&mut r, &frames);
        assert!(ev.iter().any(|e| *e == Event::ZoomOut), "{:?}", ev);
    }

    #[test]
    fn three_finger_swipes_once_per_gesture() {
        let mut r = rec();
        let mut frames = vec![];
        for k in 0..20 { let x = -(k as i32) * 60; frames.push((k as u64 * 10, frame(&[(1, 2000 + x, 1000), (2, 2300 + x, 1000), (3, 2600 + x, 1000)], false))); }
        frames.push((400, frame(&[], false)));
        let ev = run(&mut r, &frames);
        assert_eq!(ev, vec![Event::SwipeLeft]);
        let mut frames = vec![];
        for k in 0..20 { let y = -(k as i32) * 60; frames.push((1000 + k as u64 * 10, frame(&[(1, 1000, 1800 + y), (2, 1300, 1800 + y), (3, 1600, 1800 + y)], false))); }
        let ev = run(&mut r, &frames);
        assert_eq!(ev, vec![Event::SwipeUp]);
    }

    #[test]
    fn a_landing_finger_does_not_jump_the_pointer() {
        let mut r = rec();
        let ev = run(&mut r, &[(0, frame(&[(1, 100, 100)], false)), (10, frame(&[(1, 200, 100)], false)), (20, frame(&[], false)),
                               (500, frame(&[(2, 2900, 1900)], false)), (510, frame(&[(2, 2900, 1900)], false))]);
        assert!(ev.iter().all(|e| match e { Event::Move(x, y) => x.abs() < 40 && y.abs() < 40, _ => true }), "{:?}", ev);
    }
}
