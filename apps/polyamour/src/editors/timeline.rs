// SPDX-License-Identifier: GPL-3.0-only
//! Blender's keyframe editors: the Timeline, the Dope Sheet (Dope Sheet,
//! Action Editor and Shape Key Editor modes), the Graph Editor and the
//! Drivers editor. They share one view (frames across, the scrubbing ruler
//! on top, the blue playhead) and the channel list on the left (Summary >
//! object > groups > F-curves); the Dope Sheet draws a row of keyframe
//! diamonds per channel, the Graph Editor the curves with their handles,
//! the Drivers editor the driver curves with the driver settings in the
//! sidebar.
//!
//! Keys are selected with a click (Shift extends, Alt selects the column),
//! B or a drag on empty space for a box; G moves, S scales, R rotates
//! (Graph) or sets the keyframe type (Dope Sheet), E extends, Shift+D
//! duplicates, X deletes, T / V / Shift+E open the interpolation, handle and
//! extrapolation menus, Home frames everything.
pub mod side;

use super::header_menus;
use crate::anim::{self, path, Easing, FCurve, HandleType, Interp, KeyRef, KeyType, Keyframe};
use crate::doc::{Doc, Mode};
use crate::draw::{Align, Painter, Rect};
use crate::editors::EditorKind;
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{id, MenuItem, Op, Ui};
use std::collections::HashMap;

const RULER_H: i32 = 22;
const ROW_H: i32 = 20;
const CHANNEL_W: i32 = 200;
pub const SIDE_W: i32 = 260;

const GRID_MAJOR: u32 = 0x373737;
const GRID_MINOR: u32 = 0x313131;
const RULER_BG: u32 = 0x232323;
const CHANNEL_BG: u32 = 0x282828;
const HANDLE: u32 = 0x9a9a9a;
const HANDLE_SEL: u32 = 0xf0af90;

/// The Dope Sheet's modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DsMode {
    DopeSheet,
    Action,
    ShapeKey,
}

impl DsMode {
    pub fn label(self) -> &'static str {
        match self {
            DsMode::DopeSheet => "Dope Sheet",
            DsMode::Action => "Action Editor",
            DsMode::ShapeKey => "Shape Key Editor",
        }
    }
}

/// What a running key transform does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum XMode {
    Move,
    Scale,
    Rotate,
    Extend,
}

/// One key under transform: where it was, and its owner (for NLA time).
#[derive(Clone, Copy, Debug)]
struct Orig {
    r: KeyRef,
    key: Keyframe,
    /// Display scale and offset of the curve's values (units, normalize).
    vs: f32,
}

#[derive(Clone, Debug)]
enum Drag {
    None,
    Scrub,
    Press { x: i32, y: i32, on_key: bool, extend: bool },
    Box { x0: i32, y0: i32, x1: i32, y1: i32, extend: bool },
    Xform { mode: XMode, x: i32, y: i32, orig: Vec<Orig>, release_confirms: bool, pivot: (f32, f32), undo_done: bool },
    Pan { x: i32, y: i32 },
    /// Dragging the channel list's scroll.
    ChanScroll { y: i32, start: i32 },
}

/// One row of the channel list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Summary,
    /// Animation data of `Anim::objects[o]`.
    Object(usize),
    Group(usize, String),
    /// Curve `c` of action `a`, owned by `Anim::objects[o]`.
    Curve(usize, usize, usize),
    /// Shape key `k` of scene object `oi` (Shape Key Editor).
    Shape(usize, usize),
    /// Driver `d` of `Anim::objects[o]`.
    Driver(usize, usize),
}

pub struct State {
    pub kind: EditorKind,
    id: u64,
    pub x0: f32,
    pub ppf: f32,
    pub y0: f32,
    pub ppv: f32,
    framed: bool,
    drag: Drag,
    box_armed: bool,
    mouse: (i32, i32),
    summary_open: bool,
    pub keys_rect: Rect,
    chan_rect: Rect,
    want_frame_all: bool,
    pub ds_mode: DsMode,
    pub only_selected: bool,
    pub normalize: bool,
    pub show_handles: bool,
    pub show_sidebar: bool,
    pub side_tab: side::Tab,
    pub chan_scroll: i32,
    /// The active channel (last clicked).
    pub active: Option<Row>,
    pub(crate) open: HashMap<String, bool>,
    pub(crate) undo_for: Option<Option<u64>>,
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
            chan_rect: Rect::default(),
            want_frame_all: false,
            ds_mode: DsMode::DopeSheet,
            only_selected: true,
            normalize: false,
            show_handles: true,
            show_sidebar: kind == EditorKind::Drivers,
            side_tab: if kind == EditorKind::Drivers { side::Tab::Drivers } else { side::Tab::FCurve },
            chan_scroll: 0,
            active: None,
            open: HashMap::new(),
            undo_for: None,
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
    /// Graph-like: the Graph Editor and the Drivers editor draw curves.
    pub fn graph(&self) -> bool {
        matches!(self.kind, EditorKind::GraphEditor | EditorKind::Drivers)
    }
    fn drivers(&self) -> bool {
        self.kind == EditorKind::Drivers
    }
    fn prefix(&self) -> &'static str {
        if self.graph() {
            "graph"
        } else {
            "action"
        }
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
        MenuItem::op(if d.playing { "Pause" } else { "Play Animation" }, "Space", Op::new("screen.animation_play")),
        MenuItem::op("Play Reverse", "Shift Ctrl Space", Op::new("screen.animation_play_reverse")),
    ]
}

pub fn keying_menu(d: &Doc) -> Vec<MenuItem> {
    let ks = d.anim.keying_set.as_deref().map(anim::keying_set_name).unwrap_or("None");
    let mut sets = vec![MenuItem::Check("None".into(), d.anim.keying_set.is_none(), Op::with("anim.keying_set_active_set", "NONE"))];
    sets.extend(anim::KEYING_SETS.iter().map(|(id, name)| MenuItem::Check(name.to_string(), d.anim.keying_set.as_deref() == Some(*id), Op::with("anim.keying_set_active_set", id))));
    vec![
        MenuItem::Sub(format!("Active Keying Set: {ks}"), sets),
        MenuItem::Separator,
        MenuItem::Check("Auto Keying".into(), d.anim.auto_key, Op::new("anim.auto_keying_toggle")),
        MenuItem::Separator,
        MenuItem::Sub("New Keyframe Type".into(), KeyType::ALL.iter().map(|k| MenuItem::Check(k.label().into(), d.anim.key_type == *k, Op::with("anim.default_key_type", k.id()))).collect()),
        MenuItem::Sub("Default Interpolation".into(), [Interp::Constant, Interp::Linear, Interp::Bezier].iter().map(|i| MenuItem::Check(i.label().into(), d.anim.interp == *i, Op::with("anim.default_interpolation", i.id()))).collect()),
        MenuItem::Sub("Default Handle Type".into(), HandleType::ALL.iter().map(|h| MenuItem::Check(h.label().into(), d.anim.handle == *h, Op::with("anim.default_handle_type", h.id()))).collect()),
        MenuItem::Separator,
        MenuItem::op("Insert Keyframe", "I", Op::new("anim.keyframe_insert")),
        MenuItem::op("Delete Keyframes", "Alt I", Op::new("anim.keyframe_delete")),
        MenuItem::op("Clear Keyframes", "Shift Alt I", Op::new("anim.keyframes_clear")),
    ]
}

fn view_menu(s: &State) -> Vec<MenuItem> {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    let mut v = vec![
        MenuItem::Check("Sidebar".into(), s.show_sidebar, anim::mail("view.sidebar", s.id, "")),
        MenuItem::Check("Only Show Selected".into(), s.only_selected, anim::mail("view.only_selected", s.id, "")),
    ];
    if s.graph() {
        v.push(MenuItem::Check("Show Handles".into(), s.show_handles, anim::mail("view.handles", s.id, "")));
        v.push(MenuItem::Check("Normalize".into(), s.normalize, anim::mail("view.normalize", s.id, "")));
    }
    v.extend([
        MenuItem::Separator,
        m("Frame All", "Home", "view.frame_all"),
        m("Go to Current Frame", "Numpad .", "view.frame_current"),
        MenuItem::Separator,
        MenuItem::op("Set Start Frame", "Ctrl Home", Op::new("anim.start_frame_set")),
        MenuItem::op("Set End Frame", "Ctrl End", Op::new("anim.end_frame_set")),
        MenuItem::Separator,
        MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area")),
    ]);
    v
}

fn marker_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![MenuItem::op("Add Marker", "M", Op::new("marker.add")), MenuItem::op("Move Marker", "G", Op::new("marker.move"))]
}

fn select_menu(s: &State) -> Vec<MenuItem> {
    let p = s.prefix();
    vec![
        MenuItem::op("All", "A", Op::with(&format!("{p}.select_all"), "SELECT")),
        MenuItem::op("None", "Alt A", Op::with(&format!("{p}.select_all"), "DESELECT")),
        MenuItem::op("Invert", "Ctrl I", Op::with(&format!("{p}.select_all"), "INVERT")),
        MenuItem::Separator,
        MenuItem::op("Box Select", "B", anim::mail("select.box", s.id, "")),
        MenuItem::Separator,
        MenuItem::op("Columns on Selected Keys", "K", Op::new(&format!("{p}.select_column"))),
        MenuItem::op("Column on Current Frame", "Ctrl K", Op::with(&format!("{p}.select_column"), "CFRA")),
    ]
}

