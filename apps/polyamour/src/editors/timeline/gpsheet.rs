// SPDX-License-Identifier: GPL-3.0-only
//! The Dope Sheet's Grease Pencil mode: one row per layer of every Grease
//! Pencil object (top layer first) with its drawings' keyframes. Click
//! selects a key (Shift extends), dragging a selected key moves the
//! selection by whole frames, the ruler scrubs, X deletes, R sets the
//! keyframe type, Shift+D duplicates, Ctrl+E interpolates.
use super::{key_diamond, State, CHANNEL_W, GRID_MAJOR, GRID_MINOR, ROW_H, RULER_BG, RULER_H};
use crate::anim::KeyType;
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::grease_pencil::KeyframeType;

fn key_type(k: KeyframeType) -> KeyType {
    KeyType::ALL.into_iter().find(|t| t.id() == k.id()).unwrap_or(KeyType::Keyframe)
}

/// Rows: (object, layer, label, keys). Object rows have layer None.
fn rows(d: &Doc) -> Vec<(usize, Option<usize>, String, Vec<(i32, KeyframeType, bool)>)> {
    let mut out = Vec::new();
    let mut last = usize::MAX;
    for (oi, li, name, keys) in crate::gpencil::dopesheet_rows(d) {
        if oi != last {
            last = oi;
            out.push((oi, None, d.scene.objects[oi].name.clone(), Vec::new()));
        }
        out.push((oi, Some(li), name, keys));
    }
    out
}

fn deselect_all(d: &mut Doc) {
    for g in &mut d.scene.grease_pencils {
        for l in &mut g.layers {
            for f in &mut l.frames {
                f.select = false;
            }
        }
    }
}

fn gp_of(d: &Doc, oi: usize) -> Option<usize> {
    match d.scene.objects.get(oi)?.data {
        sex_3d::scene::ObjectData::GreasePencil(g) => Some(g),
        _ => None,
    }
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    let (chan, main) = r.take_left(CHANNEL_W.min(r.w / 2));
    let (ruler, keys) = main.take_top(RULER_H);
    s.keys_rect = keys;
    s.chan_rect = chan;
    if !s.framed && keys.w > 0 {
        s.framed = true;
        let (a, b) = (d.scene.frame_start as f32, d.scene.frame_end as f32);
        let span = (b - a).max(1.0) * 1.08;
        s.ppf = keys.w as f32 / span;
        s.x0 = a - (span - (b - a)) / 2.0;
    }
    let rows = rows(d);
    let row_y = |i: usize| keys.y + 4 + i as i32 * ROW_H;
    // ---- input
    match ui.input {
        Some(Input::Down { x, y, button: Button::Left, mods }) if ruler.contains(x, y) => {
            ui.input = None;
            d.frame = s.frame_at(x).round() as i32;
            s.gp_drag = Some((x, 0, true));
            let _ = mods;
        }
        Some(Input::Down { x, y, button: Button::Left, mods }) if keys.contains(x, y) => {
            ui.input = None;
            let hit = rows.iter().enumerate().find_map(|(i, row)| {
                let cy = row_y(i) + ROW_H / 2;
                if (y - cy).abs() > ROW_H / 2 {
                    return None;
                }
                let li = row.1?;
                row.3.iter().find(|k| (s.fx(k.0 as f32) - x as f32).abs() < 6.0).map(|k| (row.0, li, k.0, k.2))
            });
            match hit {
                Some((oi, li, f, sel)) => {
                    if !mods.shift && !sel {
                        deselect_all(d);
                    }
                    if let Some(g) = gp_of(d, oi) {
                        if let Some(fr) = d.scene.grease_pencils[g].layers.get_mut(li).and_then(|l| l.frames.iter_mut().find(|k| k.frame == f)) {
                            fr.select = if mods.shift { !fr.select } else { true };
                        }
                        d.scene.grease_pencils[g].active_layer = li;
                    }
                    s.gp_drag = Some((x, 0, false));
                }
                None => {
                    if !mods.shift {
                        deselect_all(d);
                    }
                }
            }
            d.touch();
        }
        Some(Input::Down { x, y, button: Button::Left, .. }) if chan.contains(x, y) => {
            ui.input = None;
            let i = ((y - keys.y - 4) / ROW_H).max(0) as usize;
            if let Some((oi, Some(li), ..)) = rows.get(i) {
                d.select_only(Some(*oi));
                if let Some(g) = gp_of(d, *oi) {
                    d.scene.grease_pencils[g].active_layer = *li;
                }
                d.touch();
            }
        }
        Some(Input::Move { x, .. }) if s.gp_drag.is_some() => {
            let (x0, _, scrub) = s.gp_drag.unwrap_or_default();
            if scrub {
                d.frame = s.frame_at(x).round() as i32;
            } else {
                let df = ((x - x0) as f32 / s.ppf).round() as i32;
                s.gp_drag = Some((x0, df, false));
            }
        }
        Some(Input::Up { button: Button::Left, .. }) if s.gp_drag.is_some() => {
            if let Some((_, df, false)) = s.gp_drag.take() {
                if df != 0 {
                    d.push_undo("Move Keyframes");
                    crate::gpencil::move_keys(d, df);
                }
            }
        }
        Some(Input::Wheel { x, y, steps, mods }) if main.contains(x, y) => {
            ui.input = None;
            if mods.ctrl {
                s.x0 += steps as f32 * 10.0 / s.ppf;
            } else {
                let f = s.frame_at(x);
                s.ppf = (s.ppf * 0.85f32.powi(steps)).clamp(0.2, 200.0);
                s.x0 = f - (x - keys.x) as f32 / s.ppf;
            }
        }
        _ => {}
    }
    // ---- drawing
    let drag_df = s.gp_drag.map_or(0, |g| if g.2 { 0 } else { g.1 });
    let p = &mut *ui.p;
    p.fill(main, t::TIMELINE);
    let step = super::nice_step(s.ppf, 50.0);
    let minor = (step / 5.0).max(1.0);
    let mut f = (s.x0 / minor).floor() * minor;
    while s.fx(f) < keys.right() as f32 {
        let x = s.fx(f).round() as i32;
        let major = (f / step).round() * step == f;
        p.fill(Rect::new(x, keys.y, 1, keys.h), if major { GRID_MAJOR } else { GRID_MINOR });
        f += minor;
    }
    p.push_clip(keys);
    for (i, (_, li, _, ks)) in rows.iter().enumerate() {
        let y = row_y(i);
        if li.is_none() {
            p.fill(Rect::new(keys.x, y + 1, keys.w, ROW_H - 2), 0x353535);
            continue;
        }
        let cy = y as f32 + ROW_H as f32 / 2.0;
        // Hold bars between a key and the next one.
        for w in ks.windows(2) {
            let (a, b) = (s.fx(w[0].0 as f32), s.fx(w[1].0 as f32));
            p.fill(Rect::new(a as i32, cy as i32 - 2, (b - a) as i32, 4), 0x45474b);
        }
        for &(fr, kt, sel) in ks {
            let shift = if sel { drag_df } else { 0 };
            key_diamond(p, s.fx((fr + shift) as f32), cy, 5.0 * key_type(kt).scale(), sel, key_type(kt));
        }
    }
    p.pop_clip();
    // Ruler and playhead.
    p.fill(ruler, RULER_BG);
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
    let px = s.fx(d.frame as f32).round() as i32;
    p.push_clip(main);
    p.fill(Rect::new(px - 1, keys.y, 2, keys.h), t::PLAYHEAD);
    let label = format!("{}", d.frame);
    let w = p.text_width(&label) + 12;
    let bx = Rect::new(px - w / 2, ruler.y + 2, w, ruler.h - 4);
    p.round_rect(bx, 4, t::PLAYHEAD);
    p.label(bx, &label, t::TEXT_HI, Align::Center);
    p.pop_clip();
    // Channels.
    p.fill(chan, t::REGION);
    p.push_clip(chan);
    let active = crate::gpencil::active_gp(d);
    for (i, (oi, li, name, _)) in rows.iter().enumerate() {
        let y = row_y(i);
        let rr = Rect::new(chan.x + if li.is_some() { 18 } else { 4 }, y, chan.w - 22, ROW_H);
        let is_active = li.is_some() && active.is_some_and(|(ao, g)| ao == *oi && Some(d.scene.grease_pencils[g].active_layer) == *li);
        if is_active {
            p.round_rect(rr, 3, t::OUTLINER_ACTIVE);
        } else if li.is_none() {
            p.fill(Rect::new(chan.x, y + 1, chan.w, ROW_H - 2), 0x353535);
        }
        p.label(Rect::new(rr.x + 6, rr.y, rr.w - 8, rr.h), name, t::TEXT, Align::Left);
    }
    if rows.is_empty() {
        p.label(Rect::new(keys.x, keys.y + 20, keys.w, 20), "No Grease Pencil objects", t::TEXT_DIM, Align::Center);
    }
    p.pop_clip();
}

