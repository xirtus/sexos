// SPDX-License-Identifier: GPL-3.0-only
//! Blender's animation editors: the Timeline, the Dope Sheet and the Graph
//! Editor. They share one view (frames across, scrubbing ruler on top, the blue
//! playhead) and differ in what they show below it: the Timeline a row of
//! keyframe diamonds for the selected objects, the Dope Sheet a channel list
//! (Summary > object > Object Transforms > channels) with a row of diamonds per
//! channel, the Graph Editor the F-curves themselves with their handles.
//!
//! Keys are selected with a click (Shift extends) or a box drag (B, or drag on
//! empty space), moved with G (or by dragging a key), deleted with X.
use super::header_menus;
use crate::anim::{self, Interp, Prop};
use crate::doc::Doc;
use crate::draw::{Align, Painter, Rect};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{id, MenuItem, Op, Ui};
use crate::editors::EditorKind;

const RULER_H: i32 = 22;
const ROW_H: i32 = 20;
const CHANNEL_W: i32 = 200;

const KEY_FILL: u32 = 0xe8e8e8;
const KEY_SEL: u32 = 0xffbe33;
const GRID_MAJOR: u32 = 0x373737;
const GRID_MINOR: u32 = 0x313131;
const RULER_BG: u32 = 0x232323;
const CHANNEL_BG: u32 = 0x282828;

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    None,
    /// Dragging the playhead.
    Scrub,
    /// LMB pressed in the keys area, not yet a click or a drag.
    Press { x: i32, y: i32, on_key: bool, extend: bool },
    /// Box select from (x0, y0).
    Box { x0: i32, y0: i32, x1: i32, y1: i32, extend: bool },
    /// Moving the selected keys (G): where the move started, each key's original
    /// (frame, value), and whether releasing the button confirms.
    Move { x: i32, y: i32, orig: Vec<KeyRef>, release_confirms: bool },
    /// Middle-mouse pan.
    Pan { x: i32, y: i32 },
}

/// A key by position, with its original frame and value.
#[derive(Clone, Copy, Debug, PartialEq)]
struct KeyRef {
    o: usize,
    c: usize,
    k: usize,
    frame: f32,
    value: f32,
}

/// One row of the channel list.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Summary,
    Object(usize),
    Group(usize),
    Curve(usize, usize),
}

pub struct State {
    pub kind: EditorKind,
    id: u64,
    /// Frame at the left edge of the keys area and pixels per frame.
    pub x0: f32,
    pub ppf: f32,
    /// Graph editor: value at the bottom edge and pixels per unit.
    pub y0: f32,
    pub ppv: f32,
    framed: bool,
    drag: Drag,
    /// B pressed: the next LMB drag is a box select.
    box_armed: bool,
    mouse: (i32, i32),
    summary_open: bool,
    /// The last drawn keys rectangle (below the ruler, right of the channels).
    keys_rect: Rect,
    want_frame_all: bool,
}

impl State {
    pub fn new(kind: EditorKind) -> State {
        State {
            kind,
            id: anim::editor_id(),
            x0: -10.0,
            ppf: 4.0,
            y0: -2.0,
            ppv: 40.0,
            framed: false,
            drag: Drag::None,
            box_armed: false,
            mouse: (0, 0),
            summary_open: true,
            keys_rect: Rect::default(),
            want_frame_all: false,
        }
    }
    fn fx(&self, frame: f32) -> f32 {
        self.keys_rect.x as f32 + (frame - self.x0) * self.ppf
    }
    fn frame_at(&self, x: i32) -> f32 {
        self.x0 + (x - self.keys_rect.x) as f32 / self.ppf
    }
    fn vy(&self, v: f32) -> f32 {
        self.keys_rect.bottom() as f32 - (v - self.y0) * self.ppv
    }
    fn value_at(&self, y: i32) -> f32 {
        self.y0 + (self.keys_rect.bottom() - y) as f32 / self.ppv
    }
    fn graph(&self) -> bool {
        self.kind == EditorKind::GraphEditor
    }
}

// ---------------------------------------------------------------- menus

fn playback_menu(d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::Heading("Playback".into()),
        MenuItem::Check("Audio Scrubbing".into(), false, Op::new("screen.audio_scrubbing")),
        MenuItem::Check("Follow Current Frame".into(), false, Op::new("screen.follow_frame")),
        MenuItem::Check("Limit to Frame Range".into(), true, Op::new("screen.limit_range")),
        MenuItem::Separator,
        MenuItem::Heading("Play In".into()),
        MenuItem::Check("Active Editor".into(), false, Op::new("screen.play_in")),
        MenuItem::Check("3D Viewport".into(), true, Op::new("screen.play_in")),
        MenuItem::Separator,
        MenuItem::Heading("Sync: Play Every Frame".into()),
        MenuItem::Separator,
        MenuItem::op(if d.playing { "Pause" } else { "Play Animation" }, "Space", Op::new("screen.animation_play")),
        MenuItem::op("Play Reverse", "Shift Ctrl Space", Op::new("screen.animation_play_reverse")),
    ]
}

fn keying_menu(d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::Heading("Active Keying Set: Location, Rotation & Scale".into()),
        MenuItem::Separator,
        MenuItem::Check("Auto Keying".into(), d.anim.auto_key, Op::new("anim.auto_keying_toggle")),
        MenuItem::Heading("Auto Keying Mode: Add & Replace".into()),
        MenuItem::Separator,
        MenuItem::Heading("New Keyframe Type: Keyframe".into()),
        MenuItem::Sub(
            "Default Interpolation".into(),
            vec![
                MenuItem::Check("Constant".into(), d.anim.interp == Interp::Constant, Op::with("anim.default_interpolation", "CONSTANT")),
                MenuItem::Check("Linear".into(), d.anim.interp == Interp::Linear, Op::with("anim.default_interpolation", "LINEAR")),
                MenuItem::Check("Bezier".into(), d.anim.interp == Interp::Bezier, Op::with("anim.default_interpolation", "BEZIER")),
            ],
        ),
        MenuItem::Separator,
        MenuItem::op("Insert Keyframe", "I", Op::new("anim.keyframe_insert")),
        MenuItem::op("Delete Keyframes", "Alt I", Op::new("anim.keyframe_delete")),
        MenuItem::op("Clear Keyframes", "Shift Alt I", Op::new("anim.keyframes_clear")),
    ]
}

