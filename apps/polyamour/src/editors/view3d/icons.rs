// SPDX-License-Identifier: GPL-3.0-only
//! Our own small line icons for the 3D Viewport's header, toolbar and
//! navigation buttons (the font has no symbols, so they are drawn).
use crate::doc::Tool;
use crate::draw::{Painter, Rect};

/// A filled triangle (no anti-aliasing; icons are tiny).
pub fn fill_tri(p: &mut Painter, a: (f32, f32), b: (f32, f32), c: (f32, f32), col: u32) {
    let y0 = a.1.min(b.1).min(c.1).floor() as i32;
    let y1 = a.1.max(b.1).max(c.1).ceil() as i32;
    let edges = [(a, b), (b, c), (c, a)];
    for y in y0..=y1 {
        let py = y as f32 + 0.5;
        let mut xs: Vec<f32> = Vec::with_capacity(3);
        for (p0, p1) in edges {
            if (p0.1 <= py && p1.1 > py) || (p1.1 <= py && p0.1 > py) {
                xs.push(p0.0 + (py - p0.1) / (p1.1 - p0.1) * (p1.0 - p0.0));
            }
        }
        if xs.len() >= 2 {
            let (l, r) = (xs[0].min(xs[1]), xs[0].max(xs[1]));
            let (l, r) = (l.round() as i32, r.round() as i32);
            if r > l {
                p.fill(Rect::new(l, y, r - l, 1), col);
            }
        }
    }
}

/// A line `w` pixels thick.
pub fn thick(p: &mut Painter, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, col: u32) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / l, dx / l);
    let n = (w.max(1.0) * 2.0).round() as i32;
    for k in 0..n {
        let o = (k as f32 / (n.max(2) - 1) as f32 - 0.5) * (w - 1.0).max(0.0);
        p.line(x0 + nx * o, y0 + ny * o, x1 + nx * o, y1 + ny * o, col);
    }
}

/// An arrow head at (x, y) pointing along (dx, dy).
pub fn head(p: &mut Painter, x: f32, y: f32, dx: f32, dy: f32, size: f32, col: u32) {
    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / l, dy / l);
    let (bx, by) = (x - ux * size, y - uy * size);
    fill_tri(p, (x, y), (bx - uy * size * 0.55, by + ux * size * 0.55), (bx + uy * size * 0.55, by - ux * size * 0.55), col);
}

fn rect_outline(p: &mut Painter, x: f32, y: f32, w: f32, h: f32, col: u32) {
    p.line(x, y, x + w, y, col);
    p.line(x + w, y, x + w, y + h, col);
    p.line(x + w, y + h, x, y + h, col);
    p.line(x, y + h, x, y, col);
}

fn dashed_rect(p: &mut Painter, x: f32, y: f32, w: f32, h: f32, col: u32) {
    let seg = |p: &mut Painter, a: (f32, f32), b: (f32, f32)| {
        let n = 5;
        for i in 0..n {
            if i % 2 == 0 {
                let t0 = i as f32 / n as f32;
                let t1 = (i + 1) as f32 / n as f32;
                p.line(a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0, a.0 + (b.0 - a.0) * t1, a.1 + (b.1 - a.1) * t1, col);
            }
        }
    };
    seg(p, (x, y), (x + w, y));
    seg(p, (x + w, y), (x + w, y + h));
    seg(p, (x + w, y + h), (x, y + h));
    seg(p, (x, y + h), (x, y));
}

