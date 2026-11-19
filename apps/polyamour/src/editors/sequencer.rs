// SPDX-License-Identifier: GPL-3.0-only
//! The Video Sequencer editor: the preview of the composited frame on top,
//! the strip timeline below (channels up, frames across, the scrubbing
//! ruler) and the sidebar with the active strip's settings.
//!
//! Click selects a strip (Shift extends), dragging moves the selection
//! (channels and frames), dragging a handle trims, a drag on empty space
//! box-selects. G grabs, E extends, K / Shift+K split soft / hard, Shift+D
//! duplicates, X deletes, Ctrl+G / Ctrl+Alt+G make or separate a meta strip,
//! H / Alt+H / Shift+H mute, Shift+L / Shift+Alt+L lock, Shift+S snaps to
//! the playhead, Shift+A adds, Home frames all, N toggles the sidebar.
use super::header_menus;
use crate::anim;
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::editors::properties::layout::L;
use crate::editors::properties::picker::{self, Picker};
use crate::gpencil::ui::{self as gui, GpOwner};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::sequencer::{Effect, Sequencer, StripKind};
use std::collections::HashMap;

const RULER_H: i32 = 22;
const SIDE_W: i32 = 300;
const HANDLE_W: i32 = 6;

enum Drag {
    None,
    Scrub,
    /// Moving the selection: press position, the editor before, whether the undo step was pushed.
    Move { x0: i32, y0: i32, orig: Box<Sequencer>, grab: bool },
    Handle { i: usize, right: bool, orig: Box<Sequencer> },
    Box { x0: i32, y0: i32, x1: i32, y1: i32, extend: bool },
}

pub struct State {
    id: u64,
    pub x0: f32,
    pub ppf: f32,
    /// Bottom channel shown (1-based, fractional when scrolled).
    pub ch0: f32,
    /// Pixels per channel.
    pub pch: f32,
    framed: bool,
    drag: Drag,
    mouse: (i32, i32),
    /// 0: preview over the timeline, 1: timeline only, 2: preview only.
    pub view: u8,
    pub show_sidebar: bool,
    preview: Option<(u64, i32, u32, u32, Vec<u8>)>,
    open: HashMap<String, bool>,
    undo_for: Option<Option<u64>>,
    picker: Option<Picker>,
    path: String,
    strips_rect: Rect,
}

impl Default for State {
    fn default() -> Self {
        State {
            id: anim::editor_id(),
            x0: -5.0,
            ppf: 4.0,
            ch0: 1.0,
            pch: 28.0,
            framed: false,
            drag: Drag::None,
            mouse: (0, 0),
            view: 0,
            show_sidebar: true,
            preview: None,
            open: HashMap::new(),
            undo_for: None,
            picker: None,
            path: String::new(),
            strips_rect: Rect::default(),
        }
    }
}

impl State {
    fn fx(&self, f: f32) -> f32 {
        self.strips_rect.x as f32 + (f - self.x0) * self.ppf
    }
    fn frame_at(&self, x: i32) -> f32 {
        self.x0 + (x - self.strips_rect.x) as f32 / self.ppf
    }
    /// Top of channel `c`'s row.
    fn cy(&self, c: f32) -> f32 {
        self.strips_rect.bottom() as f32 - (c - self.ch0 + 1.0) * self.pch
    }
    fn channel_at(&self, y: i32) -> u32 {
        ((self.strips_rect.bottom() - y) as f32 / self.pch + self.ch0).floor().max(1.0) as u32
    }
}

fn add_menu(_d: &Doc) -> Vec<MenuItem> {
    let a = |l: &str, k: &str| MenuItem::op(l, "", Op::with("sequencer.strip_add", k));
    let fx: Vec<MenuItem> = Effect::IDS.iter().filter_map(|id| Effect::new(id).map(|e| MenuItem::op(e.label(), "", Op::with("sequencer.effect_strip_add", id)))).collect();
    vec![
        a("Scene", "SCENE"),
        a("Color", "COLOR"),
        a("Text", "TEXT"),
        a("Adjustment Layer", "ADJUSTMENT"),
        MenuItem::Separator,
        MenuItem::Sub("Effect Strip".into(), fx),
        MenuItem::Separator,
        MenuItem::Heading("Image, Movie, Sound: sidebar > Add Media".into()),
    ]
}