fn view_menu(s: &State) -> Vec<MenuItem> {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    let mut v = vec![
        MenuItem::Check("Show Seconds".into(), false, Op::new("anim.time_toggle")),
        MenuItem::Check("Show Markers".into(), true, Op::new("anim.show_markers")),
        MenuItem::Check("Only Show Selected".into(), true, Op::new("anim.only_selected")),
        MenuItem::Separator,
        m("Frame All", "Home", "view.frame_all"),
        m("Go to Current Frame", "Numpad .", "view.frame_current"),
        MenuItem::Separator,
        MenuItem::op("Set Start Frame", "Ctrl Home", Op::new("anim.start_frame_set")),
        MenuItem::op("Set End Frame", "Ctrl End", Op::new("anim.end_frame_set")),
    ];
    v.push(MenuItem::Separator);
    v.push(MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area")));
    v
}

fn marker_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Add Marker", "M", Op::new("marker.add")),
        MenuItem::op("Duplicate Marker", "Shift D", Op::new("marker.duplicate")),
        MenuItem::op("Delete Marker", "X", Op::new("marker.delete")),
        MenuItem::Separator,
        MenuItem::op("Rename Marker", "F2", Op::new("marker.rename")),
        MenuItem::op("Move Marker", "G", Op::new("marker.move")),
        MenuItem::Separator,
        MenuItem::op("Jump to Next Marker", "", Op::new("screen.marker_jump")),
        MenuItem::op("Jump to Previous Marker", "", Op::new("screen.marker_jump")),
    ]
}

fn prefix(s: &State) -> &'static str {
    if s.graph() {
        "graph"
    } else {
        "action"
    }
}

fn select_menu(s: &State) -> Vec<MenuItem> {
    let p = prefix(s);
    vec![
        MenuItem::op("All", "A", Op::with(&format!("{p}.select_all"), "SELECT")),
        MenuItem::op("None", "Alt A", Op::with(&format!("{p}.select_all"), "DESELECT")),
        MenuItem::op("Invert", "Ctrl I", Op::with(&format!("{p}.select_all"), "INVERT")),
        MenuItem::Separator,
        MenuItem::op("Box Select", "B", anim::mail("select.box", s.id, "")),
    ]
}

fn channel_menu(_s: &State) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Delete Channels", "X", Op::new("anim.channels_delete")),
        MenuItem::Separator,
        MenuItem::op("Expand Channels", "Numpad +", Op::new("anim.channels_expand")),
        MenuItem::op("Collapse Channels", "Numpad -", Op::new("anim.channels_collapse")),
    ]
}

fn interp_items(p: &str) -> Vec<MenuItem> {
    let it = |l: &str, a: &str| MenuItem::op(l, "", Op::with(&format!("{p}.interpolation_type"), a));
    vec![MenuItem::Heading("Interpolation".into()), it("Constant", "CONSTANT"), it("Linear", "LINEAR"), it("Bezier", "BEZIER")]
}

fn key_menu(s: &State) -> Vec<MenuItem> {
    let p = prefix(s);
    vec![
        MenuItem::op("Transform", "G", anim::mail("transform.transform", s.id, "")),
        MenuItem::op("Jump to Keyframes", "Ctrl G", Op::new(&format!("{p}.frame_jump"))),
        MenuItem::Separator,
        MenuItem::op("Insert Keyframes", "I", Op::new(&format!("{p}.keyframe_insert"))),
        MenuItem::op("Duplicate", "Shift D", Op::new(&format!("{p}.duplicate"))),
        MenuItem::op("Delete Keyframes", "X", Op::new(&format!("{p}.delete"))),
        MenuItem::Separator,
        MenuItem::Sub("Interpolation Mode".into(), interp_items(p)),
        MenuItem::op("Snap", "Shift S", Op::new(&format!("{p}.snap"))),
    ]
}

// ---------------------------------------------------------------- header

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let mut x = hr.x;
    if s.kind != EditorKind::Timeline {
        let label = if s.graph() { "Graph Editor" } else { "Dope Sheet" };
        let mr = Rect::new(x, hr.y + 3, 100, hr.h - 6);
        if ui.dropdown(mr, label) {
            let items = if s.graph() {
                vec![MenuItem::op("Graph Editor", "", Op::new("graph.mode")), MenuItem::op("Drivers", "", Op::new("graph.drivers"))]
            } else {
                vec![
                    MenuItem::op("Dope Sheet", "", Op::new("action.mode")),
                    MenuItem::op("Action Editor", "", Op::new("action.action_editor")),
                    MenuItem::op("Shape Key Editor", "", Op::new("action.shape_key_editor")),
                    MenuItem::op("Grease Pencil", "", Op::new("action.gpencil")),
                    MenuItem::op("Mask", "", Op::new("action.mask")),
                    MenuItem::op("Cache File", "", Op::new("action.cache_file")),
                ]
            };
            ui.open_menu(mr, "", items.into_iter().filter(|i| !matches!(i, MenuItem::Op(l, _, _) if l == label)).collect());
        }
        x = mr.right() + 8;
    }
    let sv = view_menu(s);
    let ss = select_menu(s);
    let sc = channel_menu(s);
    let sk = key_menu(s);
    let view = move |_: &Doc| sv.clone();
    let sel = move |_: &Doc| ss.clone();
    let chan = move |_: &Doc| sc.clone();
    let key = move |_: &Doc| sk.clone();
    if s.kind == EditorKind::Timeline {
        let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 4] = [("Playback", &playback_menu), ("Keying", &keying_menu), ("View", &view), ("Marker", &marker_menu)];
        header_menus(ui, hr, x, &menus, d);
        transport(s, d, ui, hr);
    } else {
        let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 5] = [("View", &view), ("Select", &sel), ("Marker", &marker_menu), ("Channel", &chan), ("Key", &key)];
        let end = header_menus(ui, hr, x, &menus, d);
        // Right side: "Only Show Selected" and the snapping mode, as in Blender.
        let r = Rect::new(hr.right() - 120, hr.y + 3, 112, hr.h - 6);
        if r.x > end + 8 {
            ui.dropdown(r, "Nearest Frame");
            let o = Rect::new(r.x - 30, hr.y + 3, 24, hr.h - 6);
            ui.flat_button(o, "", true);
            arrow_icon(ui.p, o);
        }
    }
}

/// The "Only Show Selected" icon: a small pointer arrow.
fn arrow_icon(p: &mut Painter, r: Rect) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0 - 2.0, r.y as f32 + 4.0);
    tri(p, (cx, cy), (cx, cy + 11.0), (cx + 7.0, cy + 7.5), t::TEXT_HI);
}

