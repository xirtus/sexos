// SPDX-License-Identifier: GPL-3.0-only
//! Blender's colour picker popover: a hue/saturation wheel, a value strip,
//! RGB or HSV sliders and a hex field. Wheel and hex work in display (sRGB)
//! space, the RGB sliders in scene-linear values, as in Blender.
use super::ColorTarget;
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::{Button, Input};
use crate::theme as t;
use crate::ui::{id, NumOpts, Ui};
use sex_3d::render::{srgb_decode, srgb_encode};
use sex_3d::scene::ObjectData;
use sex_3d::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    None,
    Wheel,
    Value,
}

pub struct Picker {
    pub target: ColorTarget,
    anchor: Rect,
    pub rect: Rect,
    hsv: [f32; 3],
    drag: Drag,
    hsv_mode: usize,
}

const W: i32 = 196;
const H: i32 = 262;
const R: f32 = 62.0;
const FIELDS: [&str; 4] = ["picker/0", "picker/1", "picker/2", "picker/hex"];
const DRAG_ID: &str = "picker/drag";

pub enum Claim {
    /// The event belongs to the picker.
    Mine,
    /// A click outside: close it (the click still goes through).
    Close,
    No,
}

pub fn get(d: &Doc, t: ColorTarget) -> Option<[f32; 3]> {
    let v3 = |v: Vec3| [v.x, v.y, v.z];
    match t {
        ColorTarget::BaseColor(m) => d.scene.materials.get(m).map(|m| [m.base_color[0], m.base_color[1], m.base_color[2]]),
        ColorTarget::Emission(m) => d.scene.materials.get(m).map(|m| v3(m.emission)),
        ColorTarget::World => Some(v3(d.scene.world.color)),
        ColorTarget::Light(o) => match d.scene.objects.get(o).map(|o| &o.data) {
            Some(ObjectData::Light(l)) => Some(v3(l.color)),
            _ => None,
        },
    }
}

pub fn set(d: &mut Doc, t: ColorTarget, c: [f32; 3]) {
    let v = Vec3::new(c[0], c[1], c[2]);
    match t {
        ColorTarget::BaseColor(m) => {
            if let Some(m) = d.scene.materials.get_mut(m) {
                m.base_color[..3].copy_from_slice(&c);
            }
        }
        ColorTarget::Emission(m) => {
            if let Some(m) = d.scene.materials.get_mut(m) {
                m.emission = v;
            }
        }
        ColorTarget::World => d.scene.world.color = v,
        ColorTarget::Light(o) => {
            if let Some(ObjectData::Light(l)) = d.scene.objects.get_mut(o).map(|o| &mut o.data) {
                l.color = v;
            }
        }
    }
    d.touch();
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let (p, q, tt) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i % 6 {
        0 => [v, tt, p],
        1 => [q, v, p],
        2 => [p, v, tt],
        3 => [p, q, v],
        4 => [tt, p, v],
        _ => [v, p, q],
    }
}

pub fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let dlt = mx - mn;
    let h = if dlt <= 1e-6 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / dlt).rem_euclid(6.0) / 6.0
    } else if mx == c[1] {
        ((c[2] - c[0]) / dlt + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / dlt + 4.0) / 6.0
    };
    [h, if mx <= 1e-6 { 0.0 } else { dlt / mx }, mx]
}

fn to_display(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| srgb_encode(v.clamp(0.0, 1.0)))
}
fn to_linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| srgb_decode(v.clamp(0.0, 1.0)))
}
fn pack(c: [f32; 3]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
}

impl Picker {
    pub fn new(target: ColorTarget, anchor: Rect, d: &Doc) -> Picker {
        let hsv = get(d, target).map(|c| rgb_to_hsv(to_display(c))).unwrap_or([0.0, 0.0, 0.8]);
        Picker { target, anchor, rect: Rect::new(anchor.x, anchor.bottom() + 4, W, H), hsv, drag: Drag::None, hsv_mode: 0 }
    }
}

