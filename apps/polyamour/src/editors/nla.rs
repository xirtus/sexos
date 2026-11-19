// SPDX-License-Identifier: GPL-3.0-only
//! The Nonlinear Animation editor: per object, its NLA tracks (top of the
//! stack first) with their action strips, and the orange action line for
//! the active action with its Push Down button. Strips are selected with a
//! click (Shift extends), moved with G or a drag; Shift+A adds an action
//! strip, Shift+D duplicates, X deletes, Y splits at the current frame, H
//! mutes, Tab enters / leaves tweak mode. The sidebar (N) edits the active
//! strip (frames, extrapolation, blending, blend in / out, influence,
//! action clip) and the object's animation data (action, blending,
//! influence).
use crate::anim::{self, Blend, StripExtrap};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::editors::properties::layout::L;
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, NumOpts, Op, Ui};
use std::collections::HashMap;

const RULER_H: i32 = 22;
const ROW_H: i32 = 22;
const CHANNEL_W: i32 = 200;
const SIDE_W: i32 = 260;
const STRIP: u32 = 0x8c7a4a;
const STRIP_SEL: u32 = 0xd9b65a;
const STRIP_MUTE: u32 = 0x5a5a5a;
const ACTION_LINE: u32 = 0xc76f1c;
const TWEAK: u32 = 0x4f9a3f;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NlaRow {
    Object(usize),
    /// Track `t` of `Anim::objects[o]`.
    Track(usize, usize),
    /// The active action line of `Anim::objects[o]`.
    Action(usize),
}

#[derive(Clone, Debug, Default)]
enum Drag {
    #[default]
    None,
    Scrub,
    /// Moving the selected strips: start mouse x and each strip's start.
    Move { x: i32, orig: Vec<(usize, usize, usize, f32)>, release: bool, undo: bool },
}

pub struct State {
    id: u64,
    pub x0: f32,
    pub ppf: f32,
    framed: bool,
    drag: Drag,
    mouse: (i32, i32),
    pub scroll: i32,
    pub show_sidebar: bool,
    pub strips_rect: Rect,
    open: HashMap<String, bool>,
    undo_for: Option<Option<u64>>,
}

impl Default for State {
    fn default() -> State {
        State {
            id: anim::editor_id(),
            x0: -10.0,
            ppf: 4.0,
            framed: false,
            drag: Drag::None,
            mouse: (0, 0),
            scroll: 0,
            show_sidebar: true,
            strips_rect: Rect::default(),
            open: HashMap::new(),
            undo_for: None,
        }
    }
}

impl State {
    pub fn fx(&self, f: f32) -> f32 {
        self.strips_rect.x as f32 + (f - self.x0) * self.ppf
    }
    fn frame_at(&self, x: i32) -> f32 {
        self.x0 + (x - self.strips_rect.x) as f32 / self.ppf
    }
    /// Top of row `i`.
    pub fn row_y(&self, i: usize) -> i32 {
        self.strips_rect.y + i as i32 * ROW_H - self.scroll
    }
}

/// The rows: each animated object, its tracks top-down, its action line.
pub fn rows(d: &Doc) -> Vec<NlaRow> {
    let mut out = Vec::new();
    for (o, oa) in d.anim.objects.iter().enumerate() {
        if d.scene.find_object(&oa.object).is_none() || (oa.tracks.is_empty() && oa.action.is_none()) {
            continue;
        }
        out.push(NlaRow::Object(o));
        for ti in (0..oa.tracks.len()).rev() {
            out.push(NlaRow::Track(o, ti));
        }
        out.push(NlaRow::Action(o));
    }
    out
}

fn add_menu(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Action Strip".into())];
    for a in &d.anim.actions {
        v.push(MenuItem::op(&a.name, "", Op::with("nla.actionclip_add", &a.name)));
    }
    v.push(MenuItem::Separator);
    v.push(MenuItem::op("Add Tracks", "", Op::new("nla.tracks_add")));
    v
}

