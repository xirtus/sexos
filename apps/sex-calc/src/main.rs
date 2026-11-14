// SPDX-License-Identifier: GPL-3.0-only
//! Calculator: a keypad to click and a keyboard to type on, a scientific mode, degrees
//! or radians, a memory and the last twenty answers. The arithmetic is sex-calc-core's,
//! tested on the host; this file is the window around it.
//!
//! Keys: type an expression (`2(3+4)^2`, `sin(30)`, `5!`), Enter or `=` works it out,
//! Backspace takes a character back, Esc or Delete clears, Ctrl+S switches the
//! scientific keys, Ctrl+D degrees and radians, Up and Down pick a past answer and
//! Enter puts it in. A click on a past answer puts it in too.
#![no_std]
#![no_main]

extern crate alloc;

mod draw;

use alloc::string::String;
use draw::{clip, Canvas, Face};
use sex_app::{Event, Nav, Window};
use sex_calc_core::{format, Angle, Calculator, FORMAT_BYTES};
use sex_pdx::app_pointer::Kind;

const BACK: u32 = 0x0017_191E;
const PANEL: u32 = 0x000F_1014;
const KEY: u32 = 0x0027_2A31;
const KEY_OP: u32 = 0x0035_3A45;
const KEY_FN: u32 = 0x001F_2F3F;
const ACCENT: u32 = 0x0035_74E8;
const TEXT: u32 = 0x00E8_EAED;
const DIM: u32 = 0x0090_969E;
const ERROR: u32 = 0x00FF_6B6B;
const SELECT: u32 = 0x002C_3A55;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[calc.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, PartialEq)]
enum Act { Put(&'static str), Clear, Back, Equals, Negate, Recip, Square, MemClear, MemRecall, MemAdd, MemSub, Sci, Angle }

/// One key of the pad: its label, what it does and how many columns it takes.
type Pad = &'static [&'static [(&'static str, Act, u8)]];
const BASIC: Pad = &[
    &[("MC", Act::MemClear, 1), ("MR", Act::MemRecall, 1), ("M+", Act::MemAdd, 1), ("M-", Act::MemSub, 1), ("SCI", Act::Sci, 1)],
    &[("C", Act::Clear, 1), ("(", Act::Put("("), 1), (")", Act::Put(")"), 1), ("%", Act::Put("%"), 1), ("/", Act::Put("/"), 1)],
    &[("7", Act::Put("7"), 1), ("8", Act::Put("8"), 1), ("9", Act::Put("9"), 1), ("^", Act::Put("^"), 1), ("*", Act::Put("*"), 1)],
    &[("4", Act::Put("4"), 1), ("5", Act::Put("5"), 1), ("6", Act::Put("6"), 1), ("DEL", Act::Back, 1), ("-", Act::Put("-"), 1)],
    &[("1", Act::Put("1"), 1), ("2", Act::Put("2"), 1), ("3", Act::Put("3"), 1), ("ans", Act::Put("ans"), 1), ("+", Act::Put("+"), 1)],
    &[("0", Act::Put("0"), 1), (".", Act::Put("."), 1), ("+/-", Act::Negate, 1), ("=", Act::Equals, 2)],
];
const SCIENTIFIC: Pad = &[
    &[("sin", Act::Put("sin("), 1), ("cos", Act::Put("cos("), 1), ("tan", Act::Put("tan("), 1)],
    &[("asin", Act::Put("asin("), 1), ("acos", Act::Put("acos("), 1), ("atan", Act::Put("atan("), 1)],
    &[("sqrt", Act::Put("sqrt("), 1), ("ln", Act::Put("ln("), 1), ("log", Act::Put("log("), 1)],
    &[("exp", Act::Put("exp("), 1), ("abs", Act::Put("abs("), 1), ("n!", Act::Put("!"), 1)],
    &[("pi", Act::Put("pi"), 1), ("e", Act::Put("e"), 1), ("x^2", Act::Square, 1)],
    &[("DEG", Act::Angle, 1), ("1/x", Act::Recip, 1), ("mem", Act::MemRecall, 1)],
];

struct Calc {
    core: Calculator,
    expr: String,
    /// The line under the expression: the last answer or what went wrong.
    shown: String,
    error: bool,
    /// The expression on screen was just worked out: a digit starts afresh, an
    /// operator carries on from the answer.
    fresh: bool,
    scientific: bool,
    /// The past answer picked with Up and Down, 0 the newest.
    picked: Option<usize>,
}

fn record_value(v: f64) -> String {
    let mut out = [0u8; FORMAT_BYTES];
    String::from(format(v, &mut out))
}

impl Calc {
    fn put(&mut self, text: &str) {
        if self.fresh {
            let continues = text.starts_with(['+', '-', '*', '/', '^', '%', '!']);
            self.expr.clear();
            if continues { self.expr.push_str("ans"); }
            self.fresh = false;
        }
        if self.expr.len() + text.len() <= 160 { self.expr.push_str(text); }
        self.picked = None;
    }
    /// The expression in brackets with something around it, or the answer when empty.
    fn wrap(&mut self, before: &str, after: &str) {
        let inner = if self.expr.is_empty() || self.fresh { String::from("ans") } else { core::mem::take(&mut self.expr) };
        self.expr = alloc::format!("{before}({inner}){after}");
        self.fresh = false;
    }
    /// Works out what is typed (the answer itself when nothing is) for the memory keys.
    fn value_now(&mut self) -> Option<f64> {
        if self.expr.is_empty() { return Some(self.core.ctx.ans); }
        sex_calc_core::eval(&self.expr, &self.core.ctx).ok()
    }
    fn act(&mut self, act: Act) {
        match act {
            Act::Put(text) => self.put(text),
            Act::Clear => { self.expr.clear(); self.shown.clear(); self.error = false; self.fresh = false; self.picked = None; }
            Act::Back => { if self.fresh { self.fresh = false; } self.expr.pop(); }
            Act::Negate => self.wrap("-", ""),
            Act::Recip => self.wrap("1/", ""),
            Act::Square => self.wrap("", "^2"),
            Act::Equals => self.equals(),
            Act::MemClear => self.core.memory_clear(),
            Act::MemRecall => self.put("mem"),
            Act::MemAdd | Act::MemSub => if let Some(v) = self.value_now() {
                if act == Act::MemAdd { self.core.memory_add(v) } else { self.core.memory_subtract(v) }
                sex_pdx::serial_record!("[calc.memory] value={}", record_value(self.core.memory()));
            },
            Act::Sci => { self.scientific = !self.scientific; self.mode(); }
            Act::Angle => { self.core.toggle_angle(); self.mode(); }
        }
    }
    fn mode(&self) {
        sex_pdx::serial_record!("[calc.mode] sci={} angle={}", self.scientific as u8, if self.core.ctx.angle == Angle::Degrees { "deg" } else { "rad" });
    }
    fn equals(&mut self) {
        if self.expr.is_empty() { return; }
        match self.core.evaluate(&self.expr) {
            Ok(v) => {
                self.shown = record_value(v);
                self.error = false;
                self.fresh = true;
                sex_pdx::serial_record!("[calc.result] expr={} value={}", self.expr, self.shown);
            }
            Err(e) => {
                self.shown = String::from(e.message());
                self.error = true;
                sex_pdx::serial_record!("[calc.error] expr={} error={:?}", self.expr, e);
            }
        }
    }
    /// Puts past answer `i` into the expression.
    fn recall(&mut self, i: usize) {
        let Some(entry) = self.core.history(i) else { return };
        let value = record_value(entry.value);
        sex_pdx::serial_record!("[calc.recall] index={} value={}", i, value);
        // A negative answer goes in brackets so it stays one value in what follows.
        let text = if value.starts_with('-') { alloc::format!("({value})") } else { value };
        if self.fresh { self.expr.clear(); self.fresh = false; }
        self.expr.push_str(&text);
        self.picked = None;
    }
}

/// Where everything is for a window of this size.
struct Layout { w: i32, h: i32, display_h: i32, pad_x: i32, pad_w: i32, hist_x: i32 }

impl Layout {
    fn new((w, h): (usize, usize)) -> Layout {
        let (w, h) = (w.max(320) as i32, h.max(240) as i32);
        let hist_w = (w * 3 / 10).clamp(150, 280);
        Layout { w, h, display_h: 84, pad_x: 8, pad_w: w - hist_w - 16, hist_x: w - hist_w }
    }
    /// Each key's box: x, y, w, h, label, action.
    fn keys(&self, scientific: bool, mut f: impl FnMut(i32, i32, i32, i32, &'static str, Act, bool)) {
        let rows = BASIC.len() as i32;
        let top = self.display_h + 8;
        let row_h = (self.h - top - 8) / rows;
        let columns = 5 + if scientific { 3 } else { 0 };
        let col_w = self.pad_w / columns;
        let mut place = |pad: Pad, first: i32, function: bool| {
            for (r, row) in pad.iter().enumerate() {
                let mut c = first;
                for &(label, act, span) in row.iter() {
                    let span = span as i32;
                    f(self.pad_x + c * col_w + 2, top + r as i32 * row_h + 2, span * col_w - 4, row_h - 4, label, act, function);
                    c += span;
                }
            }
        };
        if scientific { place(SCIENTIFIC, 0, true); }
        place(BASIC, columns - 5, false);
    }
    /// The history rows: index i is drawn at y = hist_top + i * row.
    const HIST_TOP: i32 = 40;
    const HIST_ROW: i32 = 22;
}

fn render(window: &mut Window, calc: &Calc, small: &mut Face, big: &mut Face) {
    let (aw, ah) = window.size();
    let l = Layout::new((aw, ah));
    let (w, h) = (l.w as usize, l.h as usize);
    let mut c = Canvas { px: window.pixels(), w, h };
    c.fill(0, 0, l.w, l.h, BACK);
    // The display: modes on the left, expression and answer on the right.
    c.fill(8, 8, l.hist_x - 16, l.display_h - 8, PANEL);
    c.outline(8, 8, l.hist_x - 16, l.display_h - 8, 1, KEY);
    let angle = if calc.core.ctx.angle == Angle::Degrees { "DEG" } else { "RAD" };
    let mut x = c.text(small, 16, 14, angle, ACCENT, 60) + small.w as i32;
    if calc.scientific { x = c.text(small, x, 14, "SCI", ACCENT, 60) + small.w as i32; }
    if calc.core.memory() != 0.0 { c.text(small, x, 14, "M", ACCENT, 30); }
    let line_w = l.hist_x - 40;
    let chars = (line_w / small.w as i32) as usize;
    // The end of a long expression is what is being typed, so keep that in view.
    let mut typed = String::from(calc.expr.as_str());
    if !calc.fresh { typed.push('_'); }
    let skip = typed.chars().count().saturating_sub(chars);
    let tail: &str = typed.char_indices().nth(skip).map_or("", |(i, _)| &typed[i..]);
    let tw = (tail.chars().count() * small.w) as i32;
    c.text(small, l.hist_x - 16 - tw, 14, tail, if calc.fresh { DIM } else { TEXT }, line_w);
    let (face, colour) = if calc.error { (&mut *small, ERROR) } else { (&mut *big, TEXT) };
    let answer = clip(&calc.shown, (line_w / face.w as i32) as usize);
    let aw = (answer.chars().count() * face.w) as i32;
    c.text(face, l.hist_x - 16 - aw, l.display_h - 8 - face.h as i32, answer, colour, line_w);
    // The keypad.
    l.keys(calc.scientific, |x, y, w, h, label, act, function| {
        let fill = match act {
            Act::Equals => ACCENT,
            Act::Put(t) if !function && t.len() == 1 && !t.as_bytes()[0].is_ascii_digit() && t != "." => KEY_OP,
            _ if function => KEY_FN,
            Act::Put(_) => KEY,
            _ => KEY_OP,
        };
        c.fill(x, y, w, h, fill);
        let label = if act == Act::Angle { if calc.core.ctx.angle == Angle::Degrees { "RAD" } else { "DEG" } } else { label };
        c.centred(small, x, y, w, h, label, TEXT);
    });
    // The history, newest first.
    c.fill(l.hist_x, 0, l.w - l.hist_x, l.h, PANEL);
    c.text(small, l.hist_x + 10, 12, "History", DIM, l.w - l.hist_x);
    let cols = ((l.w - l.hist_x - 16) / small.w as i32) as usize;
    let rows = ((l.h - Layout::HIST_TOP) / Layout::HIST_ROW).max(0) as usize;
    for i in 0..calc.core.history_len().min(rows) {
        let Some(entry) = calc.core.history(i) else { break };
        let y = Layout::HIST_TOP + i as i32 * Layout::HIST_ROW;
        if calc.picked == Some(i) { c.fill(l.hist_x + 4, y - 2, l.w - l.hist_x - 8, Layout::HIST_ROW, SELECT); }
        let line = alloc::format!("{} = {}", entry.expr(), record_value(entry.value));
        c.text(small, l.hist_x + 8, y, clip(&line, cols), TEXT, l.w - l.hist_x - 12);
    }
    if calc.core.history_len() == 0 { c.text(small, l.hist_x + 10, Layout::HIST_TOP, "No answers yet", DIM, l.w - l.hist_x); }
    window.show_pixels(w, h);
}

/// What the key does, if anything.
fn key_action(calc: &mut Calc, key: sex_pdx::app_key::Key) {
    match key.code {
        0x1C => return match calc.picked { Some(i) => calc.recall(i), None => calc.act(Act::Equals) },
        0x0E => return calc.act(Act::Back),
        0x01 | 0x53 | 0x6F => return calc.act(Act::Clear),
        // Function keys belong to the desktop, so the modes are on Ctrl.
        0x1F if key.control => return calc.act(Act::Sci),
        0x20 if key.control => return calc.act(Act::Angle),
        _ => {}
    }
    match (sex_app::nav(key), sex_app::typed(key)) {
        (Some(Nav::Up), _) if calc.core.history_len() > 0 => calc.picked = Some(calc.picked.map_or(0, |i| (i + 1).min(calc.core.history_len() - 1))),
        (Some(Nav::Down), _) => calc.picked = calc.picked.and_then(|i| i.checked_sub(1)),
        (_, Some(b'=')) => calc.act(Act::Equals),
        (_, Some(c)) if c.is_ascii_alphanumeric() || b"+-*/^%!().".contains(&c) => {
            let s = [c];
            calc.put(core::str::from_utf8(&s).unwrap_or(""));
        }
        _ => {}
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Calculator");
    let mut small = Face::new(10, 20);
    let mut big = Face::new(18, 34);
    let mut calc = Calc { core: Calculator::new(), expr: String::new(), shown: String::new(), error: false, fresh: false, scientific: false, picked: None };
    render(&mut window, &calc, &mut small, &mut big);
    sex_pdx::serial_record!("[calc.ready] target={} task={}", window.target, window.task);
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[calc.closed] target={} task={}", window.target, window.task); window.close() }
            Event::Key(key) => key_action(&mut calc, key),
            Event::Pointer(p) if matches!(p.kind, Kind::Down { button: 0 }) => {
                let l = Layout::new(window.size());
                let (px, py) = (p.x as i32, p.y as i32);
                let mut hit = None;
                l.keys(calc.scientific, |x, y, w, h, label, act, _| {
                    if px >= x && px < x + w && py >= y && py < y + h { hit = Some((label, act)); }
                });
                if let Some((label, act)) = hit {
                    sex_pdx::serial_record!("[calc.key] label={}", label);
                    calc.act(act);
                } else if px >= l.hist_x && py >= Layout::HIST_TOP - 2 {
                    let i = ((py - Layout::HIST_TOP + 2) / Layout::HIST_ROW) as usize;
                    calc.recall(i);
                } else { continue; }
            }
            Event::Pointer(_) => continue,
            Event::Resize { .. } => {}
        }
        render(&mut window, &calc, &mut small, &mut big);
    }
}