/// Fill a triangle (no anti-aliasing; icons are small).
fn tri(p: &mut Painter, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: u32) {
    let ymin = a.1.min(b.1).min(c.1).floor() as i32;
    let ymax = a.1.max(b.1).max(c.1).ceil() as i32;
    for y in ymin..ymax {
        let yc = y as f32 + 0.5;
        let mut xs = [f32::INFINITY, f32::NEG_INFINITY];
        for (p0, p1) in [(a, b), (b, c), (c, a)] {
            if (p0.1 <= yc && p1.1 > yc) || (p1.1 <= yc && p0.1 > yc) {
                let x = p0.0 + (yc - p0.1) / (p1.1 - p0.1) * (p1.0 - p0.0);
                xs[0] = xs[0].min(x);
                xs[1] = xs[1].max(x);
            }
        }
        if xs[0] < xs[1] {
            let x0 = xs[0].round() as i32;
            p.fill(Rect::new(x0, y, (xs[1].round() as i32 - x0).max(1), 1), color);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    JumpStart,
    KeyPrev,
    PlayReverse,
    Play,
    Pause,
    KeyNext,
    JumpEnd,
}

fn draw_icon(p: &mut Painter, r: Rect, icon: Icon, color: u32) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    let left = |p: &mut Painter, x: f32| tri(p, (x, cy), (x + 6.0, cy - 5.0), (x + 6.0, cy + 5.0), color);
    let right = |p: &mut Painter, x: f32| tri(p, (x + 6.0, cy), (x, cy - 5.0), (x, cy + 5.0), color);
    match icon {
        Icon::JumpStart => {
            p.fill(Rect::new((cx - 7.0) as i32, (cy - 5.0) as i32, 2, 10), color);
            left(p, cx - 5.0);
            left(p, cx);
        }
        Icon::JumpEnd => {
            right(p, cx - 6.0);
            right(p, cx - 1.0);
            p.fill(Rect::new((cx + 5.0) as i32, (cy - 5.0) as i32, 2, 10), color);
        }
        Icon::KeyPrev => {
            left(p, cx - 7.0);
            p.diamond(cx + 3.0, cy, 4.5, color);
        }
        Icon::KeyNext => {
            p.diamond(cx - 3.0, cy, 4.5, color);
            right(p, cx + 1.0);
        }
        Icon::PlayReverse => left(p, cx - 3.0),
        Icon::Play => right(p, cx - 3.0),
        Icon::Pause => {
            p.fill(Rect::new((cx - 4.0) as i32, (cy - 5.0) as i32, 3, 10), color);
            p.fill(Rect::new((cx + 1.0) as i32, (cy - 5.0) as i32, 3, 10), color);
        }
    }
}

fn icon_button(ui: &mut Ui, r: Rect, icon: Icon, on: bool) -> bool {
    let hover = ui.hovered(r);
    if on {
        ui.p.round_rect(r, 4, t::SELECTED);
    } else if hover {
        ui.p.round_rect(r, 4, t::BUTTON_HOVER);
    }
    draw_icon(ui.p, r, icon, if on || hover { t::TEXT_HI } else { t::TEXT });
    ui.clicked(r)
}

/// Record button, transport controls and the frame fields.
fn transport(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let y = hr.y + 3;
    let h = hr.h - 6;
    // Right: current frame, then Start / End.
    let er = Rect::new(hr.right() - 96, y, 90, h);
    let sr = Rect::new(er.x - 94, y, 90, h);
    let fr = Rect::new(sr.x - 66, y, 58, h);
    let mut v = d.frame as f32;
    if ui.number(id(&format!("tl{}/frame", s.id)), fr, "", &mut v, 1.0, "") {
        d.frame = v.round() as i32;
    }
    let mut v = d.scene.frame_start as f32;
    if ui.number(id(&format!("tl{}/start", s.id)), sr, "Start", &mut v, 1.0, "") {
        d.scene.frame_start = (v.round() as i32).min(d.scene.frame_end);
    }
    let mut v = d.scene.frame_end as f32;
    if ui.number(id(&format!("tl{}/end", s.id)), er, "End", &mut v, 1.0, "") {
        d.scene.frame_end = (v.round() as i32).max(d.scene.frame_start);
    }
    // Centre: record, then the six transport buttons.
    let bw = 24;
    let total = bw + 10 + 6 * bw;
    let mut x = (hr.x + hr.w / 2 - total / 2).max(hr.x + 300).min(fr.x - total - 10);
    let rec = Rect::new(x, y, bw, h);
    let hover = ui.hovered(rec);
    if d.anim.auto_key {
        ui.p.round_rect(rec, 4, t::SELECTED);
    } else if hover {
        ui.p.round_rect(rec, 4, t::BUTTON_HOVER);
    }
    let (cx, cy) = rec.center();
    ui.p.circle(cx as f32, cy as f32, 5.0, if d.anim.auto_key { 0xff5050 } else { 0xb0b0b0 });
    if ui.clicked(rec) {
        d.anim.auto_key = !d.anim.auto_key;
    }
    x += bw + 10;
    let playing_fwd = d.playing && !d.anim.reverse;
    let playing_rev = d.playing && d.anim.reverse;
    let icons = [
        Icon::JumpStart,
        Icon::KeyPrev,
        if playing_rev { Icon::Pause } else { Icon::PlayReverse },
        if playing_fwd { Icon::Pause } else { Icon::Play },
        Icon::KeyNext,
        Icon::JumpEnd,
    ];
    for (i, icon) in icons.into_iter().enumerate() {
        let r = Rect::new(x + i as i32 * bw, y, bw - 2, h);
        if icon_button(ui, r, icon, false) {
            match i {
                0 => d.queue.push(Op::with("screen.frame_jump", "START")),
                1 => d.queue.push(Op::with("screen.keyframe_jump", "PREV")),
                2 => {
                    if d.playing && !d.anim.reverse {
                        d.anim.reverse = true;
                    } else {
                        d.anim.reverse = true;
                        d.queue.push(Op::new("screen.animation_play"));
                    }
                }
                3 => {
                    if d.playing && d.anim.reverse {
                        d.anim.reverse = false;
                    } else {
                        d.anim.reverse = false;
                        d.queue.push(Op::new("screen.animation_play"));
                    }
                }
                4 => d.queue.push(Op::with("screen.keyframe_jump", "NEXT")),
                _ => d.queue.push(Op::with("screen.frame_jump", "END")),
            }
        }
    }
}

// ---------------------------------------------------------------- rows

/// Animated objects shown ("Only Show Selected": selected objects only).
fn shown_objects(d: &Doc) -> Vec<usize> {
    let names: Vec<&str> = d.scene.selected.iter().filter_map(|&i| d.scene.objects.get(i)).map(|o| o.name.as_str()).collect();
    (0..d.anim.objects.len()).filter(|&i| names.contains(&d.anim.objects[i].object.as_str())).collect()
}

fn rows(s: &State, d: &Doc) -> Vec<Row> {
    let mut out = Vec::new();
    let objs = shown_objects(d);
    match s.kind {
        EditorKind::Timeline => out.push(Row::Summary),
        _ => {
            if !s.graph() {
                out.push(Row::Summary);
                if !s.summary_open {
                    return out;
                }
            }
            for o in objs {
                let oa = &d.anim.objects[o];
                out.push(Row::Object(o));
                if oa.expanded {
                    out.push(Row::Group(o));
                    if oa.group_expanded {
                        for c in 0..oa.curves.len() {
                            out.push(Row::Curve(o, c));
                        }
                    }
                }
            }
        }
    }
    out
}

/// The keys a row shows: (object, curve, key) triples.
fn row_keys(s: &State, d: &Doc, row: Row) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let objs = match row {
        Row::Summary => shown_objects(d),
        Row::Object(o) | Row::Group(o) | Row::Curve(o, _) => vec![o],
    };
    let _ = s;
    for o in objs {
        for (c, fc) in d.anim.objects[o].curves.iter().enumerate() {
            if let Row::Curve(_, rc) = row {
                if rc != c {
                    continue;
                }
            }
            for k in 0..fc.keys.len() {
                out.push((o, c, k));
            }
        }
    }
    out
}