fn strip_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Duplicate", "Shift D", Op::new("nla.duplicate")),
        MenuItem::op("Split", "Y", Op::new("nla.split")),
        MenuItem::op("Delete", "X", Op::new("nla.delete")),
        MenuItem::Separator,
        MenuItem::op("Toggle Muting", "H", Op::new("nla.mute_toggle")),
        MenuItem::Separator,
        MenuItem::op("Start Tweaking Strip Actions", "Tab", Op::new("nla.tweakmode_enter")),
        MenuItem::op("Stop Tweaking Strip Actions", "Tab", Op::new("nla.tweakmode_exit")),
    ]
}

fn select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("All", "A", Op::with("nla.select_all", "SELECT")),
        MenuItem::op("None", "Alt A", Op::with("nla.select_all", "DESELECT")),
        MenuItem::op("Invert", "Ctrl I", Op::with("nla.select_all", "INVERT")),
    ]
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let id = s.id;
    let view = move |_d: &Doc| {
        vec![
            MenuItem::op("Sidebar", "N", anim::mail("view.sidebar", id, "")),
            MenuItem::op("Frame All", "Home", anim::mail("view.frame_all", id, "")),
        ]
    };
    super::header_menus(ui, hr, hr.x + 4, &[("View", &view), ("Select", &select_menu), ("Add", &add_menu), ("Strip", &strip_menu)], d);
}

fn active_strip(d: &Doc) -> Option<(usize, usize, usize)> {
    for (o, oa) in d.anim.objects.iter().enumerate() {
        for (ti, tr) in oa.tracks.iter().enumerate().rev() {
            if let Some(si) = tr.strips.iter().position(|x| x.select) {
                return Some((o, ti, si));
            }
        }
    }
    None
}

