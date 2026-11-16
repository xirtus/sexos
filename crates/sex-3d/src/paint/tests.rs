// SPDX-License-Identifier: GPL-3.0-only
use super::texture::{TexBrush, TexStroke, TpSettings};
use super::vertex::{fill, VpBrush, VpSettings, VpStroke};
use super::*;
use crate::math::{Vec2, Vec3};
use crate::mesh::{AttrDomain, Face, Mesh};
use crate::primitives;
use crate::scene::{Image, Scene};
use crate::sculpt::{Dab, SculptSession};
use crate::undo::UndoStack;

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const UP: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

fn vert_at(m: &Mesh, x: f32, y: f32) -> usize {
    m.positions.iter().enumerate().min_by(|a, b| (a.1.x - x).hypot(a.1.y - y).total_cmp(&(b.1.x - x).hypot(b.1.y - y))).map(|(i, _)| i).unwrap_or(0)
}

fn vp_stroke(m: &mut Mesh, attr: usize, s: &VpSettings, d: Dab, n: usize) -> Vec<crate::undo::Change> {
    let mut ss = SculptSession::new(m);
    let mut st = VpStroke::new(m, attr).expect("attribute");
    for _ in 0..n {
        st.dab(m, &mut ss, s, &d);
    }
    st.finish(m, 0)
}

#[test]
fn vertex_paint_draws_point_and_corner_colors() {
    for domain in [AttrDomain::Point, AttrDomain::Corner] {
        let mut m = primitives::grid(20, 20, 2.0);
        let a = m.add_color_attribute("Col", domain, [1.0; 4]);
        let s = VpSettings { color: RED, ..Default::default() };
        let ch = vp_stroke(&mut m, a, &s, Dab::at(Vec3::ZERO, 0.5, UP), 1);
        assert!(!ch.is_empty());
        let pc = m.point_colors(a);
        let c = pc[vert_at(&m, 0.0, 0.0)];
        assert!(c[0] > 0.99 && c[1] < 0.05 && c[2] < 0.05, "{domain:?} {c:?}");
        let half = pc[vert_at(&m, 0.3, 0.0)];
        assert!(half[1] > 0.05 && half[1] < 0.95, "falloff {half:?}");
        assert_eq!(pc[vert_at(&m, 0.9, 0.9)], [1.0; 4]);
    }
}

#[test]
fn vertex_paint_blend_modes_and_fill() {
    let mut m = primitives::grid(4, 4, 2.0);
    let a = m.add_color_attribute("Col", AttrDomain::Point, [0.5, 0.5, 0.5, 1.0]);
    fill(&mut m, a, [0.5, 1.0, 0.0], BlendMode::Multiply, 1.0);
    assert!(m.color_attributes[a].data.iter().all(|c| (c[0] - 0.25).abs() < 1e-6 && (c[1] - 0.5).abs() < 1e-6 && c[2] == 0.0));
    fill(&mut m, a, [0.5, 0.5, 0.5], BlendMode::Add, 1.0);
    assert!(m.color_attributes[a].data.iter().all(|c| (c[0] - 0.75).abs() < 1e-6));
    fill(&mut m, a, [0.5, 0.5, 0.5], BlendMode::Subtract, 1.0);
    assert!(m.color_attributes[a].data.iter().all(|c| (c[0] - 0.25).abs() < 1e-6 && c[2] == 0.0));
    fill(&mut m, a, [0.0; 3], BlendMode::EraseAlpha, 0.25);
    assert!(m.color_attributes[a].data.iter().all(|c| (c[3] - 0.75).abs() < 1e-6));
    // Fill skips hidden faces.
    let mut m = primitives::grid(2, 1, 2.0);
    let a = m.add_color_attribute("Col", AttrDomain::Corner, [1.0; 4]);
    m.hidden_faces = vec![true, false];
    fill(&mut m, a, [0.0; 3], BlendMode::Mix, 1.0);
    let d = &m.color_attributes[a].data;
    assert!(d[..4].iter().all(|c| *c == [1.0; 4]) && d[4..].iter().all(|c| c[0] == 0.0));
}