/// Diamonds of a row: (frame, any selected), one per frame.
fn row_diamonds(s: &State, d: &Doc, row: Row) -> Vec<(f32, bool)> {
    let mut v: Vec<(f32, bool)> = row_keys(s, d, row)
        .into_iter()
        .map(|(o, c, k)| {
            let key = &d.anim.objects[o].curves[c].keys[k];
            (key.frame, key.select)
        })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f32, bool)> = Vec::new();
    for (f, sel) in v {
        match out.last_mut() {
            Some(l) if (l.0 - f).abs() < 0.01 => l.1 |= sel,
            _ => out.push((f, sel)),
        }
    }
    out
}

fn row_y(s: &State, i: usize) -> i32 {
    if s.kind == EditorKind::Timeline {
        s.keys_rect.y + 6
    } else {
        s.keys_rect.y + i as i32 * ROW_H
    }
}

// ---------------------------------------------------------------- body

fn nice_step(ppu: f32, min_px: f32) -> f32 {
    let mut step = 1.0f32;
    if ppu <= 0.0 {
        return 1.0;
    }
    let mut i = 0;
    while step * ppu < min_px && i < 40 {
        step *= [2.0, 2.5, 2.0][i % 3];
        i += 1;
    }
    step
}

fn nice_value_step(ppv: f32, min_px: f32) -> f32 {
    let raw = min_px / ppv.max(1e-6);
    let mag = 10f32.powf(raw.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * mag >= raw {
            return m * mag;
        }
    }
    10.0 * mag
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    for (op, _) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "view.frame_all" => s.want_frame_all = true,
            "view.frame_current" => center_on_frame(s, d.frame as f32),
            "select.box" => s.box_armed = true,
            "transform.transform" => start_move(s, d, false),
            _ => {}
        }
    }
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    let has_channels = s.kind != EditorKind::Timeline;
    let (chan, main) = if has_channels { r.take_left(CHANNEL_W.min(r.w / 2)) } else { (Rect::default(), r) };
    let (ruler, keys) = main.take_top(RULER_H);
    s.keys_rect = keys;
    if !s.framed && keys.w > 0 {
        s.framed = true;
        let (a, b) = (d.scene.frame_start as f32, d.scene.frame_end as f32);
        let span = (b - a).max(1.0) * 1.08;
        s.ppf = keys.w as f32 / span;
        s.x0 = a - (span - (b - a)) / 2.0;
        if s.graph() {
            s.y0 = -6.0;
            s.ppv = keys.h as f32 / 12.0;
            s.want_frame_all = true;
        }
    }
    if s.want_frame_all {
        frame_all(s, d);
        s.want_frame_all = false;
    }
    let rows = rows(s, d);

    // ---- input
    handle_input(s, d, ui, ruler, keys, chan, &rows);

    // ---- draw keys area
    let p = &mut *ui.p;
    p.fill(main, t::TIMELINE);
    // Grid.
    let step = nice_step(s.ppf, 50.0);
    let minor = (step / 5.0).max(1.0);
    let first = (s.x0 / minor).floor() * minor;
    let mut f = first;
    while s.fx(f) < keys.right() as f32 {
        let x = s.fx(f).round() as i32;
        let major = (f / step).round() * step == f;
        p.fill(Rect::new(x, keys.y, 1, keys.h), if major { GRID_MAJOR } else { GRID_MINOR });
        f += minor;
    }
    if s.graph() {
        let vs = nice_value_step(s.ppv, 40.0);
        let mut v = (s.y0 / vs).floor() * vs;
        while s.vy(v) > keys.y as f32 - 1.0 {
            let y = s.vy(v).round() as i32;
            p.fill(Rect::new(keys.x, y, keys.w, 1), if v.abs() < vs * 0.01 { 0x444444 } else { GRID_MAJOR });
            v += vs;
        }
    }
    // Row bands (dope sheet).
    if has_channels && !s.graph() {
        for (i, row) in rows.iter().enumerate() {
            let y = row_y(s, i);
            let band = match row {
                Row::Summary => Some(0x2f3a4c),
                Row::Object(_) => Some(0x353535),
                Row::Group(_) => Some(0x2f3631),
                Row::Curve(..) => None,
            };
            if let Some(c) = band {
                p.fill(Rect::new(keys.x, y + 1, keys.w, ROW_H - 2), c);
            }
        }
    }
    // Outside the scene range is darker.
    let xs = s.fx(d.scene.frame_start as f32).round() as i32;
    let xe = s.fx(d.scene.frame_end as f32).round() as i32;
    if xs > keys.x {
        p.fill_alpha(Rect::new(keys.x, keys.y, xs - keys.x, keys.h), 0x000000, 70);
    }
    if xe < keys.right() {
        p.fill_alpha(Rect::new(xe, keys.y, keys.right() - xe, keys.h), 0x000000, 70);
    }
    if s.graph() {
        draw_curves(s, d, p, keys);
    } else {
        p.push_clip(keys);
        for (i, row) in rows.iter().enumerate() {
            let cy = row_y(s, i) as f32 + ROW_H as f32 / 2.0;
            let big = matches!(row, Row::Summary | Row::Object(_)) && s.kind != EditorKind::Timeline;
            for (f, sel) in row_diamonds(s, d, *row) {
                key_diamond(p, s.fx(f), cy, if big { 5.5 } else { 5.0 }, sel);
            }
        }
        p.pop_clip();
    }
    // Box being dragged.
    if let Drag::Box { x0, y0, x1, y1, .. } = s.drag {
        let b = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
        p.fill_alpha(b, 0xffffff, 18);
        dashed_frame(p, b);
    }

    // ---- ruler
    p.fill(ruler, RULER_BG);
    p.fill(Rect::new(ruler.x, ruler.bottom() - 1, ruler.w, 1), 0x1a1a1a);
    let mut f = (s.x0 / step).floor() * step;
    p.push_clip(ruler);
    while s.fx(f) < ruler.right() as f32 + 40.0 {
        let x = s.fx(f).round() as i32;
        let label = format!("{}", f as i32);
        let w = p.text_width(&label);
        p.text(x - w / 2, ruler.y + 4, &label, t::TEXT_DIM);
        f += step;
    }
    p.pop_clip();
    // ---- playhead
    let px = s.fx(d.frame as f32).round() as i32;
    p.push_clip(main);
    p.fill(Rect::new(px - 1, keys.y, 2, keys.h), t::PLAYHEAD);
    let label = format!("{}", d.frame);
    let w = p.text_width(&label) + 12;
    let bx = Rect::new(px - w / 2, ruler.y + 2, w, ruler.h - 4);
    p.round_rect(bx, 4, t::PLAYHEAD);
    p.label(bx, &label, t::TEXT_HI, Align::Center);
    p.pop_clip();

    // ---- channel list
    if has_channels {
        draw_channels(s, d, p, chan, &rows);
    }
}