fn frame_all(s: &mut State, d: &Doc) {
    let (mut a, mut b) = (f32::MAX, f32::MIN);
    for oa in &d.anim.objects {
        for tr in &oa.tracks {
            for st in &tr.strips {
                a = a.min(st.frame_start);
                b = b.max(st.frame_end);
            }
        }
    }
    if a > b {
        a = d.scene.frame_start as f32;
        b = d.scene.frame_end as f32;
    }
    let span = (b - a).max(10.0) * 1.1;
    s.ppf = s.strips_rect.w.max(100) as f32 / span;
    s.x0 = a - (span - (b - a)) / 2.0;
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    for (op, _) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "view.sidebar" => s.show_sidebar = !s.show_sidebar,
            "view.frame_all" => frame_all(s, d),
            _ => {}
        }
    }
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    let (r, side_r) = if s.show_sidebar && r.w > SIDE_W + 300 { r.take_right(SIDE_W) } else { (r, Rect::default()) };
    let (chan, main) = r.take_left(CHANNEL_W.min(r.w / 2));
    let (ruler, strips) = main.take_top(RULER_H);
    s.strips_rect = strips;
    if !s.framed && strips.w > 0 {
        s.framed = true;
        let (a, b) = (d.scene.frame_start as f32, d.scene.frame_end as f32);
        let span = (b - a).max(1.0) * 1.08;
        s.ppf = strips.w as f32 / span;
        s.x0 = a - (span - (b - a)) / 2.0;
    }
    let rows = rows(d);
    if !side_r.is_empty() {
        sidebar(s, d, ui, side_r);
    }
    handle_input(s, d, ui, ruler, strips, chan, &rows);

    // ---- drawing
    let p = &mut *ui.p;
    p.fill(main, t::TIMELINE);
    p.fill(chan, 0x282828);
    p.fill(ruler, 0x232323);
    let step = nice_step(s.ppf);
    let mut f = (s.x0 / step).floor() * step;
    while s.fx(f) < (main.right()) as f32 {
        let x = s.fx(f).round() as i32;
        if x >= main.x {
            p.fill(Rect::new(x, strips.y, 1, strips.h), 0x373737);
            p.label(Rect::new(x - 20, ruler.y, 40, ruler.h), &format!("{}", f as i32), t::TEXT_DIM, Align::Center);
        }
        f += step;
    }
    // Scene range shading.
    let (fs, fe) = (s.fx(d.scene.frame_start as f32) as i32, s.fx(d.scene.frame_end as f32) as i32);
    if fs > strips.x {
        p.fill_alpha(Rect::new(strips.x, strips.y, fs - strips.x, strips.h), 0x000000, 60);
    }
    if fe < strips.right() {
        p.fill_alpha(Rect::new(fe, strips.y, strips.right() - fe, strips.h), 0x000000, 60);
    }
    p.push_clip(Rect::new(chan.x, strips.y, chan.w + main.w, strips.h));
    for (i, row) in rows.iter().enumerate() {
        let y = s.row_y(i);
        if y + ROW_H < strips.y || y > strips.bottom() {
            continue;
        }
        let cr = Rect::new(chan.x, y, chan.w, ROW_H - 1);
        let lane = Rect::new(strips.x, y, strips.w, ROW_H - 1);
        match *row {
            NlaRow::Object(o) => {
                let oa = &d.anim.objects[o];
                p.fill(cr, 0x3d4a5e);
                p.fill(lane, 0x2e3440);
                p.label(Rect::new(cr.x + 8, y, cr.w - 8, ROW_H), &oa.object, t::TEXT, Align::Left);
            }
            NlaRow::Track(o, ti) => {
                let oa = &d.anim.objects[o];
                let tr = &oa.tracks[ti];
                p.fill(cr, if tr.select { 0x4a4a52 } else { 0x343434 });
                p.fill(lane, 0x2b2b2b);
                p.label(Rect::new(cr.x + 18, y, cr.w - 60, ROW_H), &tr.name, if tr.mute { t::TEXT_DIM } else { t::TEXT }, Align::Left);
                // Mute (checkbox) and lock toggles.
                let mr = Rect::new(cr.right() - 40, y + 4, 12, 12);
                p.frame(mr, 0x8a8a8a);
                if !tr.mute {
                    p.fill(Rect::new(mr.x + 3, mr.y + 3, 6, 6), 0xd0d0d0);
                }
                let lr = Rect::new(cr.right() - 20, y + 4, 12, 12);
                p.frame(lr, if tr.lock { 0xe0e0e0 } else { 0x6a6a6a });
                for (si, st) in tr.strips.iter().enumerate() {
                    let x0 = s.fx(st.frame_start).round() as i32;
                    let x1 = s.fx(st.frame_end).round() as i32;
                    let sr = Rect::new(x0, y + 2, (x1 - x0).max(2), ROW_H - 5);
                    let tweak = oa.tweak == Some((ti, si));
                    let col = if tweak {
                        TWEAK
                    } else if st.mute || tr.mute {
                        STRIP_MUTE
                    } else if st.select {
                        STRIP_SEL
                    } else {
                        STRIP
                    };
                    p.round_rect(sr, 3, col);
                    if st.select {
                        p.frame(sr, 0xffffff);
                    }
                    // Blend in / out ramps.
                    if st.blend_in > 0.0 {
                        let bx = s.fx(st.frame_start + st.blend_in);
                        p.line(x0 as f32, sr.bottom() as f32, bx, sr.y as f32, 0x202020);
                    }
                    if st.blend_out > 0.0 {
                        let bx = s.fx(st.frame_end - st.blend_out);
                        p.line(bx, sr.y as f32, x1 as f32, sr.bottom() as f32, 0x202020);
                    }
                    p.push_clip(sr);
                    p.label(Rect::new(sr.x + 4, sr.y, sr.w - 4, sr.h), &st.name, 0x101010, Align::Left);
                    p.pop_clip();
                }
            }
            NlaRow::Action(o) => {
                let oa = &d.anim.objects[o];
                p.fill(cr, 0x3b3226);
                p.fill(lane, 0x2b2620);
                let name = oa.action.and_then(|a| d.anim.actions.get(a)).map_or("<No Action>".to_string(), |a| a.name.clone());
                p.label(Rect::new(cr.x + 18, y, cr.w - 50, ROW_H), &name, if oa.action.is_some() { 0xf0b070 } else { t::TEXT_DIM }, Align::Left);
                if let Some(a) = oa.action.and_then(|a| d.anim.actions.get(a)) {
                    // Push Down button.
                    let br = Rect::new(cr.right() - 28, y + 2, 24, ROW_H - 5);
                    p.round_rect(br, 3, t::BUTTON);
                    p.label(br, "v", t::TEXT, Align::Center);
                    let (a0, a1) = a.frame_range();
                    let x0 = s.fx(a0).round() as i32;
                    let x1 = s.fx(a1).round() as i32;
                    p.fill_alpha(Rect::new(x0, y + 4, (x1 - x0).max(2), ROW_H - 9), ACTION_LINE, 160);
                    for fr in a.key_frames() {
                        let x = s.fx(fr);
                        p.fill(Rect::new(x as i32 - 2, y + ROW_H / 2 - 3, 5, 5), 0xe0e0e0);
                    }
                }
            }
        }
    }
    if rows.is_empty() {
        p.label(strips, "No animation data: key an object or push down an action", t::TEXT_DIM, Align::Center);
    }
    p.pop_clip();
    // Playhead.
    let px = s.fx(d.frame as f32).round() as i32;
    if px >= main.x && px < main.right() {
        p.fill(Rect::new(px, ruler.y, 2, strips.bottom() - ruler.y), 0x4772b3);
        let lr = Rect::new(px - 14, ruler.y + 2, 30, ruler.h - 4);
        p.round_rect(lr, 4, 0x4772b3);
        p.label(lr, &d.frame.to_string(), 0xffffff, Align::Center);
    }
}