#[test]
fn vertex_paint_blur_average_and_smear() {
    let mut m = primitives::grid(20, 20, 2.0);
    let a = m.add_color_attribute("Col", AttrDomain::Point, [1.0; 4]);
    for (v, p) in m.positions.clone().iter().enumerate() {
        if p.x < 0.0 {
            m.color_attributes[a].data[v] = [0.0, 0.0, 0.0, 1.0];
        }
    }
    let edge = vert_at(&m, 0.0, 0.0);
    let left = vert_at(&m, -0.1, 0.0);
    let before = m.color_attributes[a].data[edge][0];
    let s = VpSettings { brush: VpBrush::Blur, ..Default::default() };
    vp_stroke(&mut m, a, &s, Dab::at(Vec3::ZERO, 0.4, UP), 2);
    let after = m.color_attributes[a].data[edge][0];
    assert!(after < before && after > 0.0, "{before} -> {after}");

    let mut b = primitives::grid(20, 20, 2.0);
    let a = b.add_color_attribute("Col", AttrDomain::Point, [1.0; 4]);
    for (v, p) in b.positions.clone().iter().enumerate() {
        if p.x < 0.0 {
            b.color_attributes[a].data[v] = [0.0, 0.0, 0.0, 1.0];
        }
    }
    let s = VpSettings { brush: VpBrush::Average, ..Default::default() };
    vp_stroke(&mut b, a, &s, Dab::at(Vec3::ZERO, 0.4, UP), 5);
    let (l, r) = (b.color_attributes[a].data[left][0], b.color_attributes[a].data[vert_at(&b, 0.1, 0.0)][0]);
    assert!(l > 0.2 && r < 0.8, "average pulls both sides together: {l} {r}");

    // Smear drags the dark side to the right.
    let mut c = primitives::grid(20, 20, 2.0);
    let a = c.add_color_attribute("Col", AttrDomain::Point, [1.0; 4]);
    for (v, p) in c.positions.clone().iter().enumerate() {
        if p.x < 0.0 {
            c.color_attributes[a].data[v] = [0.0, 0.0, 0.0, 1.0];
        }
    }
    let mut d = Dab::at(Vec3::ZERO, 0.4, UP);
    d.step = Vec3::new(0.1, 0.0, 0.0);
    let s = VpSettings { brush: VpBrush::Smear, ..Default::default() };
    vp_stroke(&mut c, a, &s, d, 1);
    assert!(c.color_attributes[a].data[vert_at(&c, 0.0, 0.0)][0] < 0.9);
}

#[test]
fn vertex_paint_undo_restores_colors() {
    let mut m = primitives::grid(10, 10, 2.0);
    let a = m.add_color_attribute("Col", AttrDomain::Corner, [1.0; 4]);
    let base = m.color_attributes[a].data.clone();
    let mut scene = Scene::new();
    scene.meshes.push(m);
    let s = VpSettings { color: RED, ..Default::default() };
    let ch = vp_stroke(&mut scene.meshes[0], a, &s, Dab::at(Vec3::ZERO, 0.5, UP), 2);
    let painted = scene.meshes[0].color_attributes[a].data.clone();
    assert_ne!(painted, base);
    let mut u = UndoStack::new();
    u.push_changes("Paint", ch);
    u.undo(&mut scene);
    assert_eq!(scene.meshes[0].color_attributes[a].data, base);
    u.redo(&mut scene);
    assert_eq!(scene.meshes[0].color_attributes[a].data, painted);
}

/// Orthographic view down -Z of the 2x2 square: 100x100 pixels, y down.
fn project(p: Vec3) -> Option<(f32, f32, f32)> {
    Some(((p.x + 1.0) * 50.0, (1.0 - p.y) * 50.0, -p.z))
}