fn dashed_frame(p: &mut Painter, b: Rect) {
    let mut x = b.x;
    while x < b.right() {
        p.fill(Rect::new(x, b.y, 4.min(b.right() - x), 1), 0xffffff);
        p.fill(Rect::new(x, b.bottom() - 1, 4.min(b.right() - x), 1), 0xffffff);
        x += 8;
    }
    let mut y = b.y;
    while y < b.bottom() {
        p.fill(Rect::new(b.x, y, 1, 4.min(b.bottom() - y)), 0xffffff);
        p.fill(Rect::new(b.right() - 1, y, 1, 4.min(b.bottom() - y)), 0xffffff);
        y += 8;
    }
}

fn key_diamond(p: &mut Painter, x: f32, y: f32, r: f32, sel: bool) {
    p.diamond(x, y, r + 1.3, 0x000000);
    p.diamond(x, y, r, if sel { KEY_SEL } else { KEY_FILL });
}

fn draw_channels(s: &State, d: &Doc, p: &mut Painter, chan: Rect, rows: &[Row]) {
    p.fill(chan, CHANNEL_BG);
    p.fill(Rect::new(chan.right() - 1, chan.y, 1, chan.h), 0x1a1a1a);
    p.push_clip(Rect::new(chan.x, chan.y + RULER_H, chan.w, chan.h - RULER_H));
    for (i, row) in rows.iter().enumerate() {
        let y = row_y(s, i);
        let rr = Rect::new(chan.x + 2, y + 1, chan.w - 5, ROW_H - 2);
        let (indent, bg, text, open) = match *row {
            Row::Summary => (0, 0x3a4f70, "Summary".to_string(), Some(s.summary_open)),
            Row::Object(o) => (8, 0x434343, d.anim.objects[o].object.clone(), Some(d.anim.objects[o].expanded)),
            Row::Group(o) => (18, 0x2d4a33, "Object Transforms".to_string(), Some(d.anim.objects[o].group_expanded)),
            Row::Curve(o, c) => {
                let fc = &d.anim.objects[o].curves[c];
                (30, if fc.select { 0x3d4e6a } else { 0x333333 }, fc.label(), None)
            }
        };
        p.round_rect(rr, 3, bg);
        let mut x = rr.x + indent + 4;
        if let Some(open) = open {
            p.arrow((x + 4) as f32, (y + ROW_H / 2) as f32, 8.0, open, t::TEXT);
            x += 14;
        }
        match *row {
            Row::Object(_) => {
                // Object icon: a small orange triangle.
                let (cx, cy) = (x as f32 + 5.0, (y + ROW_H / 2) as f32);
                tri(p, (cx, cy - 5.0), (cx - 5.0, cy + 4.0), (cx + 5.0, cy + 4.0), 0xffa040);
                x += 16;
            }
            Row::Curve(o, c) => {
                let fc = &d.anim.objects[o].curves[c];
                if s.graph() {
                    p.round_rect(Rect::new(x, y + 6, 8, 8), 2, fc.color());
                    x += 13;
                }
            }
            _ => {}
        }
        p.label(Rect::new(x, y, rr.right() - x - 4, ROW_H), &text, if matches!(row, Row::Curve(..)) { t::TEXT } else { t::TEXT_HI }, Align::Left);
    }
    p.pop_clip();
}

fn draw_curves(s: &State, d: &Doc, p: &mut Painter, keys: Rect) {
    p.push_clip(keys);
    // Value labels along the left edge.
    let vs = nice_value_step(s.ppv, 40.0);
    let mut v = (s.y0 / vs).floor() * vs;
    while s.vy(v) > keys.y as f32 - 1.0 {
        let y = s.vy(v).round() as i32;
        let label = crate::ui::trim_number(v);
        p.text(keys.x + 4, y - 14, &label, t::TEXT_DIM);
        v += vs;
    }
    for o in shown_objects(d) {
        for fc in &d.anim.objects[o].curves {
            if fc.keys.is_empty() {
                continue;
            }
            let scale = if fc.prop == Prop::Rotation { 180.0 / core::f32::consts::PI } else { 1.0 };
            let col = fc.color();
            let alpha = if fc.select || !d.anim.objects[o].curves.iter().any(|c| c.select) { 255 } else { 120 };
            let mut prev: Option<(f32, f32)> = None;
            let mut x = keys.x;
            while x <= keys.right() {
                let f = s.frame_at(x);
                let y = s.vy(fc.evaluate(f) * scale);
                if let Some((px, py)) = prev {
                    p.line_alpha(px, py, x as f32, y, col, alpha);
                }
                prev = Some((x as f32, y));
                x += 2;
            }
            for k in &fc.keys {
                let (kx, ky) = (s.fx(k.frame), s.vy(k.value * scale));
                if k.select && k.interp == Interp::Bezier {
                    for h in [k.left, k.right] {
                        let (hx, hy) = (s.fx(h[0]), s.vy(h[1] * scale));
                        p.line(kx, ky, hx, hy, 0xf0af90);
                        p.circle(hx, hy, 2.5, 0xf0af90);
                    }
                }
                p.circle(kx, ky, 4.0, 0x000000);
                p.circle(kx, ky, 3.0, if k.select { 0xff8500 } else { 0x101010 });
                if !k.select {
                    p.ring(kx, ky, 3.2, 0xc8c8c8);
                }
            }
        }
    }
    p.pop_clip();
}

// ---------------------------------------------------------------- view

fn center_on_frame(s: &mut State, f: f32) {
    let w = s.keys_rect.w as f32 / s.ppf;
    s.x0 = f - w / 2.0;
}