fn nice_step(ppf: f32) -> f32 {
    let raw = 60.0 / ppf.max(1e-3);
    let mag = 10f32.powf(raw.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * mag >= raw {
            return (m * mag).max(1.0);
        }
    }
    10.0 * mag
}

fn strip_at(s: &State, d: &Doc, rows: &[NlaRow], x: i32, y: i32) -> Option<(usize, usize, usize)> {
    let i = (0..rows.len()).find(|&i| y >= s.row_y(i) && y < s.row_y(i) + ROW_H)?;
    let NlaRow::Track(o, ti) = rows[i] else { return None };
    let f = s.frame_at(x);
    let tol = 3.0 / s.ppf;
    d.anim.objects[o].tracks[ti].strips.iter().position(|st| f >= st.frame_start - tol && f <= st.frame_end + tol).map(|si| (o, ti, si))
}

fn deselect(d: &mut Doc) {
    for oa in &mut d.anim.objects {
        for tr in &mut oa.tracks {
            tr.select = false;
            for st in &mut tr.strips {
                st.select = false;
            }
        }
    }
}

fn start_move(s: &mut State, d: &Doc, x: i32, release: bool) {
    let mut orig = Vec::new();
    for (o, oa) in d.anim.objects.iter().enumerate() {
        for (ti, tr) in oa.tracks.iter().enumerate() {
            if tr.lock {
                continue;
            }
            for (si, st) in tr.strips.iter().enumerate() {
                if st.select {
                    orig.push((o, ti, si, st.frame_start));
                }
            }
        }
    }
    if !orig.is_empty() {
        s.drag = Drag::Move { x, orig, release, undo: false };
    }
}

