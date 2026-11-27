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
    /// Continuous 3-finger horizontal swipe delta: `(dx_q12, dt_ms)` where
    /// `4096` equals one full screen-width slide (negative = fingers moved left).
    SwipeDelta(i32, u16),
    /// End of a continuous 3-finger horizontal swipe with release velocity (`q12/s`).
    SwipeEnd(i32),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwipeAxis { None, Horizontal, Vertical }

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
    swipe_axis: SwipeAxis,
    swipe_active: bool,
    swipe_last_ms: Option<u64>,
    swipe_rem: i64,
    swipe_vel_q12: i32,
    /// Sub-pixel pointer remainder (q8).
    rem: (i32, i32),
    button: Option<u8>,
}

const COORD_LIMIT: i32 = 1 << 20;

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
    // Coordinates come from the device and may be any i32: keep them to +-2^20 (well
    // past any real surface) so the arithmetic below cannot overflow.
    for c in f.down() {
        if n < out.len() {
            out[n] = *c;
            out[n].x = c.x.clamp(-COORD_LIMIT, COORD_LIMIT);
            out[n].y = c.y.clamp(-COORD_LIMIT, COORD_LIMIT);
            n += 1;
        }
    }
    out[..n].sort_unstable_by_key(|c| c.id);
    n
}

