// SPDX-License-Identifier: GPL-3.0-only
//! Painting Splicy's window in Final Cut Pro & Premiere Candy Noir layouts:
//! a macOS-crisp toolbar with segmented `[Final Cut | Premiere]` toggle and recessed
//! `#0f0f12` LCD timecode display, transcript & suggestion cards, viewer + transport,
//! right-hand Clip Inspector, and a Magnetic-style Noir timeline with role bars,
//! filmstrips, Candy Teal waveforms, In/Out amber range markers, joins, and playhead.

use crate::media::Library;
use crate::vu::{VuStrip, VU_STRIP_H};
use crate::{Editor, ToolKind};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_office::{Canvas, Pen};
use sex_splice::{Clip, Micros, Timeline, TrackKind, SECOND};

// ── Noir + SexOS Candy Palette ────────────────────────────────────────────────
pub const LCD_WELL: u32 = 0x000f0f12;
pub const TIMELINE_WELL: u32 = 0x00141417;
pub const BG: u32 = 0x0018181c;
pub const INSPECTOR_BG: u32 = 0x001c1c21;
pub const PANEL: u32 = 0x00212126;
pub const PANEL2: u32 = 0x002a2a31;
pub const HAIRLINE: u32 = 0x00111114;
pub const LINE: u32 = 0x00383842;
pub const INK: u32 = 0x00ededf2;
pub const DIM: u32 = 0x008e8e9c;
pub const ACCENT: u32 = 0x00ff2d7a;
pub const CANDY_LAVENDER: u32 = 0x009b5de5;
pub const CANDY_TEAL: u32 = 0x0000c2a8;
pub const FCP_BLUE: u32 = 0x002d7aff;
pub const FCP_GREEN: u32 = 0x0032c456;
pub const RANGE_AMBER: u32 = 0x00e8a62a;
pub const SELECT: u32 = 0x005a2440;
pub const PLAYHEAD: u32 = 0x00ffa000;
pub const CARD: u32 = 0x0028282f;
/// Picture tracks, then sound tracks, take these colours in turn.
/// Note: `[0]` values are matched by `program_splicy_gate.py` (`LABEL` and `WAVE`).
pub const VIDEO_COLOURS: [u32; 3] = [0x003949ab, 0x005e35b1, 0x008e24aa];
pub const SOUND_COLOURS: [u32; 3] = [0x0000796b, 0x002e7d32, 0x00ad1457];
pub const TOOLBAR_H: i32 = 36;
pub const STATUS_H: i32 = 24;
pub const HEAD_W: i32 = 140;
pub const RULER_H: i32 = 24;
const PAD: i32 = 8;

/// Workspace layout preset: 3-column Final Cut Pro or 4-quadrant Premiere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    #[default]
    FinalCut,
    Premiere,
}

impl LayoutMode {
    pub fn toggle(self) -> Self {
        match self {
            LayoutMode::FinalCut => LayoutMode::Premiere,
            LayoutMode::Premiere => LayoutMode::FinalCut,
        }
    }
}

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

/// Interactive hit-targets inside the top FCP toolbar.
#[derive(Clone, Copy, Debug)]
pub struct ToolbarButtons {
    pub layout_pill: Rect,
    pub fcp_segment: Rect,
    pub premiere_segment: Rect,
    pub lcd_well: Rect,
    pub select_btn: Rect,
    pub blade_btn: Rect,
    pub vu_btn: Rect,
    pub inspector_btn: Rect,
}

/// Action triggered by clicking a toolbar pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarHit {
    Layout(LayoutMode),
    Tool(ToolKind),
    ToggleVu,
    ToggleInspector,
}

/// Action triggered by clicking the Viewer transport bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportHit {
    Start,
    StepBack,
    PlayPause,
    StepFwd,
    End,
    CleanupOrAudition,
}

/// Interactive button rects inside `Areas::transport`.
#[derive(Clone, Copy, Debug)]
pub struct TransportButtons {
    pub start_btn: Rect,
    pub back_btn: Rect,
    pub play_btn: Rect,
    pub fwd_btn: Rect,
    pub end_btn: Rect,
    pub action_btn: Rect,
}

pub fn transport_buttons(r: Rect) -> TransportButtons {
    let bh = (r.h - 6).max(16);
    let by = r.y + (r.h - bh) / 2;
    let cx = r.x + r.w / 2;
    // Place transport cluster cleanly to the left of the centered timecode (`cx - 52`).
    let play_w = if r.w >= 420 { 54 } else { 34 };
    let small_w = 24;
    let gap = 3;
    let cluster_right = (cx - 54).max(r.x + 4);
    let end_x = cluster_right - small_w;
    let fwd_x = end_x - gap - small_w;
    let play_x = fwd_x - gap - play_w;
    let back_x = play_x - gap - small_w;
    let start_x = back_x - gap - small_w;

    let clamp_btn = |x: i32, w: i32| -> Rect {
        if x >= r.x + 2 && x + w <= r.x + r.w - 2 {
            Rect { x, y: by, w, h: bh }
        } else {
            Rect { x: 0, y: 0, w: 0, h: 0 }
        }
    };
    let action_btn = if start_x >= r.x + 78 {
        Rect { x: r.x + PAD, y: by, w: 66, h: bh }
    } else {
        Rect { x: 0, y: 0, w: 0, h: 0 }
    };
    TransportButtons {
        start_btn: clamp_btn(start_x, small_w),
        back_btn: clamp_btn(back_x, small_w),
        play_btn: clamp_btn(play_x, play_w),
        fwd_btn: clamp_btn(fwd_x, small_w),
        end_btn: clamp_btn(end_x, small_w),
        action_btn,
    }
}

/// Action triggered by clicking the Suggestions pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardHit {
    RunCleanup,
    Select(usize),
    Accept(usize),
    Skip(usize),
}

/// Action triggered by clicking a track header switch (`eye` or `M`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackSwitchHit {
    ToggleVideo(usize),
    ToggleMute(usize),
}

pub fn toolbar_buttons(w: i32) -> ToolbarButtons {
    let pill_x = 92.min((w / 4).max(8));
    let pill_w = 148.min((w / 3).max(0));
    let pill_y = 6;
    let pill_h = TOOLBAR_H - 12;
    let layout_pill = Rect { x: pill_x, y: pill_y, w: pill_w, h: pill_h };
    let half = pill_w / 2;
    let fcp_segment = Rect { x: pill_x, y: pill_y, w: half, h: pill_h };
    let premiere_segment = Rect { x: pill_x + half, y: pill_y, w: pill_w - half, h: pill_h };

    let lcd_w = if w >= 760 { 224 } else if w >= 520 { 164 } else { 110.min(w) };
    let lcd_x = ((w - lcd_w) / 2).max(pill_x + pill_w + 6).min((w - lcd_w).max(0));
    let lcd_well = Rect { x: lcd_x, y: 4, w: lcd_w, h: TOOLBAR_H - 8 };

    let btn_y = 6;
    let btn_h = TOOLBAR_H - 12;
    let mut rx = w - PAD;
    let make_btn = |rx: &mut i32, bw: i32, limit: i32| -> Rect {
        if *rx - bw >= limit {
            *rx -= bw;
            let r = Rect { x: *rx, y: btn_y, w: bw, h: btn_h };
            *rx -= 4;
            r
        } else {
            Rect { x: 0, y: 0, w: 0, h: 0 }
        }
    };
    let limit = lcd_well.x + lcd_well.w + 8;
    let inspector_btn = make_btn(&mut rx, 70, limit);
    let vu_btn = make_btn(&mut rx, 34, limit);
    let blade_btn = make_btn(&mut rx, 60, limit);
    let select_btn = make_btn(&mut rx, 64, limit);

    ToolbarButtons {
        layout_pill,
        fcp_segment,
        premiere_segment,
        lcd_well,
        select_btn,
        blade_btn,
        vu_btn,
        inspector_btn,
    }
}

