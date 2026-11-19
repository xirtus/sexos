// SPDX-License-Identifier: GPL-3.0-only
//! Painting Splicy's window in its Editing layout, dark like the editors people know:
//! a toolbar, the transcript with its suggestion cards on the left, the viewer with
//! timecode on the right, and across the bottom the timeline with track headers, a
//! timecode ruler, coloured clips carrying a filmstrip (pictures) or a waveform (sound),
//! the joins left by cuts, and the playhead. Cut words stay in the transcript, struck
//! through and dimmed, so the editor sees what was taken out.

use crate::media::Library;
use crate::vu::{VuStrip, VU_STRIP_H};
use crate::Editor;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_office::{Canvas, Pen};
use sex_splice::{Clip, Micros, Timeline, TrackKind, SECOND};

pub const BG: u32 = 0x001c1c1f;
pub const PANEL: u32 = 0x00252529;
pub const PANEL2: u32 = 0x002f2f35;
pub const LINE: u32 = 0x003a3a42;
pub const INK: u32 = 0x00e8e8ec;
pub const DIM: u32 = 0x008a8a96;
pub const ACCENT: u32 = 0x00ff2d7a;
pub const SELECT: u32 = 0x005a2440;
pub const PLAYHEAD: u32 = 0x00ffa000;
pub const CARD: u32 = 0x00323238;
/// Picture tracks, then sound tracks, take these colours in turn.
pub const VIDEO_COLOURS: [u32; 3] = [0x003949ab, 0x005e35b1, 0x008e24aa];
pub const SOUND_COLOURS: [u32; 3] = [0x0000796b, 0x002e7d32, 0x00ad1457];
pub const TOOLBAR_H: i32 = 34;
pub const STATUS_H: i32 = 24;
pub const HEAD_W: i32 = 140;
pub const RULER_H: i32 = 24;
const PAD: i32 = 8;

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
    pub toolbar: Rect,
    /// The transcript, above the cards.
    pub text: Rect,
    pub cards: Rect,
    /// The picture, above the transport bar.
    pub player: Rect,
    pub transport: Rect,
    /// Track headers down the left of the timeline.
    pub heads: Rect,
    pub ruler: Rect,
    /// Where the clips are drawn.
    pub tracks: Rect,
    /// Bottom VU meter strip when shown.
    pub vu: Rect,
    pub status: Rect,
}

pub fn areas_with_vu(w: i32, h: i32, show_vu: bool) -> Areas {
    let body_y = TOOLBAR_H;
    let body_h = (h - TOOLBAR_H - STATUS_H).max(0);
    let top_h = body_h * 11 / 20;
    let left_w = (w * 2 / 5).max(0);
    let cards_h = (top_h / 3).min(120);
    let text = Rect { x: PAD, y: body_y + PAD, w: (left_w - PAD * 2).max(0), h: (top_h - cards_h - PAD * 2).max(0) };
    let cards = Rect { x: PAD, y: text.y + text.h + PAD / 2, w: text.w, h: cards_h };
    let transport_h = 28;
    let player = Rect { x: left_w + PAD, y: body_y + PAD, w: (w - left_w - PAD * 2).max(0), h: (top_h - transport_h - PAD).max(0) };
    let transport = Rect { x: player.x, y: player.y + player.h, w: player.w, h: transport_h };
    let tl_y = body_y + top_h + 1;
    let total_tl_h = (body_h - top_h - 1).max(0);
    let (tl_h, vu) = if show_vu {
        let vu_h = VU_STRIP_H.min(total_tl_h);
        let tl_h = total_tl_h - vu_h;
        let vu_rect = Rect { x: 0, y: tl_y + tl_h, w, h: vu_h };
        (tl_h, vu_rect)
    } else {
        (total_tl_h, Rect { x: 0, y: h - STATUS_H, w: 0, h: 0 })
    };
    let heads = Rect { x: 0, y: tl_y + RULER_H, w: HEAD_W.min(w), h: (tl_h - RULER_H).max(0) };
    let ruler = Rect { x: heads.w, y: tl_y, w: (w - heads.w).max(0), h: RULER_H.min(tl_h) };
    let tracks = Rect { x: heads.w, y: heads.y, w: ruler.w, h: heads.h };
    Areas {
        toolbar: Rect { x: 0, y: 0, w, h: TOOLBAR_H },
        text,
        cards,
        player,
        transport,
        heads,
        ruler,
        tracks,
        vu,
        status: Rect { x: 0, y: h - STATUS_H, w, h: STATUS_H },
    }
}

