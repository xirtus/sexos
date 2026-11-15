// SPDX-License-Identifier: GPL-3.0-only
//! Painting Splicy's window: the player and the suggestion cards side by side on top,
//! the timeline strip under them, the transcript filling the rest, and a status line.
//! Cut words stay in the transcript, struck through and dimmed, so the editor sees what
//! was taken out and can select it again after an undo.

use crate::Editor;
use alloc::string::String;
use alloc::vec::Vec;
use sex_office::{Canvas, Pen};
use sex_splice::{Micros, TrackKind};

pub const BG: u32 = 0x00f6f5f3;
pub const INK: u32 = 0x00202124;
pub const DIM: u32 = 0x00a0a0a8;
pub const LINE: u32 = 0x00dadce0;
pub const ACCENT: u32 = 0x00c2185b;
pub const SELECT: u32 = 0x00f8d7e3;
pub const CLIP_A: u32 = 0x005c6bc0;
pub const CLIP_B: u32 = 0x007986cb;
pub const CARD: u32 = 0x00ffffff;
pub const STATUS_H: i32 = 26;
pub const STRIP_H: i32 = 44;
const PAD: i32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// Where each part of the window goes at `w` x `h`.
#[derive(Clone, Copy, Debug)]
pub struct Areas {
    pub player: Rect,
    pub cards: Rect,
    pub strip: Rect,
    pub text: Rect,
    pub status: Rect,
}

pub fn areas(w: i32, h: i32) -> Areas {
    let body = h - STATUS_H;
    let pw = (w * 3 / 5 - PAD * 2).max(64);
    let ph = (pw * 9 / 16).min(body * 2 / 5).max(36);
    let player = Rect { x: PAD, y: PAD, w: pw, h: ph };
    let cards = Rect { x: PAD * 2 + pw, y: PAD, w: (w - pw - PAD * 3).max(0), h: ph };
    let strip = Rect { x: PAD, y: PAD * 2 + ph, w: (w - PAD * 2).max(0), h: STRIP_H };
    let ty = strip.y + STRIP_H + PAD;
    let text = Rect { x: PAD, y: ty, w: (w - PAD * 2).max(0), h: (body - ty - PAD).max(0) };
    Areas { player, cards, strip, text, status: Rect { x: 0, y: body, w, h: STATUS_H } }
}

/// A word placed in the transcript pane, relative to the pane's top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub word: usize,
    pub x: i32,
    pub y: i32,
    pub w: i32,
}

/// Wrap words into lines `width` wide, `line_h` apart, a space's width between words.
pub fn layout(words: &[&str], width: i32, line_h: i32, mut measure: impl FnMut(&str) -> i32) -> Vec<Placed> {
    let space = measure(" ").max(1);
    let (mut x, mut y) = (0, 0);
    let mut out = Vec::with_capacity(words.len());
    for (i, word) in words.iter().enumerate() {
        let w = measure(word);
        if x > 0 && x + w > width {
            x = 0;
            y += line_h;
        }
        out.push(Placed { word: i, x, y, w });
        x += w + space;
    }
    out
}

/// The word under (`x`, `y`) in pane coordinates, or the nearest one on that line.
pub fn hit(placed: &[Placed], x: i32, y: i32, line_h: i32) -> Option<usize> {
    let line: Vec<&Placed> = placed.iter().filter(|p| y >= p.y && y < p.y + line_h).collect();
    line.iter().find(|p| x < p.x + p.w).or(line.last()).map(|p| p.word)
}

pub struct View {
    pen: Pen,
    small: Pen,
    /// How far the transcript is scrolled, in pixels.
    pub scroll: i32,
    /// The caret and anchor last brought into view, so a wheel scroll away from an
    /// unmoved caret stays where the editor put it.
    followed: Option<(usize, Option<usize>)>,
}

impl Default for View {
    fn default() -> Self {
        View { pen: Pen::sans(16), small: Pen::sans(12), scroll: 0, followed: None }
    }
}

/// A picture to show in the player: pixels, width, height.
pub type Picture<'a> = (&'a [u32], usize, usize);