/// Where each part of the window goes at `w` x `h`.
#[derive(Clone, Copy, Debug)]
pub struct Areas {
    pub toolbar: Rect,
    /// Segmented `[Final Cut | Premiere]` layout toggle pill in the toolbar.
    pub layout_pill: Rect,
    /// The transcript, above the cards.
    pub text: Rect,
    pub cards: Rect,
    /// The picture, above the transport bar.
    pub player: Rect,
    pub transport: Rect,
    /// Right-hand Inspector pane when visible.
    pub inspector: Rect,
    /// Track headers down the left of the timeline.
    pub heads: Rect,
    pub ruler: Rect,
    /// Where the clips are drawn.
    pub tracks: Rect,
    /// Bottom VU meter strip when shown.
    pub vu: Rect,
    pub status: Rect,
}

pub fn areas_for_with_inspector(
    w: i32,
    h: i32,
    layout: LayoutMode,
    show_vu: bool,
    show_inspector: bool,
) -> Areas {
    let tb = toolbar_buttons(w);
    let body_y = TOOLBAR_H;
    let body_h = (h - TOOLBAR_H - STATUS_H).max(0);
    let top_h = body_h * 11 / 20;
    let transport_h = 28;
    let tl_y = body_y + top_h + 1;
    let total_tl_h = (body_h - top_h - 1).max(0);
    let (tl_h, vu) = if show_vu {
        let vu_h = VU_STRIP_H.min(total_tl_h);
        let rem = total_tl_h - vu_h;
        let vu_rect = Rect { x: 0, y: tl_y + rem, w, h: vu_h };
        (rem, vu_rect)
    } else {
        (total_tl_h, Rect { x: 0, y: h - STATUS_H, w: 0, h: 0 })
    };

    match layout {
        LayoutMode::FinalCut => {
            let insp_w = if show_inspector && w >= 900 {
                (w * 20 / 100).clamp(200, 260)
            } else {
                0
            };
            let left_w = if insp_w > 0 {
                (w * 32 / 100).max(240).min((w - insp_w) / 2)
            } else {
                (w * 2 / 5).max(0)
            };
            let cards_h = (top_h / 3).min(120);
            let text = Rect {
                x: PAD,
                y: body_y + PAD,
                w: (left_w - PAD * 2).max(0),
                h: (top_h - cards_h - PAD * 2).max(0),
            };
            let cards = Rect {
                x: PAD,
                y: text.y + text.h + PAD / 2,
                w: text.w,
                h: cards_h,
            };
            let player = Rect {
                x: left_w + PAD,
                y: body_y + PAD,
                w: (w - left_w - insp_w - PAD * 2).max(0),
                h: (top_h - transport_h - PAD).max(0),
            };
            let transport = Rect {
                x: player.x,
                y: player.y + player.h,
                w: player.w,
                h: transport_h,
            };
            let inspector = if insp_w > 0 {
                Rect { x: w - insp_w, y: body_y, w: insp_w, h: top_h }
            } else {
                Rect { x: w, y: body_y, w: 0, h: 0 }
            };
            let heads = Rect {
                x: 0,
                y: tl_y + RULER_H,
                w: HEAD_W.min(w),
                h: (tl_h - RULER_H).max(0),
            };
            let ruler = Rect {
                x: heads.w,
                y: tl_y,
                w: (w - heads.w).max(0),
                h: RULER_H.min(tl_h),
            };
            let tracks = Rect {
                x: heads.w,
                y: heads.y,
                w: ruler.w,
                h: heads.h,
            };
            Areas {
                toolbar: Rect { x: 0, y: 0, w, h: TOOLBAR_H },
                layout_pill: tb.layout_pill,
                text,
                cards,
                player,
                transport,
                inspector,
                heads,
                ruler,
                tracks,
                vu,
                status: Rect { x: 0, y: h - STATUS_H, w, h: STATUS_H },
            }
        }
        LayoutMode::Premiere => {
            // 4-quadrant Premiere layout:
            // Top-Left: Source / Transcript
            // Top-Right: Program Viewer (+ optional Inspector)
            // Bottom-Left: Project & Suggestions
            // Bottom-Right: Timeline
            let left_w = (w * 36 / 100).max(220).min(w / 2);
            let insp_w = if show_inspector && w >= 1020 { 220 } else { 0 };
            let text = Rect {
                x: PAD,
                y: body_y + PAD,
                w: (left_w - PAD * 2).max(0),
                h: (top_h - PAD * 2).max(0),
            };
            let player = Rect {
                x: left_w + PAD,
                y: body_y + PAD,
                w: (w - left_w - insp_w - PAD * 2).max(0),
                h: (top_h - transport_h - PAD).max(0),
            };
            let transport = Rect {
                x: player.x,
                y: player.y + player.h,
                w: player.w,
                h: transport_h,
            };
            let inspector = if insp_w > 0 {
                Rect { x: w - insp_w, y: body_y, w: insp_w, h: top_h }
            } else {
                Rect { x: w, y: body_y, w: 0, h: 0 }
            };
            let cards = Rect {
                x: PAD,
                y: tl_y + PAD,
                w: (left_w - PAD * 2).max(0),
                h: (tl_h - PAD * 2).max(0),
            };
            let br_x = left_w;
            let br_w = (w - br_x).max(0);
            let head_w = HEAD_W.min(br_w / 3).max(80.min(br_w));
            let heads = Rect {
                x: br_x,
                y: tl_y + RULER_H,
                w: head_w,
                h: (tl_h - RULER_H).max(0),
            };
            let ruler = Rect {
                x: br_x + head_w,
                y: tl_y,
                w: (br_w - head_w).max(0),
                h: RULER_H.min(tl_h),
            };
            let tracks = Rect {
                x: ruler.x,
                y: heads.y,
                w: ruler.w,
                h: heads.h,
            };
            Areas {
                toolbar: Rect { x: 0, y: 0, w, h: TOOLBAR_H },
                layout_pill: tb.layout_pill,
                text,
                cards,
                player,
                transport,
                inspector,
                heads,
                ruler,
                tracks,
                vu,
                status: Rect { x: 0, y: h - STATUS_H, w, h: STATUS_H },
            }
        }
    }
}

pub fn areas_for(w: i32, h: i32, layout: LayoutMode, show_vu: bool) -> Areas {
    areas_for_with_inspector(w, h, layout, show_vu, true)
}

pub fn areas_with_vu(w: i32, h: i32, show_vu: bool) -> Areas {
    areas_for(w, h, LayoutMode::FinalCut, show_vu)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineCacheKey {
    w: i32,
    h: i32,
    layout: LayoutMode,
    show_vu: bool,
    show_inspector: bool,
    zoom: Micros,
    start: Micros,
    head: usize,
    in_point: Option<Micros>,
    out_point: Option<Micros>,
    active_v_clip: Option<u32>,
    active_a_clip: Option<u32>,
    thumbs_count: usize,
    peaks_count: usize,
}

pub struct View {
    pen: Pen,
    small: Pen,
    big: Pen,
    mono_small: Pen,
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
    /// Active workspace preset (FinalCut 3-col or Premiere 4-quadrant).
    pub layout: LayoutMode,
    /// Whether the right-hand Inspector column is shown when space permits.
    pub show_inspector: bool,
    /// Cached word layout `(word_count, width, text_hash)` so playback never re-measures glyphs.
    placed_key: Option<(usize, i32, u64)>,
    placed_cache: Vec<Placed>,
    /// Cached rendered timeline rows (`y = ruler.y .. tracks.y + tracks.h`) without playhead.
    timeline_key: Option<TimelineCacheKey>,
    timeline_px: Vec<u32>,
}

impl Default for View {
    fn default() -> Self {
        View {
            pen: Pen::sans(15),
            small: Pen::sans(12),
            big: Pen::mono(17),
            mono_small: Pen::mono(11),
            scroll: 0,
            followed: None,
            zoom: 0,
            start: 0,
            vu: VuStrip::default(),
            layout: LayoutMode::FinalCut,
            show_inspector: true,
            placed_key: None,
            placed_cache: Vec::new(),
            timeline_key: None,
            timeline_px: Vec::new(),
        }
    }
}

/// A picture to show in the viewer: pixels, width, height.
pub type Picture<'a> = (&'a [u32], usize, usize);