pub fn areas(w: i32, h: i32) -> Areas {
    areas_with_vu(w, h, false)
}

/// `t` as HH:MM:SS:FF at `fps`.
pub fn timecode(t: Micros, fps: (u32, u32)) -> String {
    let t = t.max(0);
    let (num, den) = (fps.0.max(1) as i128, fps.1.max(1) as i128);
    // Non-drop-frame: count frames, then split by the nominal rate (30 for 29.97), so
    // the label always names the frame the edit and the export are on.
    let frames = t as i128 * num / (den * SECOND as i128);
    let nominal = ((num + den - 1) / den).max(1);
    let (secs, ff) = (frames / nominal, frames % nominal);
    format!("{:02}:{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60, ff)
}

/// The ruler's label spacing: the shortest step of at least `min_px` pixels.
pub fn ruler_step(us_per_px: Micros, min_px: i64) -> Micros {
    const STEPS: [i64; 12] = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600];
    let want = us_per_px.max(1) * min_px;
    STEPS.iter().map(|s| s * SECOND).find(|s| *s >= want).unwrap_or(3600 * SECOND)
}

/// How tall each track's row is when `rows` share `height`.
pub fn row_height(kind: TrackKind, rows: usize, height: i32) -> i32 {
    let want = if kind == TrackKind::Video { 84 } else { 60 };
    let fit = if rows == 0 { want } else { height / rows as i32 };
    want.min(fit).max(18)
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

/// The last part of a path, for a clip's label.
fn short(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path).trim_start_matches('@')
}

pub struct View {
    pen: Pen,
    small: Pen,
    big: Pen,
    /// How far the transcript is scrolled, in pixels.
    pub scroll: i32,
    /// The caret and anchor last brought into view, so a wheel scroll away from an
    /// unmoved caret stays where the editor put it.
    followed: Option<(usize, Option<usize>)>,
    /// Timeline zoom in microseconds per pixel; 0 fits the whole edit.
    pub zoom: Micros,
    /// The timeline time at the left edge of the tracks.
    pub start: Micros,
    /// Bottom-strip VU meter.
    pub vu: VuStrip,
}

impl Default for View {
    fn default() -> Self {
        View {
            pen: Pen::sans(16),
            small: Pen::sans(12),
            big: Pen::mono(18),
            scroll: 0,
            followed: None,
            zoom: 0,
            start: 0,
            vu: VuStrip::default(),
        }
    }
}

/// A picture to show in the viewer: pixels, width, height.
pub type Picture<'a> = (&'a [u32], usize, usize);

/// Copy `pic` into `r` scaled to cover its width (cropped to `clip`), nearest pixel.
fn blit(c: &mut Canvas, r: Rect, clip: Rect, pic: Picture) {
    let (px, pw, ph) = pic;
    if pw == 0 || ph == 0 || r.w <= 0 || r.h <= 0 || px.len() < pw * ph {
        return;
    }
    for y in r.y.max(clip.y).max(0)..(r.y + r.h).min(clip.y + clip.h).min(c.height as i32) {
        let sy = ((y - r.y) as usize * ph / r.h as usize).min(ph - 1);
        for x in r.x.max(clip.x).max(0)..(r.x + r.w).min(clip.x + clip.w).min(c.width as i32) {
            let sx = ((x - r.x) as usize * pw / r.w as usize).min(pw - 1);
            c.px[y as usize * c.stride + x as usize] = px[sy * pw + sx];
        }
    }
}