fn quad(z: f32, x0: f32, x1: f32, uv0: Vec2, uv1: Vec2, flip: bool) -> Mesh {
    let mut m = Mesh::new();
    for (x, y) in [(x0, -1.0), (x1, -1.0), (x1, 1.0), (x0, 1.0)] {
        m.add_vertex(Vec3::new(x, y, z));
    }
    let uvs = vec![uv0, Vec2::new(uv1.x, uv0.y), uv1, Vec2::new(uv0.x, uv1.y)];
    let mut f = Face { verts: vec![0, 1, 2, 3], uvs, ..Default::default() };
    if flip {
        f.verts.reverse();
        f.uvs.reverse();
    }
    m.faces.push(f);
    m
}

fn px(img: &Image, x: u32, y: u32) -> [f32; 4] {
    img.pixels[(y * img.width + x) as usize]
}

#[test]
fn texture_paint_writes_texels_through_uvs() {
    let m = quad(0.0, -1.0, 1.0, Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), false);
    let mut img = Image::new("T", 64, 64);
    let s = TpSettings { color: RED, bleed: 0, ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    // Screen (25, 25) is the point (-0.5, 0.5): UV (0.25, 0.75), texel (16, 16).
    let n = st.dab_3d(&mut img, &m, &project, &s, (25.0, 25.0), 10.0, 1.0, (0.0, 0.0), false, None);
    assert!(n > 20);
    assert!(px(&img, 16, 16)[0] > 0.9);
    assert!(px(&img, 20, 16)[0] > 0.0 && px(&img, 20, 16)[0] < px(&img, 16, 16)[0] + 1e-6);
    assert_eq!(px(&img, 24, 16), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&img, 48, 48), [0.0, 0.0, 0.0, 1.0]);
    // Undo keeps only the touched tile and restores it.
    assert_eq!(st.undo.tiles(), 1);
    let ch = st.undo.finish(&img);
    let mut scene = Scene::new();
    scene.images.push(img.clone());
    let mut u = UndoStack::new();
    u.push_changes("Paint", ch);
    u.undo(&mut scene);
    assert!(scene.images[0].pixels.iter().all(|p| *p == [0.0, 0.0, 0.0, 1.0]));
    u.redo(&mut scene);
    assert_eq!(scene.images[0].pixels, img.pixels);
}

#[test]
fn texture_paint_culls_backfaces_and_occluded_texels() {
    let back = quad(0.0, -1.0, 1.0, Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), true);
    let mut img = Image::new("T", 64, 64);
    let s = TpSettings { color: RED, bleed: 0, ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    assert_eq!(st.dab_3d(&mut img, &back, &project, &s, (50.0, 50.0), 20.0, 1.0, (0.0, 0.0), false, None), 0);
    let s2 = TpSettings { backface_culling: false, ..s.clone() };
    assert!(st.dab_3d(&mut img, &back, &project, &s2, (50.0, 50.0), 20.0, 1.0, (0.0, 0.0), false, None) > 0);

    // A plane in front (z = 0.5) maps to the right strip of the image; the
    // plane behind it is not painted where it is covered.
    let mut m = quad(0.0, -1.0, 1.0, Vec2::new(0.0, 0.0), Vec2::new(0.5, 1.0), false);
    m.append(&quad(0.5, -1.0, 0.0, Vec2::new(0.75, 0.0), Vec2::new(1.0, 1.0), false));
    let mut img = Image::new("T", 64, 64);
    let mut st = TexStroke::new(0, &img);
    st.dab_3d(&mut img, &m, &project, &s, (25.0, 50.0), 10.0, 1.0, (0.0, 0.0), false, None);
    // Back plane texel under the brush: UV (0.125, 0.5) -> (8, 32).
    assert_eq!(px(&img, 8, 32)[0], 0.0, "occluded texel was painted");
    // Front plane texel under the brush: UV (0.875, 0.5) -> (56, 32).
    assert!(px(&img, 56, 32)[0] > 0.9);
    let s3 = TpSettings { occlude: false, ..s.clone() };
    st.dab_3d(&mut img, &m, &project, &s3, (25.0, 50.0), 10.0, 1.0, (0.0, 0.0), false, None);
    assert!(px(&img, 8, 32)[0] > 0.9);
}