fn select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("All", "A", Op::with("sequencer.select_all", "SELECT")),
        MenuItem::op("None", "Alt A", Op::with("sequencer.select_all", "DESELECT")),
        MenuItem::op("Invert", "Ctrl I", Op::with("sequencer.select_all", "INVERT")),
    ]
}

fn strip_menu(_d: &Doc) -> Vec<MenuItem> {
    let o = |l: &str, k: &str, id: &str, a: &str| MenuItem::op(l, k, Op::with(id, a));
    vec![
        o("Split", "K", "sequencer.split", "SOFT"),
        o("Hold Split", "Shift K", "sequencer.split", "HARD"),
        MenuItem::Separator,
        o("Duplicate", "Shift D", "sequencer.duplicate", ""),
        o("Delete", "X", "sequencer.delete", ""),
        MenuItem::Separator,
        o("Snap to Playhead", "Shift S", "sequencer.snap", ""),
        MenuItem::Separator,
        o("Make Meta", "Ctrl G", "sequencer.meta_make", ""),
        o("UnMeta", "Ctrl Alt G", "sequencer.meta_separate", ""),
        MenuItem::Separator,
        o("Mute", "H", "sequencer.mute", ""),
        o("Unmute", "Alt H", "sequencer.unmute", ""),
        o("Mute Unselected", "Shift H", "sequencer.mute", "UNSELECTED"),
        o("Lock", "Shift L", "sequencer.lock", ""),
        o("Unlock", "Shift Alt L", "sequencer.unlock", ""),
    ]
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let id = s.id;
    let view = move |_d: &Doc| {
        vec![
            MenuItem::op("Sidebar", "N", anim::mail("seq.sidebar", id, "")),
            MenuItem::op("Frame All", "Home", anim::mail("seq.frame_all", id, "")),
            MenuItem::Separator,
            MenuItem::op("Sequencer & Preview", "", anim::mail("seq.view", id, "0")),
            MenuItem::op("Sequencer", "", anim::mail("seq.view", id, "1")),
            MenuItem::op("Preview", "", anim::mail("seq.view", id, "2")),
        ]
    };
    let end = header_menus(ui, hr, hr.x + 4, &[("View", &view), ("Select", &select_menu), ("Add", &add_menu), ("Strip", &strip_menu)], d);
    let on = d.scene.sequence_editor.as_ref().is_none_or(|q| q.use_sequencer);
    let r = Rect::new(end + 12, hr.y + 3, 150, hr.h - 6);
    let mut v = on;
    if ui.checkbox(r, "Use for Render", &mut v) {
        d.queue.push(Op::new("sequencer.use_toggle"));
    }
}

fn frame_all(s: &mut State, d: &Doc) {
    let (a, b) = d.scene.sequence_editor.as_ref().and_then(|q| q.range()).unwrap_or((d.scene.frame_start as f32, d.scene.frame_end as f32));
    let span = (b - a).max(10.0) * 1.1;
    s.ppf = s.strips_rect.w.max(100) as f32 / span;
    s.x0 = a - (span - (b - a)) / 2.0;
    s.ch0 = 1.0;
}

fn strip_color(k: &StripKind) -> u32 {
    match k {
        StripKind::Color { .. } => 0x8f8f5a,
        StripKind::Image { .. } => 0x8a6b9f,
        StripKind::Movie { .. } => 0x4a6f9a,
        StripKind::Sound { .. } => 0x3f8a8a,
        StripKind::Scene { .. } => 0x6f9a52,
        StripKind::Text(_) => 0x9a7a4a,
        StripKind::Adjustment => 0x9a9a9a,
        StripKind::Effect { .. } => 0x9a5a7a,
        StripKind::Meta { .. } => 0x5a5a8a,
    }
}