pub fn claims(pk: &Picker, ui: &Ui) -> Claim {
    let r = pk.rect;
    let mine_id = |i: u64| FIELDS.iter().any(|f| id(f) == i) || i == id(DRAG_ID);
    match ui.input {
        Some(Input::Down { x, y, .. }) => {
            if r.contains(x, y) {
                Claim::Mine
            } else {
                Claim::Close
            }
        }
        Some(Input::Move { x, y }) | Some(Input::Up { x, y, .. }) | Some(Input::Wheel { x, y, .. }) => {
            let busy = pk.drag != Drag::None || ui.st.active.is_some_and(mine_id) || ui.st.editing.as_ref().is_some_and(|e| mine_id(e.0));
            let far = Rect::new(r.x - 60, r.y - 60, r.w + 120, r.h + 120);
            if r.contains(x, y) || busy {
                Claim::Mine
            } else if !far.contains(x, y) && matches!(ui.input, Some(Input::Move { .. })) {
                // Like Blender's popovers: moving well away closes it.
                Claim::Close
            } else {
                Claim::No
            }
        }
        Some(Input::Key { .. }) => {
            if ui.st.editing.as_ref().is_some_and(|e| mine_id(e.0)) {
                Claim::Mine
            } else {
                Claim::No
            }
        }
        _ => Claim::No,
    }
}

fn commit(d: &mut Doc, ui: &Ui, undo_for: &mut Option<Option<u64>>) {
    let active = ui.st.active;
    if active.is_none() || *undo_for != Some(active) {
        d.push_undo("Color");
        *undo_for = Some(active);
    }
}