/// Copy `pic` into `r` scaled to cover its width (cropped to `clip`), 16.16 fixed-point row blitz.
fn blit(c: &mut Canvas, r: Rect, clip: Rect, pic: Picture) {
    let (px, pw, ph) = pic;
    if pw == 0 || ph == 0 || r.w <= 0 || r.h <= 0 || px.len() < pw * ph {
        return;
    }
    let y0 = r.y.max(clip.y).max(0);
    let y1 = (r.y + r.h).min(clip.y + clip.h).min(c.height as i32);
    let x0 = r.x.max(clip.x).max(0);
    let x1 = (r.x + r.w).min(clip.x + clip.w).min(c.width as i32);
    if y1 <= y0 || x1 <= x0 {
        return;
    }
    let step_x = ((pw as u64) << 16) / (r.w as u64);
    let init_sx = ((x0 - r.x) as u64) * step_x;
    let max_sx = pw - 1;
    let max_sy = ph - 1;
    let rw = r.h as usize;
    let count = (x1 - x0) as usize;
    for y in y0..y1 {
        let sy = ((y - r.y) as usize * ph / rw).min(max_sy);
        let src_row = &px[sy * pw..sy * pw + pw];
        let dst_off = y as usize * c.stride + x0 as usize;
        let dst_row = &mut c.px[dst_off..dst_off + count];
        let mut sx_fp = init_sx;
        for dst in dst_row.iter_mut() {
            let sx = ((sx_fp >> 16) as usize).min(max_sx);
            *dst = src_row[sx];
            sx_fp += step_x;
        }
    }
}

/// Mix `over` onto `under` by `alpha` of 255.
fn shade(under: u32, over: u32, alpha: u32) -> u32 {
    sex_office::mix(under, over, alpha)
}

impl View {
    pub fn areas(&self, w: i32, h: i32) -> Areas {
        areas_for_with_inspector(w, h, self.layout, self.vu.visible, self.show_inspector)
    }

    pub fn line_height(&self) -> i32 {
        self.pen.line_height() + 6
    }

    pub fn layout(&mut self, ed: &Editor, width: i32) -> Vec<Placed> {
        let mut h = 0xcbf29ce484222325u64;
        for w in &ed.transcript.words {
            for &b in w.text.as_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h ^= 0xff;
        }
        let key = (ed.transcript.words.len(), width, h);
        if self.placed_key == Some(key) {
            return self.placed_cache.clone();
        }
        let words: Vec<&str> = ed.transcript.words.iter().map(|w| w.text.as_str()).collect();
        let lh = self.line_height();
        let pen = &mut self.pen;
        let out = layout(&words, width, lh, |s| pen.measure(s));
        self.placed_key = Some(key);
        self.placed_cache = out.clone();
        out
    }

    /// Hit-test interactive pills in the top toolbar.
    pub fn toolbar_hit(&self, w: i32, _h: i32, x: i32, y: i32) -> Option<ToolbarHit> {
        if y < 0 || y >= TOOLBAR_H {
            return None;
        }
        let tb = toolbar_buttons(w);
        if tb.fcp_segment.contains(x, y) {
            return Some(ToolbarHit::Layout(LayoutMode::FinalCut));
        }
        if tb.premiere_segment.contains(x, y) {
            return Some(ToolbarHit::Layout(LayoutMode::Premiere));
        }
        if tb.select_btn.contains(x, y) {
            return Some(ToolbarHit::Tool(ToolKind::Select));
        }
        if tb.blade_btn.contains(x, y) {
            return Some(ToolbarHit::Tool(ToolKind::Blade));
        }
        if tb.vu_btn.contains(x, y) {
            return Some(ToolbarHit::ToggleVu);
        }
        if tb.inspector_btn.contains(x, y) {
            return Some(ToolbarHit::ToggleInspector);
        }
        None
    }

    /// Hit-test interactive buttons in the Viewer transport bar.
    pub fn transport_hit(&self, w: i32, h: i32, x: i32, y: i32) -> Option<TransportHit> {
        let tr = self.areas(w, h).transport;
        if !tr.contains(x, y) {
            return None;
        }
        let b = transport_buttons(tr);
        if b.play_btn.contains(x, y) {
            return Some(TransportHit::PlayPause);
        }
        if b.start_btn.contains(x, y) {
            return Some(TransportHit::Start);
        }
        if b.back_btn.contains(x, y) {
            return Some(TransportHit::StepBack);
        }
        if b.fwd_btn.contains(x, y) {
            return Some(TransportHit::StepFwd);
        }
        if b.end_btn.contains(x, y) {
            return Some(TransportHit::End);
        }
        if b.action_btn.contains(x, y) {
            return Some(TransportHit::CleanupOrAudition);
        }
        // Clicking the centered timecode area in transport also toggles play/pause.
        Some(TransportHit::PlayPause)
    }

    /// Whether (`x`, `y`) is inside the Viewer picture area (click to play/pause).
    pub fn player_hit(&self, w: i32, h: i32, x: i32, y: i32) -> bool {
        self.areas(w, h).player.contains(x, y)
    }

    /// Hit-test the Suggestions pane (`RunCleanup` header or per-card `[Take]`/`[Skip]`/focus).
    pub fn card_hit(&self, ed: &Editor, w: i32, h: i32, x: i32, y: i32) -> Option<CardHit> {
        let r = self.areas(w, h).cards;
        if !r.contains(x, y) || r.w < 40 || r.h < 20 {
            return None;
        }
        let lh = self.small.line_height();
        let pending = ed.pending();
        if y < r.y + lh + 8 {
            if pending.is_empty() {
                return Some(CardHit::RunCleanup);
            }
            return None;
        }
        let mut cy = r.y + lh + 8;
        let card_h = lh * 2 + 10;
        for (n, _id) in pending.iter().enumerate() {
            if cy + card_h > r.y + r.h {
                break;
            }
            let card_rect = Rect { x: r.x, y: cy, w: r.w, h: card_h };
            if card_rect.contains(x, y) {
                if r.w >= 150 {
                    let btn_h = 16;
                    let btn_y = cy + card_h - btn_h - 4;
                    let skip_r = Rect { x: r.x + r.w - 42, y: btn_y, w: 36, h: btn_h };
                    let take_r = Rect { x: skip_r.x - 44, y: btn_y, w: 40, h: btn_h };
                    if take_r.contains(x, y) {
                        return Some(CardHit::Accept(n));
                    }
                    if skip_r.contains(x, y) {
                        return Some(CardHit::Skip(n));
                    }
                }
                return Some(CardHit::Select(n));
            }
            cy += card_h + 4;
        }
        if pending.is_empty() {
            Some(CardHit::RunCleanup)
        } else {
            None
        }
    }

    /// Hit-test track header switches (`eye` on Video tracks, `M` on Audio tracks).
    pub fn track_switch_hit(&self, tl: &Timeline, w: i32, h: i32, x: i32, y: i32) -> Option<TrackSwitchHit> {
        let a = self.areas(w, h);
        if !a.heads.contains(x, y) {
            return None;
        }
        let mut order: Vec<usize> = (0..tl.tracks.len()).filter(|i| tl.tracks[*i].kind == TrackKind::Video).rev().collect();
        order.extend((0..tl.tracks.len()).filter(|i| tl.tracks[*i].kind == TrackKind::Audio));
        let rows = order.len();
        let mut ry = a.tracks.y + 4;
        for &ti in &order {
            let track = &tl.tracks[ti];
            let rh = row_height(track.kind, rows, a.tracks.h - 8);
            let sx = a.heads.x + 10;
            let sy = ry + rh - 22;
            let first_box = Rect { x: sx, y: sy, w: 20, h: 16 };
            if first_box.contains(x, y) {
                return Some(match track.kind {
                    TrackKind::Video => TrackSwitchHit::ToggleVideo(ti),
                    TrackKind::Audio => TrackSwitchHit::ToggleMute(ti),
                });
            }
            ry += rh;
        }
        None
    }

