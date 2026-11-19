// SPDX-License-Identifier: GPL-3.0-only
//! Small icons drawn from shapes (the UI font has no symbol glyphs), coloured
//! the way Blender's default theme colours its icon families: objects orange,
//! object data green, modifiers and physics blue, shading pink-red. Shared by
//! the Properties editor and the Outliner.
use crate::draw::{Painter, Rect};

pub const OBJECT: u32 = 0xffaf29;
pub const DATA: u32 = 0x3fd4a3;
pub const MODIFIER: u32 = 0x84b8ff;
pub const SHADING: u32 = 0xea7581;
pub const WORLD: u32 = 0xe96a6a;
pub const COLLECTION: u32 = 0xe6e6e6;
pub const GREY: u32 = 0xc8c8c8;
const LIGHT: u32 = 0xd8d8d8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    SceneCollection,
    Collection,
    NewCollection,
    ObMesh,
    ObCamera,
    ObLight,
    ObEmpty,
    Mesh,
    CameraData,
    LightData,
    EmptyData,
    Modifier,
    Material,
    World,
    Scene,
    Render,
    Output,
    ViewLayer,
    Tool,
    Particles,
    Physics,
    Constraint,
    Texture,
    Object,
    Eye,
    EyeClosed,
    RenderOn,
    RenderOff,
    Funnel,
    Search,
    Pin,
    Lock,
    Monitor,
    Plus,
    Minus,
    Cross,
    Down,
    Dots,
    /// A bone (Bone tab, armature data, bone lists).
    Bone,
    BoneConstraint,
    ArmatureData,
    LatticeData,
    ShapeKey,
    VertexGroup,
    /// Grease Pencil object / data: a pencil.
    ObGreasePencil,
    GreasePencilData,
    /// Visual effects (shader FX): a sparkle.
    ShaderFx,
    /// A Grease Pencil layer: a sheet with a stroke.
    GpLayer,
    /// Video strips: a film frame.
    Strip,
}

/// A pencil pointing down-left.
fn pencil(p: &mut Painter, x: f32, y: f32, c: u32) {
    thick(p, x - 2.5, y + 2.5, x + 4.5, y - 4.5, c);
    thick(p, x - 1.0, y + 4.0, x + 6.0, y - 3.0, c);
    p.line(x - 2.5, y + 2.5, x - 6.0, y + 6.0, c);
    p.line(x - 1.0, y + 4.0, x - 6.0, y + 6.0, c);
    p.circle(x - 5.2, y + 5.2, 1.0, c);
}

/// A two-pixel line.
pub fn thick(p: &mut Painter, x0: f32, y0: f32, x1: f32, y1: f32, c: u32) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / l * 0.5, dx / l * 0.5);
    p.line(x0 + nx, y0 + ny, x1 + nx, y1 + ny, c);
    p.line(x0 - nx, y0 - ny, x1 - nx, y1 - ny, c);
}

fn tri(p: &mut Painter, cx: f32, cy: f32, c: u32) {
    let (a, b, d) = ((cx, cy - 5.5), (cx - 6.0, cy + 4.5), (cx + 6.0, cy + 4.5));
    p.line(a.0, a.1, b.0, b.1, c);
    p.line(b.0, b.1, d.0, d.1, c);
    p.line(d.0, d.1, a.0, a.1, c);
    for v in [a, b, d] {
        p.circle(v.0, v.1, 1.6, c);
    }
}

fn camera(p: &mut Painter, cx: f32, cy: f32, c: u32, filled: bool) {
    let body = Rect::new(cx as i32 - 7, cy as i32 - 4, 9, 8);
    if filled {
        p.round_rect(body, 2, c);
    } else {
        p.round_frame(body, 2, c);
    }
    p.line(cx + 2.0, cy - 1.0, cx + 6.5, cy - 4.0, c);
    p.line(cx + 2.0, cy + 1.0, cx + 6.5, cy + 4.0, c);
    p.line(cx + 6.5, cy - 4.0, cx + 6.5, cy + 4.0, c);
}

fn bulb(p: &mut Painter, cx: f32, cy: f32, c: u32) {
    p.ring(cx, cy - 1.5, 4.5, c);
    p.circle(cx, cy - 1.5, 2.0, c);
    p.fill(Rect::new(cx as i32 - 2, cy as i32 + 4, 5, 2), c);
}