/// Draw and run the picker inside `area`. Returns false once it closed.
pub fn draw(pk: &mut Picker, d: &mut Doc, ui: &mut Ui, area: Rect, undo_for: &mut Option<Option<u64>>) -> bool {
    let Some(lin) = get(d, pk.target) else { return false };
    let x = pk.anchor.x.min(area.right() - W - 4).max(area.x + 4);
    let mut y = pk.anchor.bottom() + 4;
    if y + H > area.bottom() {
        y = (pk.anchor.y - H - 4).max(area.y + 4);
    }
    let r = Rect::new(x, y, W, H);
    pk.rect = r;
    if pk.drag == Drag::None {
        let h = rgb_to_hsv(to_display(lin));
        // Keep the hue while the colour is grey or black.
        pk.hsv = if h[1] < 1e-4 || h[2] < 1e-4 { [pk.hsv[0], h[1], h[2]] } else { h };
    }
    ui.p.round_rect_alpha(Rect::new(r.x + 2, r.y + 3, r.w, r.h), 6, 0x000000, 90);
    ui.p.round_rect(r, 6, t::MENU_BG);
    ui.p.round_frame(r, 6, 0x2a2a2a);

    // Hue/saturation wheel at the current value.
    let (cx, cy) = ((r.x + 14) as f32 + R, (r.y + 12) as f32 + R);
    let v = pk.hsv[2];
    for py in (cy - R - 1.0) as i32..=(cy + R + 1.0) as i32 {
        for px in (cx - R - 1.0) as i32..=(cx + R + 1.0) as i32 {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            let dist = (dx * dx + dy * dy).sqrt();
            let a = (R - dist + 0.5).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let hue = (dy.atan2(dx) / core::f32::consts::TAU).rem_euclid(1.0);
            let c = pack(hsv_to_rgb(hue, (dist / R).min(1.0), v));
            ui.p.fill_alpha(Rect::new(px, py, 1, 1), c, (a * 255.0) as u32);
        }
    }
    let ang = pk.hsv[0] * core::f32::consts::TAU;
    let (mx, my) = (cx + ang.cos() * pk.hsv[1] * R, cy + ang.sin() * pk.hsv[1] * R);
    ui.p.ring(mx, my, 4.0, if v > 0.5 { 0x000000 } else { 0xffffff });
    // Value strip.
    let vb = Rect::new(r.right() - 34, r.y + 12, 18, (2.0 * R) as i32);
    for i in 0..vb.h {
        let vv = 1.0 - i as f32 / (vb.h - 1).max(1) as f32;
        ui.p.fill(Rect::new(vb.x, vb.y + i, vb.w, 1), pack(hsv_to_rgb(pk.hsv[0], pk.hsv[1], vv)));
    }
    let vy = vb.y + ((1.0 - v) * (vb.h - 1) as f32) as i32;
    ui.p.frame(Rect::new(vb.x - 2, vy - 2, vb.w + 4, 5), 0xffffff);

    // Wheel and strip dragging.
    let mut new_hsv = None;
    match ui.input {
        Some(Input::Down { x: px, y: py, button: Button::Left, .. }) => {
            let (dx, dy) = (px as f32 - cx, py as f32 - cy);
            if (dx * dx + dy * dy).sqrt() <= R + 2.0 {
                pk.drag = Drag::Wheel;
            } else if vb.contains(px, py) {
                pk.drag = Drag::Value;
            }
            if pk.drag != Drag::None {
                ui.input = None;
                ui.st.active = Some(id(DRAG_ID));
                *undo_for = None;
                new_hsv = Some((px, py));
            }
        }
        Some(Input::Move { x: px, y: py }) if pk.drag != Drag::None => {
            ui.input = None;
            new_hsv = Some((px, py));
        }
        Some(Input::Up { button: Button::Left, .. }) if pk.drag != Drag::None => {
            ui.input = None;
            pk.drag = Drag::None;
            ui.st.active = None;
        }
        _ => {}
    }
    if let Some((px, py)) = new_hsv {
        match pk.drag {
            Drag::Wheel => {
                let (dx, dy) = (px as f32 - cx, py as f32 - cy);
                pk.hsv[0] = (dy.atan2(dx) / core::f32::consts::TAU).rem_euclid(1.0);
                pk.hsv[1] = ((dx * dx + dy * dy).sqrt() / R).min(1.0);
            }
            Drag::Value => pk.hsv[2] = (1.0 - (py - vb.y) as f32 / (vb.h - 1).max(1) as f32).clamp(0.0, 1.0),
            Drag::None => {}
        }
        commit(d, ui, undo_for);
        set(d, pk.target, to_linear(hsv_to_rgb(pk.hsv[0], pk.hsv[1], pk.hsv[2])));
    }

    // RGB / HSV sliders and hex.
    y = r.y + 12 + (2.0 * R) as i32 + 10;
    let mut mode = pk.hsv_mode;
    if ui.segmented(Rect::new(r.x + 10, y, r.w - 20, 20), &["RGB", "HSV"], &mut mode) {
        pk.hsv_mode = mode;
    }
    y += 26;
    let lin = get(d, pk.target).unwrap_or(lin);
    let names = if pk.hsv_mode == 0 { ["R", "G", "B"] } else { ["H", "S", "V"] };
    for i in 0..3 {
        let fr = Rect::new(r.x + 10, y, r.w - 20, 20);
        let mut val = if pk.hsv_mode == 0 { lin[i] } else { pk.hsv[i] };
        let opts = NumOpts { step: 0.01, min: 0.0, max: 1.0, slider: true, ..NumOpts::default() };
        if ui.number_ex(id(FIELDS[i]), fr, names[i], &mut val, opts) {
            commit(d, ui, undo_for);
            if pk.hsv_mode == 0 {
                let mut c = lin;
                c[i] = val;
                set(d, pk.target, c);
            } else {
                pk.hsv[i] = val;
                set(d, pk.target, to_linear(hsv_to_rgb(pk.hsv[0], pk.hsv[1], pk.hsv[2])));
            }
        }
        y += 21;
    }
    y += 4;
    let disp = to_display(get(d, pk.target).unwrap_or(lin));
    let mut hex = format!("{:06X}", pack(disp));
    ui.p.label(Rect::new(r.x + 10, y, 30, 20), "Hex", t::TEXT, Align::Left);
    if ui.text_field(id(FIELDS[3]), Rect::new(r.x + 44, y, r.w - 54, 20), &mut hex) {
        let digits = hex.trim_start_matches('#');
        if let Ok(n) = u32::from_str_radix(digits, 16) {
            commit(d, ui, undo_for);
            let c = [((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0];
            set(d, pk.target, to_linear(c));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for c in [[1.0, 0.0, 0.0], [0.2, 0.5, 0.9], [0.3, 0.3, 0.3]] {
            let back = hsv_to_rgb(rgb_to_hsv(c)[0], rgb_to_hsv(c)[1], rgb_to_hsv(c)[2]);
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-5, "{c:?} -> {back:?}");
            }
        }
    }
}
