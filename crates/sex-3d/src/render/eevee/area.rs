// SPDX-License-Identifier: GPL-3.0-only
//! Area lights for EEVEE, integrated over the light's shape instead of from
//! a representative point.
//!
//! Lambertian lobes (diffuse and translucent) get the exact irradiance of
//! the polygon (rectangles as four corners, disks and ellipses as 16-gons)
//! clipped to the lobe's hemisphere: Lambert's edge-integral formula, which
//! is the linearly transformed cosine (LTC) integral for the identity
//! transform, the cosine distribution itself. Glossy lobes are integrated by
//! stratified samples over the light's area, rotated per pixel and per TAA
//! sample, so the accumulation converges to the exact integral.
use crate::math::Vec3;
use core::f32::consts::{PI, TAU};

/// Corners of an area light `center ± u ± v` (the ellipse inscribed in it
/// when `ellipse`), in order around the outline.
pub(super) fn outline(center: Vec3, u: Vec3, v: Vec3, ellipse: bool, out: &mut [Vec3; 16]) -> usize {
    if !ellipse {
        out[0] = center - u - v;
        out[1] = center + u - v;
        out[2] = center + u + v;
        out[3] = center - u + v;
        return 4;
    }
    // A 16-gon of the same area as the ellipse.
    let k = (PI / 16.0 / (TAU / 16.0).sin() * 2.0).sqrt();
    for (i, o) in out.iter_mut().enumerate() {
        let a = i as f32 * TAU / 16.0;
        *o = center + u * (a.cos() * k) + v * (a.sin() * k);
    }
    16
}

/// Form factor of the polygon `verts` seen from `p` with unit normal `n`:
/// the cosine-weighted solid angle over pi (the part below the horizon is
/// clipped away). Irradiance from a uniform radiance `L` is `pi * L * F`.
pub(super) fn form_factor(p: Vec3, n: Vec3, verts: &[Vec3]) -> f32 {
    // Clip against the tangent plane (Sutherland-Hodgman), relative to p.
    let mut poly = [Vec3::ZERO; 20];
    let mut m = 0;
    let k = verts.len();
    for i in 0..k {
        let (a, b) = (verts[i] - p, verts[(i + 1) % k] - p);
        let (da, db) = (a.dot(n), b.dot(n));
        if da >= 0.0 && m < poly.len() {
            poly[m] = a;
            m += 1;
        }
        if (da >= 0.0) != (db >= 0.0) && m < poly.len() {
            let t = da / (da - db);
            poly[m] = a + (b - a) * t;
            m += 1;
        }
    }
    if m < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..m {
        let (a, b) = (poly[i].normalize(), poly[(i + 1) % m].normalize());
        let c = a.cross(b);
        let len = c.length();
        if !(len > 1e-9) {
            continue;
        }
        let theta = a.dot(b).clamp(-1.0, 1.0).acos();
        sum += theta * c.dot(n) / len;
    }
    (sum.abs() / TAU).min(1.0)
}

/// Point `k` of `n` stratified samples on the light (offsets `ru`, `rv`
/// in 0..1 rotate the strata).
pub(super) fn sample_point(center: Vec3, u: Vec3, v: Vec3, ellipse: bool, k: usize, n: usize, ru: f32, rv: f32) -> Vec3 {
    let side = (n as f32).sqrt().ceil().max(1.0) as usize;
    let (i, j) = (k % side, k / side);
    let s = ((i as f32 + ru) / side as f32).fract();
    let t = ((j as f32 + rv) / side as f32).fract();
    if ellipse {
        // Concentric disk mapping (Shirley-Chiu).
        let (a, b) = (2.0 * s - 1.0, 2.0 * t - 1.0);
        let (r, phi) = if a == 0.0 && b == 0.0 {
            (0.0, 0.0)
        } else if a.abs() > b.abs() {
            (a, PI / 4.0 * (b / a))
        } else {
            (b, PI / 2.0 - PI / 4.0 * (a / b))
        };
        center + u * (r * phi.cos()) + v * (r * phi.sin())
    } else {
        center + u * (2.0 * s - 1.0) + v * (2.0 * t - 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_factor_of_a_large_overhead_square_tends_to_one() {
        let mut o = [Vec3::ZERO; 16];
        let k = outline(Vec3::new(0.0, 0.0, 1.0), Vec3::X * 1000.0, Vec3::Y * 1000.0, false, &mut o);
        let f = form_factor(Vec3::ZERO, Vec3::Z, &o[..k]);
        assert!((f - 1.0).abs() < 1e-3, "{f}");
        // Half below the horizon: half the form factor of the whole sphere side.
        let k = outline(Vec3::new(1.0, 0.0, 0.0), Vec3::Y * 1000.0, Vec3::Z * 1000.0, false, &mut o);
        let f = form_factor(Vec3::ZERO, Vec3::Z, &o[..k]);
        assert!((f - 0.5).abs() < 1e-3, "{f}");
    }

    #[test]
    fn form_factor_of_a_small_disk_matches_the_point_formula() {
        // Small disk of radius r at height h facing down: F ≈ r² / h².
        let (r, h) = (0.05f32, 2.0f32);
        let mut o = [Vec3::ZERO; 16];
        let k = outline(Vec3::new(0.0, 0.0, h), Vec3::X * r, Vec3::Y * r, true, &mut o);
        let f = form_factor(Vec3::ZERO, Vec3::Z, &o[..k]);
        let want = r * r / (h * h + r * r);
        assert!((f - want).abs() < 0.01 * want, "{f} vs {want}");
    }
}