/// Mix `over` onto `under` by `alpha` of 255.
fn shade(under: u32, over: u32, alpha: u32) -> u32 {
    sex_office::mix(under, over, alpha)
}

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

    /// Microseconds per pixel on the tracks for `tl` at width `w`.
    pub fn us_per_px(&self, tl: &Timeline, w: i32) -> Micros {
        if self.zoom > 0 {
            return self.zoom;
        }
        let width = (w - HEAD_W).max(1) as Micros;
        // Fit the edit with a little room after its end.
        ((tl.duration().max(SECOND) * 21 / 20) + width - 1) / width
    }

    /// The timeline time under window point (`x`, `y`) when it is on the ruler or the
    /// tracks: a click there moves the playhead.
    pub fn time_at(&self, tl: &Timeline, w: i32, h: i32, x: i32, y: i32) -> Option<Micros> {
        let a = areas(w, h);
        if !(a.ruler.contains(x, y) || a.tracks.contains(x, y)) {
            return None;
        }
        Some((self.start + (x - a.tracks.x) as Micros * self.us_per_px(tl, w)).clamp(0, tl.duration()))
    }

    /// Zoom in (`steps` > 0) or out around the playhead.
    pub fn zoom_by(&mut self, tl: &Timeline, w: i32, steps: i32, playhead: Micros) {
        let now = self.us_per_px(tl, w);
        let next = if steps > 0 { now / 2 } else { now * 2 };
        let fit = { View { zoom: 0, ..View::default() } }.us_per_px(tl, w);
        self.zoom = next.clamp(1000, fit.max(1000));
        if self.zoom >= fit - fit / 8 {
            self.zoom = 0;
            self.start = 0;
        } else {
            let half = (w - HEAD_W) as Micros * self.zoom / 2;
            self.start = (playhead - half).max(0);
        }
    }

    /// Paint the whole window into `px` (`w` x `h`, 0x00RRGGBB). `playhead` is a timeline
    /// time; `message` replaces the status line's key help when it is not empty.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, px: &mut [u32], w: usize, h: usize, ed: &Editor, lib: &Library, picture: Option<Picture>, playhead: Micros, message: &str) {
        let (wi, hi) = (w as i32, h as i32);
        let a = areas_with_vu(wi, hi, self.vu.visible);
        let mut c = Canvas::new(px, w, h);
        c.fill(0, 0, wi, hi, BG);
        self.toolbar(&mut c, a.toolbar, ed, playhead);
        c.fill(a.text.x - PAD / 2, a.text.y - PAD / 2, a.text.w + PAD, a.text.h + a.cards.h + PAD * 2, PANEL);
        self.transcript(&mut c, a.text, ed);
        self.cards(&mut c, a.cards, ed);
        self.player(&mut c, a.player, picture);
        self.transport(&mut c, a.transport, ed, playhead);
        self.timeline(&mut c, &a, ed, lib, playhead);
        if self.vu.visible {
            self.vu.update_from_library(ed.timeline(), lib, playhead, 50_000);
            self.vu.paint(&mut c, a.vu, &mut self.small);
        }
        c.fill(0, a.status.y, wi, STATUS_H, PANEL);
        c.fill(0, a.status.y, wi, 1, LINE);
        let status = if message.is_empty() {
            String::from("Delete cuts  Shift+arrows select  S sentence  C clean-up  \\ hear the join  =/- zoom  Ctrl+Z/Y undo/redo  Ctrl+E export")
        } else {
            String::from(message)
        };
        let y = a.status.y + (STATUS_H - self.small.line_height()) / 2;
        self.small.text(&mut c, PAD, y, &status, DIM, Some((0, wi - PAD)));
    }

    fn toolbar(&mut self, c: &mut Canvas, r: Rect, ed: &Editor, playhead: Micros) {
        c.fill(r.x, r.y, r.w, r.h, PANEL);
        c.fill(r.x, r.y + r.h - 1, r.w, 1, LINE);
        let y = r.y + (r.h - self.pen.line_height()) / 2;
        let x = self.pen.text(c, PAD + 4, y, "Splicy", ACCENT, None);
        let tl = ed.timeline();
        let name = tl.media.first().map_or("", |m| short(&m.path));
        let x = self.small.text(c, x + 16, y + 3, name, INK, Some((0, r.w / 2)));
        // The workspace tabs: Editing is the one there is so far.
        let tab = "Editing";
        let tw = self.small.measure(tab) + 16;
        let tx = (r.w - tw) / 2;
        if tx > x + 8 {
            c.fill(tx, r.y + 6, tw, r.h - 12, PANEL2);
            c.fill(tx, r.y + r.h - 8, tw, 2, ACCENT);
            self.small.text(c, tx + 8, y + 3, tab, INK, None);
        }
        let tc = timecode(playhead, (tl.fps_num, tl.fps_den));
        let fw = self.small.measure(&tc);
        self.small.text(c, r.w - fw - PAD * 2, y + 3, &tc, DIM, None);
    }

    fn player(&mut self, c: &mut Canvas, r: Rect, picture: Option<Picture>) {
        c.fill(r.x, r.y, r.w, r.h, 0);
        let Some((pixels, pw, ph)) = picture else { return };
        if pw == 0 || ph == 0 || pixels.len() < pw * ph {
            return;
        }
        // Fit inside the box, keeping the shape.
        let (sw, sh) = if r.w as usize * ph > r.h as usize * pw {
            ((r.h as usize * pw / ph) as i32, r.h)
        } else {
            (r.w, (r.w as usize * ph / pw) as i32)
        };
        let fit = Rect { x: r.x + (r.w - sw) / 2, y: r.y + (r.h - sh) / 2, w: sw, h: sh };
        blit(c, fit, r, (pixels, pw, ph));
    }

    fn transport(&mut self, c: &mut Canvas, r: Rect, ed: &Editor, playhead: Micros) {
        c.fill(r.x, r.y, r.w, r.h, PANEL);
        let tl = ed.timeline();
        let fps = (tl.fps_num, tl.fps_den);
        let now = timecode(playhead, fps);
        let total = timecode(tl.duration(), fps);
        let y = r.y + (r.h - self.big.line_height()) / 2 + 1;
        let w = self.big.measure(&now);
        let x = r.x + (r.w - w) / 2;
        self.big.text(c, x, y, &now, INK, Some((r.x, r.w)));
        let tw = self.small.measure(&total);
        self.small.text(c, r.x + r.w - tw - PAD, y + 4, &total, DIM, Some((r.x, r.w)));
        // A play mark left of the timecode.
        for i in 0..10 {
            c.fill(x - 26 + i / 2, r.y + 9 + i / 2, 1, 10 - (i / 2) * 2, INK);
        }
    }

    fn cards(&mut self, c: &mut Canvas, r: Rect, ed: &Editor) {
        if r.w < 40 || r.h < 20 {
            return;
        }
        let lh = self.small.line_height();
        let pending = ed.pending();
        c.fill(r.x, r.y, r.w, 1, LINE);
        let title = if pending.is_empty() { String::from("Suggestions: none waiting. C runs the clean-up pass.") } else { format!("Suggestions ({})", pending.len()) };
        self.small.text(c, r.x, r.y + 4, &title, DIM, Some((r.x, r.w)));
        let mut y = r.y + lh + 8;
        let card_h = lh * 2 + 10;
        for (n, id) in pending.iter().enumerate() {
            if y + card_h > r.y + r.h {
                break;
            }
            let Some(s) = ed.suggestions.get(*id) else { continue };
            c.fill(r.x, y, r.w, card_h, CARD);
            c.fill(r.x, y, 3, card_h, if n == ed.card { ACCENT } else { LINE });
            self.small.text(c, r.x + 10, y + 4, &s.summary, INK, Some((r.x + 10, r.w - 16)));
            let hint = if n == ed.card { "Enter takes it, Esc skips, Tab next" } else { "Tab to focus" };
            self.small.text(c, r.x + 10, y + 6 + lh, hint, DIM, Some((r.x + 10, r.w - 16)));
            y += card_h + 4;
        }
    }

    fn transcript(&mut self, c: &mut Canvas, r: Rect, ed: &Editor) {
        if ed.transcript.words.is_empty() {
            let msg = "No transcript. Put NAME.words beside the clip (START_MS END_MS WORD per line).";
            self.small.text(c, r.x, r.y, msg, DIM, Some((r.x, r.w)));
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
            if y < r.y || y + lh > r.y + r.h {
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

    fn timeline(&mut self, c: &mut Canvas, a: &Areas, ed: &Editor, lib: &Library, playhead: Micros) {
        let tl = ed.timeline();
        let width = a.ruler.x + a.ruler.w;
        let upp = self.us_per_px(tl, width);
        // Keep the playhead on screen.
        let span = a.tracks.w.max(1) as Micros * upp;
        if self.zoom == 0 {
            self.start = 0;
        } else if playhead < self.start || playhead >= self.start + span {
            self.start = (playhead - span / 4).max(0);
        }
        let start = self.start;
        let x_of = |t: Micros| a.tracks.x + ((t - start) / upp) as i32;
        c.fill(a.heads.x, a.ruler.y, a.heads.w, a.ruler.h, PANEL);
        c.fill(a.heads.x, a.heads.y, a.heads.w, a.heads.h, PANEL);
        c.fill(a.tracks.x, a.tracks.y, a.tracks.w, a.tracks.h, BG);
        c.fill(0, a.ruler.y - 1, width, 1, LINE);
        c.fill(a.heads.w - 1, a.ruler.y, 1, a.ruler.h + a.heads.h, LINE);

        // The ruler: a label every `step`, minor ticks between.
        c.fill(a.ruler.x, a.ruler.y, a.ruler.w, a.ruler.h, PANEL2);
        c.fill(a.ruler.x, a.ruler.y + a.ruler.h - 1, a.ruler.w, 1, LINE);
        let step = ruler_step(upp, 110);
        let minor = step / 5;
        let mut t = start - start % minor.max(1);
        let end = start + span;
        while t <= end {
            let x = x_of(t);
            if x >= a.ruler.x {
                if t % step == 0 {
                    c.fill(x, a.ruler.y + 8, 1, a.ruler.h - 8, DIM);
                    let label = timecode(t, (tl.fps_num, tl.fps_den));
                    self.small.text(c, x + 4, a.ruler.y + 3, &label, DIM, Some((a.ruler.x, a.ruler.w)));
                } else {
                    c.fill(x, a.ruler.y + a.ruler.h - 6, 1, 5, LINE);
                }
            }
            t += minor.max(1);
        }

        // Tracks: pictures on top as editors expect (V2 above V1), then sound.
        let mut order: Vec<usize> = (0..tl.tracks.len()).filter(|i| tl.tracks[*i].kind == TrackKind::Video).rev().collect();
        order.extend((0..tl.tracks.len()).filter(|i| tl.tracks[*i].kind == TrackKind::Audio));
        let rows = order.len();
        let mut y = a.tracks.y + 4;
        let (mut nv, mut na) = (0usize, 0usize);
        let video_count = tl.tracks.iter().filter(|t| t.kind == TrackKind::Video).count();
        for &ti in &order {
            let track = &tl.tracks[ti];
            let rh = row_height(track.kind, rows, a.tracks.h - 8);
            let (label, colour) = if track.kind == TrackKind::Video {
                nv += 1;
                (format!("V{}", video_count + 1 - nv), VIDEO_COLOURS[(video_count - nv) % 3])
            } else {
                na += 1;
                (format!("A{na}"), SOUND_COLOURS[(na - 1) % 3])
            };
            // Header: the track's name, a coloured tab, and its switches.
            c.fill(a.heads.x, y, a.heads.w - 1, rh - 2, PANEL2);
            c.fill(a.heads.x, y, 4, rh - 2, colour);
            self.small.text(c, a.heads.x + 10, y + 4, &label, INK, None);
            self.small.text(c, a.heads.x + 34, y + 4, &track.name, DIM, Some((a.heads.x + 34, a.heads.w - 40)));
            self.switches(c, a.heads.x + 10, y + rh - 22, track.kind, track.muted);
            // Clips.
            for clip in &track.clips {
                let (x0, x1) = (x_of(clip.at), x_of(clip.end()));
                if x1 < a.tracks.x || x0 > a.tracks.x + a.tracks.w {
                    continue;
                }
                let r = Rect { x: x0, y: y + 1, w: (x1 - x0 - 1).max(2), h: rh - 4 };
                let vis = Rect { x: a.tracks.x, y: r.y, w: a.tracks.w, h: r.h };
                let name = tl.media(clip.media).map_or("", |m| short(&m.path));
                match track.kind {
                    TrackKind::Video => self.video_clip(c, r, vis, clip, colour, lib, upp, start, a.tracks.x, name),
                    TrackKind::Audio => self.sound_clip(c, r, vis, clip, colour, lib, upp, start, a.tracks.x, name),
                }
            }
            c.fill(a.tracks.x, y + rh - 2, a.tracks.w, 1, PANEL);
            y += rh;
        }
        // Joins left by cuts, then the playhead over everything.
        for j in ed.joins() {
            let x = x_of(j);
            if x >= a.tracks.x {
                c.fill(x - 1, a.ruler.y + a.ruler.h - 6, 3, 6, ACCENT);
                c.fill(x - 1, a.tracks.y, 2, (y - a.tracks.y).max(0), ACCENT);
            }
        }
        let px = x_of(playhead);
        if px >= a.tracks.x && px < a.tracks.x + a.tracks.w {
            c.fill(px, a.ruler.y, 2, a.ruler.h + a.tracks.h, PLAYHEAD);
            for i in 0..6 {
                c.fill(px - 5 + i, a.ruler.y + i, 11 - 2 * i, 1, PLAYHEAD);
            }
        }
    }

    /// A track's switches: an eye for pictures, M (mute) and S (solo) for sound.
    fn switches(&mut self, c: &mut Canvas, x: i32, y: i32, kind: TrackKind, muted: bool) {
        let boxes: &[&str] = if kind == TrackKind::Video { &["eye"] } else { &["M", "S"] };
        for (i, b) in boxes.iter().enumerate() {
            let bx = x + i as i32 * 24;
            let on = *b == "M" && muted;
            c.fill(bx, y, 20, 16, if on { ACCENT } else { BG });
            c.frame(bx, y, 20, 16, LINE);
            if *b == "eye" {
                c.fill(bx + 4, y + 7, 12, 2, DIM);
                c.fill(bx + 7, y + 5, 6, 6, DIM);
                c.fill(bx + 9, y + 7, 2, 2, BG);
            } else {
                self.small.text(c, bx + 6, y + 1, b, if on { INK } else { DIM }, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn video_clip(&mut self, c: &mut Canvas, r: Rect, vis: Rect, clip: &Clip, colour: u32, lib: &Library, upp: Micros, start: Micros, x0: i32, name: &str) {
        c.fill(r.x.max(vis.x), r.y, (r.x + r.w).min(vis.x + vis.w) - r.x.max(vis.x), r.h, colour);
        let label_h = 15;
        let strip = Rect { x: r.x, y: r.y + label_h, w: r.w, h: r.h - label_h - 1 };
        // The filmstrip: pictures side by side, each from the source time under it.
        if strip.h > 6 {
            let tw = (strip.h * 16 / 9).max(8);
            let clip_vis = Rect { x: r.x.max(vis.x), y: strip.y, w: (r.x + r.w).min(vis.x + vis.w) - r.x.max(vis.x), h: strip.h };
            let mut x = r.x + 1;
            while x < r.x + r.w - 1 {
                if x + tw >= vis.x && x <= vis.x + vis.w {
                    let t = start + (x - x0) as Micros * upp;
                    if let Some(th) = lib.thumb(clip.media, clip.source_at(t.clamp(clip.at, clip.end() - 1))) {
                        let cell = Rect { x, y: strip.y, w: tw, h: strip.h };
                        let inside = Rect { x: clip_vis.x.max(cell.x), y: cell.y, w: (clip_vis.x + clip_vis.w).min(r.x + r.w - 1) - clip_vis.x.max(cell.x), h: cell.h };
                        blit(c, cell, inside, (&th.px, th.w, th.h));
                        c.fill(x + tw - 1, strip.y, 1, strip.h, shade(colour, 0, 120));
                    }
                }
                x += tw;
            }
        }
        c.fill(r.x.max(vis.x), r.y, (r.x + r.w).min(vis.x + vis.w) - r.x.max(vis.x), label_h, shade(colour, 0, 70));
        self.small.text(c, r.x + 5, r.y + 1, name, INK, Some((r.x.max(vis.x) + 2, r.w - 8)));
        c.frame(r.x, r.y, r.w, r.h, shade(colour, 0xffffff, 90));
    }

    #[allow(clippy::too_many_arguments)]
    fn sound_clip(&mut self, c: &mut Canvas, r: Rect, vis: Rect, clip: &Clip, colour: u32, lib: &Library, upp: Micros, start: Micros, x0: i32, name: &str) {
        let (lx, rx) = (r.x.max(vis.x), (r.x + r.w).min(vis.x + vis.w));
        c.fill(lx, r.y, rx - lx, r.h, shade(colour, 0, 60));
        let wave = shade(colour, 0xffffff, 110);
        let label_h = 14;
        let mid = r.y + label_h + (r.h - label_h) / 2;
        let half = ((r.h - label_h) / 2 - 1).max(1);
        if let Some(peaks) = lib.peaks.get(&clip.media) {
            for x in lx..rx {
                let t = start + (x - x0) as Micros * upp;
                let (s0, s1) = (clip.source_at(t.max(clip.at)), clip.source_at((t + upp).min(clip.end())));
                let v = peaks.over(s0, s1.max(s0 + 1)) as i32 * clip.volume.min(2000) as i32 / 1000;
                let len = (v * half / 255).min(half);
                c.fill(x, mid - len, 1, len * 2 + 1, wave);
            }
        }
        c.fill(lx, mid, rx - lx, 1, shade(colour, 0xffffff, 60));
        self.small.text(c, r.x + 5, r.y, name, INK, Some((lx + 2, r.w - 8)));
        c.frame(r.x, r.y, r.w, r.h, shade(colour, 0xffffff, 90));
    }
}