fn frame_all(s: &mut State, d: &Doc) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    let mut vlo = f32::INFINITY;
    let mut vhi = f32::NEG_INFINITY;
    for o in shown_objects(d) {
        for fc in &d.anim.objects[o].curves {
            let scale = if fc.prop == Prop::Rotation { 180.0 / core::f32::consts::PI } else { 1.0 };
            for k in &fc.keys {
                lo = lo.min(k.frame);
                hi = hi.max(k.frame);
                vlo = vlo.min(k.value * scale);
                vhi = vhi.max(k.value * scale);
            }
        }
    }
    if !lo.is_finite() {
        lo = d.scene.frame_start as f32;
        hi = d.scene.frame_end as f32;
        vlo = -1.0;
        vhi = 1.0;
    }
    if hi - lo < 10.0 {
        let c = (hi + lo) / 2.0;
        lo = c - 5.0;
        hi = c + 5.0;
    }
    let w = s.keys_rect.w.max(1) as f32;
    let span = (hi - lo) * 1.1;
    s.ppf = w / span;
    s.x0 = lo - (span - (hi - lo)) / 2.0;
    if s.graph() {
        if vhi - vlo < 1e-3 {
            vlo -= 1.0;
            vhi += 1.0;
        }
        let h = s.keys_rect.h.max(1) as f32;
        let vspan = (vhi - vlo) * 1.2;
        s.ppv = h / vspan;
        s.y0 = vlo - (vspan - (vhi - vlo)) / 2.0;
    }
}

fn zoom_at(s: &mut State, x: i32, y: i32, factor: f32, vertical: bool) {
    let f = s.frame_at(x);
    s.ppf = (s.ppf * factor).clamp(0.05, 400.0);
    s.x0 = f - (x - s.keys_rect.x) as f32 / s.ppf;
    if vertical {
        let v = s.value_at(y);
        s.ppv = (s.ppv * factor).clamp(1e-3, 1e5);
        s.y0 = v - (s.keys_rect.bottom() - y) as f32 / s.ppv;
    }
}

// ---------------------------------------------------------------- picking and editing

/// Key under (x, y): the keys a click would select.
fn pick(s: &State, d: &Doc, rows: &[Row], x: i32, y: i32) -> Vec<(usize, usize, usize)> {
    if s.graph() {
        let mut best: Option<((usize, usize, usize), f32)> = None;
        for o in shown_objects(d) {
            for (c, fc) in d.anim.objects[o].curves.iter().enumerate() {
                let scale = if fc.prop == Prop::Rotation { 180.0 / core::f32::consts::PI } else { 1.0 };
                for (k, key) in fc.keys.iter().enumerate() {
                    let dx = s.fx(key.frame) - x as f32;
                    let dy = s.vy(key.value * scale) - y as f32;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < 8.0 && best.is_none_or(|b| dist < b.1) {
                        best = Some(((o, c, k), dist));
                    }
                }
            }
        }
        return best.map(|b| vec![b.0]).unwrap_or_default();
    }
    for (i, row) in rows.iter().enumerate() {
        let ry = row_y(s, i);
        if y < ry || y >= ry + ROW_H {
            continue;
        }
        let mut best: Option<(f32, f32)> = None;
        for (f, _) in row_diamonds(s, d, *row) {
            let dx = (s.fx(f) - x as f32).abs();
            if dx < 7.0 && best.is_none_or(|b| dx < b.1) {
                best = Some((f, dx));
            }
        }
        if let Some((f, _)) = best {
            return row_keys(s, d, *row).into_iter().filter(|&(o, c, k)| (d.anim.objects[o].curves[c].keys[k].frame - f).abs() < 0.01).collect();
        }
    }
    Vec::new()
}

fn deselect_all(d: &mut Doc) {
    for k in d.anim.keys_mut() {
        k.select = false;
    }
}

fn box_select(s: &State, d: &mut Doc, rows: &[Row], b: Rect) {
    let mut hits = Vec::new();
    if s.graph() {
        for o in shown_objects(d) {
            for (c, fc) in d.anim.objects[o].curves.iter().enumerate() {
                let scale = if fc.prop == Prop::Rotation { 180.0 / core::f32::consts::PI } else { 1.0 };
                for (k, key) in fc.keys.iter().enumerate() {
                    if b.contains(s.fx(key.frame).round() as i32, s.vy(key.value * scale).round() as i32) {
                        hits.push((o, c, k));
                    }
                }
            }
        }
    } else {
        let (f0, f1) = (s.frame_at(b.x), s.frame_at(b.right()));
        for (i, row) in rows.iter().enumerate() {
            let ry = row_y(s, i);
            if ry + ROW_H <= b.y || ry >= b.bottom() {
                continue;
            }
            for (o, c, k) in row_keys(s, d, *row) {
                let f = d.anim.objects[o].curves[c].keys[k].frame;
                if f >= f0 && f <= f1 {
                    hits.push((o, c, k));
                }
            }
        }
    }
    for (o, c, k) in hits {
        d.anim.objects[o].curves[c].keys[k].select = true;
    }
}

fn start_move(s: &mut State, d: &Doc, release_confirms: bool) {
    let mut orig = Vec::new();
    for o in shown_objects(d) {
        for (c, fc) in d.anim.objects[o].curves.iter().enumerate() {
            for (k, key) in fc.keys.iter().enumerate() {
                if key.select {
                    orig.push(KeyRef { o, c, k, frame: key.frame, value: key.value });
                }
            }
        }
    }
    if !orig.is_empty() {
        s.drag = Drag::Move { x: s.mouse.0, y: s.mouse.1, orig, release_confirms };
    }
}

fn apply_move(s: &State, d: &mut Doc, x: i32, y: i32, orig: &[KeyRef], mx: i32, my: i32) {
    let df = ((mx - x) as f32 / s.ppf).round();
    let dv = if s.graph() { (y - my) as f32 / s.ppv } else { 0.0 };
    for kr in orig {
        let Some(fc) = d.anim.objects.get_mut(kr.o).and_then(|o| o.curves.get_mut(kr.c)) else { continue };
        let scale = if fc.prop == Prop::Rotation { core::f32::consts::PI / 180.0 } else { 1.0 };
        if let Some(k) = fc.keys.get_mut(kr.k) {
            let (nf, nv) = (kr.frame + df, kr.value + dv * scale);
            let (ddf, ddv) = (nf - k.frame, nv - k.value);
            k.frame = nf;
            k.value = nv;
            k.left = [k.left[0] + ddf, k.left[1] + ddv];
            k.right = [k.right[0] + ddf, k.right[1] + ddv];
        }
    }
    d.anim.dirty = true;
}