impl View {
    pub fn line_height(&self) -> i32 {
        self.pen.line_height() + 6
    }

    pub fn layout(&mut self, ed: &Editor, width: i32) -> Vec<Placed> {
        let words: Vec<&str> = ed.transcript.words.iter().map(|w| w.text.as_str()).collect();
        let lh = self.line_height();
        let pen = &mut self.pen;
        layout(&words, width, lh, |s| pen.measure(s))
    }

    /// The word a click at window point (`x`, `y`) lands on.
    pub fn word_at(&mut self, ed: &Editor, w: i32, h: i32, x: i32, y: i32) -> Option<usize> {
        let a = areas(w, h).text;
        if !a.contains(x, y) {
            return None;
        }
        let placed = self.layout(ed, a.w);
        hit(&placed, x - a.x, y - a.y + self.scroll, self.line_height())
    }

    /// Paint the whole window into `px` (`w` x `h`, 0x00RRGGBB). `playhead` is a timeline
    /// time; `message` replaces the status line's key help when it is not empty.
    pub fn paint(&mut self, px: &mut [u32], w: usize, h: usize, ed: &Editor, picture: Option<Picture>, playhead: Micros, message: &str) {
        let (wi, hi) = (w as i32, h as i32);
        let a = areas(wi, hi);
        let mut c = Canvas::new(px, w, h);
        c.fill(0, 0, wi, hi, BG);
        self.player(&mut c, a.player, picture);
        self.cards(&mut c, a.cards, ed);
        self.strip(&mut c, a.strip, ed, playhead);
        self.transcript(&mut c, a.text, ed);
        c.fill(0, a.status.y, wi, STATUS_H, CARD);
        c.fill(0, a.status.y, wi, 1, LINE);
        let status = if message.is_empty() {
            String::from("Delete cuts the selection  Shift+arrows select  S sentence  \\ hear the join  C clean-up  Ctrl+Z/Y undo/redo  Ctrl+E export")
        } else {
            String::from(message)
        };
        let y = a.status.y + (STATUS_H - self.small.line_height()) / 2;
        self.small.text(&mut c, PAD, y, &status, INK, Some((0, wi - PAD)));
    }

    fn player(&mut self, c: &mut Canvas, r: Rect, picture: Option<Picture>) {
        c.fill(r.x, r.y, r.w, r.h, 0);
        let Some((pixels, pw, ph)) = picture else { return };
        if pw == 0 || ph == 0 || pixels.len() < pw * ph {
            return;
        }
        // Fit inside the box, keeping the shape, nearest pixel.
        let (sw, sh) = if r.w as usize * ph > r.h as usize * pw {
            ((r.h as usize * pw / ph) as i32, r.h)
        } else {
            (r.w, (r.w as usize * ph / pw) as i32)
        };
        let (ox, oy) = (r.x + (r.w - sw) / 2, r.y + (r.h - sh) / 2);
        for y in 0..sh {
            let sy = y as usize * ph / sh as usize;
            let (row, cy) = (sy * pw, oy + y);
            if cy < 0 || cy as usize >= c.height {
                continue;
            }
            for x in 0..sw {
                let cx = ox + x;
                if cx < 0 || cx as usize >= c.width {
                    continue;
                }
                c.px[cy as usize * c.stride + cx as usize] = pixels[row + x as usize * pw / sw as usize];
            }
        }
    }