/// Which strip (and handle) is under a point.
fn hit(s: &State, q: &Sequencer, x: i32, y: i32) -> Option<(usize, Option<bool>)> {
    let ch = s.channel_at(y);
    let f = s.frame_at(x);
    let i = q.strips.iter().enumerate().rev().find(|(_, st)| st.channel == ch && f >= st.final_start() - 2.0 / s.ppf && f < st.final_end() + 2.0 / s.ppf).map(|(i, _)| i)?;
    let st = &q.strips[i];
    let (x0, x1) = (s.fx(st.final_start()), s.fx(st.final_end()));
    let hw = (HANDLE_W as f32).min((x1 - x0) / 3.0);
    let side = if (x as f32) < x0 + hw {
        Some(false)
    } else if (x as f32) > x1 - hw {
        Some(true)
    } else {
        None
    };
    Some((i, side))
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    for (op, arg) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "seq.sidebar" => s.show_sidebar = !s.show_sidebar,
            "seq.frame_all" => frame_all(s, d),
            "seq.view" => s.view = arg.parse().unwrap_or(0),
            _ => {}
        }
    }
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    let (side, r) = if s.show_sidebar && r.w > SIDE_W + 300 { r.take_right(SIDE_W) } else { (Rect::default(), r) };
    let (prev, tl) = match s.view {
        1 => (Rect::default(), r),
        2 => (r, Rect::default()),
        _ => r.take_top((r.h as f32 * 0.5) as i32),
    };
    if !side.is_empty() {
        sidebar(s, d, ui, side);
    }
    if !prev.is_empty() {
        preview(s, d, ui, prev);
    }
    if !tl.is_empty() {
        timeline(s, d, ui, tl);
    }
    if let Some(pk) = &mut s.picker {
        if !picker::draw(pk, d, ui, side, &mut s.undo_for) {
            s.picker = None;
        }
    }
}

/// The composited frame, rendered at a reduced size and cached.
fn preview(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    ui.p.fill(r, 0x1a1a1a);
    let rw = d.settings.resolution_x.max(1) as f32;
    let rh = d.settings.resolution_y.max(1) as f32;
    let k = ((r.w - 16) as f32 / rw).min((r.h - 16) as f32 / rh).max(0.01);
    let (w, h) = ((rw * k) as u32, (rh * k) as u32);
    if w < 4 || h < 4 {
        return;
    }
    // Render at most 480 px wide; the blit scales up.
    let scale = (480.0 / w as f32).min(1.0);
    let (pw, ph) = (((w as f32 * scale) as u32).max(2), ((h as f32 * scale) as u32).max(2));
    let fresh = s.preview.as_ref().is_some_and(|p| p.0 == d.generation && p.1 == d.frame && p.2 == pw && p.3 == ph);
    if !fresh {
        let img = match d.scene.sequence_editor.as_ref() {
            Some(q) => {
                let base = d.settings.render_scene(&d.scene);
                let mut st = d.settings.render_settings();
                st.samples = 4;
                st.denoise = false;
                let (anim, mb) = (&d.anim, d.settings.motion_blur);
                crate::seqedit::render_seq(q, d.frame, pw, ph, &|f| crate::anim::scene_at(anim, &base, f as f32, mb), &st, None)
            }
            None => sex_3d::scene::Image { name: String::new(), width: pw, height: ph, pixels: vec![[0.0; 4]; (pw * ph) as usize] },
        };
        let rgba = sex_3d::render::to_display8(&img, &d.settings.color_management());
        s.preview = Some((d.generation, d.frame, pw, ph, rgba));
    }
    let dst = Rect::new(r.x + (r.w - w as i32) / 2, r.y + (r.h - h as i32) / 2, w as i32, h as i32);
    // Checker behind transparent pixels.
    ui.p.fill(dst, 0x2a2a2a);
    if let Some((_, _, pw, ph, rgba)) = &s.preview {
        let (pw, ph) = (*pw as usize, *ph as usize);
        let mut up = vec![0u8; dst.w as usize * dst.h as usize * 4];
        for y in 0..dst.h as usize {
            let sy = (y * ph / dst.h as usize).min(ph - 1);
            for x in 0..dst.w as usize {
                let sx = (x * pw / dst.w as usize).min(pw - 1);
                let si = (sy * pw + sx) * 4;
                let a = rgba[si + 3] as u32;
                let di = (y * dst.w as usize + x) * 4;
                for c in 0..3 {
                    // Over the dark backdrop.
                    up[di + c] = ((rgba[si + c] as u32 * a + 0x2a * (255 - a)) / 255) as u8;
                }
                up[di + 3] = 255;
            }
        }
        ui.p.blit_rgba(dst, &up, dst.w as usize, dst.h as usize);
    }
    ui.p.frame(dst, 0x000000);
}