fn finish_move(d: &mut Doc, orig: &[KeyRef], cancel: bool) {
    if !cancel {
        // Record the step with the keys where they started (handles shifted
        // back with them), then put the moved keys back.
        let get = |d: &mut Doc, kr: &KeyRef| d.anim.objects.get_mut(kr.o).and_then(|o| o.curves.get_mut(kr.c)).and_then(|c| c.keys.get_mut(kr.k)).map(|k| *k);
        let moved: Vec<Option<crate::anim::Keyframe>> = orig.iter().map(|kr| get(d, kr)).collect();
        let put = |d: &mut Doc, kr: &KeyRef, key: crate::anim::Keyframe| {
            if let Some(k) = d.anim.objects.get_mut(kr.o).and_then(|o| o.curves.get_mut(kr.c)).and_then(|c| c.keys.get_mut(kr.k)) {
                *k = key;
            }
        };
        for (kr, m) in orig.iter().zip(&moved) {
            if let Some(mut k) = *m {
                let (df, dv) = (k.frame - kr.frame, k.value - kr.value);
                k.frame = kr.frame;
                k.value = kr.value;
                k.left = [k.left[0] - df, k.left[1] - dv];
                k.right = [k.right[0] - df, k.right[1] - dv];
                put(d, kr, k);
            }
        }
        d.push_undo("Move Keyframes");
        for (kr, m) in orig.iter().zip(moved) {
            if let Some(k) = m {
                put(d, kr, k);
            }
        }
    }
    for kr in orig {
        if cancel {
            if let Some(k) = d.anim.objects.get_mut(kr.o).and_then(|o| o.curves.get_mut(kr.c)).and_then(|c| c.keys.get_mut(kr.k)) {
                k.frame = kr.frame;
                k.value = kr.value;
            }
        }
    }
    for o in &mut d.anim.objects {
        for c in &mut o.curves {
            c.sort();
        }
    }
    d.anim.dirty = true;
}