fn key_type_items() -> Vec<MenuItem> {
    ["KEYFRAME", "BREAKDOWN", "MOVING_HOLD", "EXTREME", "JITTER", "GENERATED"]
        .iter()
        .filter_map(|id| KeyframeType::from_id(id).map(|k| MenuItem::op(key_type(k).label(), "", Op::with("gpencil.dopesheet_keytype", id))))
        .collect()
}

/// Keys while the pointer is over the Grease Pencil Dope Sheet.
pub fn hotkey(s: &mut State, d: &mut Doc, key: Key, mods: Mods) -> bool {
    match (key, mods) {
        (Key::Char('a'), Mods::NONE) | (Key::Char('a'), Mods { alt: true, .. }) => {
            let on = !mods.alt;
            for g in &mut d.scene.grease_pencils {
                for l in &mut g.layers {
                    for f in &mut l.frames {
                        f.select = on;
                    }
                }
            }
            d.touch();
        }
        (Key::Char('x'), Mods::NONE) | (Key::Delete, _) => {
            d.push_undo("Delete Keyframes");
            for g in &mut d.scene.grease_pencils {
                for l in &mut g.layers {
                    l.frames.retain(|f| !f.select);
                }
            }
            d.touch();
        }
        (Key::Char('d'), Mods::SHIFT) => {
            d.push_undo("Duplicate Keyframes");
            for g in &mut d.scene.grease_pencils {
                for l in &mut g.layers {
                    let dup: Vec<_> = l.frames.iter().filter(|f| f.select).cloned().collect();
                    for f in &mut l.frames {
                        f.select = false;
                    }
                    for mut f in dup {
                        f.frame += 1;
                        f.select = true;
                        match l.key_at(f.frame) {
                            Some(i) => l.frames[i] = f,
                            None => l.frames.push(f),
                        }
                    }
                    l.sort_frames();
                }
            }
            d.touch();
        }
        (Key::Char('r'), Mods::NONE) => d.menu = Some(crate::ui::Popup::new("Set Keyframe Type", key_type_items(), s.mouse.0 - 20, s.mouse.1 - 10)),
        (Key::Char('e'), m) if m.ctrl => {
            let id = if m.shift { "grease_pencil.interpolate_sequence" } else { "grease_pencil.interpolate" };
            d.queue.push(Op::new(id));
        }
        (Key::Char('i'), Mods::NONE) => d.queue.push(Op::new("grease_pencil.insert_blank_frame")),
        _ => return false,
    }
    true
}