fn timeline(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    let (ruler, sr) = r.take_top(RULER_H);
    s.strips_rect = sr;
    if !s.framed && sr.w > 0 {
        s.framed = true;
        frame_all(s, d);
    }
    handle_input(s, d, ui, ruler, sr);
    let p = &mut *ui.p;
    p.fill(sr, t::TIMELINE);
    // Channel rows.
    let mut c = s.ch0.floor();
    while s.cy(c) + s.pch > sr.y as f32 {
        let y = s.cy(c) as i32;
        if (c as i32) % 2 == 0 {
            p.fill(Rect::new(sr.x, y, sr.w, s.pch as i32), 0x2c2c2c);
        }
        c += 1.0;
    }
    // Frame grid.
    let step = nice_step(s.ppf);
    let mut f = (s.x0 / step).floor() * step;
    while s.fx(f) < sr.right() as f32 {
        p.fill(Rect::new(s.fx(f) as i32, sr.y, 1, sr.h), 0x353535);
        f += step;
    }
    let xs = s.fx(d.scene.frame_start as f32) as i32;
    let xe = s.fx(d.scene.frame_end as f32 + 1.0) as i32;
    if xs > sr.x {
        p.fill_alpha(Rect::new(sr.x, sr.y, xs - sr.x, sr.h), 0x000000, 70);
    }
    if xe < sr.right() {
        p.fill_alpha(Rect::new(xe, sr.y, sr.right() - xe, sr.h), 0x000000, 70);
    }
    // Strips.
    p.push_clip(sr);
    if let Some(q) = d.scene.sequence_editor.as_ref() {
        for (i, st) in q.strips.iter().enumerate() {
            let x0 = s.fx(st.final_start());
            let x1 = s.fx(st.final_end());
            let y = s.cy(st.channel as f32);
            let rr = Rect::new(x0 as i32, y as i32 + 2, (x1 - x0).max(2.0) as i32, s.pch as i32 - 4);
            let base = strip_color(&st.kind);
            p.round_rect(rr, 3, if st.mute { 0x444444 } else { base });
            // Hidden content (soft split) as a thin band.
            let (c0, c1) = (s.fx(st.start), s.fx(st.start + st.length));
            if c0 < x0 - 1.0 || c1 > x1 + 1.0 {
                p.fill_alpha(Rect::new(c0 as i32, rr.bottom() - 3, (c1 - c0) as i32, 3), base, 90);
            }
            if let StripKind::Sound { waveform, .. } = &st.kind {
                let n = waveform.len().max(1);
                let mid = rr.y + rr.h / 2;
                for px in 0..rr.w {
                    let f = st.final_start() + px as f32 / s.ppf;
                    let k = (((f - st.start) / st.length.max(1.0)) * n as f32) as usize;
                    let a = waveform.get(k.min(n - 1)).copied().unwrap_or(0.0);
                    let hh = (a * rr.h as f32 * 0.45) as i32;
                    p.fill(Rect::new(rr.x + px, mid - hh, 1, hh * 2 + 1), 0x2c5f5f);
                }
            }
            let sel = st.select;
            if sel {
                p.round_frame(rr, 3, if q.active == Some(i) { 0xffffff } else { 0xff9f40 });
                if st.select_left || st.select_right {
                    let hx = if st.select_left { rr.x } else { rr.right() - HANDLE_W };
                    p.fill(Rect::new(hx, rr.y, HANDLE_W, rr.h), 0xffffff);
                }
            } else {
                p.round_frame(rr, 3, 0x1a1a1a);
            }
            if st.lock {
                p.fill_alpha(rr, 0x000000, 60);
            }
            let label = format!("{} | {}", st.name, st.duration() as i32);
            p.label(Rect::new(rr.x + 4, rr.y, rr.w - 8, rr.h), &label, if st.mute { t::TEXT_DIM } else { 0x101010 }, Align::Left);
        }
    }
    if let Drag::Box { x0, y0, x1, y1, .. } = s.drag {
        let b = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
        p.fill_alpha(b, 0xffffff, 18);
        p.frame(b, 0xffffff);
    }
    p.pop_clip();
    // Ruler and playhead.
    p.fill(ruler, 0x232323);
    let mut f = (s.x0 / step).floor() * step;
    p.push_clip(ruler);
    while s.fx(f) < ruler.right() as f32 + 40.0 {
        let x = s.fx(f) as i32;
        let lab = format!("{}", f as i32);
        let w = p.text_width(&lab);
        p.text(x - w / 2, ruler.y + 4, &lab, t::TEXT_DIM);
        f += step;
    }
    p.pop_clip();
    let px = s.fx(d.frame as f32) as i32;
    p.push_clip(r);
    p.fill(Rect::new(px - 1, sr.y, 2, sr.h), t::PLAYHEAD);
    let lab = format!("{}", d.frame);
    let w = p.text_width(&lab) + 12;
    let bx = Rect::new(px - w / 2, ruler.y + 2, w, ruler.h - 4);
    p.round_rect(bx, 4, t::PLAYHEAD);
    p.label(bx, &lab, t::TEXT_HI, Align::Center);
    p.pop_clip();
}