fn channel_menu(s: &State) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Delete Channels", "X", anim::mail("channels.delete", s.id, "")),
        MenuItem::Separator,
        MenuItem::op("Expand Channels", "Numpad +", anim::mail("channels.expand", s.id, "")),
        MenuItem::op("Collapse Channels", "Numpad -", anim::mail("channels.collapse", s.id, "")),
        MenuItem::Separator,
        MenuItem::op("Extrapolation Mode", "Shift E", anim::mail("menu.extrapolation", s.id, "")),
    ]
}

pub fn interp_items(p: &str) -> Vec<MenuItem> {
    let it = |i: Interp| MenuItem::op(i.label(), "", Op::with(&format!("{p}.interpolation_type"), i.id()));
    let mut v = vec![MenuItem::Heading("Interpolation".into()), it(Interp::Constant), it(Interp::Linear), it(Interp::Bezier), MenuItem::Separator, MenuItem::Heading("Easing (by strength)".into())];
    v.extend(Interp::ALL.iter().skip(3).take(6).map(|&i| it(i)));
    v.push(MenuItem::Separator);
    v.push(MenuItem::Heading("Dynamic Effects".into()));
    v.extend(Interp::ALL.iter().skip(9).map(|&i| it(i)));
    v
}

pub fn easing_items(p: &str) -> Vec<MenuItem> {
    Easing::ALL.iter().map(|e| MenuItem::op(e.label(), "", Op::with(&format!("{p}.easing_type"), e.id()))).collect()
}

pub fn handle_items(p: &str) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Set Keyframe Handle Type".into())];
    v.extend(HandleType::ALL.iter().map(|h| MenuItem::op(h.label(), "", Op::with(&format!("{p}.handle_type"), h.id()))));
    v
}

pub fn keytype_items(p: &str) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Set Keyframe Type".into())];
    v.extend(KeyType::ALL.iter().map(|k| MenuItem::op(k.label(), "", Op::with(&format!("{p}.keyframe_type"), k.id()))));
    v
}

pub fn extrapolation_items(p: &str) -> Vec<MenuItem> {
    let e = |l: &str, a: &str| MenuItem::op(l, "", Op::with(&format!("{p}.extrapolation_type"), a));
    vec![
        MenuItem::Heading("Set F-Curve Extrapolation".into()),
        e("Constant Extrapolation", "CONSTANT"),
        e("Linear Extrapolation", "LINEAR"),
        e("Make Cyclic (F-Modifier)", "MAKE_CYCLIC"),
        e("Clear Cyclic (F-Modifier)", "CLEAR_CYCLIC"),
    ]
}

fn key_menu(s: &State) -> Vec<MenuItem> {
    let p = s.prefix();
    let mut v = vec![
        MenuItem::Sub(
            "Transform".into(),
            vec![
                MenuItem::op("Move", "G", anim::mail("transform.translate", s.id, "")),
                MenuItem::op("Extend", "E", anim::mail("transform.extend", s.id, "")),
                MenuItem::op("Scale", "S", anim::mail("transform.resize", s.id, "")),
            ],
        ),
        MenuItem::Sub(
            "Snap".into(),
            vec![MenuItem::op("Selection to Current Frame", "", Op::with(&format!("{p}.snap"), "CFRA")), MenuItem::op("Selection to Nearest Frame", "", Op::with(&format!("{p}.snap"), "NEAREST_FRAME"))],
        ),
        MenuItem::Sub(
            "Mirror".into(),
            vec![MenuItem::op("By Times over Current Frame", "", Op::with(&format!("{p}.mirror"), "CFRA")), MenuItem::op("By Values over Zero Value", "", Op::with(&format!("{p}.mirror"), "XAXIS"))],
        ),
        MenuItem::Separator,
        MenuItem::op("Insert Keyframes", "I", Op::new(&format!("{p}.keyframe_insert"))),
        MenuItem::op("Jump to Keyframes", "Ctrl G", Op::new(&format!("{p}.frame_jump"))),
        MenuItem::Separator,
        MenuItem::op("Duplicate", "Shift D", anim::mail("duplicate_move", s.id, "")),
        MenuItem::op("Delete Keyframes", "X", Op::new(&format!("{p}.delete"))),
        MenuItem::op("Clean Keyframes", "", Op::new(&format!("{p}.clean"))),
        MenuItem::Separator,
        MenuItem::Sub("Handle Type".into(), handle_items(p)),
        MenuItem::Sub("Interpolation Mode".into(), interp_items(p)),
        MenuItem::Sub("Easing Mode".into(), easing_items(p)),
        MenuItem::Sub("Extrapolation Mode".into(), extrapolation_items(p)),
    ];
    if !s.graph() {
        v.insert(9, MenuItem::Sub("Keyframe Type".into(), keytype_items(p)));
    } else {
        v.push(MenuItem::Separator);
        v.push(MenuItem::Sub(
            "Add F-Curve Modifier".into(),
            vec![MenuItem::op("Cycles", "", Op::with("graph.fmodifier_add", "CYCLES:ACTIVE")), MenuItem::op("Noise", "", Op::with("graph.fmodifier_add", "NOISE:ACTIVE"))],
        ));
    }
    v
}

fn mode_items(s: &State) -> Vec<MenuItem> {
    match s.kind {
        EditorKind::GraphEditor | EditorKind::Drivers => vec![
            MenuItem::Check("Graph Editor".into(), s.kind == EditorKind::GraphEditor, anim::mail("mode.graph", s.id, "")),
            MenuItem::Check("Drivers".into(), s.kind == EditorKind::Drivers, anim::mail("mode.drivers", s.id, "")),
        ],
        _ => [DsMode::DopeSheet, DsMode::Action, DsMode::ShapeKey].iter().map(|m| MenuItem::Check(m.label().into(), s.ds_mode == *m, anim::mail("mode.ds", s.id, m.label()))).collect(),
    }
}

// ---------------------------------------------------------------- header

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let mut x = hr.x;
    if s.kind != EditorKind::Timeline {
        let label = match s.kind {
            EditorKind::GraphEditor => "Graph Editor",
            EditorKind::Drivers => "Drivers",
            _ => s.ds_mode.label(),
        };
        let mr = Rect::new(x, hr.y + 3, 120, hr.h - 6);
        if ui.dropdown(mr, label) {
            ui.open_menu(mr, "", mode_items(s));
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
        return;
    }
    let end = if s.drivers() {
        let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 3] = [("View", &view), ("Select", &sel), ("Channel", &chan)];
        header_menus(ui, hr, x, &menus, d)
    } else {
        let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 5] = [("View", &view), ("Select", &sel), ("Marker", &marker_menu), ("Channel", &chan), ("Key", &key)];
        header_menus(ui, hr, x, &menus, d)
    };
    let mut x = end + 10;
    // Action Editor: the action selector, Push Down and Stash.
    if s.kind == EditorKind::DopeSheet && s.ds_mode == DsMode::Action {
        if let Some(a) = d.active() {
            let name = d.scene.objects[a].name.clone();
            let act = d.anim.get(&name).and_then(|o| o.action).and_then(|i| d.anim.actions.get(i)).map(|a| a.name.clone());
            let r = Rect::new(x, hr.y + 3, 150, hr.h - 6);
            if r.right() < hr.right() - 60 {
                if ui.dropdown(r, act.as_deref().unwrap_or("New Action")) {
                    let mut items: Vec<MenuItem> = d.anim.actions.iter().map(|a| MenuItem::op(&a.name, "", Op::with("action.assign", &a.name))).collect();
                    if !items.is_empty() {
                        items.push(MenuItem::Separator);
                    }
                    items.push(MenuItem::op("New Action", "", Op::new("action.new")));
                    if act.is_some() {
                        items.push(MenuItem::op("Unlink Action", "", Op::new("action.unlink")));
                    }
                    ui.open_menu(r, "Action", items);
                }
                x = r.right() + 6;
                for (label, op) in [("Push Down", "action.push_down"), ("Stash", "action.stash")] {
                    let w = ui.p.text_width(label) + 16;
                    let b = Rect::new(x, hr.y + 3, w, hr.h - 6);
                    if b.right() > hr.right() - 4 {
                        break;
                    }
                    if ui.button(b, label) {
                        d.queue.push(Op::new(op));
                    }
                    x = b.right() + 4;
                }
            }
        }
    }
    let _ = x;
    // Right side: Only Show Selected.
    let o = Rect::new(hr.right() - 30, hr.y + 3, 24, hr.h - 6);
    if o.x > end + 8 {
        if ui.flat_button(o, "", s.only_selected) {
            s.only_selected = !s.only_selected;
        }
        arrow_icon(ui.p, o);
    }
}