#[test]
fn texture_paint_bleeds_past_island_borders() {
    let m = quad(0.0, -1.0, 1.0, Vec2::new(0.25, 0.25), Vec2::new(0.75, 0.75), false);
    let mut img = Image::new("T", 64, 64);
    let s = TpSettings { color: RED, bleed: 2, curve: crate::sculpt::Curve::Constant, ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    st.dab_3d(&mut img, &m, &project, &s, (50.0, 50.0), 200.0, 1.0, (0.0, 0.0), false, None);
    // Island spans texels 16..48.
    assert!(px(&img, 16, 32)[0] > 0.9);
    assert!(px(&img, 15, 32)[0] > 0.9, "bleed one texel out");
    assert!(px(&img, 14, 32)[0] > 0.9, "bleed two texels out");
    assert_eq!(px(&img, 12, 32)[0], 0.0);
}

#[test]
fn image_editor_painting() {
    let mut img = Image::new("T", 32, 32);
    let s = TpSettings { color: [0.0, 0.0, 1.0], ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    st.dab_2d(&mut img, &s, (10.0, 10.0), 4.0, 1.0, (0.0, 0.0), false, None);
    assert!(px(&img, 9, 9)[2] > 0.5);
    assert_eq!(px(&img, 20, 20)[2], 0.0);
    // Mask brush, then painting skips the masked texels.
    let mut mask = Vec::new();
    let ms = TpSettings { brush: TexBrush::Mask, curve: crate::sculpt::Curve::Constant, ..Default::default() };
    st.dab_2d(&mut img, &ms, (25.0, 25.0), 3.0, 1.0, (0.0, 0.0), false, Some(&mut mask));
    let red = TpSettings { color: RED, curve: crate::sculpt::Curve::Constant, ..Default::default() };
    st.dab_2d(&mut img, &red, (25.0, 25.0), 6.0, 1.0, (0.0, 0.0), false, Some(&mut mask));
    assert_eq!(px(&img, 25, 25)[0], 0.0);
    assert!(px(&img, 25, 29)[0] > 0.9);
    // Bucket fill stops at different colors.
    let mut img = Image::new("T", 16, 16);
    for y in 0..16 {
        img.pixels[(y * 16 + 8) as usize] = [1.0; 4];
    }
    let f = TpSettings { brush: TexBrush::Fill, color: RED, ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    let n = st.dab_2d(&mut img, &f, (2.0, 2.0), 1.0, 1.0, (0.0, 0.0), false, None);
    assert_eq!(n, 8 * 16);
    assert_eq!(px(&img, 12, 3)[0], 0.0);
    // Soften blurs an edge; clone copies from an offset.
    let mut img = Image::new("T", 16, 16);
    for y in 0..16 {
        for x in 8..16 {
            img.pixels[(y * 16 + x) as usize] = [1.0; 4];
        }
    }
    let soft = TpSettings { brush: TexBrush::Soften, ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    st.dab_2d(&mut img, &soft, (8.0, 8.0), 4.0, 1.0, (0.0, 0.0), false, None);
    let v = px(&img, 7, 8)[0];
    assert!(v > 0.05 && v < 0.95, "{v}");
    let mut img = Image::new("T", 16, 16);
    img.pixels[(2 * 16 + 2) as usize] = [0.0, 1.0, 0.0, 1.0];
    let cl = TpSettings { brush: TexBrush::Clone, curve: crate::sculpt::Curve::Constant, ..Default::default() };
    let mut st = TexStroke::new(0, &img);
    st.clone_offset = (-8.0, -8.0);
    st.dab_2d(&mut img, &cl, (10.5, 10.5), 1.0, 1.0, (0.0, 0.0), false, None);
    assert!(px(&img, 10, 10)[1] > 0.9);
}