fn nice_step(ppf: f32) -> f32 {
    for s in [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 250.0, 500.0, 1000.0] {
        if s * ppf >= 50.0 {
            return s;
        }
    }
    5000.0
}

fn handle_input(s: &mut State, d: &mut Doc, ui: &mut Ui, ruler: Rect, sr: Rect) {
    let Some(inp) = ui.input else { return };
    match inp {
        Input::Down { x, y, button: Button::Left, .. } if ruler.contains(x, y) => {
            ui.input = None;
            d.frame = s.frame_at(x).round() as i32;
            s.drag = Drag::Scrub;
        }
        Input::Down { x, y, button: Button::Left, mods } if sr.contains(x, y) => {
            ui.input = None;
            if let Drag::Move { grab: true, .. } = s.drag {
                // A click confirms G.
                s.drag = Drag::None;
                return;
            }
            let q = crate::seqedit::seq(d);
            match hit(s, q, x, y) {
                Some((i, side)) => {
                    if !mods.shift && !q.strips[i].select {
                        q.select_all(Some(false));
                    }
                    let st = &mut q.strips[i];
                    if mods.shift && side.is_none() {
                        st.select = !st.select;
                    } else {
                        st.select = true;
                    }
                    st.select_left = side == Some(false);
                    st.select_right = side == Some(true);
                    q.active = Some(i);
                    let orig = Box::new(q.clone());
                    s.drag = match side {
                        Some(right) => Drag::Handle { i, right, orig },
                        None => Drag::Move { x0: x, y0: y, orig, grab: false },
                    };
                }
                None => {
                    if !mods.shift {
                        q.select_all(Some(false));
                    }
                    s.drag = Drag::Box { x0: x, y0: y, x1: x, y1: y, extend: mods.shift };
                }
            }
            d.touch();
        }
        Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } if matches!(s.drag, Drag::Move { .. } | Drag::Handle { .. }) => {
            ui.input = None;
            match core::mem::replace(&mut s.drag, Drag::None) {
                Drag::Move { orig, .. } | Drag::Handle { orig, .. } => {
                    d.scene.sequence_editor = Some(*orig);
                    d.touch();
                }
                _ => {}
            }
        }
        Input::Move { x, y } => match &mut s.drag {
            Drag::Scrub => d.frame = s.frame_at(x).round() as i32,
            Drag::Move { x0, y0, orig, .. } => {
                let df = ((x - *x0) as f32 / s.ppf).round();
                let dc = ((*y0 - y) as f32 / s.pch).round() as i32;
                let mut q = (**orig).clone();
                q.translate(df, dc);
                d.scene.sequence_editor = Some(q);
                d.touch();
            }
            Drag::Handle { i, right, orig } => {
                let mut q = (**orig).clone();
                q.set_handle(*i, *right, s.frame_at(x).round());
                q.sync_effects();
                d.scene.sequence_editor = Some(q);
                d.touch();
            }
            Drag::Box { x1, y1, .. } => {
                *x1 = x;
                *y1 = y;
            }
            Drag::None => {}
        },
        Input::Up { button: Button::Left, .. } => match core::mem::replace(&mut s.drag, Drag::None) {
            Drag::Move { orig, grab: false, .. } | Drag::Handle { orig, .. } => {
                if d.scene.sequence_editor.as_ref() != Some(&*orig) {
                    // One undo step holding the state before the drag.
                    let now = d.scene.sequence_editor.replace(*orig);
                    d.push_undo("Move Strips");
                    d.scene.sequence_editor = now;
                    d.touch();
                }
            }
            m @ Drag::Move { grab: true, .. } => s.drag = m,
            Drag::Box { x0, y0, x1, y1, extend } => {
                let (f0, f1) = (s.frame_at(x0.min(x1)), s.frame_at(x0.max(x1)));
                let (c0, c1) = (s.channel_at(y0.max(y1)), s.channel_at(y0.min(y1)));
                if (x1 - x0).abs() > 2 || (y1 - y0).abs() > 2 {
                    crate::seqedit::seq(d).select_box(f0, f1, c0, c1, extend);
                    d.touch();
                }
            }
            _ => {}
        },
        Input::Wheel { x, y, steps, mods } if sr.contains(x, y) || ruler.contains(x, y) => {
            ui.input = None;
            if mods.ctrl {
                s.x0 += steps as f32 * 10.0 / s.ppf;
            } else if mods.shift {
                s.ch0 = (s.ch0 - steps as f32 * 0.5).max(1.0);
            } else {
                let f = s.frame_at(x);
                s.ppf = (s.ppf * 0.85f32.powi(steps)).clamp(0.05, 100.0);
                s.x0 = f - (x - sr.x) as f32 / s.ppf;
            }
        }
        _ => {}
    }
}