fn handle_input(s: &mut State, d: &mut Doc, ui: &mut Ui, ruler: Rect, strips: Rect, chan: Rect, rows: &[NlaRow]) {
    let Some(input) = ui.input else { return };
    let clip = ui.p.clip();
    let inside = |r: Rect, x: i32, y: i32| r.contains(x, y) && clip.contains(x, y);
    match (&mut s.drag, input) {
        (Drag::Move { x, orig, release, undo }, ev) => {
            let (x0, release) = (*x, *release);
            match ev {
                Input::Move { x: mx, .. } => {
                    if !*undo {
                        d.push_undo("Move Strips");
                        *undo = true;
                    }
                    let df = ((mx - x0) as f32 / s.ppf).round();
                    for &(o, ti, si, f0) in orig.iter() {
                        if let Some(st) = d.anim.objects.get_mut(o).and_then(|x| x.tracks.get_mut(ti)).and_then(|x| x.strips.get_mut(si)) {
                            st.frame_start = f0 + df;
                            st.update_end();
                        }
                    }
                    d.anim.dirty = true;
                }
                Input::Up { button: Button::Left, .. } if release => s.drag = Drag::None,
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => s.drag = Drag::None,
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    for &(o, ti, si, f0) in orig.iter() {
                        if let Some(st) = d.anim.objects.get_mut(o).and_then(|x| x.tracks.get_mut(ti)).and_then(|x| x.strips.get_mut(si)) {
                            st.frame_start = f0;
                            st.update_end();
                        }
                    }
                    d.anim.dirty = true;
                    s.drag = Drag::None;
                }
                _ => return,
            }
            if matches!(s.drag, Drag::None) {
                for oa in &mut d.anim.objects {
                    for tr in &mut oa.tracks {
                        tr.sort();
                    }
                }
            }
            ui.input = None;
        }
        (Drag::Scrub, Input::Move { x, .. }) => {
            d.frame = s.frame_at(x).round() as i32;
            ui.input = None;
        }
        (Drag::Scrub, Input::Up { .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::None, Input::Down { x, y, button: Button::Left, mods }) => {
            if inside(ruler, x, y) {
                s.drag = Drag::Scrub;
                d.frame = s.frame_at(x).round() as i32;
                ui.input = None;
            } else if inside(strips, x, y) {
                match strip_at(s, d, rows, x, y) {
                    Some((o, ti, si)) => {
                        let was = d.anim.objects[o].tracks[ti].strips[si].select;
                        if !mods.shift {
                            if !was {
                                deselect(d);
                            }
                            d.anim.objects[o].tracks[ti].strips[si].select = true;
                            start_move(s, d, x, true);
                        } else {
                            d.anim.objects[o].tracks[ti].strips[si].select = !was;
                        }
                    }
                    None if !mods.shift => deselect(d),
                    None => {}
                }
                ui.input = None;
            } else if inside(chan, x, y) {
                if let Some(i) = (0..rows.len()).find(|&i| y >= s.row_y(i) && y < s.row_y(i) + ROW_H) {
                    let cr = Rect::new(chan.x, s.row_y(i), chan.w, ROW_H);
                    match rows[i] {
                        NlaRow::Track(o, ti) => {
                            if x >= cr.right() - 42 && x < cr.right() - 26 {
                                d.push_undo("Mute Track");
                                let tr = &mut d.anim.objects[o].tracks[ti];
                                tr.mute = !tr.mute;
                                d.anim.dirty = true;
                            } else if x >= cr.right() - 22 {
                                d.push_undo("Lock Track");
                                let tr = &mut d.anim.objects[o].tracks[ti];
                                tr.lock = !tr.lock;
                            } else {
                                if !mods.shift {
                                    deselect(d);
                                }
                                d.anim.objects[o].tracks[ti].select = true;
                            }
                        }
                        NlaRow::Action(o) if x >= cr.right() - 28 => {
                            let name = d.anim.objects[o].object.clone();
                            d.queue.push(Op::with("nla.action_pushdown", &name));
                        }
                        NlaRow::Object(o) => {
                            let name = d.anim.objects[o].object.clone();
                            if let Some(oi) = d.scene.find_object(&name) {
                                d.select_only(Some(oi));
                            }
                        }
                        _ => {}
                    }
                }
                ui.input = None;
            }
        }
        (Drag::None, Input::Wheel { x, y, steps, mods }) if inside(Rect::new(chan.x, strips.y, chan.w + strips.w, strips.h), x, y) => {
            if mods.ctrl || inside(chan, x, y) {
                s.scroll = (s.scroll - steps * ROW_H).max(0);
            } else {
                let f = s.frame_at(x);
                let k = if steps > 0 { 1.15 } else { 1.0 / 1.15 };
                s.ppf = (s.ppf * k).clamp(0.05, 200.0);
                s.x0 = f - (x - strips.x) as f32 / s.ppf;
            }
            ui.input = None;
        }
        _ => {}
    }
}