    /// The word a click at window point (`x`, `y`) lands on.
    pub fn word_at(&mut self, ed: &Editor, w: i32, h: i32, x: i32, y: i32) -> Option<usize> {
        let a = self.areas(w, h).text;
        if !a.contains(x, y) {
            return None;
        }
        let placed = self.layout(ed, a.w);
        hit(&placed, x - a.x, y - a.y + self.scroll, self.line_height())
    }

    /// Microseconds per pixel on the tracks for `tl` at window width `w`.
    pub fn us_per_px(&self, tl: &Timeline, w: i32) -> Micros {
        let width = (w - HEAD_W).max(1);
        self.us_per_px_in(tl, width)
    }

    /// Microseconds per pixel for a specific track-area width `tracks_w`.
    pub fn us_per_px_in(&self, tl: &Timeline, tracks_w: i32) -> Micros {
        if self.zoom > 0 {
            return self.zoom;
        }
        let width = tracks_w.max(1) as Micros;
        // Fit the edit with a little room after its end.
        ((tl.duration().max(SECOND) * 21 / 20) + width - 1) / width
    }

    /// The timeline time under window point (`x`, `y`) when it is on the ruler or the
    /// tracks: a click there moves the playhead.
    pub fn time_at(&self, tl: &Timeline, w: i32, h: i32, x: i32, y: i32) -> Option<Micros> {
        let a = self.areas(w, h);
        if !(a.ruler.contains(x, y) || a.tracks.contains(x, y)) {
            return None;
        }
        let upp = self.us_per_px_in(tl, a.tracks.w);
        Some((self.start + (x - a.tracks.x) as Micros * upp).clamp(0, tl.duration()))
    }

    /// Timeline time corresponding to any `x` coordinate while dragging a scrub gesture.
    pub fn scrub_time_at_x(&self, tl: &Timeline, w: i32, h: i32, x: i32) -> Micros {
        let a = self.areas(w, h);
        let upp = self.us_per_px_in(tl, a.tracks.w);
        (self.start + (x - a.tracks.x) as Micros * upp).clamp(0, tl.duration())
    }

    /// Zoom in (`steps` > 0) or out around the playhead.
    pub fn zoom_by(&mut self, tl: &Timeline, w: i32, steps: i32, playhead: Micros) {
        let tracks_w = self.areas(w, 600).tracks.w.max(1);
        let now = self.us_per_px_in(tl, tracks_w);
        let next = if steps > 0 { now / 2 } else { now * 2 };
        let fit = { View { zoom: 0, layout: self.layout, show_inspector: self.show_inspector, ..View::default() } }
            .us_per_px_in(tl, tracks_w);
        self.zoom = next.clamp(1000, fit.max(1000));
        if self.zoom >= fit - fit / 8 {
            self.zoom = 0;
            self.start = 0;
        } else {
            let half = tracks_w as Micros * self.zoom / 2;
            self.start = (playhead - half).max(0);
        }
    }

    /// Paint the whole window into `px` (`w` x `h`, 0x00RRGGBB). `playhead` is a timeline
    /// time; `message` replaces the status line's key help when it is not empty.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, px: &mut [u32], w: usize, h: usize, ed: &Editor, lib: &Library, picture: Option<Picture>, playhead: Micros, message: &str) {
        let (wi, hi) = (w as i32, h as i32);
        let a = self.areas(wi, hi);
        let mut c = Canvas::new(px, w, h);
        c.fill(0, 0, wi, hi, BG);
        self.toolbar(&mut c, a.toolbar, ed, playhead);