/// Draw `t`'s toolbar icon centred at (cx, cy), about 18 px across.
pub fn tool(p: &mut Painter, t: Tool, cx: f32, cy: f32, col: u32) {
    let acc = 0xffaa40;
    match t {
        Tool::SelectBox => {
            dashed_rect(p, cx - 8.0, cy - 7.0, 12.0, 11.0, col);
            fill_tri(p, (cx + 1.0, cy), (cx + 1.0, cy + 9.0), (cx + 8.0, cy + 6.0), col);
        }
        Tool::Cursor => {
            p.ring(cx, cy, 6.0, 0xff4444);
            p.ring(cx, cy, 5.0, col);
            p.line(cx - 9.0, cy, cx - 3.0, cy, col);
            p.line(cx + 3.0, cy, cx + 9.0, cy, col);
            p.line(cx, cy - 9.0, cx, cy - 3.0, col);
            p.line(cx, cy + 3.0, cx, cy + 9.0, col);
        }
        Tool::Move => {
            p.line(cx - 8.0, cy, cx + 8.0, cy, col);
            p.line(cx, cy - 8.0, cx, cy + 8.0, col);
            head(p, cx + 9.0, cy, 1.0, 0.0, 4.0, col);
            head(p, cx - 9.0, cy, -1.0, 0.0, 4.0, col);
            head(p, cx, cy - 9.0, 0.0, -1.0, 4.0, col);
            head(p, cx, cy + 9.0, 0.0, 1.0, 4.0, col);
        }
        Tool::Rotate => {
            for i in 0..24 {
                let a0 = 0.6 + i as f32 / 24.0 * 5.0;
                let a1 = 0.6 + (i + 1) as f32 / 24.0 * 5.0;
                thick(p, cx + 7.0 * a0.cos(), cy - 7.0 * a0.sin(), cx + 7.0 * a1.cos(), cy - 7.0 * a1.sin(), 1.5, col);
            }
            let a = 0.6f32;
            head(p, cx + 7.0 * a.cos() + 1.0, cy - 7.0 * a.sin() + 3.0, 0.2, 1.0, 5.0, col);
        }
        Tool::Scale => {
            rect_outline(p, cx - 8.0, cy - 2.0, 10.0, 10.0, col);
            p.line(cx - 1.0, cy + 1.0, cx + 7.0, cy - 7.0, col);
            head(p, cx + 8.0, cy - 8.0, 1.0, -1.0, 5.0, col);
        }
        Tool::Transform => {
            rect_outline(p, cx - 6.0, cy - 6.0, 12.0, 12.0, col);
            p.line(cx - 9.0, cy, cx + 9.0, cy, col);
            p.line(cx, cy - 9.0, cx, cy + 9.0, col);
            head(p, cx + 10.0, cy, 1.0, 0.0, 3.5, col);
            head(p, cx, cy - 10.0, 0.0, -1.0, 3.5, col);
        }
        Tool::Annotate => {
            thick(p, cx - 6.0, cy + 6.0, cx + 6.0, cy - 6.0, 4.0, col);
            fill_tri(p, (cx - 9.0, cy + 9.0), (cx - 8.0, cy + 4.0), (cx - 4.0, cy + 8.0), col);
            p.line(cx - 2.0, cy + 9.0, cx + 9.0, cy + 9.0, acc);
        }
        Tool::Measure => {
            thick(p, cx - 8.0, cy + 5.0, cx + 6.0, cy - 9.0, 1.0, col);
            thick(p, cx - 5.0, cy + 8.0, cx + 9.0, cy - 6.0, 1.0, col);
            for k in 0..4 {
                let t = 0.2 + k as f32 * 0.2;
                let (x, y) = (cx - 8.0 + 14.0 * t, cy + 5.0 - 14.0 * t);
                p.line(x, y, x + 2.5, y + 2.5, col);
            }
        }
        Tool::AddCube => {
            rect_outline(p, cx - 8.0, cy - 3.0, 10.0, 10.0, col);
            p.line(cx - 8.0, cy - 3.0, cx - 4.0, cy - 7.0, col);
            p.line(cx + 2.0, cy - 3.0, cx + 6.0, cy - 7.0, col);
            p.line(cx - 4.0, cy - 7.0, cx + 6.0, cy - 7.0, col);
            p.line(cx + 6.0, cy - 7.0, cx + 6.0, cy + 3.0, col);
            p.line(cx + 2.0, cy + 7.0, cx + 6.0, cy + 3.0, col);
            p.line(cx + 6.0, cy + 6.0, cx + 10.0, cy + 6.0, acc);
            p.line(cx + 8.0, cy + 4.0, cx + 8.0, cy + 8.0, acc);
        }
        Tool::Extrude => {
            rect_outline(p, cx - 8.0, cy + 1.0, 16.0, 7.0, col);
            p.line(cx, cy + 1.0, cx, cy - 6.0, acc);
            head(p, cx, cy - 9.0, 0.0, -1.0, 5.0, acc);
        }
        Tool::Inset => {
            rect_outline(p, cx - 8.0, cy - 8.0, 16.0, 16.0, col);
            rect_outline(p, cx - 4.0, cy - 4.0, 8.0, 8.0, acc);
        }
        Tool::Bevel => {
            p.line(cx - 8.0, cy + 8.0, cx - 8.0, cy - 3.0, col);
            p.line(cx - 8.0, cy - 3.0, cx - 3.0, cy - 8.0, acc);
            p.line(cx - 3.0, cy - 8.0, cx + 8.0, cy - 8.0, col);
            p.line(cx + 8.0, cy - 8.0, cx + 8.0, cy + 8.0, col);
            p.line(cx + 8.0, cy + 8.0, cx - 8.0, cy + 8.0, col);
        }
        Tool::LoopCut => {
            rect_outline(p, cx - 8.0, cy - 8.0, 16.0, 16.0, col);
            p.line(cx, cy - 8.0, cx, cy + 8.0, 0xffff40);
        }
        Tool::Knife => {
            fill_tri(p, (cx - 8.0, cy + 8.0), (cx + 6.0, cy - 8.0), (cx + 2.0, cy + 3.0), col);
            thick(p, cx + 5.0, cy - 7.0, cx + 9.0, cy - 10.0, 2.0, acc);
        }
        Tool::PolyBuild => {
            p.line(cx - 8.0, cy + 7.0, cx + 8.0, cy + 7.0, col);
            p.line(cx + 8.0, cy + 7.0, cx, cy - 7.0, col);
            p.line(cx, cy - 7.0, cx - 8.0, cy + 7.0, col);
            p.circle(cx, cy - 7.0, 2.5, acc);
        }
        Tool::Spin => {
            for k in 0..6 {
                let a = 3.4 + k as f32 * 0.55;
                p.circle(cx + 8.0 * a.cos(), cy - 8.0 * a.sin() + 3.0, 1.7, col);
            }
        }
        Tool::Smooth => {
            for i in 0..16 {
                let x0 = cx - 8.0 + i as f32;
                p.line(x0, cy + 4.0 * (i as f32 * 0.4).sin(), x0 + 1.0, cy + 4.0 * ((i + 1) as f32 * 0.4).sin(), col);
            }
        }
        Tool::EdgeSlide => {
            p.line(cx - 8.0, cy + 6.0, cx + 8.0, cy + 6.0, col);
            p.line(cx - 6.0, cy - 2.0, cx + 6.0, cy - 2.0, acc);
            head(p, cx - 8.0, cy - 2.0, -1.0, 0.0, 4.0, acc);
            head(p, cx + 8.0, cy - 2.0, 1.0, 0.0, 4.0, acc);
        }
        Tool::ShrinkFatten => {
            p.ring(cx, cy, 4.0, col);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                p.line(cx + dx * 5.0, cy + dy * 5.0, cx + dx * 8.0, cy + dy * 8.0, col);
                head(p, cx + dx * 10.0, cy + dy * 10.0, dx, dy, 3.0, col);
            }
        }
        Tool::Shear => {
            p.line(cx - 8.0, cy + 7.0, cx + 4.0, cy + 7.0, col);
            p.line(cx + 4.0, cy + 7.0, cx + 9.0, cy - 7.0, col);
            p.line(cx + 9.0, cy - 7.0, cx - 3.0, cy - 7.0, col);
            p.line(cx - 3.0, cy - 7.0, cx - 8.0, cy + 7.0, col);
        }
        Tool::Rip => {
            p.line(cx - 8.0, cy - 7.0, cx, cy + 7.0, col);
            p.line(cx, cy + 7.0, cx + 8.0, cy - 7.0, col);
            p.line(cx - 4.0, cy - 7.0, cx, cy + 1.0, acc);
            p.line(cx, cy + 1.0, cx + 4.0, cy - 7.0, acc);
        }
    }
}