fn arrow_icon(p: &mut Painter, r: Rect) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0 - 2.0, r.y as f32 + 4.0);
    tri(p, (cx, cy), (cx, cy + 11.0), (cx + 7.0, cy + 7.5), t::TEXT_HI);
}

/// Fill a triangle (no anti-aliasing; icons are small).
pub fn tri(p: &mut Painter, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: u32) {
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

/// Record button (auto-keying), keying set, transport and the frame fields.
fn transport(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let y = hr.y + 3;
    let h = hr.h - 6;
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
    let bw = 24;
    let total = bw + 10 + 6 * bw;
    let mut x = (hr.x + hr.w / 2 - total / 2).max(hr.x + 300).min(fr.x - total - 10);
    // The record button turns auto-keying on.
    let rec = Rect::new(x, y, bw, h);
    let hover = ui.hovered(rec);
    if d.anim.auto_key {
        ui.p.round_rect(rec, 4, t::SELECTED);
    } else if hover {
        ui.p.round_rect(rec, 4, t::BUTTON_HOVER);
    }
    let (cx, cy) = rec.center();
    ui.p.circle(cx as f32, cy as f32, 5.0, if d.anim.auto_key { 0xff5050 } else { 0xb0b0b0 });
    if hover {
        ui.st.hint = "Automatic Keyframe Insertion for Objects and Bones".into();
    }
    if ui.clicked(rec) {
        d.anim.auto_key = !d.anim.auto_key;
    }
    x += bw + 10;
    let playing_fwd = d.playing && !d.anim.reverse;
    let playing_rev = d.playing && d.anim.reverse;
    let icons = [Icon::JumpStart, Icon::KeyPrev, if playing_rev { Icon::Pause } else { Icon::PlayReverse }, if playing_fwd { Icon::Pause } else { Icon::Play }, Icon::KeyNext, Icon::JumpEnd];
    for (i, icon) in icons.into_iter().enumerate() {
        let r = Rect::new(x + i as i32 * bw, y, bw - 2, h);
        if icon_button(ui, r, icon, false) {
            match i {
                0 => d.queue.push(Op::with("screen.frame_jump", "START")),
                1 => d.queue.push(Op::with("screen.keyframe_jump", "PREV")),
                2 => {
                    let was = d.playing && !d.anim.reverse;
                    d.anim.reverse = true;
                    if !was {
                        d.queue.push(Op::new("screen.animation_play"));
                    }
                }
                3 => {
                    let was = d.playing && d.anim.reverse;
                    d.anim.reverse = false;
                    if !was {
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

/// Animation data shown: selected objects ("Only Show Selected"), or all.
fn shown_objects(s: &State, d: &Doc) -> Vec<usize> {
    let names: Vec<&str> = d.scene.selected.iter().chain(d.scene.active_object.iter()).filter_map(|&i| d.scene.objects.get(i)).map(|o| o.name.as_str()).collect();
    (0..d.anim.objects.len()).filter(|&i| !s.only_selected || names.contains(&d.anim.objects[i].object.as_str())).collect()
}

/// Bones whose channels show (Pose Mode with Only Show Selected).
fn bone_filter(s: &State, d: &Doc, o: usize) -> Option<Vec<String>> {
    if !s.only_selected || d.mode != Mode::Pose {
        return None;
    }
    let oi = crate::rig::pose_object(d)?;
    (d.scene.objects[oi].name == d.anim.objects[o].object).then(|| crate::rig::selected_pose_bones(d, oi))
}

/// Curves of `o`'s edited action, grouped, in group order.
fn grouped(s: &State, d: &Doc, o: usize) -> Vec<(String, Vec<usize>)> {
    let Some(a) = d.anim.objects[o].edit_action().filter(|&a| a < d.anim.actions.len()) else { return Vec::new() };
    let act = &d.anim.actions[a];
    let filter = bone_filter(s, d, o);
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (c, fc) in act.curves.iter().enumerate() {
        if let (Some(f), Some(b)) = (&filter, path::bone_of(&fc.path)) {
            if !f.contains(&b) {
                continue;
            }
        }
        match out.iter_mut().find(|g| g.0 == fc.group) {
            Some(g) => g.1.push(c),
            None => out.push((fc.group.clone(), vec![c])),
        }
    }
    out
}

pub fn rows(s: &State, d: &Doc) -> Vec<Row> {
    let mut out = Vec::new();
    if s.kind == EditorKind::Timeline {
        out.push(Row::Summary);
        return out;
    }
    if s.drivers() {
        for o in shown_objects(s, d) {
            let oa = &d.anim.objects[o];
            if oa.drivers.is_empty() {
                continue;
            }
            out.push(Row::Object(o));
            if oa.expanded {
                for di in 0..oa.drivers.len() {
                    out.push(Row::Driver(o, di));
                }
            }
        }
        return out;
    }
    if s.kind == EditorKind::DopeSheet && s.ds_mode == DsMode::ShapeKey {
        out.push(Row::Summary);
        if let Some(a) = d.active() {
            if let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[a].data {
                for k in 1..d.scene.meshes[mi].shape_keys.len() {
                    out.push(Row::Shape(a, k));
                }
            }
        }
        return out;
    }
    let objs: Vec<usize> = if s.kind == EditorKind::DopeSheet && s.ds_mode == DsMode::Action {
        d.active().and_then(|a| d.anim.objects.iter().position(|o| o.object == d.scene.objects[a].name)).into_iter().collect()
    } else {
        shown_objects(s, d)
    };
    if !s.graph() {
        out.push(Row::Summary);
        if !s.summary_open {
            return out;
        }
    }
    for o in objs {
        let oa = &d.anim.objects[o];
        let groups = grouped(s, d, o);
        if groups.is_empty() {
            continue;
        }
        let action_mode = s.kind == EditorKind::DopeSheet && s.ds_mode == DsMode::Action;
        if !action_mode {
            out.push(Row::Object(o));
            if !oa.expanded {
                continue;
            }
        }
        let a = oa.edit_action().unwrap_or(0);
        for (g, cs) in groups {
            out.push(Row::Group(o, g.clone()));
            if oa.collapsed.contains(&g) {
                continue;
            }
            for c in cs {
                out.push(Row::Curve(o, a, c));
            }
        }
    }
    out
}

/// Keys a row shows, with their owner (for NLA time mapping).
fn row_keys(s: &State, d: &Doc, row: &Row) -> Vec<(usize, KeyRef)> {
    let mut out = Vec::new();
    let curves_of = |o: usize, only_group: Option<&str>| -> Vec<(usize, usize)> {
        let oa = &d.anim.objects[o];
        let Some(a) = oa.edit_action().filter(|&a| a < d.anim.actions.len()) else { return Vec::new() };
        let filter = bone_filter(s, d, o);
        d.anim.actions[a]
            .curves
            .iter()
            .enumerate()
            .filter(|(_, fc)| only_group.is_none_or(|g| fc.group == g))
            .filter(|(_, fc)| match (&filter, path::bone_of(&fc.path)) {
                (Some(f), Some(b)) => f.contains(&b),
                _ => true,
            })
            .map(|(c, _)| (a, c))
            .collect()
    };
    let push = |out: &mut Vec<(usize, KeyRef)>, o: usize, a: usize, c: usize| {
        for k in 0..d.anim.actions[a].curves[c].keys.len() {
            out.push((o, KeyRef { a, c, k }));
        }
    };
    match row {
        Row::Summary => {
            if s.kind == EditorKind::DopeSheet && s.ds_mode == DsMode::ShapeKey {
                for r in rows(s, d).iter().filter(|r| matches!(r, Row::Shape(..))) {
                    out.extend(row_keys(s, d, r));
                }
                return out;
            }
            let objs: Vec<usize> = if s.kind == EditorKind::Timeline {
                let mut v = shown_objects(s, d);
                // The Timeline shows the posed armature's selected bones only.
                v.retain(|&o| !d.anim.objects[o].object.is_empty());
                v
            } else if s.ds_mode == DsMode::Action && s.kind == EditorKind::DopeSheet {
                d.active().and_then(|a| d.anim.objects.iter().position(|o| o.object == d.scene.objects[a].name)).into_iter().collect()
            } else {
                shown_objects(s, d)
            };
            for o in objs {
                for (a, c) in curves_of(o, None) {
                    push(&mut out, o, a, c);
                }
            }
        }
        Row::Object(o) => {
            for (a, c) in curves_of(*o, None) {
                push(&mut out, *o, a, c);
            }
        }
        Row::Group(o, g) => {
            for (a, c) in curves_of(*o, Some(g)) {
                push(&mut out, *o, a, c);
            }
        }
        Row::Curve(o, a, c) => push(&mut out, *o, *a, *c),
        Row::Shape(oi, k) => {
            let name = &d.scene.objects[*oi].name;
            let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[*oi].data else { return out };
            let Some(key) = d.scene.meshes[mi].shape_keys.get(*k) else { return out };
            let p = path::shape_key_path(&key.name);
            if let (Some(o), Some(a)) = (d.anim.objects.iter().position(|x| &x.object == name), d.anim.edit_action(name)) {
                if let Some(c) = d.anim.actions[a].find(&p, 0) {
                    push(&mut out, o, a, c);
                }
            }
        }
        Row::Driver(..) => {}
    }
    out
}

/// Diamonds of a row: (scene frame, any selected, key type), one per frame.
fn row_diamonds(s: &State, d: &Doc, row: &Row) -> Vec<(f32, bool, KeyType)> {
    let mut v: Vec<(f32, bool, KeyType)> = row_keys(s, d, row)
        .into_iter()
        .filter_map(|(o, r)| {
            let k = d.anim.key(r)?;
            Some((d.anim.objects[o].scene_time(k.frame), k.select, k.key_type))
        })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f32, bool, KeyType)> = Vec::new();
    for (f, sel, kt) in v {
        match out.last_mut() {
            Some(l) if (l.0 - f).abs() < 0.01 => l.1 |= sel,
            _ => out.push((f, sel, kt)),
        }
    }
    out
}

fn row_y(s: &State, i: usize) -> i32 {
    if s.kind == EditorKind::Timeline {
        s.keys_rect.y + 6
    } else {
        s.keys_rect.y + i as i32 * ROW_H - s.chan_scroll
    }
}

/// Curves the Graph / Drivers editor draws: (owner, action, curve).
fn graph_curves(s: &State, d: &Doc, rows: &[Row]) -> Vec<(usize, usize, usize)> {
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for o in shown_objects(s, d) {
        for (_, cs) in grouped(s, d, o) {
            let a = d.anim.objects[o].edit_action().unwrap_or(0);
            for c in cs {
                if !d.anim.actions[a].curves[c].hide {
                    out.push((o, a, c));
                }
            }
        }
    }
    let _ = rows;
    out
}

/// Display mapping of a curve's values: unit (degrees for angles) and
/// normalisation. Returns (scale, offset): shown = value * scale + offset.
fn value_map(s: &State, fc: &FCurve) -> (f32, f32) {
    let unit = if path::is_angle(&fc.path) { 180.0 / core::f32::consts::PI } else { 1.0 };
    if !s.normalize {
        return (unit, 0.0);
    }
    let (lo, hi) = fc.keys.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), k| (lo.min(k.value), hi.max(k.value)));
    if !lo.is_finite() || hi - lo < 1e-6 {
        return (1.0, -lo.min(1e9).max(-1e9) * 0.0);
    }
    let sc = 2.0 / (hi - lo);
    (sc, -1.0 - lo * sc)
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
    for (op, arg) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "view.frame_all" => s.want_frame_all = true,
            "view.frame_current" => center_on_frame(s, d.frame as f32),
            "view.sidebar" => s.show_sidebar = !s.show_sidebar,
            "view.only_selected" => s.only_selected = !s.only_selected,
            "view.handles" => s.show_handles = !s.show_handles,
            "view.normalize" => {
                s.normalize = !s.normalize;
                s.want_frame_all = true;
            }
            "select.box" => s.box_armed = true,
            "transform.translate" | "transform.transform" => start_xform(s, d, XMode::Move, false),
            "transform.resize" => start_xform(s, d, XMode::Scale, false),
            "transform.extend" => start_xform(s, d, XMode::Extend, false),
            "duplicate_move" => {
                d.push_undo("Duplicate Keyframes");
                anim::duplicate_selected_keys(d);
                start_xform(s, d, XMode::Move, false);
                if let Drag::Xform { undo_done, .. } = &mut s.drag {
                    *undo_done = true;
                }
            }
            "mode.graph" => {
                s.kind = EditorKind::GraphEditor;
                s.side_tab = side::Tab::FCurve;
                s.framed = false;
            }
            "mode.drivers" => {
                s.kind = EditorKind::Drivers;
                s.side_tab = side::Tab::Drivers;
                s.show_sidebar = true;
                s.framed = false;
            }
            "mode.ds" => {
                s.ds_mode = match arg.as_str() {
                    "Action Editor" => DsMode::Action,
                    "Shape Key Editor" => DsMode::ShapeKey,
                    _ => DsMode::DopeSheet,
                }
            }
            "channels.expand" | "channels.collapse" => {
                let open = op == "channels.expand";
                for oa in &mut d.anim.objects {
                    oa.expanded = open;
                    if open {
                        oa.collapsed.clear();
                    }
                }
            }
            "channels.delete" => delete_channels(s, d),
            "menu.extrapolation" => d.menu = Some(crate::ui::Popup::new("Extrapolation", extrapolation_items(s.prefix()), s.mouse.0, s.mouse.1)),
            _ => {}
        }
    }
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    let has_channels = s.kind != EditorKind::Timeline;
    let (r, side_r) = if s.show_sidebar && has_channels && r.w > SIDE_W + 300 { r.take_right(SIDE_W) } else { (r, Rect::default()) };
    let (chan, main) = if has_channels { r.take_left(CHANNEL_W.min(r.w / 2)) } else { (Rect::default(), r) };
    let (ruler, keys) = main.take_top(RULER_H);
    s.keys_rect = keys;
    s.chan_rect = chan;
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
    // Keep the channel scroll in range.
    let max_scroll = (rows.len() as i32 * ROW_H - keys.h + ROW_H).max(0);
    s.chan_scroll = s.chan_scroll.clamp(0, max_scroll);

    // ---- sidebar first: its widgets take their events.
    if !side_r.is_empty() {
        side::draw(s, d, ui, side_r, &rows);
    }

    handle_input(s, d, ui, ruler, keys, chan, &rows);

    let p = &mut *ui.p;
    p.fill(main, t::TIMELINE);
    let step = nice_step(s.ppf, 50.0);
    let minor = (step / 5.0).max(1.0);
    let mut f = (s.x0 / minor).floor() * minor;
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
    if has_channels && !s.graph() {
        p.push_clip(keys);
        for (i, row) in rows.iter().enumerate() {
            let y = row_y(s, i);
            let band = match row {
                Row::Summary => Some(0x2f3a4c),
                Row::Object(_) => Some(0x353535),
                Row::Group(..) => Some(0x2f3631),
                _ => None,
            };
            if let Some(c) = band {
                p.fill(Rect::new(keys.x, y + 1, keys.w, ROW_H - 2), c);
            }
        }
        p.pop_clip();
    }
    let xs = s.fx(d.scene.frame_start as f32).round() as i32;
    let xe = s.fx(d.scene.frame_end as f32).round() as i32;
    if !s.drivers() {
        if xs > keys.x {
            p.fill_alpha(Rect::new(keys.x, keys.y, xs - keys.x, keys.h), 0x000000, 70);
        }
        if xe < keys.right() {
            p.fill_alpha(Rect::new(xe, keys.y, keys.right() - xe, keys.h), 0x000000, 70);
        }
    }
    if s.drivers() {
        draw_drivers(s, d, p, keys);
    } else if s.graph() {
        draw_curves(s, d, p, keys, &rows);
    } else {
        p.push_clip(keys);
        for (i, row) in rows.iter().enumerate() {
            let cy = row_y(s, i) as f32 + ROW_H as f32 / 2.0;
            if cy < keys.y as f32 - ROW_H as f32 || cy > keys.bottom() as f32 + ROW_H as f32 {
                continue;
            }
            let big = matches!(row, Row::Summary | Row::Object(_)) && s.kind != EditorKind::Timeline;
            let ds = row_diamonds(s, d, row);
            // Holds: a bar between equal neighbouring keys.
            for w in ds.windows(2) {
                let _ = w;
            }
            for (f, sel, kt) in ds {
                key_diamond(p, s.fx(f), cy, if big { 5.5 } else { 5.0 } * kt.scale(), sel, kt);
            }
        }
        p.pop_clip();
    }
    if let Drag::Box { x0, y0, x1, y1, .. } = s.drag {
        let b = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
        p.fill_alpha(b, 0xffffff, 18);
        dashed_frame(p, b);
    }
    // Ruler.
    p.fill(ruler, RULER_BG);
    p.fill(Rect::new(ruler.x, ruler.bottom() - 1, ruler.w, 1), 0x1a1a1a);
    let mut f = (s.x0 / step).floor() * step;
    p.push_clip(ruler);
    while s.fx(f) < ruler.right() as f32 + 40.0 {
        let x = s.fx(f).round() as i32;
        let label = if s.drivers() { crate::ui::trim_number(f) } else { format!("{}", f as i32) };
        let w = p.text_width(&label);
        p.text(x - w / 2, ruler.y + 4, &label, t::TEXT_DIM);
        f += step;
    }
    p.pop_clip();
    if !s.drivers() {
        let px = s.fx(d.frame as f32).round() as i32;
        p.push_clip(main);
        p.fill(Rect::new(px - 1, keys.y, 2, keys.h), t::PLAYHEAD);
        let label = format!("{}", d.frame);
        let w = p.text_width(&label) + 12;
        let bx = Rect::new(px - w / 2, ruler.y + 2, w, ruler.h - 4);
        p.round_rect(bx, 4, t::PLAYHEAD);
        p.label(bx, &label, t::TEXT_HI, Align::Center);
        p.pop_clip();
    }
    if has_channels {
        draw_channels(s, d, ui, chan, &rows);
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

fn key_diamond(p: &mut Painter, x: f32, y: f32, r: f32, sel: bool, kt: KeyType) {
    let (fill, sfill) = kt.colors();
    p.diamond(x, y, r + 1.3, 0x000000);
    p.diamond(x, y, r, if sel { sfill } else { fill });
}

fn row_label(d: &Doc, row: &Row) -> (i32, u32, String, Option<bool>) {
    match row {
        Row::Summary => (0, 0x3a4f70, "Summary".into(), None),
        Row::Object(o) => (8, 0x434343, d.anim.objects[*o].object.clone(), Some(d.anim.objects[*o].expanded)),
        Row::Group(o, g) => (18, 0x2d4a33, g.clone(), Some(!d.anim.objects[*o].collapsed.contains(g))),
        Row::Curve(_, a, c) => {
            let fc = &d.anim.actions[*a].curves[*c];
            (30, if fc.select { 0x3d4e6a } else { 0x333333 }, fc.label(), None)
        }
        Row::Shape(oi, k) => {
            let name = match d.scene.objects[*oi].data {
                sex_3d::scene::ObjectData::Mesh(mi) => d.scene.meshes[mi].shape_keys.get(*k).map(|s| s.name.clone()).unwrap_or_default(),
                _ => String::new(),
            };
            (12, 0x333333, name, None)
        }
        Row::Driver(o, di) => {
            let drv = &d.anim.objects[*o].drivers[*di];
            (18, if drv.select { 0x3d4e6a } else { 0x333333 }, drv.label(), None)
        }
    }
}

fn draw_channels(s: &mut State, d: &mut Doc, ui: &mut Ui, chan: Rect, rows: &[Row]) {
    let p = &mut *ui.p;
    p.fill(chan, CHANNEL_BG);
    p.fill(Rect::new(chan.right() - 1, chan.y, 1, chan.h), 0x1a1a1a);
    let clip = Rect::new(chan.x, chan.y + RULER_H, chan.w, chan.h - RULER_H);
    p.push_clip(clip);
    let summary_open = s.summary_open;
    for (i, row) in rows.iter().enumerate() {
        let y = row_y(s, i);
        if y + ROW_H < clip.y || y > clip.bottom() {
            continue;
        }
        let rr = Rect::new(chan.x + 2, y + 1, chan.w - 5, ROW_H - 2);
        let (indent, bg, text, open) = row_label(d, row);
        let open = if matches!(row, Row::Summary) && s.kind != EditorKind::DopeSheet || s.ds_mode != DsMode::ShapeKey { if matches!(row, Row::Summary) { Some(summary_open) } else { open } } else { open };
        let active = s.active.as_ref() == Some(row);
        ui.p.round_rect(rr, 3, if active { 0x4a5a78 } else { bg });
        let mut x = rr.x + indent + 4;
        if let Some(open) = open {
            ui.p.arrow((x + 4) as f32, (y + ROW_H / 2) as f32, 8.0, open, t::TEXT);
            x += 14;
        }
        match row {
            Row::Object(_) => {
                let (cx, cy) = (x as f32 + 5.0, (y + ROW_H / 2) as f32);
                tri(ui.p, (cx, cy - 5.0), (cx - 5.0, cy + 4.0), (cx + 5.0, cy + 4.0), 0xffa040);
                x += 16;
            }
            Row::Curve(_, a, c) if s.graph() => {
                let fc = &d.anim.actions[*a].curves[*c];
                ui.p.round_rect(Rect::new(x, y + 6, 8, 8), 2, fc.color());
                x += 13;
            }
            Row::Driver(o, di) => {
                let drv = &d.anim.objects[*o].drivers[*di];
                if drv.error.is_some() {
                    ui.p.round_rect(Rect::new(x, y + 6, 8, 8), 2, 0xd04040);
                    x += 13;
                }
            }
            _ => {}
        }
        let right_w = match row {
            Row::Shape(..) => 70,
            Row::Curve(..) => 34,
            _ => 4,
        };
        ui.p.label(Rect::new(x, y, rr.right() - x - right_w, ROW_H), &text, if matches!(row, Row::Curve(..) | Row::Driver(..) | Row::Shape(..)) { t::TEXT } else { t::TEXT_HI }, Align::Left);
        // Curve rows: mute and lock toggles at the right.
        if let Row::Curve(_, a, c) = row {
            let fc = &d.anim.actions[*a].curves[*c];
            let mr = Rect::new(rr.right() - 30, y + 3, 13, ROW_H - 6);
            let lr = Rect::new(rr.right() - 15, y + 3, 13, ROW_H - 6);
            ui.p.round_rect(mr, 2, if fc.mute { 0x6a6a6a } else { 0x262626 });
            ui.p.round_rect(lr, 2, if fc.lock { 0x6a6a6a } else { 0x262626 });
            let (mute, lock) = (fc.mute, fc.lock);
            if ui.clicked(mr) {
                d.queue.push(Op::with("anim.prop_set", &format!("fcu:{a}:{c}\tmute={}", !mute as u8)));
            }
            if ui.clicked(lr) {
                d.queue.push(Op::with("anim.prop_set", &format!("fcu:{a}:{c}\tlock={}", !lock as u8)));
            }
        }
        // Shape Key Editor: the key's value slider.
        if let Row::Shape(oi, k) = row {
            if let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[*oi].data {
                if let Some(key) = d.scene.meshes[mi].shape_keys.get(*k) {
                    let (lo, hi) = (key.slider_min, key.slider_max);
                    let mut v = key.value;
                    let fr = Rect::new(rr.right() - 66, y + 2, 64, ROW_H - 4);
                    let wid = id(&format!("tl{}/sk{}/{}", s.id, oi, k));
                    if ui.number_ex(wid, fr, "", &mut v, crate::ui::NumOpts { step: 0.01, min: lo, max: hi, slider: true, ..Default::default() }) {
                        if ui.st.active.is_none() || s.undo_for != Some(ui.st.active) {
                            d.push_undo("Shape Key Value");
                            s.undo_for = Some(ui.st.active);
                        }
                        d.scene.meshes[mi].shape_keys[*k].value = v;
                        if d.anim.auto_key {
                            let p = path::shape_key_path(&d.scene.meshes[mi].shape_keys[*k].name);
                            anim::insert_key_value(d, *oi, &p, 0, d.frame as f32, v);
                        }
                        d.touch();
                    }
                }
            }
        }
    }
    ui.p.pop_clip();
    if ui.st.active.is_none() {
        s.undo_for = None;
    }
}

fn draw_curve_line(s: &State, p: &mut Painter, keys: Rect, fc: &FCurve, map: (f32, f32), col: u32, alpha: u32, time: &dyn Fn(f32) -> f32) {
    let mut prev: Option<(f32, f32)> = None;
    let mut x = keys.x;
    while x <= keys.right() {
        let f = time(s.frame_at(x));
        let y = s.vy(fc.evaluate(f) * map.0 + map.1);
        if let Some((px, py)) = prev {
            p.line_alpha(px, py, x as f32, y, col, alpha);
        }
        prev = Some((x as f32, y));
        x += 2;
    }
}

fn draw_curves(s: &State, d: &Doc, p: &mut Painter, keys: Rect, rows: &[Row]) {
    p.push_clip(keys);
    let vs = nice_value_step(s.ppv, 40.0);
    let mut v = (s.y0 / vs).floor() * vs;
    while s.vy(v) > keys.y as f32 - 1.0 {
        let y = s.vy(v).round() as i32;
        p.text(keys.x + 4, y - 14, &crate::ui::trim_number(v), t::TEXT_DIM);
        v += vs;
    }
    let curves = graph_curves(s, d, rows);
    let any_sel = curves.iter().any(|&(_, a, c)| d.anim.actions[a].curves[c].select);
    for &(o, a, c) in &curves {
        let fc = &d.anim.actions[a].curves[c];
        if fc.keys.is_empty() {
            continue;
        }
        let oa = &d.anim.objects[o];
        let map = value_map(s, fc);
        let col = fc.color();
        let alpha = if fc.select || !any_sel { 255 } else { 110 };
        draw_curve_line(s, p, keys, fc, map, if fc.mute { 0x707070 } else { col }, alpha, &|f| oa.action_time(f));
        for k in &fc.keys {
            let kx = s.fx(oa.scene_time(k.frame));
            let ky = s.vy(k.value * map.0 + map.1);
            let bez = k.interp == Interp::Bezier;
            if s.show_handles && bez && (fc.select || k.any_selected()) {
                for (h, hs) in [(k.left, k.select_left), (k.right, k.select_right)] {
                    let (hx, hy) = (s.fx(oa.scene_time(h[0])), s.vy(h[1] * map.0 + map.1));
                    p.line(kx, ky, hx, hy, if hs { HANDLE_SEL } else { HANDLE });
                    p.circle(hx, hy, 3.0, 0x000000);
                    p.circle(hx, hy, 2.2, if hs { HANDLE_SEL } else { 0xd0d0d0 });
                }
            }
            p.circle(kx, ky, 4.0, 0x000000);
            p.circle(kx, ky, 3.0, if k.select { 0xff8500 } else { 0x101010 });
            if !k.select {
                p.ring(kx, ky, 3.2, 0xc8c8c8);
            }
        }
    }
    p.pop_clip();
}

/// Drivers editor: the driver's mapping (input across, output up) with its
/// current value.
fn draw_drivers(s: &State, d: &Doc, p: &mut Painter, keys: Rect) {
    p.push_clip(keys);
    for o in shown_objects(s, d) {
        for drv in &d.anim.objects[o].drivers {
            let col = anim::FCurve::new(&drv.path, drv.index).color();
            let alpha = if drv.select { 255 } else { 120 };
            match &drv.curve {
                Some(fc) if !fc.keys.is_empty() => draw_curve_line(s, p, keys, fc, (1.0, 0.0), col, alpha, &|f| f),
                _ => {
                    let (x0, x1) = (s.frame_at(keys.x), s.frame_at(keys.right()));
                    p.line_alpha(s.fx(x0), s.vy(x0), s.fx(x1), s.vy(x1), col, alpha);
                }
            }
            let (vx, vy) = (s.fx(drv.value), s.vy(drv.value));
            p.circle(vx, vy, 4.0, 0x000000);
            p.circle(vx, vy, 3.0, if drv.error.is_some() { 0xd04040 } else { col });
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
    let (mut lo, mut hi, mut vlo, mut vhi) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    if s.drivers() {
        for o in shown_objects(s, d) {
            for drv in &d.anim.objects[o].drivers {
                for v in [drv.value - 1.0, drv.value + 1.0] {
                    lo = lo.min(v);
                    hi = hi.max(v);
                    vlo = vlo.min(v);
                    vhi = vhi.max(v);
                }
            }
        }
    } else {
        for o in shown_objects(s, d) {
            let oa = &d.anim.objects[o];
            let Some(a) = oa.edit_action().filter(|&a| a < d.anim.actions.len()) else { continue };
            for fc in &d.anim.actions[a].curves {
                let map = value_map(s, fc);
                for k in &fc.keys {
                    let f = oa.scene_time(k.frame);
                    lo = lo.min(f);
                    hi = hi.max(f);
                    vlo = vlo.min(k.value * map.0 + map.1);
                    vhi = vhi.max(k.value * map.0 + map.1);
                }
            }
        }
    }
    if !lo.is_finite() {
        lo = d.scene.frame_start as f32;
        hi = d.scene.frame_end as f32;
        vlo = -1.0;
        vhi = 1.0;
    }
    let min_span = if s.drivers() { 2.0 } else { 10.0 };
    if hi - lo < min_span {
        let c = (hi + lo) / 2.0;
        lo = c - min_span / 2.0;
        hi = c + min_span / 2.0;
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

/// What a click in the keys area hit: keys (all of a diamond's), or one
/// Graph Editor handle (key ref, right side).
enum Hit {
    Keys(Vec<KeyRef>),
    Handle(KeyRef, bool),
}

fn pick(s: &State, d: &Doc, rows: &[Row], x: i32, y: i32) -> Option<Hit> {
    if s.drivers() {
        return None;
    }
    if s.graph() {
        let mut best: Option<(Hit, f32)> = None;
        for (o, a, c) in graph_curves(s, d, rows) {
            let fc = &d.anim.actions[a].curves[c];
            let oa = &d.anim.objects[o];
            let map = value_map(s, fc);
            for (k, key) in fc.keys.iter().enumerate() {
                let r = KeyRef { a, c, k };
                let mut cand = vec![(Hit::Keys(vec![r]), s.fx(oa.scene_time(key.frame)), s.vy(key.value * map.0 + map.1))];
                if s.show_handles && key.interp == Interp::Bezier && (fc.select || key.any_selected()) {
                    cand.push((Hit::Handle(r, false), s.fx(oa.scene_time(key.left[0])), s.vy(key.left[1] * map.0 + map.1)));
                    cand.push((Hit::Handle(r, true), s.fx(oa.scene_time(key.right[0])), s.vy(key.right[1] * map.0 + map.1)));
                }
                for (h, hx, hy) in cand {
                    let dist = ((hx - x as f32).powi(2) + (hy - y as f32).powi(2)).sqrt();
                    if dist < 8.0 && best.as_ref().is_none_or(|b| dist < b.1) {
                        best = Some((h, dist));
                    }
                }
            }
        }
        return best.map(|b| b.0);
    }
    for (i, row) in rows.iter().enumerate() {
        let ry = row_y(s, i);
        if y < ry || y >= ry + ROW_H {
            continue;
        }
        let mut best: Option<(f32, f32)> = None;
        for (f, _, _) in row_diamonds(s, d, row) {
            let dx = (s.fx(f) - x as f32).abs();
            if dx < 7.0 && best.is_none_or(|b| dx < b.1) {
                best = Some((f, dx));
            }
        }
        if let Some((f, _)) = best {
            let ks: Vec<KeyRef> = row_keys(s, d, row).into_iter().filter(|(o, r)| d.anim.key(*r).is_some_and(|k| (d.anim.objects[*o].scene_time(k.frame) - f).abs() < 0.01)).map(|(_, r)| r).collect();
            return Some(Hit::Keys(ks));
        }
    }
    None
}

fn deselect_all(d: &mut Doc) {
    for k in d.anim.keys_mut() {
        k.select = false;
        k.select_left = false;
        k.select_right = false;
    }
}

fn select_key(d: &mut Doc, r: KeyRef, on: bool) {
    if let Some(k) = d.anim.key_mut(r) {
        k.select = on;
        k.select_left = on;
        k.select_right = on;
    }
}

fn box_select(s: &State, d: &mut Doc, rows: &[Row], b: Rect) {
    let mut hits = Vec::new();
    if s.graph() {
        for (o, a, c) in graph_curves(s, d, rows) {
            let fc = &d.anim.actions[a].curves[c];
            let oa = &d.anim.objects[o];
            let map = value_map(s, fc);
            for (k, key) in fc.keys.iter().enumerate() {
                if b.contains(s.fx(oa.scene_time(key.frame)).round() as i32, s.vy(key.value * map.0 + map.1).round() as i32) {
                    hits.push(KeyRef { a, c, k });
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
            for (o, r) in row_keys(s, d, row) {
                let Some(k) = d.anim.key(r) else { continue };
                let f = d.anim.objects[o].scene_time(k.frame);
                if f >= f0 && f <= f1 {
                    hits.push(r);
                }
            }
        }
    }
    for r in hits {
        select_key(d, r, true);
    }
}

/// Start G / S / R / E on the selected keys (and Graph Editor handles).
fn start_xform(s: &mut State, d: &Doc, mode: XMode, release_confirms: bool) {
    let mut orig = Vec::new();
    let (mut fsum, mut vsum, mut n) = (0.0, 0.0, 0.0);
    for (ai, act) in d.anim.actions.iter().enumerate() {
        for (ci, fc) in act.curves.iter().enumerate() {
            if fc.lock {
                continue;
            }
            let map = value_map(s, fc);
            for (ki, k) in fc.keys.iter().enumerate() {
                if k.any_selected() {
                    orig.push(Orig { r: KeyRef { a: ai, c: ci, k: ki }, key: *k, vs: map.0 });
                    if k.select {
                        fsum += k.frame;
                        vsum += k.value * map.0 + map.1;
                        n += 1.0;
                    }
                }
            }
        }
    }
    if orig.is_empty() {
        return;
    }
    let pivot = if n > 0.0 { (fsum / n, vsum / n) } else { (d.frame as f32, 0.0) };
    // The Dope Sheet scales around the current frame.
    let pivot = if !s.graph() { (d.frame as f32, 0.0) } else { pivot };
    s.drag = Drag::Xform { mode, x: s.mouse.0, y: s.mouse.1, orig, release_confirms, pivot, undo_done: false };
}

fn apply_xform(s: &State, d: &mut Doc, mode: XMode, x: i32, y: i32, orig: &[Orig], pivot: (f32, f32), mx: i32, my: i32) {
    let snap_frames = !s.graph() || true;
    let df_raw = (mx - x) as f32 / s.ppf;
    let df = if snap_frames { df_raw.round() } else { df_raw };
    let dv = if s.graph() { (y - my) as f32 / s.ppv } else { 0.0 };
    let current = d.frame as f32;
    // Scale factor from the distance to the pivot on screen.
    let px = s.fx(pivot.0);
    let py = if s.graph() { s.vy(pivot.1) } else { y as f32 };
    let d0 = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt().max(1.0);
    let d1 = ((mx as f32 - px).powi(2) + (my as f32 - py).powi(2)).sqrt();
    let fac = d1 / d0;
    let ang = {
        let a0 = (-(y as f32 - py)).atan2(x as f32 - px);
        let a1 = (-(my as f32 - py)).atan2(mx as f32 - px);
        a1 - a0
    };
    for o in orig {
        let Some(fc) = d.anim.actions.get_mut(o.r.a).and_then(|a| a.curves.get_mut(o.r.c)) else { continue };
        let Some(k) = fc.keys.get_mut(o.r.k) else { continue };
        let vs = o.vs.max(1e-12);
        let mut nk = o.key;
        let map_pt = |p: [f32; 2]| -> [f32; 2] {
            match mode {
                XMode::Move => [p[0] + df, p[1] + dv / vs],
                XMode::Extend => {
                    // Only keys on the pointer's side of the current frame move.
                    let side = (o.key.frame - current) * (mx - x).signum() as f32 >= 0.0 || (o.key.frame - current).signum() == (s.frame_at(mx) - current).signum();
                    if side {
                        [p[0] + df, p[1]]
                    } else {
                        p
                    }
                }
                XMode::Scale => {
                    let f = pivot.0 + (p[0] - pivot.0) * fac;
                    let v = if s.graph() { p[1] + ((p[1] * vs - pivot.1) * (fac - 1.0)) / vs } else { p[1] };
                    [if snap_frames && !s.graph() { f.round() } else { f }, v]
                }
                XMode::Rotate => {
                    // Rotate in screen space around the pivot.
                    let sx = (p[0] - pivot.0) * s.ppf;
                    let sy = (p[1] * vs - pivot.1) * s.ppv;
                    let (c, sn) = (ang.cos(), ang.sin());
                    let (rx, ry) = (sx * c - sy * sn, sx * sn + sy * c);
                    [pivot.0 + rx / s.ppf, (pivot.1 + ry / s.ppv) / vs]
                }
            }
        };
        if o.key.select {
            let c = map_pt([o.key.frame, o.key.value]);
            let (ddf, ddv) = (c[0] - o.key.frame, c[1] - o.key.value);
            nk.frame = c[0];
            nk.value = c[1];
            if mode == XMode::Move || mode == XMode::Extend {
                nk.left = [o.key.left[0] + ddf, o.key.left[1] + ddv];
                nk.right = [o.key.right[0] + ddf, o.key.right[1] + ddv];
            } else {
                nk.left = map_pt(o.key.left);
                nk.right = map_pt(o.key.right);
            }
        } else {
            // Handles alone: auto handles become aligned (vector: free).
            let conv = |h: HandleType| match h {
                HandleType::Auto | HandleType::AutoClamped => HandleType::Aligned,
                HandleType::Vector => HandleType::Free,
                x => x,
            };
            if o.key.select_left {
                nk.left = map_pt(o.key.left);
                nk.h1 = conv(o.key.h1);
                nk.h2 = conv(o.key.h2);
            }
            if o.key.select_right {
                nk.right = map_pt(o.key.right);
                nk.h1 = conv(o.key.h1);
                nk.h2 = conv(o.key.h2);
            }
        }
        *k = nk;
        if !o.key.select {
            let right = o.key.select_right;
            fc.align_handles(o.r.k, right);
        }
    }
    // Auto handles follow the moved keys.
    let mut touched: Vec<(usize, usize)> = orig.iter().map(|o| (o.r.a, o.r.c)).collect();
    touched.dedup();
    for (a, c) in touched {
        if let Some(fc) = d.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)) {
            fc.recalc_handles();
        }
    }
    d.anim.dirty = true;
}

fn finish_xform(d: &mut Doc, orig: &[Orig], cancel: bool, undo_done: bool) {
    if cancel {
        for o in orig {
            if let Some(k) = d.anim.key_mut(o.r) {
                *k = o.key;
            }
        }
    } else if !undo_done {
        // Record the step with the keys where they started, then put the
        // moved keys back.
        let moved: Vec<Option<Keyframe>> = orig.iter().map(|o| d.anim.key(o.r).copied()).collect();
        for o in orig {
            if let Some(k) = d.anim.key_mut(o.r) {
                *k = o.key;
            }
        }
        d.push_undo("Transform Keyframes");
        for (o, m) in orig.iter().zip(moved) {
            if let (Some(k), Some(m)) = (d.anim.key_mut(o.r), m) {
                *k = m;
            }
        }
    }
    let mut touched: Vec<(usize, usize)> = orig.iter().map(|o| (o.r.a, o.r.c)).collect();
    touched.sort();
    touched.dedup();
    for (a, c) in touched {
        if let Some(fc) = d.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)) {
            fc.sort();
        }
    }
    d.anim.dirty = true;
}

fn delete_channels(s: &State, d: &mut Doc) {
    d.push_undo("Delete Channels");
    if s.drivers() {
        for oa in &mut d.anim.objects {
            oa.drivers.retain(|x| !x.select);
        }
    } else {
        for act in &mut d.anim.actions {
            act.curves.retain(|c| !c.select);
        }
    }
    d.anim.gc();
    d.anim.dirty = true;
}

fn handle_input(s: &mut State, d: &mut Doc, ui: &mut Ui, ruler: Rect, keys: Rect, chan: Rect, rows: &[Row]) {
    let Some(input) = ui.input else { return };
    let clip = ui.p.clip();
    let inside = |r: Rect, x: i32, y: i32| r.contains(x, y) && clip.contains(x, y);
    match (&s.drag, input) {
        (Drag::Xform { .. }, _) => {
            let Drag::Xform { mode, x, y, orig, release_confirms, pivot, undo_done } = s.drag.clone() else { return };
            match input {
                Input::Move { x: mx, y: my } => apply_xform(s, d, mode, x, y, &orig, pivot, mx, my),
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => {
                    finish_xform(d, &orig, false, undo_done);
                    s.drag = Drag::None;
                }
                Input::Up { button: Button::Left, .. } if release_confirms => {
                    finish_xform(d, &orig, false, undo_done);
                    s.drag = Drag::None;
                }
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    finish_xform(d, &orig, true, undo_done);
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
            } else {
                s.chan_scroll -= y - ly;
            }
            s.drag = Drag::Pan { x, y };
            ui.input = None;
        }
        (Drag::Pan { .. }, Input::Up { button: Button::Middle, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::ChanScroll { y: y0, start }, Input::Move { y, .. }) => {
            s.chan_scroll = start - (y - y0);
            ui.input = None;
        }
        (Drag::ChanScroll { .. }, Input::Up { button: Button::Middle, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Press { x: px, y: py, on_key, extend }, Input::Move { x, y }) => {
            let (px, py, on_key, extend) = (*px, *py, *on_key, *extend);
            if (x - px).abs() + (y - py).abs() > 3 {
                if on_key {
                    s.mouse = (px, py);
                    start_xform(s, d, XMode::Move, true);
                    if let Drag::Xform { mode, x: ox, y: oy, orig, pivot, .. } = s.drag.clone() {
                        apply_xform(s, d, mode, ox, oy, &orig, pivot, x, y);
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
                if !s.drivers() {
                    s.drag = Drag::Scrub;
                    d.frame = s.frame_at(x).round() as i32;
                }
                ui.input = None;
            } else if inside(keys, x, y) {
                let hit = pick(s, d, rows, x, y);
                if s.box_armed {
                    s.box_armed = false;
                    s.drag = Drag::Box { x0: x, y0: y, x1: x, y1: y, extend: true };
                } else if mods.alt && !s.graph() {
                    // Alt+click: the whole column.
                    let f = s.frame_at(x).round();
                    if !mods.shift {
                        deselect_all(d);
                    }
                    for k in d.anim.keys_mut() {
                        if (k.frame - f).abs() < 0.5 {
                            k.select = true;
                        }
                    }
                } else if let Some(hit) = hit {
                    match hit {
                        Hit::Keys(ks) => {
                            let all_sel = ks.iter().all(|r| d.anim.key(*r).is_some_and(|k| k.select));
                            if mods.shift {
                                for r in &ks {
                                    select_key(d, *r, !all_sel);
                                }
                            } else if !all_sel {
                                deselect_all(d);
                                for r in &ks {
                                    select_key(d, *r, true);
                                }
                            }
                            // The clicked curve becomes the active one.
                            if let Some(r) = ks.first() {
                                set_active_curve(s, d, r.a, r.c);
                            }
                        }
                        Hit::Handle(r, right) => {
                            if !mods.shift {
                                deselect_all(d);
                            }
                            if let Some(k) = d.anim.key_mut(r) {
                                if right {
                                    k.select_right = !mods.shift || !k.select_right;
                                } else {
                                    k.select_left = !mods.shift || !k.select_left;
                                }
                            }
                            set_active_curve(s, d, r.a, r.c);
                        }
                    }
                    s.drag = Drag::Press { x, y, on_key: true, extend: mods.shift };
                } else if s.kind == EditorKind::Timeline {
                    s.drag = Drag::Scrub;
                    d.frame = s.frame_at(x).round() as i32;
                } else {
                    s.drag = Drag::Press { x, y, on_key: false, extend: mods.shift };
                }
                ui.input = None;
            } else if inside(chan, x, y) {
                channel_click(s, d, rows, x, y, mods.shift || mods.ctrl);
                ui.input = None;
            }
        }
        (Drag::None, Input::Down { x, y, button: Button::Middle, .. }) if inside(Rect::new(ruler.x, ruler.y, ruler.w, keys.bottom() - ruler.y), x, y) => {
            s.drag = Drag::Pan { x, y };
            ui.input = None;
        }
        (Drag::None, Input::Down { x, y, button: Button::Middle, .. }) if inside(chan, x, y) => {
            s.drag = Drag::ChanScroll { y, start: s.chan_scroll };
            ui.input = None;
        }
        (Drag::None, Input::Wheel { x, y, steps, mods }) if inside(chan, x, y) => {
            s.chan_scroll -= steps * ROW_H;
            let _ = mods;
            ui.input = None;
        }
        (Drag::None, Input::Wheel { x, y, steps, mods }) if inside(Rect::new(ruler.x, ruler.y, ruler.w, keys.bottom() - ruler.y), x, y) => {
            if mods.ctrl {
                s.x0 -= steps as f32 * 30.0 / s.ppf;
            } else if mods.shift && s.graph() {
                s.y0 += steps as f32 * 30.0 / s.ppv;
            } else if mods.shift {
                s.chan_scroll -= steps * ROW_H;
            } else {
                zoom_at(s, x, y, 1.2f32.powi(steps), s.graph());
            }
            ui.input = None;
        }
        _ => {}
    }
}

fn set_active_curve(s: &mut State, d: &mut Doc, a: usize, c: usize) {
    for act in &mut d.anim.actions {
        for fc in &mut act.curves {
            fc.select = false;
        }
    }
    if let Some(fc) = d.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)) {
        fc.select = true;
    }
    let o = d.anim.objects.iter().position(|o| o.edit_action() == Some(a)).unwrap_or(0);
    s.active = Some(Row::Curve(o, a, c));
}

fn channel_click(s: &mut State, d: &mut Doc, rows: &[Row], x: i32, y: i32, extend: bool) {
    let Some(i) = (0..rows.len()).find(|&i| y >= row_y(s, i) && y < row_y(s, i) + ROW_H) else { return };
    let row = rows[i].clone();
    let disclose = x < s.chan_rect.x + 40 + row_label(d, &row).0;
    match &row {
        Row::Summary => s.summary_open = !s.summary_open,
        Row::Object(o) => {
            if disclose {
                d.anim.objects[*o].expanded = !d.anim.objects[*o].expanded;
            } else {
                let name = d.anim.objects[*o].object.clone();
                if let Some(oi) = d.scene.find_object(&name) {
                    if d.mode == Mode::Object {
                        d.select_only(Some(oi));
                    }
                }
            }
        }
        Row::Group(o, g) => {
            if disclose {
                let set = &mut d.anim.objects[*o].collapsed;
                if !set.remove(g) {
                    set.insert(g.clone());
                }
            } else {
                // Clicking a bone's group selects the bone in Pose Mode.
                if let Some(oi) = crate::rig::pose_object(d) {
                    if d.scene.objects[oi].name == d.anim.objects[*o].object && d.scene.armature_of(oi).is_some_and(|a| a.find(g).is_some()) {
                        let bi = d.scene.armature_of(oi).and_then(|a| a.find(g));
                        crate::rig::pose::select_pick(d, oi, bi, extend);
                    }
                }
                // Select every curve of the group.
                let a = d.anim.objects[*o].edit_action();
                for (ai, act) in d.anim.actions.iter_mut().enumerate() {
                    for fc in &mut act.curves {
                        let inside = Some(ai) == a && fc.group == *g;
                        if inside {
                            fc.select = true;
                        } else if !extend {
                            fc.select = false;
                        }
                    }
                }
            }
        }
        Row::Curve(_, a, c) => {
            let was = d.anim.actions[*a].curves[*c].select;
            if !extend {
                for act in &mut d.anim.actions {
                    for fc in &mut act.curves {
                        fc.select = false;
                    }
                }
            }
            d.anim.actions[*a].curves[*c].select = !was || !extend;
        }
        Row::Shape(oi, k) => {
            if d.active() == Some(*oi) {
                d.queue.push(Op::with("object.shape_key_set_active", &k.to_string()));
            }
        }
        Row::Driver(o, di) => {
            for oa in &mut d.anim.objects {
                for drv in &mut oa.drivers {
                    if !extend {
                        drv.select = false;
                    }
                }
            }
            d.anim.objects[*o].drivers[*di].select = true;
        }
    }
    s.active = Some(row);
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    let p = s.prefix();
    let popup = |d: &mut Doc, s: &State, title: &str, items: Vec<MenuItem>| d.menu = Some(crate::ui::Popup::new(title, items, s.mouse.0 - 20, s.mouse.1 - 10));
    let over_chan = s.chan_rect.contains(s.mouse.0, s.mouse.1);
    match (key, mods) {
        (Key::Char('i'), Mods::NONE) => d.queue.push(Op::new(&format!("{p}.keyframe_insert"))),
        (Key::Char('g'), Mods::NONE) => start_xform(s, d, XMode::Move, false),
        (Key::Char('s'), Mods::NONE) => start_xform(s, d, XMode::Scale, false),
        (Key::Char('e'), Mods::NONE) if !s.graph() => start_xform(s, d, XMode::Extend, false),
        (Key::Char('r'), Mods::NONE) if s.graph() => start_xform(s, d, XMode::Rotate, false),
        (Key::Char('r'), Mods::NONE) => popup(d, s, "Set Keyframe Type", keytype_items(p)),
        (Key::Char('d'), Mods::SHIFT) => {
            d.push_undo("Duplicate Keyframes");
            anim::duplicate_selected_keys(d);
            start_xform(s, d, XMode::Move, false);
            if let Drag::Xform { undo_done, .. } = &mut s.drag {
                *undo_done = true;
            }
        }
        (Key::Char('x'), Mods::NONE) | (Key::Delete, Mods::NONE) => {
            if over_chan {
                delete_channels(s, d);
            } else {
                d.queue.push(Op::new(&format!("{p}.delete")));
            }
        }
        (Key::Char('a'), Mods::NONE) => d.queue.push(Op::with(&format!("{p}.select_all"), "SELECT")),
        (Key::Char('a'), mm) if mm.alt => d.queue.push(Op::with(&format!("{p}.select_all"), "DESELECT")),
        (Key::Char('i'), Mods::CTRL) => d.queue.push(Op::with(&format!("{p}.select_all"), "INVERT")),
        (Key::Char('b'), Mods::NONE) => s.box_armed = true,
        (Key::Char('k'), Mods::NONE) => d.queue.push(Op::new(&format!("{p}.select_column"))),
        (Key::Char('k'), Mods::CTRL) => d.queue.push(Op::with(&format!("{p}.select_column"), "CFRA")),
        (Key::Char('g'), Mods::CTRL) => d.queue.push(Op::new(&format!("{p}.frame_jump"))),
        (Key::Char('t'), Mods::NONE) => popup(d, s, "Set Keyframe Interpolation", interp_items(p)),
        (Key::Char('e'), Mods::CTRL) => popup(d, s, "Set Keyframe Easing", easing_items(p)),
        (Key::Char('v'), Mods::NONE) => popup(d, s, "Set Keyframe Handle Type", handle_items(p)),
        (Key::Char('e'), Mods::SHIFT) => popup(d, s, "Set F-Curve Extrapolation", extrapolation_items(p)),
        (Key::Char('m'), Mods::CTRL_SHIFT) if s.graph() => popup(
            d,
            s,
            "Add F-Curve Modifier",
            vec![MenuItem::op("Cycles", "", Op::with("graph.fmodifier_add", "CYCLES")), MenuItem::op("Noise", "", Op::with("graph.fmodifier_add", "NOISE"))],
        ),
        (Key::Char('m'), Mods::CTRL) => popup(
            d,
            s,
            "Mirror",
            vec![MenuItem::op("By Times over Current Frame", "", Op::with(&format!("{p}.mirror"), "CFRA")), MenuItem::op("By Values over Zero Value", "", Op::with(&format!("{p}.mirror"), "XAXIS"))],
        ),
        (Key::Char('s'), Mods::SHIFT) => popup(
            d,
            s,
            "Snap",
            vec![MenuItem::op("Selection to Current Frame", "", Op::with(&format!("{p}.snap"), "CFRA")), MenuItem::op("Selection to Nearest Frame", "", Op::with(&format!("{p}.snap"), "NEAREST_FRAME"))],
        ),
        (Key::Char('n'), Mods::NONE) if s.kind != EditorKind::Timeline => s.show_sidebar = !s.show_sidebar,
        (Key::Char('h'), Mods::CTRL) if s.graph() => s.show_handles = !s.show_handles,
        (Key::Home, Mods::NONE) => s.want_frame_all = true,
        (Key::NumDot, Mods::NONE) => center_on_frame(s, d.frame as f32),
        (Key::NumPlus, Mods::CTRL) | (Key::NumPlus, Mods::NONE) if over_chan => {
            for oa in &mut d.anim.objects {
                oa.expanded = true;
                oa.collapsed.clear();
            }
        }
        (Key::NumMinus, Mods::CTRL) | (Key::NumMinus, Mods::NONE) if over_chan => {
            for oa in &mut d.anim.objects {
                oa.expanded = false;
            }
        }
        (Key::Home, Mods::CTRL) => d.queue.push(Op::new("anim.start_frame_set")),
        (Key::End, Mods::CTRL) => d.queue.push(Op::new("anim.end_frame_set")),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests;