/// Keys over the sequencer.
pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    let q = |d: &mut Doc, id: &str, arg: &str| d.queue.push(Op::with(id, arg));
    let alt = Mods { alt: true, ..Mods::NONE };
    let ctrl_alt = Mods { alt: true, ctrl: true, ..Mods::NONE };
    let shift_alt = Mods { alt: true, shift: true, ..Mods::NONE };
    match (key, mods) {
        (Key::Char('g'), Mods::NONE) => {
            let orig = Box::new(crate::seqedit::seq(d).clone());
            d.push_undo("Move Strips");
            s.drag = Drag::Move { x0: s.mouse.0, y0: s.mouse.1, orig, grab: true };
        }
        (Key::Char('e'), Mods::NONE) => {
            // Extend by the frames between the playhead and the mouse.
            let df = (s.frame_at(s.mouse.0) - d.frame as f32).round();
            q(d, "sequencer.extend", &df.to_string());
        }
        (Key::Char('k'), Mods::NONE) => q(d, "sequencer.split", "SOFT"),
        (Key::Char('k'), Mods::SHIFT) => q(d, "sequencer.split", "HARD"),
        (Key::Char('d'), Mods::SHIFT) => {
            d.push_undo("Duplicate Strips");
            crate::seqedit::seq(d).duplicate();
            let orig = Box::new(crate::seqedit::seq(d).clone());
            s.drag = Drag::Move { x0: s.mouse.0, y0: s.mouse.1, orig, grab: true };
            d.touch();
        }
        (Key::Char('x'), Mods::NONE) | (Key::Delete, _) => q(d, "sequencer.delete", ""),
        (Key::Char('g'), Mods::CTRL) => q(d, "sequencer.meta_make", ""),
        (Key::Char('g'), m) if m == ctrl_alt => q(d, "sequencer.meta_separate", ""),
        (Key::Char('h'), Mods::NONE) => q(d, "sequencer.mute", ""),
        (Key::Char('h'), m) if m == alt => q(d, "sequencer.unmute", ""),
        (Key::Char('h'), Mods::SHIFT) => q(d, "sequencer.mute", "UNSELECTED"),
        (Key::Char('l'), Mods::SHIFT) => q(d, "sequencer.lock", ""),
        (Key::Char('l'), m) if m == shift_alt => q(d, "sequencer.unlock", ""),
        (Key::Char('s'), Mods::SHIFT) => q(d, "sequencer.snap", ""),
        (Key::Char('a'), Mods::NONE) => q(d, "sequencer.select_all", "SELECT"),
        (Key::Char('a'), m) if m == alt => q(d, "sequencer.select_all", "DESELECT"),
        (Key::Char('i'), Mods::CTRL) => q(d, "sequencer.select_all", "INVERT"),
        (Key::Char('a'), Mods::SHIFT) => d.menu = Some(crate::ui::Popup::new("Add", add_menu(d), s.mouse.0 - 20, s.mouse.1 - 10)),
        (Key::Char('n'), Mods::NONE) => s.show_sidebar = !s.show_sidebar,
        (Key::Home, _) => frame_all(s, d),
        (Key::Enter, _) if matches!(s.drag, Drag::Move { grab: true, .. }) => s.drag = Drag::None,
        _ => return false,
    }
    true
}

