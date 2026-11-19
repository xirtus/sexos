// SPDX-License-Identifier: GPL-3.0-only
//! Node buttons that need more than a field: Blender's colour ramp (stops
//! on a gradient bar, add/remove, interpolation, position and colour of the
//! active stop) and curve mapping (C/R/G/B channels, drag points, click to
//! add a point, X deletes the selected one).
use crate::draw::Rect;
use crate::input::{Button, Input};
use crate::ui::{id, MenuItem, NumOpts, Op, Ui};
use sex_3d::nodes::{enums, ColorRamp, CurveMapping};

fn display(c: [f32; 4]) -> u32 {
    let b = |v: f32| (sex_3d::render::srgb_encode(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u32;
    b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
}

/// What a ramp edit asks the editor to do besides changing the ramp.
pub enum RampAsk {
    None,
    /// Open the colour picker for stop `0` at the swatch.
    Pick(usize, Rect),
}

/// Draw and edit a colour ramp in `r` (3 rows plus the bar). `active` is
/// the selected stop. `menu_op` builds the operator a menu item runs for an
/// interpolation choice. Returns (changed, ask).
pub fn ramp(ui: &mut Ui, base: &str, r: Rect, row: i32, ramp: &mut ColorRamp, active: &mut usize, menu_op: &dyn Fn(&str) -> Op) -> (bool, RampAsk) {
    let mut changed = false;
    let mut ask = RampAsk::None;
    if ramp.elements.is_empty() {
        ramp.elements.push((0.0, [0.0, 0.0, 0.0, 1.0]));
    }
    *active = (*active).min(ramp.elements.len() - 1);
    let fh = row - 3;
    // Row 1: + - interpolation.
    let b = fh;
    let plus = Rect::new(r.x, r.y, b, fh);
    let minus = Rect::new(r.x + b + 2, r.y, b, fh);
    if ui.button(plus, "+") {
        let pa = ramp.elements[*active].0;
        let next = ramp.elements.get(*active + 1).map(|e| e.0).unwrap_or(if pa >= 1.0 { pa } else { 1.0 });
        let pos = if (next - pa).abs() < 1e-6 { ramp.elements.get(active.wrapping_sub(1)).map_or(pa * 0.5, |p| (p.0 + pa) * 0.5) } else { (pa + next) * 0.5 };
        let c = ramp.eval(pos);
        ramp.elements.push((pos, c));
        sort(ramp, active, pos);
        changed = true;
    }
    if ui.button(minus, "-") && ramp.elements.len() > 1 {
        ramp.elements.remove(*active);
        *active = active.saturating_sub(1);
        changed = true;
    }
    let ir = Rect::new(minus.right() + 4, r.y, r.right() - minus.right() - 4, fh);
    if ui.dropdown(ir, enums::label("ShaderNodeValToRGB", "interpolation", &ramp.interpolation)) {
        let items = enums::items("ShaderNodeValToRGB", "interpolation").iter().map(|(i, l)| MenuItem::op(l, "", menu_op(i))).collect();
        ui.open_menu(ir, "Interpolation", items);
    }
    // Row 2: the bar with stop markers under it.
    let bar = Rect::new(r.x, r.y + row, r.w, fh + 4);
    for x in 0..bar.w.max(1) {
        let f = x as f32 / (bar.w - 1).max(1) as f32;
        let c = ramp.eval(f);
        // Checker behind transparent colours, as Blender shows alpha.
        let chk = if ((x / 6) + (0)) % 2 == 0 { 0x666666 } else { 0x999999 };
        let col = crate::draw_mix(display(c), chk, (c[3].clamp(0.0, 1.0) * 255.0) as u32);
        ui.p.fill(Rect::new(bar.x + x, bar.y, 1, bar.h), col);
    }
    ui.p.frame(bar, 0x1d1d1d);
    let sx = |p: f32| bar.x as f32 + p.clamp(0.0, 1.0) * (bar.w - 1) as f32;
    for (i, e) in ramp.elements.iter().enumerate() {
        let x = sx(e.0);
        let col = if i == *active { 0xffffff } else { 0x000000 };
        ui.p.fill(Rect::new(x as i32 - 1, bar.y, 3, bar.h), col);
        ui.p.fill(Rect::new(x as i32, bar.y + 1, 1, bar.h - 2), if i == *active { 0x000000 } else { 0xffffff });
    }
    let drag_id = id(&format!("{base}/ramp_drag"));
    match ui.input {
        Some(Input::Down { x, y, button: Button::Left, .. }) if bar.contains(x, y) && ui.p.clip().contains(x, y) => {
            ui.input = None;
            // Nearest stop becomes active and follows the pointer.
            let mut best = (f32::INFINITY, *active);
            for (i, e) in ramp.elements.iter().enumerate() {
                let d = (sx(e.0) - x as f32).abs();
                if d < best.0 {
                    best = (d, i);
                }
            }
            *active = best.1;
            ui.st.active = Some(drag_id);
        }
        Some(Input::Move { x, .. }) if ui.st.active == Some(drag_id) => {
            ui.input = None;
            let pos = ((x - bar.x) as f32 / (bar.w - 1).max(1) as f32).clamp(0.0, 1.0);
            if let Some(e) = ramp.elements.get_mut(*active) {
                e.0 = pos;
            }
            sort(ramp, active, pos);
            changed = true;
        }
        Some(Input::Up { button: Button::Left, .. }) if ui.st.active == Some(drag_id) => {
            ui.input = None;
            ui.st.active = None;
        }
        _ => {}
    }
    // Row 3: position and colour of the active stop.
    let y3 = bar.bottom() + 3;
    let half = r.w / 2;
    let pr = Rect::new(r.x, y3, half - 2, fh);
    let mut pos = ramp.elements[*active].0;
    if ui.number_ex(id(&format!("{base}/ramp_pos")), pr, "Pos", &mut pos, NumOpts { step: 0.01, min: 0.0, max: 1.0, slider: true, ..NumOpts::default() }) {
        ramp.elements[*active].0 = pos;
        sort(ramp, active, pos);
        changed = true;
    }
    let cr = Rect::new(r.x + half + 2, y3, r.w - half - 2, fh);
    let c = ramp.elements[*active].1;
    if ui.swatch(cr, [c[0], c[1], c[2]]) {
        ask = RampAsk::Pick(*active, cr);
    }
    (changed, ask)
}

/// Keep stops ordered by position, following the one at `pos` with `active`.
fn sort(ramp: &mut ColorRamp, active: &mut usize, pos: f32) {
    let cur = ramp.elements.get(*active).copied();
    ramp.elements.sort_by(|a, b| a.0.total_cmp(&b.0));
    if let Some(cur) = cur {
        *active = ramp.elements.iter().position(|e| *e == cur).unwrap_or(0);
    } else {
        *active = ramp.elements.iter().position(|e| (e.0 - pos).abs() < 1e-6).unwrap_or(0);
    }
}

/// Draw and edit a curve mapping in `r`: a channel row, then the graph.
/// `chan` is the shown curve, `sel` the selected point. Returns changed.
pub fn curve(ui: &mut Ui, base: &str, r: Rect, row: i32, cm: &mut CurveMapping, chan: &mut usize, sel: &mut Option<usize>) -> bool {
    let mut changed = false;
    let fh = row - 3;
    let names: &[&str] = match cm.curves.len() {
        4 => &["C", "R", "G", "B"],
        3 => &["X", "Y", "Z"],
        _ => &[],
    };
    *chan = (*chan).min(cm.curves.len().saturating_sub(1));
    if !names.is_empty() {
        let w = 22;
        for (i, nm) in names.iter().enumerate() {
            let br = Rect::new(r.x + i as i32 * (w + 1), r.y, w, fh);
            if ui.flat_button(br, nm, i == *chan) {
                *chan = i;
                *sel = None;
            }
        }
    }
    let del = Rect::new(r.right() - fh, r.y, fh, fh);
    if ui.button(del, "X") {
        if let (Some(p), Some(c)) = (*sel, cm.curves.get_mut(*chan)) {
            if c.len() > 2 && p < c.len() {
                c.remove(p);
                *sel = None;
                changed = true;
            }
        }
    }
    let g = Rect::new(r.x, r.y + row, r.w, r.h - row);
    ui.p.fill(g, 0x2b2b2b);
    for k in 1..4 {
        let x = g.x + g.w * k / 4;
        let y = g.y + g.h * k / 4;
        ui.p.fill(Rect::new(x, g.y, 1, g.h), 0x353535);
        ui.p.fill(Rect::new(g.x, y, g.w, 1), 0x353535);
    }
    ui.p.frame(g, 0x1d1d1d);
    let to = |p: [f32; 2]| (g.x as f32 + p[0] * (g.w - 1) as f32, g.bottom() as f32 - 1.0 - p[1] * (g.h - 1) as f32);
    let from = |x: i32, y: i32| [((x - g.x) as f32 / (g.w - 1).max(1) as f32).clamp(0.0, 1.0), ((g.bottom() - 1 - y) as f32 / (g.h - 1).max(1) as f32).clamp(0.0, 1.0)];
    let colors = [0xdddddd, 0xe05050, 0x50d050, 0x5080e0];
    if cm.curves.is_empty() {
        return changed;
    }
    for ci in 0..cm.curves.len() {
        if ci != *chan && !names.is_empty() && *chan != 0 {
            continue;
        }
        let col = if ci == *chan { colors[ci.min(3)] } else { 0x777777 };
        let mut prev = None;
        for k in 0..=48 {
            let x = k as f32 / 48.0;
            let y = cm.eval(ci, x).clamp(-0.5, 1.5);
            let p = to([x, y]);
            if let Some(q) = prev {
                let (a, b): ((f32, f32), (f32, f32)) = (q, p);
                ui.p.line(a.0, a.1, b.0, b.1, col);
            }
            prev = Some(p);
        }
    }
    let pts = cm.curves[*chan].clone();
    for (i, p) in pts.iter().enumerate() {
        let (x, y) = to(*p);
        ui.p.circle(x, y, 3.0, if Some(i) == *sel { 0xffffff } else { 0x000000 });
        ui.p.ring(x, y, 3.0, 0xdddddd);
    }
    let drag_id = id(&format!("{base}/curve_drag"));
    match ui.input {
        Some(Input::Down { x, y, button: Button::Left, .. }) if g.contains(x, y) && ui.p.clip().contains(x, y) => {
            ui.input = None;
            let hit = pts.iter().position(|p| {
                let (px, py) = to(*p);
                (px - x as f32).powi(2) + (py - y as f32).powi(2) <= 36.0
            });
            let i = match hit {
                Some(i) => i,
                None => {
                    // Click on the graph adds a point there.
                    let p = from(x, y);
                    let c = &mut cm.curves[*chan];
                    c.push(p);
                    c.sort_by(|a, b| a[0].total_cmp(&b[0]));
                    changed = true;
                    c.iter().position(|q| *q == p).unwrap_or(0)
                }
            };
            *sel = Some(i);
            ui.st.active = Some(drag_id);
        }
        Some(Input::Move { x, y }) if ui.st.active == Some(drag_id) => {
            ui.input = None;
            if let Some(i) = *sel {
                let p = from(x, y);
                let c = &mut cm.curves[*chan];
                if i < c.len() {
                    c[i] = p;
                    c.sort_by(|a, b| a[0].total_cmp(&b[0]));
                    *sel = c.iter().position(|q| *q == p);
                    changed = true;
                }
            }
        }
        Some(Input::Up { button: Button::Left, .. }) if ui.st.active == Some(drag_id) => {
            ui.input = None;
            ui.st.active = None;
        }
        _ => {}
    }
    changed
}