fn sidebar(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    ui.p.fill(r, t::PROPERTIES);
    ui.p.fill(Rect::new(r.x, r.y, 1, r.h), 0x222222);
    ui.p.push_clip(r);
    let mut l = L {
        ui: &mut *ui,
        open: &mut s.open,
        x: r.x + 4,
        w: r.w - 8,
        y: r.y + 6,
        query: String::new(),
        dry: false,
        hits: 0,
        depth: 0,
        inactive: false,
        prefix: "nlaside".into(),
        undo_for: s.undo_for,
        fields: Vec::new(),
    };
    let ao = d.active().and_then(|a| d.anim.objects.iter().position(|o| o.object == d.scene.objects[a].name));
    if let Some(o) = ao {
        let oa = d.anim.objects[o].clone();
        let target = format!("adata:{o}");
        l.panel(d, "adata", "Animation Data", true, &mut |l, d| {
            let act = oa.action.and_then(|a| d.anim.actions.get(a)).map_or("(none)".to_string(), |a| a.name.clone());
            let names: Vec<String> = d.anim.actions.iter().map(|a| a.name.clone()).collect();
            l.dropdown_row("Action", "action", &act, &|| names.iter().map(|n| MenuItem::op(n, "", Op::with("action.assign", n))).collect());
            let tg = target.clone();
            l.dropdown_row("Blending", "ablend", oa.blend.label(), &|| Blend::ALL.iter().map(|b| MenuItem::op(b.label(), "", Op::with("anim.prop_set", &format!("{tg}\tblend_type={}", b.id())))).collect());
            let mut inf = oa.influence;
            if l.slider("Influence", "ainf", &mut inf, 0.0, 1.0) {
                l.commit(d, "Change Influence");
                d.anim.objects[o].influence = inf;
                d.anim.dirty = true;
            }
        });
    }
    match active_strip(d) {
        None => l.note("No active strip"),
        Some((o, ti, si)) => {
            let st = d.anim.objects[o].tracks[ti].strips[si].clone();
            let target = format!("strip:{o}:{ti}:{si}");
            l.panel(d, "strip", "Strip", true, &mut |l, d| {
                let mut name = st.name.clone();
                if l.text("Name", "sname", &mut name) {
                    l.commit(d, "Rename Strip");
                    d.anim.objects[o].tracks[ti].strips[si].name = name;
                }
                let mut fs = st.frame_start;
                if l.number("Frame Start", "fs", &mut fs, NumOpts { step: 1.0, ..NumOpts::default() }) {
                    l.commit(d, "Move Strip");
                    let x = &mut d.anim.objects[o].tracks[ti].strips[si];
                    x.frame_start = fs;
                    x.update_end();
                    d.anim.dirty = true;
                }
                let mut fe = st.frame_end;
                if l.number("End", "fe", &mut fe, NumOpts { step: 1.0, ..NumOpts::default() }) {
                    l.commit(d, "Change Strip End");
                    let x = &mut d.anim.objects[o].tracks[ti].strips[si];
                    // The end scales the strip, as in Blender.
                    let len = ((x.action_end - x.action_start) * x.repeat).max(1e-3);
                    x.scale = ((fe - x.frame_start) / len).max(1e-3);
                    x.update_end();
                    d.anim.dirty = true;
                }
                let tg = target.clone();
                l.dropdown_row("Extrapolation", "sext", st.extrapolation.label(), &|| StripExtrap::ALL.iter().map(|b| MenuItem::op(b.label(), "", Op::with("anim.prop_set", &format!("{tg}\textrapolation={}", b.id())))).collect());
                let tg = target.clone();
                l.dropdown_row("Blending", "sblend", st.blend.label(), &|| Blend::ALL.iter().map(|b| MenuItem::op(b.label(), "", Op::with("anim.prop_set", &format!("{tg}\tblend_type={}", b.id())))).collect());
                for (lab, key, v) in [("Blend In", "bin", st.blend_in), ("Out", "bout", st.blend_out)] {
                    let mut x = v;
                    if l.number(lab, key, &mut x, NumOpts { step: 1.0, min: 0.0, ..NumOpts::default() }) {
                        l.commit(d, "Change Blend");
                        let sp = &mut d.anim.objects[o].tracks[ti].strips[si];
                        if key == "bin" {
                            sp.blend_in = x;
                        } else {
                            sp.blend_out = x;
                        }
                        d.anim.dirty = true;
                    }
                }
                let mut ui_ = st.use_influence;
                if l.check("", "Animated Influence", &mut ui_) {
                    l.commit(d, "Animated Influence");
                    d.anim.objects[o].tracks[ti].strips[si].use_influence = ui_;
                    d.anim.dirty = true;
                }
                if st.use_influence {
                    let mut inf = st.influence;
                    if l.slider("Influence", "sinf", &mut inf, 0.0, 1.0) {
                        l.commit(d, "Change Influence");
                        d.anim.objects[o].tracks[ti].strips[si].influence = inf;
                        d.anim.dirty = true;
                    }
                }
                for (lab, key, v) in [("Reversed", "use_reverse", st.reversed), ("Mute", "mute", st.mute)] {
                    let mut x = v;
                    if l.check("", lab, &mut x) {
                        l.commit(d, "Strip Property");
                        anim::props::set(d, &target, key, if x { "1" } else { "0" });
                        d.anim.dirty = true;
                    }
                }
            });
            l.panel(d, "clip", "Action Clip", true, &mut |l, d| {
                let names: Vec<String> = d.anim.actions.iter().map(|a| a.name.clone()).collect();
                let cur = d.anim.actions.get(st.action).map_or("(none)".to_string(), |a| a.name.clone());
                let tg = target.clone();
                l.dropdown_row("Action", "sact", &cur, &|| names.iter().map(|n| MenuItem::op(n, "", Op::with("anim.prop_set", &format!("{tg}\taction={n}")))).collect());
                for (lab, key, v, step) in [("Frame Start", "as", st.action_start, 1.0), ("End", "ae", st.action_end, 1.0), ("Playback Scale", "sc", st.scale, 0.1), ("Repeat", "rp", st.repeat, 0.1)] {
                    let mut x = v;
                    if l.number(lab, key, &mut x, NumOpts { step, ..NumOpts::default() }) {
                        l.commit(d, "Change Action Clip");
                        let sp = &mut d.anim.objects[o].tracks[ti].strips[si];
                        match key {
                            "as" => sp.action_start = x,
                            "ae" => sp.action_end = x.max(sp.action_start),
                            "sc" => sp.scale = x,
                            _ => sp.repeat = x,
                        }
                        sp.update_end();
                        d.anim.dirty = true;
                    }
                }
            });
        }
    }
    s.undo_for = l.undo_for;
    if ui.st.active.is_none() {
        s.undo_for = None;
    }
    ui.p.pop_clip();
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    match (key, mods) {
        (Key::Tab, Mods::NONE) => d.queue.push(Op::new("nla.tweakmode_toggle")),
        (Key::Char('a'), Mods::SHIFT) => d.menu = Some(crate::ui::Popup::new("Add Action Strip", add_menu(d), s.mouse.0 - 20, s.mouse.1 - 10)),
        (Key::Char('d'), Mods::SHIFT) => {
            d.queue.push(Op::new("nla.duplicate"));
        }
        (Key::Char('x'), Mods::NONE) | (Key::Delete, Mods::NONE) => d.queue.push(Op::new("nla.delete")),
        (Key::Char('y'), Mods::NONE) => d.queue.push(Op::new("nla.split")),
        (Key::Char('h'), Mods::NONE) => d.queue.push(Op::new("nla.mute_toggle")),
        (Key::Char('a'), Mods::NONE) => d.queue.push(Op::with("nla.select_all", "SELECT")),
        (Key::Char('a'), Mods::ALT) => d.queue.push(Op::with("nla.select_all", "DESELECT")),
        (Key::Char('i'), Mods::CTRL) => d.queue.push(Op::with("nla.select_all", "INVERT")),
        (Key::Char('g'), Mods::NONE) => {
            if active_strip(d).is_some() {
                start_move(s, d, s.mouse.0, false);
            }
        }
        (Key::Char('n'), Mods::NONE) => s.show_sidebar = !s.show_sidebar,
        (Key::Home, Mods::NONE) => frame_all(s, d),
        _ => return false,
    }
    true
}