    fn cards(&mut self, c: &mut Canvas, r: Rect, ed: &Editor) {
        if r.w < 40 {
            return;
        }
        let lh = self.small.line_height();
        self.small.text(c, r.x, r.y, "Suggestions", DIM, Some((r.x, r.w)));
        let pending = ed.pending();
        let mut y = r.y + lh + 6;
        if pending.is_empty() {
            self.small.text(c, r.x, y, "None waiting. C runs the clean-up pass.", DIM, Some((r.x, r.w)));
            return;
        }
        let card_h = lh * 2 + 12;
        for (n, id) in pending.iter().enumerate() {
            if y + card_h > r.y + r.h {
                break;
            }
            let Some(s) = ed.suggestions.get(*id) else { continue };
            c.fill(r.x, y, r.w, card_h, CARD);
            c.frame(r.x, y, r.w, card_h, if n == ed.card { ACCENT } else { LINE });
            self.small.text(c, r.x + 8, y + 5, &s.summary, INK, Some((r.x + 8, r.w - 16)));
            let hint = if n == ed.card { "Enter takes it, Esc skips, Tab next" } else { "Tab to focus" };
            self.small.text(c, r.x + 8, y + 7 + lh, hint, DIM, Some((r.x + 8, r.w - 16)));
            y += card_h + 6;
        }
    }

    fn strip(&mut self, c: &mut Canvas, r: Rect, ed: &Editor, playhead: Micros) {
        let tl = ed.timeline();
        let total = tl.duration().max(1);
        let x_of = |t: Micros| r.x + (t.clamp(0, total) as i128 * r.w as i128 / total as i128) as i32;
        c.fill(r.x, r.y, r.w, r.h, LINE);
        let rows: Vec<_> = tl.tracks.iter().collect();
        let row_h = if rows.is_empty() { r.h } else { (r.h - 2) / rows.len() as i32 };
        for (ti, track) in rows.iter().enumerate() {
            let y = r.y + 1 + ti as i32 * row_h;
            for (ci, clip) in track.clips.iter().enumerate() {
                let (x0, x1) = (x_of(clip.at), x_of(clip.end()));
                let colour = match (track.kind, ci % 2) {
                    (TrackKind::Video, 0) => CLIP_A,
                    (TrackKind::Video, _) => CLIP_B,
                    (_, 0) => 0x0026a69a,
                    _ => 0x004db6ac,
                };
                c.fill(x0, y + 1, (x1 - x0 - 1).max(1), row_h - 2, colour);
            }
        }
        for j in ed.joins() {
            c.fill(x_of(j) - 1, r.y, 2, r.h, ACCENT);
        }
        c.fill(x_of(playhead), r.y - 3, 1, r.h + 6, INK);
    }

    fn transcript(&mut self, c: &mut Canvas, r: Rect, ed: &Editor) {
        if ed.transcript.words.is_empty() {
            let msg = "No transcript. Put NAME.words beside the clip (START_MS END_MS WORD per line).";
            self.pen.text(c, r.x, r.y, msg, DIM, Some((r.x, r.w)));
            return;
        }
        let lh = self.line_height();
        let placed = self.layout(ed, r.w);
        // Bring the caret's line into view when the caret or selection moved.
        let now = Some((ed.caret, ed.anchor));
        if let Some(p) = placed.get(ed.caret).filter(|_| self.followed != now) {
            self.followed = now;
            if p.y < self.scroll {
                self.scroll = p.y;
            }
            if p.y + lh > self.scroll + r.h {
                self.scroll = p.y + lh - r.h;
            }
        }
        let playing = ed.playing();
        // Only a real selection is shaded; the caret alone is underlined.
        let sel = ed.anchor.and(ed.selection());
        let space = self.pen.measure(" ");
        for p in &placed {
            let y = r.y + p.y - self.scroll;
            if y + lh <= r.y || y >= r.y + r.h {
                continue;
            }
            let selected = sel.is_some_and(|(a, b)| p.word >= a && p.word <= b);
            if selected {
                let extend = if sel.is_some_and(|(_, b)| p.word < b) { space } else { 0 };
                c.fill(r.x + p.x - 2, y, p.w + 4 + extend, lh - 2, SELECT);
            }
            let colour = if playing[p.word] { INK } else { DIM };
            self.pen.text(c, r.x + p.x, y + 2, &ed.transcript.words[p.word].text, colour, Some((r.x, r.w)));
            if !playing[p.word] {
                c.fill(r.x + p.x, y + lh / 2, p.w, 1, DIM);
            }
            if p.word == ed.caret {
                c.fill(r.x + p.x, y + lh - 4, p.w.max(4), 2, ACCENT);
            }
        }
    }
}