/// Header icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    VertSel,
    EdgeSel,
    FaceSel,
    Wireframe,
    Solid,
    Material,
    Rendered,
    XRay,
    Overlays,
    Gizmo,
    Orientation,
    Pivot,
    Magnet,
    Proportional,
    Visibility,
    Zoom,
    Hand,
    Camera,
    Grid,
    ObjectMode,
    EditMode,
}

pub fn icon(p: &mut Painter, i: Icon, cx: f32, cy: f32, col: u32) {
    match i {
        Icon::VertSel | Icon::EdgeSel | Icon::FaceSel => {
            let (x, y, s) = (cx - 6.0, cy - 6.0, 12.0);
            if i == Icon::FaceSel {
                p.fill(Rect::new(x as i32, y as i32, s as i32, s as i32), crate::draw_mix(col, 0x000000, 150));
            }
            rect_outline(p, x, y, s, s, col);
            match i {
                Icon::VertSel => p.fill(Rect::new(x as i32 - 2, (y + s) as i32 - 2, 5, 5), 0xffffff),
                Icon::EdgeSel => thick(p, x, y + s, x + s, y + s, 2.5, 0xffffff),
                _ => {}
            }
        }
        Icon::Wireframe => {
            p.ring(cx, cy, 7.0, col);
            p.line(cx - 7.0, cy, cx + 7.0, cy, col);
            p.line(cx, cy - 7.0, cx, cy + 7.0, col);
            for k in 0..20 {
                let a0 = k as f32 / 20.0 * core::f32::consts::TAU;
                let a1 = (k + 1) as f32 / 20.0 * core::f32::consts::TAU;
                p.line(cx + 3.5 * a0.cos(), cy + 7.0 * a0.sin(), cx + 3.5 * a1.cos(), cy + 7.0 * a1.sin(), col);
            }
        }
        Icon::Solid => p.circle(cx, cy, 7.0, col),
        Icon::Material => {
            p.circle(cx, cy, 7.0, col);
            p.circle(cx - 2.0, cy - 2.0, 3.5, 0x2a2a2a);
            p.circle(cx + 2.5, cy + 2.5, 2.0, 0x2a2a2a);
        }
        Icon::Rendered => {
            p.circle(cx, cy, 7.0, crate::draw_mix(col, 0x000000, 120));
            p.circle(cx - 1.5, cy - 1.5, 5.0, crate::draw_mix(col, 0x000000, 200));
            p.circle(cx - 2.5, cy - 2.5, 2.5, col);
        }
        Icon::XRay => {
            rect_outline(p, cx - 7.0, cy - 7.0, 10.0, 10.0, col);
            p.fill(Rect::new(cx as i32 - 3, cy as i32 - 3, 10, 10), crate::draw_mix(col, 0x303030, 110));
            rect_outline(p, cx - 3.0, cy - 3.0, 10.0, 10.0, col);
        }
        Icon::Overlays => {
            p.ring(cx - 2.5, cy, 5.0, col);
            p.ring(cx + 2.5, cy, 5.0, col);
        }
        Icon::Gizmo => {
            p.line(cx - 6.0, cy + 6.0, cx + 5.0, cy + 6.0, 0xff3352);
            p.line(cx - 6.0, cy + 6.0, cx - 6.0, cy - 5.0, 0x2890ff);
            p.line(cx - 6.0, cy + 6.0, cx + 2.0, cy - 2.0, 0x8bdc00);
            head(p, cx + 8.0, cy + 6.0, 1.0, 0.0, 3.5, 0xff3352);
            head(p, cx - 6.0, cy - 8.0, 0.0, -1.0, 3.5, 0x2890ff);
        }
        Icon::Orientation => {
            p.line(cx - 6.0, cy + 6.0, cx + 6.0, cy + 6.0, col);
            p.line(cx - 6.0, cy + 6.0, cx - 6.0, cy - 6.0, col);
            p.line(cx - 6.0, cy + 6.0, cx + 2.0, cy - 2.0, col);
        }
        Icon::Pivot => {
            p.ring(cx, cy, 6.0, col);
            p.circle(cx, cy, 2.0, col);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                p.line(cx + dx * 6.0, cy + dy * 6.0, cx + dx * 9.0, cy + dy * 9.0, col);
            }
        }
        Icon::Magnet => {
            for k in 0..14 {
                let a0 = core::f32::consts::PI * k as f32 / 14.0;
                let a1 = core::f32::consts::PI * (k + 1) as f32 / 14.0;
                thick(p, cx + 5.0 * a0.cos(), cy + 5.0 * a0.sin(), cx + 5.0 * a1.cos(), cy + 5.0 * a1.sin(), 3.0, col);
            }
            thick(p, cx - 5.0, cy, cx - 5.0, cy - 6.0, 3.0, col);
            thick(p, cx + 5.0, cy, cx + 5.0, cy - 6.0, 3.0, col);
            p.fill(Rect::new(cx as i32 - 7, cy as i32 - 8, 4, 3), 0xe6e6e6);
            p.fill(Rect::new(cx as i32 + 4, cy as i32 - 8, 4, 3), 0xe6e6e6);
        }
        Icon::Proportional => {
            p.ring(cx, cy, 7.0, col);
            p.ring(cx, cy, 3.5, col);
            p.circle(cx, cy, 1.5, col);
        }
        Icon::Visibility => {
            for k in 0..16 {
                let t0 = k as f32 / 16.0;
                let t1 = (k + 1) as f32 / 16.0;
                let x0 = cx - 8.0 + 16.0 * t0;
                let x1 = cx - 8.0 + 16.0 * t1;
                let h0 = 5.0 * (t0 * core::f32::consts::PI).sin();
                let h1 = 5.0 * (t1 * core::f32::consts::PI).sin();
                p.line(x0, cy - h0, x1, cy - h1, col);
                p.line(x0, cy + h0, x1, cy + h1, col);
            }
            p.circle(cx, cy, 2.5, col);
        }
        Icon::Zoom => {
            p.ring(cx - 2.0, cy - 2.0, 5.0, col);
            thick(p, cx + 2.0, cy + 2.0, cx + 7.0, cy + 7.0, 2.5, col);
        }
        Icon::Hand => {
            p.round_rect(Rect::new(cx as i32 - 5, cy as i32 - 2, 10, 9), 3, col);
            for k in 0..4 {
                let x = cx - 4.5 + k as f32 * 3.0;
                thick(p, x, cy - 1.0, x, cy - 7.0 + (k as f32 - 1.5).abs(), 2.0, col);
            }
        }
        Icon::Camera => {
            p.round_rect(Rect::new(cx as i32 - 7, cy as i32 - 4, 10, 9), 2, col);
            fill_tri(p, (cx + 3.0, cy), (cx + 8.0, cy - 4.0), (cx + 8.0, cy + 5.0), col);
        }
        Icon::Grid => {
            for k in 0..3 {
                let o = -6.0 + k as f32 * 6.0;
                p.line(cx + o, cy - 7.0, cx + o, cy + 7.0, col);
                p.line(cx - 7.0, cy + o, cx + 7.0, cy + o, col);
            }
        }
        Icon::ObjectMode => {
            p.ring(cx, cy, 5.5, col);
            p.circle(cx, cy, 2.0, col);
        }
        Icon::EditMode => {
            p.line(cx - 6.0, cy + 5.0, cx + 6.0, cy + 5.0, col);
            p.line(cx + 6.0, cy + 5.0, cx, cy - 6.0, col);
            p.line(cx, cy - 6.0, cx - 6.0, cy + 5.0, col);
            for (x, y) in [(cx - 6.0, cy + 5.0), (cx + 6.0, cy + 5.0), (cx, cy - 6.0)] {
                p.fill(Rect::new(x as i32 - 1, y as i32 - 1, 3, 3), col);
            }
        }
    }
}