        // Transcript & Suggestions cards panel(s).
        match self.layout {
            LayoutMode::FinalCut => {
                c.fill(a.text.x - PAD / 2, a.text.y - PAD / 2, a.text.w + PAD, a.text.h + a.cards.h + PAD * 2, PANEL);
                c.frame(a.text.x - PAD / 2, a.text.y - PAD / 2, a.text.w + PAD, a.text.h + a.cards.h + PAD * 2, HAIRLINE);
            }
            LayoutMode::Premiere => {
                c.fill(a.text.x - PAD / 2, a.text.y - PAD / 2, a.text.w + PAD, a.text.h + PAD, PANEL);
                c.frame(a.text.x - PAD / 2, a.text.y - PAD / 2, a.text.w + PAD, a.text.h + PAD, HAIRLINE);
                c.fill(a.cards.x - PAD / 2, a.cards.y - PAD / 2, a.cards.w + PAD, a.cards.h + PAD, PANEL);
                c.frame(a.cards.x - PAD / 2, a.cards.y - PAD / 2, a.cards.w + PAD, a.cards.h + PAD, HAIRLINE);
            }
        }
        self.transcript(&mut c, a.text, ed);
        self.cards(&mut c, a.cards, ed);
        self.player(&mut c, a.player, picture, ed, lib, playhead);
        self.transport(&mut c, a.transport, ed, playhead);
        if a.inspector.w > 0 && a.inspector.h > 0 {
            self.inspector(&mut c, a.inspector, ed, playhead);
        }
        self.timeline(&mut c, &a, ed, lib, playhead);
        if self.vu.visible {
            self.vu.update_from_library(ed.timeline(), lib, playhead, 50_000);
            self.vu.paint(&mut c, a.vu, &mut self.small);
        }
        c.fill(0, a.status.y, wi, STATUS_H, PANEL);
        c.fill(0, a.status.y, wi, 1, HAIRLINE);
        let status = if message.is_empty() {
            String::from("Space play/pause  J/K/L shuttle  I/O mark  B blade  A/T tool  P layout  V meter  S sentence  C clean-up  Ctrl+Z/Y/E")
        } else {
            String::from(message)
        };
        let y = a.status.y + (STATUS_H - self.small.line_height()) / 2;
        self.small.text(&mut c, PAD, y, &status, DIM, Some((0, wi - PAD)));
    }

    fn draw_pill(&mut self, c: &mut Canvas, r: Rect, label: &str, active: bool, accent: u32) {
        if r.w <= 4 || r.h <= 4 {
            return;
        }
        let bg = if active { shade(PANEL2, accent, 80) } else { PANEL2 };
        let fg = if active { INK } else { DIM };
        let border = if active { accent } else { LINE };
        c.fill(r.x, r.y, r.w, r.h, bg);
        c.frame(r.x, r.y, r.w, r.h, border);
        let lw = self.small.measure(label);
        let tx = r.x + ((r.w - lw) / 2).max(2);
        let ty = r.y + (r.h - self.small.line_height()) / 2;
        self.small.text(c, tx, ty, label, fg, Some((r.x + 2, (r.w - 4).max(0))));
    }

    fn toolbar(&mut self, c: &mut Canvas, r: Rect, ed: &Editor, playhead: Micros) {
        c.fill(r.x, r.y, r.w, r.h, PANEL);
        c.fill(r.x, r.y + r.h - 1, r.w, 1, HAIRLINE);
        let tb = toolbar_buttons(r.w);
        let ty = r.y + (r.h - self.pen.line_height()) / 2;

        // Left: Splicy monogram badge + title
        let badge = Rect { x: PAD, y: r.y + 8, w: 18, h: r.h - 16 };
        c.fill(badge.x, badge.y, badge.w, badge.h, ACCENT);
        c.frame(badge.x, badge.y, badge.w, badge.h, shade(ACCENT, 0xffffff, 70));
        self.mono_small.text(c, badge.x + 5, badge.y + 3, "S", INK, None);
        self.pen.text(c, badge.x + badge.w + 6, ty, "Splicy", INK, Some((0, tb.layout_pill.x - 4)));

        // Segmented [Final Cut | Premiere] layout pill
        if tb.layout_pill.w >= 80 {
            c.fill(tb.layout_pill.x, tb.layout_pill.y, tb.layout_pill.w, tb.layout_pill.h, LCD_WELL);
            c.frame(tb.layout_pill.x, tb.layout_pill.y, tb.layout_pill.w, tb.layout_pill.h, LINE);
            let fcp_on = self.layout == LayoutMode::FinalCut;
            let pre_on = self.layout == LayoutMode::Premiere;
            if fcp_on {
                c.fill(tb.fcp_segment.x + 1, tb.fcp_segment.y + 1, tb.fcp_segment.w - 1, tb.fcp_segment.h - 2, CANDY_LAVENDER);
            } else if pre_on {
                c.fill(tb.premiere_segment.x, tb.premiere_segment.y + 1, tb.premiere_segment.w - 1, tb.premiere_segment.h - 2, FCP_BLUE);
            }
            let fcp_lbl = "Final Cut";
            let fw = self.small.measure(fcp_lbl);
            self.small.text(
                c,
                tb.fcp_segment.x + ((tb.fcp_segment.w - fw) / 2).max(2),
                tb.fcp_segment.y + (tb.fcp_segment.h - self.small.line_height()) / 2,
                fcp_lbl,
                if fcp_on { INK } else { DIM },
                Some((tb.fcp_segment.x, tb.fcp_segment.w)),
            );
            let pre_lbl = "Premiere";
            let pw = self.small.measure(pre_lbl);
            self.small.text(
                c,
                tb.premiere_segment.x + ((tb.premiere_segment.w - pw) / 2).max(2),
                tb.premiere_segment.y + (tb.premiere_segment.h - self.small.line_height()) / 2,
                pre_lbl,
                if pre_on { INK } else { DIM },
                Some((tb.premiere_segment.x, tb.premiere_segment.w)),
            );
        }

        // Clip short name between layout pill and LCD
        let tl = ed.timeline();
        let name = tl.media.first().map_or("", |m| short(&m.path));
        let name_x = tb.layout_pill.x + tb.layout_pill.w + 10;
        if tb.lcd_well.x > name_x + 48 && !name.is_empty() {
            self.small.text(c, name_x, ty + 2, name, DIM, Some((name_x, tb.lcd_well.x - name_x - 8)));
        }

        // Center: Recessed #0f0f12 macOS FCP LCD Timecode Display
        if tb.lcd_well.w > 40 {
            let lcd = tb.lcd_well;
            c.fill(lcd.x, lcd.y, lcd.w, lcd.h, LCD_WELL);
            c.frame(lcd.x, lcd.y, lcd.w, lcd.h, HAIRLINE);
            c.fill(lcd.x + 1, lcd.y + lcd.h - 1, lcd.w - 2, 1, LINE);

            let tc = timecode(playhead, (tl.fps_num, tl.fps_den));
            let tcw = self.big.measure(&tc);
            let tcx = lcd.x + ((lcd.w - tcw) / 2).max(4);
            let tcy = lcd.y + (lcd.h - self.big.line_height()) / 2 + 1;
            self.big.text(c, tcx, tcy, &tc, INK, Some((lcd.x + 2, lcd.w - 4)));

            if lcd.w >= 190 {
                let rate_txt = match ed.shuttle_rate {
                    0 => String::from("JKL"),
                    r if r > 0 => format!("{r}x>"),
                    r => format!("<{}x", -r),
                };
                let rate_col = if ed.shuttle_rate != 0 { ACCENT } else { DIM };
                self.mono_small.text(
                    c,
                    lcd.x + 8,
                    lcd.y + (lcd.h - self.mono_small.line_height()) / 2,
                    &rate_txt,
                    rate_col,
                    Some((lcd.x + 4, (tcx - lcd.x - 6).max(0))),
                );

                let io_txt = match (ed.in_point, ed.out_point) {
                    (Some(_), Some(_)) => "I/O",
                    (Some(_), None) => "IN",
                    (None, Some(_)) => "OUT",
                    (None, None) => "",
                };
                if !io_txt.is_empty() {
                    let iw = self.mono_small.measure(io_txt);
                    self.mono_small.text(
                        c,
                        lcd.x + lcd.w - iw - 8,
                        lcd.y + (lcd.h - self.mono_small.line_height()) / 2,
                        io_txt,
                        RANGE_AMBER,
                        Some((lcd.x, lcd.w)),
                    );
                }
            }
        }

        // Right: Tool & Panel pills
        self.draw_pill(c, tb.select_btn, "A Select", ed.active_tool == ToolKind::Select, FCP_BLUE);
        self.draw_pill(c, tb.blade_btn, "B Blade", ed.active_tool == ToolKind::Blade, ACCENT);
        self.draw_pill(c, tb.vu_btn, "VU", self.vu.visible, CANDY_TEAL);
        self.draw_pill(c, tb.inspector_btn, "Inspector", self.show_inspector, CANDY_LAVENDER);
    }

    fn player(&mut self, c: &mut Canvas, r: Rect, picture: Option<Picture>, ed: &Editor, lib: &Library, playhead: Micros) {
        c.fill(r.x, r.y, r.w, r.h, 0);
        c.frame(r.x, r.y, r.w, r.h, HAIRLINE);
        if let Some((pixels, pw, ph)) = picture {
            if pw > 0 && ph > 0 && pixels.len() >= pw * ph {
                // Fit inside the box, keeping the shape.
                let (sw, sh) = if r.w as usize * ph > r.h as usize * pw {
                    ((r.h as usize * pw / ph) as i32, r.h)
                } else {
                    (r.w, (r.w as usize * ph / pw) as i32)
                };
                let fit = Rect { x: r.x + (r.w - sw) / 2, y: r.y + (r.h - sh) / 2, w: sw, h: sh };
                blit(c, fit, r, (pixels, pw, ph));
                return;
            }
        }
        // Dynamic Candy Noir live-preview slate when running without decoded video frames
        // (e.g. audio-only media or stub-decoder build), so playback & scrubbing always animate!
        if r.w < 60 || r.h < 40 {
            return;
        }
        let pad = 12;
        let slate = Rect {
            x: r.x + pad,
            y: r.y + pad,
            w: (r.w - pad * 2).max(20),
            h: (r.h - pad * 2).max(20),
        };
        c.fill(slate.x, slate.y, slate.w, slate.h, LCD_WELL);
        c.frame(slate.x, slate.y, slate.w, slate.h, LINE);
        // Safe-action corner brackets
        let br = 12.min(slate.w / 4).min(slate.h / 4);
        let br_col = if ed.shuttle_rate != 0 { ACCENT } else { CANDY_LAVENDER };
        c.fill(slate.x + 6, slate.y + 6, br, 2, br_col);
        c.fill(slate.x + 6, slate.y + 6, 2, br, br_col);
        c.fill(slate.x + slate.w - 6 - br, slate.y + 6, br, 2, br_col);
        c.fill(slate.x + slate.w - 8, slate.y + 6, 2, br, br_col);
        c.fill(slate.x + 6, slate.y + slate.h - 8, br, 2, br_col);
        c.fill(slate.x + 6, slate.y + slate.h - 6 - br, 2, br, br_col);
        c.fill(slate.x + slate.w - 6 - br, slate.y + slate.h - 8, br, 2, br_col);
        c.fill(slate.x + slate.w - 8, slate.y + slate.h - 6 - br, 2, br, br_col);

        let tl = ed.timeline();
        let clip_name = tl.media.first().map_or("Timeline Preview", |m| short(&m.path));
        let state_lbl = if ed.shuttle_rate != 0 { "PLAYING" } else { "READY (SPACE / CLICK TO PLAY)" };
        self.mono_small.text(
            c,
            slate.x + 14,
            slate.y + 12,
            clip_name,
            FCP_BLUE,
            Some((slate.x + 14, (slate.w / 2).max(0))),
        );
        let sw_lbl = self.mono_small.measure(state_lbl);
        self.mono_small.text(
            c,
            (slate.x + slate.w - 14 - sw_lbl).max(slate.x + 14),
            slate.y + 12,
            state_lbl,
            if ed.shuttle_rate != 0 { ACCENT } else { DIM },
            Some((slate.x + 14, (slate.w - 28).max(0))),
        );

        // Live spoken word caption centered in the slate
        let word_txt = ed
            .word_at_time(playhead)
            .and_then(|i| ed.transcript.words.get(i))
            .map(|w| w.text.as_str())
            .unwrap_or("...");
        let ww = self.big.measure(word_txt);
        let wx = slate.x + ((slate.w - ww) / 2).max(8);
        let wy = slate.y + slate.h / 2 - self.big.line_height();
        self.big.text(c, wx, wy, word_txt, INK, Some((slate.x + 8, (slate.w - 16).max(0))));

        // Animated Candy Teal / Cobalt rhythm visualizer bars driven by `playhead`
        let bars = (slate.w - 48).max(0) / 8;
        let base_y = slate.y + slate.h - 26;
        let max_bh = (slate.h / 4).clamp(8, 36);
        let phase = (playhead / 25_000) as i32;
        for b in 0..bars.min(48) {
            let bx = slate.x + (slate.w - bars.min(48) * 8) / 2 + b * 8;
            let sample_t = (playhead + (b as i64 - 24) * 30_000).clamp(0, tl.duration());
            let peak_h = if let Some(peaks) = lib.peaks.get(&ed.transcript.media) {
                let v = peaks.over(sample_t, sample_t + 30_000) as i32;
                (v * max_bh / 255).max(3)
            } else {
                // Deterministic pseudo-wave when audio peaks weren't scanned
                let osc = ((b * 7 + phase * 3).abs() % 16) - 8;
                let env = if ed.word_at_time(playhead).is_some() { max_bh * 3 / 4 } else { max_bh / 4 };
                (env - osc.abs() * max_bh / 18).clamp(3, max_bh)
            };
            let bar_col = if (b - bars.min(48) / 2).abs() <= 2 {
                ACCENT
            } else if b % 2 == 0 {
                CANDY_TEAL
            } else {
                FCP_BLUE
            };
            c.fill(bx, base_y - peak_h, 5, peak_h, bar_col);
        }
        // Subtle progress bar along the bottom of the slate
        let dur = tl.duration().max(1);
        let prog_w = ((slate.w - 4) as i64 * playhead.clamp(0, dur) / dur) as i32;
        c.fill(slate.x + 2, slate.y + slate.h - 4, slate.w - 4, 2, PANEL2);
        if prog_w > 0 {
            c.fill(slate.x + 2, slate.y + slate.h - 4, prog_w, 2, PLAYHEAD);
        }
    }

    fn transport(&mut self, c: &mut Canvas, r: Rect, ed: &Editor, playhead: Micros) {
        c.fill(r.x, r.y, r.w, r.h, PANEL);
        c.fill(r.x, r.y, r.w, 1, HAIRLINE);
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

        let tb = transport_buttons(r);
        let playing = ed.shuttle_rate != 0;
        self.draw_pill(c, tb.start_btn, "|<", false, FCP_BLUE);
        self.draw_pill(c, tb.back_btn, "<", ed.shuttle_rate < 0, FCP_BLUE);
        let play_lbl = if tb.play_btn.w >= 48 {
            if playing { "Pause" } else { "Play" }
        } else if playing {
            "||"
        } else {
            ">"
        };
        self.draw_pill(c, tb.play_btn, play_lbl, playing, ACCENT);
        self.draw_pill(c, tb.fwd_btn, ">", ed.shuttle_rate > 1, FCP_BLUE);
        self.draw_pill(c, tb.end_btn, ">|", false, FCP_BLUE);

        if tb.action_btn.w > 0 {
            let has_joins = !ed.joins().is_empty();
            let act_lbl = if has_joins { "\\ Join" } else { "Clean-up" };
            self.draw_pill(c, tb.action_btn, act_lbl, has_joins, CANDY_LAVENDER);
        }
    }

    fn inspector_row(&mut self, c: &mut Canvas, r: Rect, y: &mut i32, k: &str, v: &str, col: u32) {
        let lh = self.small.line_height();
        if *y + lh > r.y + r.h - 4 {
            return;
        }
        let pad_x = r.x + 10;
        let max_w = (r.w - 18).max(0);
        self.small.text(c, pad_x, *y, k, DIM, Some((pad_x, max_w)));
        let vw = self.mono_small.measure(v);
        let vx = (r.x + r.w - 10 - vw).max(pad_x + 56);
        self.mono_small.text(c, vx, *y + 1, v, col, Some((vx, (r.x + r.w - 8 - vx).max(0))));
        *y += lh + 5;
    }

    fn inspector(&mut self, c: &mut Canvas, r: Rect, ed: &Editor, playhead: Micros) {
        c.fill(r.x, r.y, r.w, r.h, INSPECTOR_BG);
        c.fill(r.x, r.y, 1, r.h, HAIRLINE);
        let lh = self.small.line_height();
        let tl = ed.timeline();
        let fps = (tl.fps_num, tl.fps_den);

        // Header bar
        c.fill(r.x + 1, r.y, r.w - 1, 26, PANEL);
        c.fill(r.x + 1, r.y + 25, r.w - 1, 1, HAIRLINE);
        c.fill(r.x + 1, r.y, 3, 26, CANDY_LAVENDER);
        self.small.text(c, r.x + 10, r.y + 6, "Inspector", INK, Some((r.x, r.w)));

        let mut y = r.y + 34;
        let pad_x = r.x + 10;
        let max_w = (r.w - 18).max(0);
        let clip_v = tl
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .flat_map(|t| t.clips.iter())
            .find(|cl| cl.at <= playhead && playhead < cl.end())
            .or_else(|| tl.tracks.iter().flat_map(|t| t.clips.iter()).next())
            .cloned();
        let clip_a = tl
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .flat_map(|t| t.clips.iter())
            .find(|cl| cl.at <= playhead && playhead < cl.end())
            .cloned();

        if let Some(cl) = clip_v {
            let name = tl.media(cl.media).map_or("Clip", |m| short(&m.path));
            self.small.text(c, pad_x, y, name, FCP_BLUE, Some((pad_x, max_w)));
            y += lh + 6;
            c.fill(pad_x, y - 3, max_w, 1, LINE);
            y += 4;
            self.inspector_row(c, r, &mut y, "Scale", "100.0%", INK);
            self.inspector_row(c, r, &mut y, "Pos", "0, 0 px", INK);
            self.inspector_row(c, r, &mut y, "Speed", &format!("{}%", cl.speed / 10), INK);
            self.inspector_row(
                c,
                r,
                &mut y,
                "State",
                if cl.enabled { "Enabled" } else { "Disabled" },
                if cl.enabled { FCP_GREEN } else { ACCENT },
            );
        } else {
            self.small.text(c, pad_x, y, "No active clip", DIM, Some((pad_x, max_w)));
            y += lh + 6;
        }

        if let Some(ac) = clip_a {
            self.inspector_row(c, r, &mut y, "Volume", &format!("{}%", ac.volume / 10), CANDY_TEAL);
        }
        self.inspector_row(c, r, &mut y, "FPS", &format!("{}/{}", tl.fps_num, tl.fps_den), DIM);

        if ed.in_point.is_some() || ed.out_point.is_some() {
            c.fill(pad_x, y, max_w, 1, LINE);
            y += 6;
            if let Some(inp) = ed.in_point {
                self.inspector_row(c, r, &mut y, "In", &timecode(inp, fps), RANGE_AMBER);
            }
            if let Some(outp) = ed.out_point {
                self.inspector_row(c, r, &mut y, "Out", &timecode(outp, fps), RANGE_AMBER);
            }
        }
    }

    fn cards(&mut self, c: &mut Canvas, r: Rect, ed: &Editor) {
        if r.w < 40 || r.h < 20 {
            return;
        }
        let lh = self.small.line_height();
        let pending = ed.pending();
        c.fill(r.x, r.y, r.w, 1, LINE);
        let title = if pending.is_empty() {
            String::from("Suggestions: none waiting. Click or press C for clean-up.")
        } else {
            format!("Suggestions ({})", pending.len())
        };
        let title_col = if pending.is_empty() { DIM } else { CANDY_LAVENDER };
        self.small.text(c, r.x, r.y + 4, &title, title_col, Some((r.x, r.w)));
        let mut y = r.y + lh + 8;
        let card_h = lh * 2 + 10;
        for (n, id) in pending.iter().enumerate() {
            if y + card_h > r.y + r.h {
                break;
            }
            let Some(s) = ed.suggestions.get(*id) else { continue };
            c.fill(r.x, y, r.w, card_h, CARD);
            c.frame(r.x, y, r.w, card_h, HAIRLINE);
            c.fill(r.x, y, 3, card_h, if n == ed.card { CANDY_LAVENDER } else { LINE });
            self.small.text(c, r.x + 10, y + 4, &s.summary, INK, Some((r.x + 10, (r.w - 16).max(0))));
            let hint = if n == ed.card { "Enter takes, Esc skips" } else { "Click to focus" };
            let hint_max = if r.w >= 150 { (r.w - 100).max(0) } else { (r.w - 16).max(0) };
            self.small.text(c, r.x + 10, y + 6 + lh, hint, DIM, Some((r.x + 10, hint_max)));
            if r.w >= 150 {
                let btn_h = 16;
                let btn_y = y + card_h - btn_h - 4;
                let skip_r = Rect { x: r.x + r.w - 42, y: btn_y, w: 36, h: btn_h };
                let take_r = Rect { x: skip_r.x - 44, y: btn_y, w: 40, h: btn_h };
                self.draw_pill(c, take_r, "Take", n == ed.card, FCP_GREEN);
                self.draw_pill(c, skip_r, "Skip", false, LINE);
            }
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
        let upp = self.us_per_px_in(tl, a.tracks.w);
        // Keep the playhead on screen.
        let span = a.tracks.w.max(1) as Micros * upp;
        if self.zoom == 0 {
            self.start = 0;
        } else if playhead < self.start || playhead >= self.start + span {
            self.start = (playhead - span / 4).max(0);
        }
        let start = self.start;
        let x_of = |t: Micros| a.tracks.x + ((t - start) / upp) as i32;

        let active_v_clip = tl
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .flat_map(|t| t.clips.iter())
            .find(|cl| cl.at <= playhead && playhead < cl.end())
            .map(|cl| cl.id);
        let active_a_clip = tl
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .flat_map(|t| t.clips.iter())
            .find(|cl| cl.at <= playhead && playhead < cl.end())
            .map(|cl| cl.id);
        let thumbs_count = lib.thumbs.values().map(|v| v.len()).sum();
        let peaks_count = lib.peaks.values().map(|p| p.v.len()).sum();

        let tl_rect = Rect {
            x: a.heads.x.max(0),
            y: (a.ruler.y - 1).max(0),
            w: (a.heads.w + a.ruler.w).min(c.width as i32 - a.heads.x.max(0)).max(0),
            h: (a.ruler.h + a.tracks.h + 1).min(c.height as i32 - (a.ruler.y - 1).max(0)).max(0),
        };
        let key = TimelineCacheKey {
            w: c.width as i32,
            h: c.height as i32,
            layout: self.layout,
            show_vu: self.vu.visible,
            show_inspector: self.show_inspector,
            zoom: self.zoom,
            start,
            head: ed.history.head(),
            in_point: ed.in_point,
            out_point: ed.out_point,
            active_v_clip,
            active_a_clip,
            thumbs_count,
            peaks_count,
        };
        let cache_len = (tl_rect.w as usize).saturating_mul(tl_rect.h as usize);

        if self.timeline_key == Some(key) && self.timeline_px.len() == cache_len && cache_len > 0 {
            let rw = tl_rect.w as usize;
            let x0 = tl_rect.x as usize;
            for row in 0..tl_rect.h as usize {
                let dst = (tl_rect.y as usize + row) * c.stride + x0;
                c.px[dst..dst + rw].copy_from_slice(&self.timeline_px[row * rw..(row + 1) * rw]);
            }
        } else {
            c.fill(a.heads.x, a.ruler.y, a.heads.w, a.ruler.h, PANEL);
            c.fill(a.heads.x, a.heads.y, a.heads.w, a.heads.h, PANEL);
            c.fill(a.tracks.x, a.tracks.y, a.tracks.w, a.tracks.h, TIMELINE_WELL);
            c.fill(a.heads.x, a.ruler.y - 1, a.heads.w + a.ruler.w, 1, HAIRLINE);
            c.fill(a.heads.x + a.heads.w - 1, a.ruler.y, 1, a.ruler.h + a.heads.h, HAIRLINE);

            // The ruler: a label every `step`, minor ticks between.
            c.fill(a.ruler.x, a.ruler.y, a.ruler.w, a.ruler.h, PANEL2);
            c.fill(a.ruler.x, a.ruler.y + a.ruler.h - 1, a.ruler.w, 1, HAIRLINE);
            let step = ruler_step(upp, 110);
            let minor = step / 5;
            let mut t = start - start % minor.max(1);
            let end = start + span;
            while t <= end {
                let x = x_of(t);
                if x >= a.ruler.x && x < a.ruler.x + a.ruler.w {
                    if t % step == 0 {
                        c.fill(x, a.ruler.y + 8, 1, a.ruler.h - 8, DIM);
                        let label = timecode(t, (tl.fps_num, tl.fps_den));
                        self.mono_small.text(c, x + 4, a.ruler.y + 4, &label, DIM, Some((a.ruler.x, a.ruler.w)));
                    } else {
                        c.fill(x, a.ruler.y + a.ruler.h - 6, 1, 5, LINE);
                    }
                }
                t += minor.max(1);
            }

            // Cycle/In-Out Range Amber bar on ruler
            match (ed.in_point, ed.out_point) {
                (Some(inp), Some(outp)) if outp >= inp => {
                    let x0 = x_of(inp).clamp(a.ruler.x, a.ruler.x + a.ruler.w);
                    let x1 = x_of(outp).clamp(a.ruler.x, a.ruler.x + a.ruler.w);
                    if x1 > x0 {
                        c.tint(x0, a.ruler.y, x1 - x0, a.ruler.h, RANGE_AMBER, 55);
                        c.fill(x0, a.ruler.y, x1 - x0, 3, RANGE_AMBER);
                        c.fill(x0, a.ruler.y, 2, a.ruler.h, RANGE_AMBER);
                        c.fill((x1 - 2).max(a.ruler.x), a.ruler.y, 2, a.ruler.h, RANGE_AMBER);
                    }
                }
                (Some(inp), _) => {
                    let x0 = x_of(inp);
                    if x0 >= a.ruler.x && x0 < a.ruler.x + a.ruler.w {
                        c.fill(x0, a.ruler.y, 2, a.ruler.h, RANGE_AMBER);
                        c.fill(x0, a.ruler.y, 6, 3, RANGE_AMBER);
                    }
                }
                (None, Some(outp)) => {
                    let x1 = x_of(outp);
                    if x1 >= a.ruler.x && x1 < a.ruler.x + a.ruler.w {
                        c.fill(x1, a.ruler.y, 2, a.ruler.h, RANGE_AMBER);
                        c.fill((x1 - 4).max(a.ruler.x), a.ruler.y, 6, 3, RANGE_AMBER);
                    }
                }
                _ => {}
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
                // Header: the track's name, a coloured role bar, and its switches.
                c.fill(a.heads.x, y, a.heads.w - 1, rh - 2, PANEL2);
                let role_col = if track.kind == TrackKind::Video { FCP_BLUE } else { FCP_GREEN };
                c.fill(a.heads.x, y, 4, rh - 2, role_col);
                self.small.text(c, a.heads.x + 10, y + 4, &label, INK, None);
                self.small.text(c, a.heads.x + 34, y + 4, &track.name, DIM, Some((a.heads.x + 34, (a.heads.w - 40).max(0))));
                let v_hidden = track.kind == TrackKind::Video && track.clips.first().is_some_and(|cl| !cl.enabled);
                self.switches(c, a.heads.x + 10, y + rh - 22, track.kind, track.muted, v_hidden);
                // Clips.
                for clip in &track.clips {
                    let (x0, x1) = (x_of(clip.at), x_of(clip.end()));
                    if x1 < a.tracks.x || x0 > a.tracks.x + a.tracks.w {
                        continue;
                    }
                    let r = Rect { x: x0, y: y + 1, w: (x1 - x0 - 1).max(2), h: rh - 4 };
                    let vis = Rect { x: a.tracks.x, y: r.y, w: a.tracks.w, h: r.h };
                    let name = tl.media(clip.media).map_or("", |m| short(&m.path));
                    let active = Some(clip.id) == active_v_clip || Some(clip.id) == active_a_clip;
                    match track.kind {
                        TrackKind::Video => self.video_clip(c, r, vis, clip, colour, lib, upp, start, a.tracks.x, name, active),
                        TrackKind::Audio => self.sound_clip(c, r, vis, clip, colour, lib, upp, start, a.tracks.x, name, active),
                    }
                }
                c.fill(a.tracks.x, y + rh - 2, a.tracks.w, 1, HAIRLINE);
                y += rh;
            }
            // Joins left by cuts.
            for j in ed.joins() {
                let x = x_of(j);
                if x >= a.tracks.x && x < a.tracks.x + a.tracks.w {
                    c.fill(x - 1, a.ruler.y + a.ruler.h - 6, 3, 6, ACCENT);
                    c.fill(x - 1, a.tracks.y, 2, (y - a.tracks.y).max(0), ACCENT);
                }
            }
            if cache_len > 0 && (self.timeline_px.len() == cache_len || self.timeline_px.try_reserve(cache_len.saturating_sub(self.timeline_px.len())).is_ok()) {
                self.timeline_px.resize(cache_len, 0);
                let rw = tl_rect.w as usize;
                let x0 = tl_rect.x as usize;
                for row in 0..tl_rect.h as usize {
                    let src = (tl_rect.y as usize + row) * c.stride + x0;
                    self.timeline_px[row * rw..(row + 1) * rw].copy_from_slice(&c.px[src..src + rw]);
                }
                self.timeline_key = Some(key);
            }
        }

        // Playhead drawn dynamically on top of the cached timeline in <0.01 ms!
        let px = x_of(playhead);
        if px >= a.tracks.x && px < a.tracks.x + a.tracks.w {
            let ph_col = if ed.active_tool == ToolKind::Blade { ACCENT } else { PLAYHEAD };
            c.fill(px, a.ruler.y, 2, a.ruler.h + a.tracks.h, ph_col);
            for i in 0..6 {
                c.fill(px - 5 + i, a.ruler.y + i, 11 - 2 * i, 1, ph_col);
            }
        }
    }

    /// A track's switches: an eye for pictures, M (mute) and S (solo) for sound.
    fn switches(&mut self, c: &mut Canvas, x: i32, y: i32, kind: TrackKind, muted: bool, v_hidden: bool) {
        let boxes: &[&str] = if kind == TrackKind::Video { &["eye"] } else { &["M", "S"] };
        for (i, b) in boxes.iter().enumerate() {
            let bx = x + i as i32 * 24;
            let on = (*b == "M" && muted) || (*b == "eye" && v_hidden);
            let active_bg = if *b == "eye" { CANDY_LAVENDER } else { ACCENT };
            c.fill(bx, y, 20, 16, if on { active_bg } else { TIMELINE_WELL });
            c.frame(bx, y, 20, 16, LINE);
            if *b == "eye" {
                let eye_col = if v_hidden { INK } else { FCP_BLUE };
                c.fill(bx + 4, y + 7, 12, 2, eye_col);
                c.fill(bx + 7, y + 5, 6, 6, eye_col);
                c.fill(bx + 9, y + 7, 2, 2, TIMELINE_WELL);
            } else {
                self.small.text(c, bx + 6, y + 1, b, if on { INK } else { DIM }, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn video_clip(&mut self, c: &mut Canvas, r: Rect, vis: Rect, clip: &Clip, colour: u32, lib: &Library, upp: Micros, start: Micros, x0: i32, name: &str, active: bool) {
        let lx = r.x.max(vis.x);
        let rx = (r.x + r.w).min(vis.x + vis.w);
        if rx <= lx {
            return;
        }
        c.fill(lx, r.y, rx - lx, r.h, colour);
        let label_h = 15;
        let strip = Rect { x: r.x, y: r.y + label_h, w: r.w, h: r.h - label_h - 1 };
        // The filmstrip: pictures side by side, each from the source time under it.
        if strip.h > 6 {
            let tw = (strip.h * 16 / 9).max(8);
            let clip_vis = Rect { x: lx, y: strip.y, w: rx - lx, h: strip.h };
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
        // Keep exact `shade(colour, 0, 70)` label strip for gate compatibility, plus 2px role bar.
        c.fill(lx, r.y, rx - lx, label_h, shade(colour, 0, 70));
        c.fill(lx, r.y, rx - lx, 2, if clip.enabled { FCP_BLUE } else { CANDY_LAVENDER });
        self.small.text(c, r.x + 5, r.y + 1, name, if clip.enabled { INK } else { DIM }, Some((lx + 2, (r.w - 8).max(0))));
        if !clip.enabled {
            c.tint(lx, r.y + 2, rx - lx, (r.h - 2).max(0), TIMELINE_WELL, 140);
        }
        let border = if active { FCP_BLUE } else { shade(colour, 0xffffff, 90) };
        c.frame(r.x, r.y, r.w, r.h, border);
        // Subtle rounded corner notches in TIMELINE_WELL
        if r.w >= 6 && r.h >= 6 {
            c.fill(r.x, r.y, 1, 1, TIMELINE_WELL);
            c.fill(r.x + r.w - 1, r.y, 1, 1, TIMELINE_WELL);
            c.fill(r.x, r.y + r.h - 1, 1, 1, TIMELINE_WELL);
            c.fill(r.x + r.w - 1, r.y + r.h - 1, 1, 1, TIMELINE_WELL);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sound_clip(&mut self, c: &mut Canvas, r: Rect, vis: Rect, clip: &Clip, colour: u32, lib: &Library, upp: Micros, start: Micros, x0: i32, name: &str, active: bool) {
        let (lx, rx) = (r.x.max(vis.x), (r.x + r.w).min(vis.x + vis.w));
        if rx <= lx {
            return;
        }
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
                // Candy Teal peak highlight crest just outside tall waveform columns
                if len >= 3 && mid - len - 1 >= r.y + label_h {
                    c.fill(x, mid - len - 1, 1, 1, CANDY_TEAL);
                    if mid + len + 1 < r.y + r.h - 1 {
                        c.fill(x, mid + len + 1, 1, 1, CANDY_TEAL);
                    }
                }
            }
        }
        c.fill(lx, mid, rx - lx, 1, shade(CANDY_TEAL, colour, 150));
        c.fill(lx, r.y, rx - lx, 2, if clip.enabled { FCP_GREEN } else { CANDY_LAVENDER });
        self.small.text(c, r.x + 5, r.y + 1, name, if clip.enabled { INK } else { DIM }, Some((lx + 2, (r.w - 8).max(0))));
        if !clip.enabled {
            c.tint(lx, r.y + 2, rx - lx, (r.h - 2).max(0), TIMELINE_WELL, 140);
        }
        let border = if active { FCP_BLUE } else { shade(colour, 0xffffff, 90) };
        c.frame(r.x, r.y, r.w, r.h, border);
        if r.w >= 6 && r.h >= 6 {
            c.fill(r.x, r.y, 1, 1, TIMELINE_WELL);
            c.fill(r.x + r.w - 1, r.y, 1, 1, TIMELINE_WELL);
            c.fill(r.x, r.y + r.h - 1, 1, 1, TIMELINE_WELL);
            c.fill(r.x + r.w - 1, r.y + r.h - 1, 1, 1, TIMELINE_WELL);
        }
    }
}