impl Recognizer {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            last: None,
            mode: Mode::Undecided,
            started_ms: None,
            most_fingers: 0,
            travel: 0,
            acc: (0, 0),
            spread_base: 0,
            swipe_done: false,
            swipe_axis: SwipeAxis::None,
            swipe_active: false,
            swipe_last_ms: None,
            swipe_rem: 0,
            swipe_vel_q12: 0,
            rem: (0, 0),
            button: None,
        }
    }

    fn finish_swipe(&mut self, now_ms: u64, emit: &mut impl FnMut(Event)) {
        if self.swipe_active {
            let stale = self
                .swipe_last_ms
                .is_some_and(|t| now_ms.saturating_sub(t) >= 50);
            let vel = if stale { 0 } else { self.swipe_vel_q12 };
            self.swipe_active = false;
            self.swipe_last_ms = None;
            self.swipe_rem = 0;
            self.swipe_vel_q12 = 0;
            emit(Event::SwipeEnd(vel));
        }
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
            self.finish_swipe(now_ms, &mut emit);
            if let Some(start) = self.started_ms.take() {
                let slop = if self.most_fingers >= 2 {
                    self.config.tap_slop.saturating_mul(2)
                } else {
                    self.config.tap_slop
                };
                let tap = now_ms.saturating_sub(start) <= self.config.tap_ms
                    && self.travel <= slop
                    && matches!(self.mode, Mode::Undecided | Mode::Pointer)
                    && !self.swipe_done
                    && !frame.button;
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
        if pn >= 3 && n < 3 {
            self.finish_swipe(now_ms, &mut emit);
        }
        let same = pn == n && prev.iter().zip(cur).all(|(a, b)| a.id == b.id);
        if n > self.most_fingers {
            self.most_fingers = n;
            // More fingers landed: the gesture starts over with them.
            if self.mode == Mode::Undecided {
                self.travel = 0;
            }
            if self.mode != Mode::Swipe {
                self.mode = Mode::Undecided;
                self.acc = (0, 0);
                self.swipe_axis = SwipeAxis::None;
                self.swipe_rem = 0;
                self.swipe_vel_q12 = 0;
            }
            self.spread_base = spread(cur);
        }
        if same {
            let (c0, c1) = (centroid(prev), centroid(cur));
            let (dx, dy) = (c1.0 - c0.0, c1.1 - c0.1);
            match n {
                1 => {
                    // During staggered multi-finger liftoff, ignore the last residual finger so
                    // it neither drifts the pointer nor spoils a 2/3-finger tap.
                    if self.most_fingers <= 1 {
                        self.travel = self.travel.saturating_add(dx.abs() + dy.abs());
                        if self.mode == Mode::Undecided && self.travel > self.config.tap_slop {
                            self.mode = Mode::Pointer;
                        }
                        if self.mode == Mode::Pointer || self.mode == Mode::Undecided {
                            self.pointer(dx, dy, &mut emit);
                        }
                    }
                }
                2 => {
                    if self.most_fingers <= 2 {
                        self.travel = self.travel.saturating_add(dx.abs() + dy.abs());
                        self.two_fingers(cur, dx, dy, &mut emit);
                    }
                }
                _ => {
                    self.travel = self.travel.saturating_add(dx.abs() + dy.abs());
                    self.three_fingers(dx, dy, now_ms, &mut emit);
                }
            }
        }
        self.last = Some(*frame);
    }

    fn pointer(&mut self, dx: i32, dy: i32, emit: &mut impl FnMut(Event)) {
        let q = self.config.pointer_q8 as i64;
        let (x, y) = (dx as i64 * q + self.rem.0 as i64, dy as i64 * q + self.rem.1 as i64);
        let (px, py) = (x / 256, y / 256);
        self.rem = ((x - px * 256) as i32, (y - py * 256) as i32);
        let clamp = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        if px != 0 || py != 0 { emit(Event::Move(clamp(px), clamp(py))); }
    }

    fn two_fingers(&mut self, cur: &[Contact], _dx: i32, dy: i32, emit: &mut impl FnMut(Event)) {
        let s = spread(cur);
        let ds = (s - self.spread_base).abs();
        if self.mode == Mode::Undecided || self.mode == Mode::Pointer {
            let change = ((s as i64 - self.spread_base as i64).abs() * 100 / self.spread_base.max(1) as i64).min(i32::MAX as i64) as i32;
            if change >= self.config.pinch_percent / 2
                && self.spread_base > self.config.tap_slop * 2
                && ds >= self.config.tap_slop / 2
                && ds >= dy.abs()
            {
                self.mode = Mode::Pinch;
            } else if self.travel > self.config.tap_slop {
                self.mode = Mode::Scroll;
            }
        }
        match self.mode {
            Mode::Scroll => {
                // Clear opposite-sign remainder immediately on direction reversal.
                if (self.acc.1 > 0 && dy < 0) || (self.acc.1 < 0 && dy > 0) {
                    self.acc.1 = 0;
                }
                self.acc.1 = self.acc.1.saturating_add(dy);
                let step = self.config.scroll_step;
                let steps = (self.acc.1 / step).clamp(-127, 127);
                if steps != 0 {
                    self.acc.1 -= steps * step;
                    // Fingers moving down pull the content down: the view goes up.
                    emit(Event::Scroll(steps as i8));
                }
            }
            Mode::Pinch => {
                let (s64, base) = (s as i64, self.spread_base.max(1) as i64);
                let p = self.config.pinch_percent as i64;
                if s64 * 100 >= base * (100 + p) { emit(Event::ZoomIn); self.spread_base = s; }
                else if s64 * 100 <= base * (100 - p) { emit(Event::ZoomOut); self.spread_base = s; }
            }
            _ => {}
        }
    }

    fn three_fingers(&mut self, dx: i32, dy: i32, now_ms: u64, emit: &mut impl FnMut(Event)) {
        self.acc.0 = self.acc.0.saturating_add(dx);
        self.acc.1 = self.acc.1.saturating_add(dy);
        let first_horizontal = if self.swipe_axis == SwipeAxis::None {
            if self.acc.0.abs() >= self.config.tap_slop || self.acc.1.abs() >= self.config.tap_slop {
                if self.acc.0.abs() >= self.acc.1.abs() {
                    self.swipe_axis = SwipeAxis::Horizontal;
                    true
                } else {
                    self.swipe_axis = SwipeAxis::Vertical;
                    false
                }
            } else {
                return;
            }
        } else {
            false
        };

        let d = self.config.swipe_distance;
        match self.swipe_axis {
            SwipeAxis::None => {}
            SwipeAxis::Vertical => {
                if !self.swipe_done {
                    if self.acc.1 <= -d {
                        self.swipe_done = true;
                        self.mode = Mode::Swipe;
                        emit(Event::SwipeUp);
                    } else if self.acc.1 >= d {
                        self.swipe_done = true;
                        self.mode = Mode::Swipe;
                        emit(Event::SwipeDown);
                    }
                }
            }
            SwipeAxis::Horizontal => {
                self.mode = Mode::Swipe;
                let step_dx = if first_horizontal { self.acc.0 } else { dx };
                let dt = self.swipe_last_ms.map_or(8, |t| now_ms.saturating_sub(t).clamp(1, 250)) as u16;
                let span = ((d.max(1) as i64 * 8) / 5).max(1);
                let total = (step_dx as i64) * 4096 + self.swipe_rem;
                let dx_q12 = (total / span) as i32;
                self.swipe_rem = total - (dx_q12 as i64) * span;
                if dx_q12 != 0 {
                    let inst_vel = ((dx_q12 as i64 * 1000) / (dt as i64).max(1)).clamp(-65536, 65536) as i32;
                    self.swipe_vel_q12 = if self.swipe_active {
                        (self.swipe_vel_q12 * 3 + inst_vel) / 4
                    } else {
                        inst_vel
                    };
                    self.swipe_active = true;
                    self.swipe_last_ms = Some(now_ms);
                    emit(Event::SwipeDelta(dx_q12, dt));
                } else if self.swipe_last_ms.is_some_and(|t| now_ms.saturating_sub(t) >= 35) {
                    self.swipe_vel_q12 = 0;
                }
                if !self.swipe_done {
                    if self.acc.0 <= -d {
                        self.swipe_done = true;
                        emit(Event::SwipeLeft);
                    } else if self.acc.0 >= d {
                        self.swipe_done = true;
                        emit(Event::SwipeRight);
                    }
                }
            }
        }
    }

    fn reset_touch(&mut self) {
        self.mode = Mode::Undecided;
        self.most_fingers = 0;
        self.travel = 0;
        self.acc = (0, 0);
        self.spread_base = 0;
        self.swipe_done = false;
        self.swipe_axis = SwipeAxis::None;
        self.swipe_active = false;
        self.swipe_last_ms = None;
        self.swipe_rem = 0;
        self.swipe_vel_q12 = 0;
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
        let discrete: Vec<Event> = ev.iter().copied().filter(|e| !matches!(e, Event::SwipeDelta(..) | Event::SwipeEnd(_))).collect();
        assert_eq!(discrete, vec![Event::SwipeLeft]);
        assert!(ev.iter().any(|e| matches!(e, Event::SwipeDelta(dx, _) if *dx < 0)));
        assert!(matches!(ev.last(), Some(Event::SwipeEnd(_))));

        let mut frames = vec![];
        for k in 0..20 { let y = -(k as i32) * 60; frames.push((1000 + k as u64 * 10, frame(&[(1, 1000, 1800 + y), (2, 1300, 1800 + y), (3, 1600, 1800 + y)], false))); }
        frames.push((1400, frame(&[], false)));
        let ev = run(&mut r, &frames);
        assert_eq!(ev, vec![Event::SwipeUp]);

        let mut frames = vec![];
        for k in 0..20 { let y = k as i32 * 60; frames.push((2000 + k as u64 * 10, frame(&[(1, 1000, 500 + y), (2, 1300, 500 + y), (3, 1600, 500 + y)], false))); }
        frames.push((2400, frame(&[], false)));
        let ev = run(&mut r, &frames);
        assert_eq!(ev, vec![Event::SwipeDown]);
    }

    #[test]
    fn continuous_horizontal_swipe_emits_deltas_and_velocity_on_liftoff() {
        let mut r = rec();
        let mut frames = vec![];
        // Rapid 3-finger flick right: total dx = +480 units in 80ms -> ~0.5 * Q12 with high +vel.
        for k in 0..9 {
            let x = k as i32 * 60;
            frames.push((k as u64 * 10, frame(&[(1, 800 + x, 1000), (2, 1100 + x, 1000), (3, 1400 + x, 1000)], false)));
        }
        // Staggered liftoff: drops to 2 fingers at 90ms (SwipeEnd must fire immediately!).
        frames.push((90, frame(&[(1, 1280, 1000), (2, 1580, 1000)], false)));
        frames.push((100, frame(&[], false)));
        let ev = run(&mut r, &frames);
        let sum_q12: i32 = ev.iter().map(|e| if let Event::SwipeDelta(dx, _) = e { *dx } else { 0 }).sum();
        // span = 600 * 8 / 5 = 960; 480 / 960 * 4096 == 2048.
        assert!((2040..=2056).contains(&sum_q12), "sum_q12={sum_q12}, ev={ev:?}");
        let end_vel = ev.iter().find_map(|e| if let Event::SwipeEnd(v) = e { Some(*v) } else { None });
        assert!(end_vel.is_some_and(|v| v > 2400), "end_vel={end_vel:?}");
        assert!(!ev.iter().any(|e| matches!(e, Event::Scroll(_) | Event::Move(..))));
    }

    #[test]
    fn staggered_two_finger_tap_emits_right_click_without_pointer_drift() {
        let mut r = rec();
        let ev = run(&mut r, &[
            (0, frame(&[(1, 1000, 1000)], false)),
            (8, frame(&[(1, 1015, 1005)], false)),
            (16, frame(&[(1, 1015, 1005), (2, 1400, 1005)], false)),
            (40, frame(&[(1, 1020, 1008), (2, 1408, 1012)], false)),
            // Finger 2 lifts early; remaining finger wobbles slightly before lifting completely.
            (50, frame(&[(1, 1025, 1012)], false)),
            (60, frame(&[(1, 1045, 1020)], false)),
            (70, frame(&[], false)),
        ]);
        let buttons: Vec<Event> = ev.iter().copied().filter(|e| matches!(e, Event::Button(..))).collect();
        assert_eq!(buttons, vec![Event::Button(2, true), Event::Button(2, false)], "{ev:?}");
    }

    #[test]
    fn two_finger_scroll_resists_false_pinch_and_reverses_cleanly() {
        let mut r = rec();
        let mut frames = vec![];
        // Fingers move vertically down while spreading by ~13% (typical finger roll) -> must still scroll!
        for k in 0..8 {
            frames.push((k * 10, frame(&[(1, 1000, 400 + k as i32 * 50), (2, 1400 + k as i32 * 6, 400 + k as i32 * 50)], false)));
        }
        // Sudden reversal back up should emit negative Scroll immediately.
        frames.push((90, frame(&[(1, 1000, 700), (2, 1442, 700)], false)));
        frames.push((100, frame(&[(1, 1000, 620), (2, 1442, 620)], false)));
        frames.push((120, frame(&[], false)));
        let ev = run(&mut r, &frames);
        assert!(ev.iter().any(|e| matches!(e, Event::Scroll(s) if *s > 0)), "{ev:?}");
        assert!(ev.iter().any(|e| matches!(e, Event::Scroll(s) if *s < 0)), "{ev:?}");
        assert!(!ev.iter().any(|e| matches!(e, Event::ZoomIn | Event::ZoomOut)), "{ev:?}");
    }

    #[test]
    fn a_landing_finger_does_not_jump_the_pointer() {
        let mut r = rec();
        let ev = run(&mut r, &[(0, frame(&[(1, 100, 100)], false)), (10, frame(&[(1, 200, 100)], false)), (20, frame(&[], false)),
                               (500, frame(&[(2, 2900, 1900)], false)), (510, frame(&[(2, 2900, 1900)], false))]);
        assert!(ev.iter().all(|e| match e { Event::Move(x, y) => x.abs() < 40 && y.abs() < 40, _ => true }), "{:?}", ev);
    }

    #[test]
    fn extreme_coordinates_do_not_overflow() {
        let mut r = rec();
        let (lo, hi) = (i32::MIN, i32::MAX);
        let _ = run(&mut r, &[(0, frame(&[(1, lo, lo)], false)), (5, frame(&[(1, hi, hi)], false)), (10, frame(&[(1, lo, hi)], false)),
            (15, frame(&[(1, lo, lo), (2, hi, hi)], false)), (20, frame(&[(1, hi, lo), (2, lo, hi)], false)),
            (25, frame(&[(1, hi, hi), (2, hi, hi), (3, hi, hi), (4, hi, hi), (5, hi, hi)], false)),
            (30, frame(&[(1, lo, lo), (2, lo, lo), (3, lo, lo), (4, lo, lo), (5, lo, lo)], false)), (300, frame(&[], false))]);
    }
}