/// While G is held the strips follow the mouse.
pub fn is_modal(s: &State) -> bool {
    matches!(s.drag, Drag::Move { grab: true, .. })
}

fn sidebar(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    ui.p.fill(r, t::PROPERTIES);
    ui.p.push_clip(r);
    let active = d.scene.sequence_editor.as_ref().and_then(|q| q.active.filter(|&a| a < q.strips.len()));
    let mut picker = None;
    let mut path = core::mem::take(&mut s.path);
    {
        let mut l = L {
            ui: &mut *ui,
            open: &mut s.open,
            x: r.x + 6,
            w: r.w - 12,
            y: r.y + 6,
            query: String::new(),
            dry: false,
            hits: 0,
            depth: 0,
            inactive: false,
            prefix: "seq".into(),
            undo_for: s.undo_for,
            fields: Vec::new(),
        };
        match active {
            Some(a) => {
                let name = d.scene.sequence_editor.as_ref().map(|q| q.strips[a].name.clone()).unwrap_or_default();
                let kind = d.scene.sequence_editor.as_ref().map(|q| q.strips[a].kind.type_id()).unwrap_or("");
                let mut n = name.clone();
                if l.text("Name", "seq_name", &mut n) && !n.trim().is_empty() && n != name {
                    l.commit(d, "Rename Strip");
                    let q = crate::seqedit::seq(d);
                    let n = q.unique_name(n.trim());
                    // Effects refer to their inputs by name.
                    for st in &mut q.strips {
                        if let StripKind::Effect { inputs, .. } = &mut st.kind {
                            for i in inputs.iter_mut().filter(|i| **i == name) {
                                *i = n.clone();
                            }
                        }
                    }
                    q.strips[a].name = n;
                }
                l.info("Type", kind);
                for (key, title, owner, open) in [
                    ("seq.kind", "Strip", GpOwner::StripKind(a), true),
                    ("seq.look", "Compositing and Color", GpOwner::StripLook(a), true),
                    ("seq.time", "Time", GpOwner::StripTime(a), true),
                ] {
                    l.panel(d, key, title, open, &mut |l, d| {
                        if let Some(p) = gui::draw(l, d, owner, &[]) {
                            picker = Some(p);
                        }
                    });
                }
                l.panel(d, "seq.mods", "Modifiers", true, &mut |l, d| {
                    let add: Vec<MenuItem> = [("COLOR_BALANCE", "Color Balance"), ("CURVES", "Curves"), ("BRIGHT_CONTRAST", "Brightness/Contrast"), ("WHITE_BALANCE", "White Balance")]
                        .iter()
                        .map(|(id, lab)| MenuItem::op(lab, "", Op::with("sequencer.modifier_add", id)))
                        .collect();
                    l.dropdown_row("", "seq_add_mod", "Add Modifier", &|| add.clone());
                    let n = d.scene.sequence_editor.as_ref().map_or(0, |q| q.strips[a].modifiers.len());
                    for mi in 0..n {
                        let title = d.scene.sequence_editor.as_ref().map(|q| q.strips[a].modifiers[mi].name.clone()).unwrap_or_default();
                        l.note(&title);
                        if let Some(p) = gui::draw(l, d, GpOwner::StripMod(a, mi), &[]) {
                            picker = Some(p);
                        }
                        if l.button(&format!("Remove {title}")) {
                            d.queue.push(Op::with("sequencer.modifier_remove", &mi.to_string()));
                        }
                    }
                });
            }
            None => l.note("No active strip"),
        }
        l.panel(d, "seq.media", "Add Media", false, &mut |l, d| {
            l.text("Path", "seq_path", &mut path);
            for (lab, k) in [("Add Image", "IMAGE"), ("Add Movie", "MOVIE"), ("Add Sound", "SOUND")] {
                if l.button(lab) {
                    d.queue.push(Op::with("sequencer.strip_add", &format!("{k}:{}", path.trim())));
                }
            }
            l.note("Movies show a placeholder: no video decoder");
        });
        s.undo_for = if l.ui.st.active.is_none() { None } else { l.undo_for };
    }
    s.path = path;
    ui.p.pop_clip();
    if let Some((target, anchor)) = picker {
        s.picker = Some(Picker::new(target, anchor, d));
    }
}