fn eye_arc(p: &mut Painter, x: f32, y: f32, h: f32, c: u32) {
    let n = 10;
    for i in 0..n {
        let a0 = i as f32 / n as f32 * core::f32::consts::PI;
        let a1 = (i + 1) as f32 / n as f32 * core::f32::consts::PI;
        p.line(x - 6.5 * a0.cos(), y - h * a0.sin(), x - 6.5 * a1.cos(), y - h * a1.sin(), c);
    }
}

/// Draw `icon` centred at (cx, cy), about 14 px across.
pub fn draw(p: &mut Painter, icon: Icon, cx: i32, cy: i32) {
    draw_tinted(p, icon, cx, cy, None)
}

/// [`draw`] in one colour (`tint`), e.g. greyed out.
pub fn draw_tinted(p: &mut Painter, icon: Icon, cx: i32, cy: i32, tint: Option<u32>) {
    let (x, y) = (cx as f32 + 0.5, cy as f32 + 0.5);
    let col = |c: u32| tint.unwrap_or(c);
    match icon {
        Icon::SceneCollection | Icon::Collection | Icon::NewCollection => {
            let c = col(COLLECTION);
            p.round_frame(Rect::new(cx - 6, cy - 5, 13, 11), 2, c);
            p.fill(Rect::new(cx - 6, cy - 2, 13, 1), c);
            if icon == Icon::SceneCollection {
                p.fill(Rect::new(cx - 3, cy + 1, 7, 2), c);
            }
            if icon == Icon::NewCollection {
                p.fill(Rect::new(cx + 2, cy + 1, 8, 8), crate::theme::HEADER);
                p.fill(Rect::new(cx + 3, cy + 5, 7, 1), c);
                p.fill(Rect::new(cx + 6, cy + 2, 1, 7), c);
            }
        }
        Icon::ObMesh => tri(p, x, y, col(OBJECT)),
        Icon::Mesh => tri(p, x, y, col(DATA)),
        Icon::ObCamera => camera(p, x, y, col(OBJECT), false),
        Icon::CameraData => camera(p, x, y, col(DATA), false),
        Icon::ObLight => bulb(p, x, y, col(OBJECT)),
        Icon::LightData => bulb(p, x, y, col(DATA)),
        Icon::ObEmpty | Icon::EmptyData => {
            let c = col(if icon == Icon::ObEmpty { OBJECT } else { DATA });
            p.line(x - 6.0, y, x + 6.0, y, c);
            p.line(x, y - 6.0, x, y + 6.0, c);
            p.line(x - 4.0, y + 4.0, x + 4.0, y - 4.0, c);
        }
        Icon::Modifier => {
            let c = col(MODIFIER);
            thick(p, x - 5.0, y + 5.0, x + 1.0, y - 1.0, c);
            p.ring(x + 2.5, y - 2.5, 3.2, c);
            p.ring(x + 2.5, y - 2.5, 2.4, c);
        }
        Icon::Material => {
            let c = col(SHADING);
            p.circle(x, y, 6.0, c);
            p.circle(x - 2.0, y - 2.0, 1.8, crate::draw_mix(0xffffff, c, 150));
        }
        Icon::World => {
            let c = col(WORLD);
            p.ring(x, y, 6.0, c);
            p.line(x - 6.0, y, x + 6.0, y, c);
            p.line(x, y - 6.0, x, y + 6.0, c);
        }
        Icon::Scene => {
            let c = col(GREY);
            p.line(x - 6.0, y + 5.0, x - 2.5, y - 5.0, c);
            p.line(x - 2.5, y - 5.0, x + 1.0, y + 5.0, c);
            p.line(x - 6.0, y + 5.0, x + 1.0, y + 5.0, c);
            p.circle(x + 4.0, y + 1.5, 3.0, c);
        }
        Icon::Render => {
            let c = col(GREY);
            p.round_frame(Rect::new(cx - 7, cy - 4, 15, 11), 2, c);
            p.ring(x, y + 1.5, 3.0, c);
            p.fill(Rect::new(cx - 4, cy - 6, 5, 2), c);
        }
        Icon::Output => {
            let c = col(GREY);
            p.round_frame(Rect::new(cx - 7, cy - 2, 15, 7), 2, c);
            p.frame(Rect::new(cx - 4, cy - 7, 9, 5), c);
            p.fill(Rect::new(cx - 4, cy + 3, 9, 4), c);
        }
        Icon::ViewLayer => {
            let c = col(GREY);
            p.frame(Rect::new(cx - 3, cy - 7, 10, 9), c);
            p.frame(Rect::new(cx - 7, cy - 3, 10, 9), c);
            p.line(x - 6.0, y + 4.5, x - 3.5, y + 1.5, c);
            p.line(x - 3.5, y + 1.5, x + 1.5, y + 4.5, c);
        }
        Icon::Tool => {
            let c = col(GREY);
            thick(p, x - 5.0, y + 5.0, x + 3.0, y - 3.0, c);
            p.ring(x + 3.5, y - 3.5, 2.5, c);
            thick(p, x - 5.0, y - 5.0, x + 5.0, y + 5.0, c);
        }
        Icon::Particles => {
            let c = col(MODIFIER);
            for (dx, dy) in [(-4.0, -3.0), (3.0, -5.0), (0.0, 0.0), (5.0, 3.0), (-3.0, 5.0)] {
                p.circle(x + dx, y + dy, 1.7, c);
            }
        }
        Icon::Physics => {
            let c = col(MODIFIER);
            p.ring(x, y, 6.0, c);
            p.circle(x + 4.2, y - 4.2, 2.0, c);
            p.circle(x, y, 2.0, c);
        }
        Icon::Constraint => {
            let c = col(MODIFIER);
            p.round_frame(Rect::new(cx - 7, cy - 3, 9, 7), 3, c);
            p.round_frame(Rect::new(cx - 1, cy - 3, 9, 7), 3, c);
        }
        Icon::Texture => {
            let c = col(SHADING);
            for i in 0..3 {
                for j in 0..3 {
                    if (i + j) % 2 == 0 {
                        p.fill(Rect::new(cx - 6 + i * 4, cy - 6 + j * 4, 4, 4), c);
                    }
                }
            }
            p.frame(Rect::new(cx - 6, cy - 6, 12, 12), c);
        }
        Icon::Bone | Icon::ArmatureData => {
            let c = col(if icon == Icon::Bone { GREY } else { DATA });
            // A slanted bone: two knobs and a shaft.
            p.ring(x - 4.0, y + 4.0, 2.4, c);
            p.ring(x + 4.0, y - 4.0, 2.4, c);
            thick(p, x - 2.5, y + 2.5, x + 2.5, y - 2.5, c);
        }
        Icon::BoneConstraint => {
            let c = col(MODIFIER);
            p.round_frame(Rect::new(cx - 7, cy - 1, 9, 7), 3, c);
            p.round_frame(Rect::new(cx - 1, cy - 1, 9, 7), 3, c);
            p.ring(x - 3.0, y - 4.5, 1.8, c);
            p.line(x - 1.5, y - 3.0, x + 3.0, y - 6.0, c);
        }
        Icon::LatticeData => {
            let c = col(DATA);
            for i in 0..3 {
                let o = -5.0 + i as f32 * 5.0;
                p.line(x - 5.0, y + o, x + 5.0, y + o, c);
                p.line(x + o, y - 5.0, x + o, y + 5.0, c);
            }
        }
        Icon::ShapeKey => {
            let c = col(DATA);
            p.line(x - 6.0, y + 4.0, x, y - 5.0, c);
            p.line(x, y - 5.0, x + 6.0, y + 4.0, c);
            p.line(x - 6.0, y + 4.0, x + 6.0, y + 4.0, c);
            p.circle(x, y - 5.0, 1.8, c);
        }
        Icon::VertexGroup => {
            let c = col(DATA);
            for (dx, dy) in [(-4.0, -4.0), (4.0, -4.0), (-4.0, 4.0), (4.0, 4.0)] {
                p.circle(x + dx, y + dy, 1.8, c);
            }
            p.frame(Rect::new(cx - 4, cy - 4, 9, 9), c);
        }
        Icon::Object => {
            let c = col(OBJECT);
            p.round_frame(Rect::new(cx - 6, cy - 6, 13, 13), 2, c);
            p.fill(Rect::new(cx - 2, cy - 2, 5, 5), c);
        }
        Icon::Eye => {
            let c = col(LIGHT);
            eye_arc(p, x, y, 4.5, c);
            eye_arc(p, x, y, -4.5, c);
            p.circle(x, y, 2.3, c);
        }
        Icon::EyeClosed => {
            let c = col(0x8a8a8a);
            eye_arc(p, x, y - 1.0, -3.5, c);
            for dx in [-4.0f32, 0.0, 4.0] {
                p.line(x + dx, y + 2.0, x + dx * 1.3, y + 5.0, c);
            }
        }
        Icon::RenderOn => camera(p, x - 0.5, y, col(LIGHT), true),
        Icon::RenderOff => camera(p, x - 0.5, y, col(0x7a7a7a), false),
        Icon::Funnel => {
            let c = col(LIGHT);
            p.line(x - 6.0, y - 5.0, x + 6.0, y - 5.0, c);
            p.line(x - 6.0, y - 5.0, x - 1.0, y + 1.0, c);
            p.line(x + 6.0, y - 5.0, x + 1.0, y + 1.0, c);
            p.fill(Rect::new(cx - 1, cy + 1, 3, 5), c);
        }
        Icon::Search => {
            let c = col(0xb0b0b0);
            p.ring(x - 1.5, y - 1.5, 4.0, c);
            thick(p, x + 1.5, y + 1.5, x + 5.0, y + 5.0, c);
        }
        Icon::Pin => {
            let c = col(LIGHT);
            thick(p, x - 5.0, y + 5.0, x - 1.0, y + 1.0, c);
            p.circle(x + 1.5, y - 1.5, 3.5, c);
        }
        Icon::Lock => {
            let c = col(0x8a8a8a);
            p.ring(x, y - 2.5, 3.0, c);
            p.round_rect(Rect::new(cx - 4, cy - 1, 9, 7), 2, c);
        }
        Icon::Monitor => {
            let c = col(LIGHT);
            p.round_frame(Rect::new(cx - 7, cy - 6, 15, 10), 2, c);
            p.fill(Rect::new(cx - 3, cy + 5, 7, 1), c);
        }
        Icon::Plus => {
            let c = col(LIGHT);
            p.fill(Rect::new(cx - 5, cy, 11, 1), c);
            p.fill(Rect::new(cx, cy - 5, 1, 11), c);
        }
        Icon::Minus => p.fill(Rect::new(cx - 5, cy, 11, 1), col(LIGHT)),
        Icon::Cross => {
            let c = col(LIGHT);
            p.line(x - 4.0, y - 4.0, x + 4.0, y + 4.0, c);
            p.line(x - 4.0, y + 4.0, x + 4.0, y - 4.0, c);
        }
        Icon::Down => p.arrow(x, y, 8.0, true, col(0xb0b0b0)),
        Icon::Dots => {
            for dx in [-4.0, 0.0, 4.0] {
                p.circle(x + dx, y, 1.3, col(LIGHT));
            }
        }
        Icon::ObGreasePencil => pencil(p, x, y, col(OBJECT)),
        Icon::GreasePencilData => pencil(p, x, y, col(DATA)),
        Icon::ShaderFx => {
            let c = col(MODIFIER);
            p.line(x, y - 6.0, x, y + 6.0, c);
            p.line(x - 6.0, y, x + 6.0, y, c);
            p.line(x - 3.5, y - 3.5, x + 3.5, y + 3.5, c);
            p.line(x - 3.5, y + 3.5, x + 3.5, y - 3.5, c);
            p.circle(x, y, 2.0, c);
        }
        Icon::GpLayer => {
            let c = col(LIGHT);
            p.round_frame(Rect::new(cx - 6, cy - 5, 13, 11), 2, c);
            for i in 0..6 {
                let t0 = i as f32 / 6.0;
                let t1 = (i + 1) as f32 / 6.0;
                let f = |t: f32| (x - 4.0 + 8.0 * t, y + 2.0 - 3.5 * (t * core::f32::consts::PI).sin());
                let (a, b) = (f(t0), f(t1));
                p.line(a.0, a.1, b.0, b.1, c);
            }
        }
        Icon::Strip => {
            let c = col(0x9fb9e8);
            p.round_frame(Rect::new(cx - 7, cy - 5, 15, 11), 1, c);
            for k in 0..4 {
                p.fill(Rect::new(cx - 5 + k * 4, cy - 4, 2, 1), c);
                p.fill(Rect::new(cx - 5 + k * 4, cy + 4, 2, 1), c);
            }
            p.fill(Rect::new(cx - 4, cy - 2, 9, 5), c);
        }
    }
}