fn handle_input(s: &mut State, d: &mut Doc, ui: &mut Ui, ruler: Rect, keys: Rect, chan: Rect, rows: &[Row]) {
    let Some(input) = ui.input else { return };
    let clip = ui.p.clip();
    let inside = |r: Rect, x: i32, y: i32| r.contains(x, y) && clip.contains(x, y);
    match (&s.drag, input) {
        // Modal move: everything goes here until it ends.
        (Drag::Move { .. }, _) => {
            let Drag::Move { x, y, orig, release_confirms } = s.drag.clone() else { return };
            match input {
                Input::Move { x: mx, y: my } => apply_move(s, d, x, y, &orig, mx, my),
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => {
                    finish_move(d, &orig, false);
                    s.drag = Drag::None;
                }
                Input::Up { button: Button::Left, .. } if release_confirms => {
                    finish_move(d, &orig, false);
                    s.drag = Drag::None;
                }
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    finish_move(d, &orig, true);
                    s.drag = Drag::None;
                }
                _ => return,
            }
            ui.input = None;
        }
        (Drag::Scrub, Input::Move { x, .. }) => {
            d.frame = s.frame_at(x).round() as i32;
            ui.input = None;
        }
        (Drag::Scrub, Input::Up { button: Button::Left, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Pan { x: lx, y: ly }, Input::Move { x, y }) => {
            let (lx, ly) = (*lx, *ly);
            s.x0 -= (x - lx) as f32 / s.ppf;
            if s.graph() {
                s.y0 += (y - ly) as f32 / s.ppv;
            }
            s.drag = Drag::Pan { x, y };
            ui.input = None;
        }
        (Drag::Pan { .. }, Input::Up { button: Button::Middle, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Press { x: px, y: py, on_key, extend }, Input::Move { x, y }) => {
            let (px, py, on_key, extend) = (*px, *py, *on_key, *extend);
            if (x - px).abs() + (y - py).abs() > 3 {
                if on_key {
                    s.mouse = (px, py);
                    start_move(s, d, true);
                    if let Drag::Move { x: ox, y: oy, orig, .. } = s.drag.clone() {
                        apply_move(s, d, ox, oy, &orig, x, y);
                    }
                } else {
                    s.drag = Drag::Box { x0: px, y0: py, x1: x, y1: y, extend };
                }
            }
            ui.input = None;
        }
        (Drag::Press { on_key, extend, .. }, Input::Up { button: Button::Left, .. }) => {
            if !*on_key && !*extend {
                deselect_all(d);
            }
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Box { x0, y0, extend, .. }, Input::Move { x, y }) => {
            s.drag = Drag::Box { x0: *x0, y0: *y0, x1: x, y1: y, extend: *extend };
            ui.input = None;
        }
        (Drag::Box { x0, y0, x1, y1, extend }, Input::Up { button: Button::Left, .. }) => {
            let b = Rect::new((*x0).min(*x1), (*y0).min(*y1), (x1 - x0).abs() + 1, (y1 - y0).abs() + 1);
            if !*extend {
                deselect_all(d);
            }
            box_select(s, d, rows, b);
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::None, Input::Down { x, y, button: Button::Left, mods }) => {
            if inside(ruler, x, y) {
                s.drag = Drag::Scrub;
                d.frame = s.frame_at(x).round() as i32;
                ui.input = None;
            } else if inside(keys, x, y) {
                let hit = pick(s, d, rows, x, y);
                if s.box_armed {
                    s.box_armed = false;
                    s.drag = Drag::Box { x0: x, y0: y, x1: x, y1: y, extend: true };
                } else if !hit.is_empty() {
                    let all_sel = hit.iter().all(|&(o, c, k)| d.anim.objects[o].curves[c].keys[k].select);
                    if mods.shift {
                        for &(o, c, k) in &hit {
                            d.anim.objects[o].curves[c].keys[k].select = !all_sel;
                        }
                    } else if !all_sel {
                        deselect_all(d);
                        for &(o, c, k) in &hit {
                            d.anim.objects[o].curves[c].keys[k].select = true;
                        }
                    }
                    s.drag = Drag::Press { x, y, on_key: true, extend: mods.shift };
                } else if s.kind == EditorKind::Timeline {
                    // The Timeline scrubs anywhere that is not a key.
                    s.drag = Drag::Scrub;
                    d.frame = s.frame_at(x).round() as i32;
                } else {
                    s.drag = Drag::Press { x, y, on_key: false, extend: mods.shift };
                }
                ui.input = None;
            } else if inside(chan, x, y) {
                channel_click(s, d, rows, x, y, mods.shift);
                ui.input = None;
            }
        }
        (Drag::None, Input::Down { x, y, button: Button::Middle, .. }) if inside(Rect::new(ruler.x, ruler.y, ruler.w, keys.bottom() - ruler.y), x, y) => {
            s.drag = Drag::Pan { x, y };
            ui.input = None;
        }
        (Drag::None, Input::Wheel { x, y, steps, mods }) if inside(Rect::new(ruler.x, ruler.y, ruler.w, keys.bottom() - ruler.y), x, y) => {
            if mods.ctrl {
                s.x0 -= steps as f32 * 30.0 / s.ppf;
            } else if mods.shift && s.graph() {
                s.y0 += steps as f32 * 30.0 / s.ppv;
            } else {
                zoom_at(s, x, y, 1.2f32.powi(steps), s.graph());
            }
            ui.input = None;
        }
        _ => {}
    }
}

fn channel_click(s: &mut State, d: &mut Doc, rows: &[Row], x: i32, y: i32, extend: bool) {
    let Some(i) = rows.iter().enumerate().position(|(i, _)| y >= row_y(s, i) && y < row_y(s, i) + ROW_H) else { return };
    let chan_x = x - s.keys_rect.x + CHANNEL_W;
    let _ = chan_x;
    match rows[i] {
        Row::Summary => s.summary_open = !s.summary_open,
        Row::Object(o) => {
            let disclose = x < s.keys_rect.x - CHANNEL_W + 30;
            if disclose {
                d.anim.objects[o].expanded = !d.anim.objects[o].expanded;
            } else {
                let name = d.anim.objects[o].object.clone();
                if let Some(oi) = d.scene.find_object(&name) {
                    d.select_only(Some(oi));
                }
            }
        }
        Row::Group(o) => d.anim.objects[o].group_expanded = !d.anim.objects[o].group_expanded,
        Row::Curve(o, c) => {
            let was = d.anim.objects[o].curves[c].select;
            if !extend {
                for oa in &mut d.anim.objects {
                    for fc in &mut oa.curves {
                        fc.select = false;
                    }
                }
            }
            d.anim.objects[o].curves[c].select = !was || !extend;
        }
    }
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    let p = prefix(s);
    match (key, mods) {
        (Key::Char('i'), Mods::NONE) => d.queue.push(Op::new("anim.keyframe_insert")),
        (Key::Char('g'), Mods::NONE) => start_move(s, d, false),
        (Key::Char('x'), Mods::NONE) | (Key::Delete, Mods::NONE) => d.queue.push(Op::new(&format!("{p}.delete"))),
        (Key::Char('a'), Mods::NONE) => d.queue.push(Op::with(&format!("{p}.select_all"), "SELECT")),
        (Key::Char('a'), Mods::ALT) => d.queue.push(Op::with(&format!("{p}.select_all"), "DESELECT")),
        (Key::Char('i'), Mods::CTRL) => d.queue.push(Op::with(&format!("{p}.select_all"), "INVERT")),
        (Key::Char('b'), Mods::NONE) => s.box_armed = true,
        (Key::Char('g'), Mods::CTRL) => d.queue.push(Op::new(&format!("{p}.frame_jump"))),
        (Key::Home, Mods::NONE) => s.want_frame_all = true,
        (Key::NumDot, Mods::NONE) => center_on_frame(s, d.frame as f32),
        (Key::Home, Mods::CTRL) => d.queue.push(Op::new("anim.start_frame_set")),
        (Key::End, Mods::CTRL) => d.queue.push(Op::new("anim.end_frame_set")),
        (Key::Char('t'), Mods::NONE) => {
            // Blender's T menu; here it cycles the selected keys' interpolation.
            let cur = d.anim.objects.iter().flat_map(|o| o.curves.iter().flat_map(|c| c.keys.iter())).find(|k| k.select).map(|k| k.interp);
            let next = match cur {
                Some(Interp::Bezier) => "CONSTANT",
                Some(Interp::Constant) => "LINEAR",
                _ => "BEZIER",
            };
            d.queue.push(Op::with(&format!("{p}.interpolation_type"), next));
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::input::{Button, Input, Key, Mods};
    use crate::App;

    fn click(app: &mut App, x: i32, y: i32) {
        app.frame(Input::Move { x, y });
        app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Up { x, y, button: Button::Left });
    }

    fn timeline_rect(app: &App) -> crate::draw::Rect {
        let ws = &app.workspaces[app.active_ws];
        ws.areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::Timeline).unwrap().rect
    }

    #[test]
    fn i_keys_and_frames_interpolate() {
        let mut app = App::new(1280, 720);
        app.frame(Input::Tick);
        let cube = app.doc.active().unwrap();
        let tr = timeline_rect(&app);
        app.frame(Input::Move { x: tr.x + 300, y: tr.y + 60 });
        app.frame(Input::key(Key::Char('i')));
        app.doc.frame = 20;
        app.frame(Input::Tick);
        app.doc.scene.objects[cube].transform.location.x = 6.0;
        app.frame(Input::key(Key::Char('i')));
        assert_eq!(app.doc.anim.objects[0].key_frames(), vec![1.0, 20.0]);
        app.doc.frame = 10;
        app.frame(Input::Tick);
        let x = app.doc.scene.objects[cube].transform.location.x;
        assert!(x > 1.0 && x < 5.0, "interpolated x {x}");
        // Up/Down jump between keys.
        app.frame(Input::key(Key::Up));
        assert_eq!(app.doc.frame, 20);
        app.frame(Input::key(Key::Down));
        app.frame(Input::key(Key::Down));
        assert_eq!(app.doc.frame, 1);
    }

    #[test]
    fn ruler_click_sets_frame_and_keys_move() {
        let mut app = App::new(1280, 720);
        app.frame(Input::Tick);
        let tr = timeline_rect(&app);
        // Ruler sits right under the header.
        let ry = tr.y + crate::theme::HEADER_H + 1 + 8;
        let s = match &app.workspaces[0].areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::Timeline).unwrap().editor {
            crate::editors::Editor::Timeline(s) => (s.x0, s.ppf, s.keys_rect),
            _ => unreachable!(),
        };
        let fx = |f: f32| (s.2.x as f32 + (f - s.0) * s.1).round() as i32;
        click(&mut app, fx(100.0), ry);
        assert_eq!(app.doc.frame, 100);
        // Key at 100, select it by clicking its diamond, G and move by 10 frames.
        app.frame(Input::key(Key::Char('i')));
        let ky = s.2.y + 6 + 10;
        click(&mut app, fx(100.0), ky);
        assert!(app.doc.anim.objects[0].curves[0].keys[0].select);
        app.frame(Input::key(Key::Char('g')));
        app.frame(Input::Move { x: fx(110.0), y: ky });
        app.frame(Input::Down { x: fx(110.0), y: ky, button: Button::Left, mods: Mods::NONE });
        assert_eq!(app.doc.anim.objects[0].key_frames(), vec![110.0]);
        app.frame(Input::key(Key::Char('x')));
        assert!(app.doc.anim.objects.is_empty());
    }
}
