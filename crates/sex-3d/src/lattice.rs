// SPDX-License-Identifier: GPL-3.0-only
//! Lattices (Blender's `Lattice` data) and lattice deformation
//! (`lattice_deform.cc`).
//!
//! A lattice is a `points_u × points_v × points_w` grid whose rest
//! positions span -0.5..0.5 on each axis of the lattice object (a single
//! point sits at 0). [`Lattice::points`] holds the current (possibly moved)
//! positions, U fastest. Deformation evaluates the grid's displacement at a
//! point with the per-axis interpolation weights of
//! [`KeyInterpolation::weights`] (B-spline by default), clamping indices at
//! the borders, exactly like `BKE_lattice_deform_data_eval_co`. An unmoved
//! lattice therefore leaves points where they are, whatever the type.
use crate::math::{Mat4, Vec3};
use crate::mesh::KeyInterpolation;

#[derive(Clone, Debug, PartialEq)]
pub struct Lattice {
    pub points_u: u32,
    pub points_v: u32,
    pub points_w: u32,
    pub interpolation_u: KeyInterpolation,
    pub interpolation_v: KeyInterpolation,
    pub interpolation_w: KeyInterpolation,
    /// Only the outside points are shown/edited (display option; kept for
    /// files).
    pub use_outside: bool,
    /// Point positions in lattice space, U fastest then V then W.
    pub points: Vec<Vec3>,
}

impl Default for Lattice {
    fn default() -> Self {
        Lattice::new(2, 2, 2)
    }
}

/// Blender's `calc_lat_fudu`: first coordinate and step of a lattice axis.
fn fudu(res: u32) -> (f32, f32) {
    if res <= 1 {
        (0.0, 0.0)
    } else {
        (-0.5, 1.0 / (res - 1) as f32)
    }
}

impl Lattice {
    /// An undeformed lattice (B-spline interpolation, Blender's default).
    pub fn new(u: u32, v: u32, w: u32) -> Lattice {
        let (u, v, w) = (u.clamp(1, 64), v.clamp(1, 64), w.clamp(1, 64));
        let mut l = Lattice {
            points_u: u,
            points_v: v,
            points_w: w,
            interpolation_u: KeyInterpolation::BSpline,
            interpolation_v: KeyInterpolation::BSpline,
            interpolation_w: KeyInterpolation::BSpline,
            use_outside: false,
            points: Vec::new(),
        };
        l.points = l.rest_points();
        l
    }

    pub fn point_count(&self) -> usize {
        (self.points_u * self.points_v * self.points_w) as usize
    }

    pub fn index(&self, u: u32, v: u32, w: u32) -> usize {
        ((w * self.points_v + v) * self.points_u + u) as usize
    }

    /// The undeformed grid positions.
    pub fn rest_points(&self) -> Vec<Vec3> {
        let (fu, du) = fudu(self.points_u);
        let (fv, dv) = fudu(self.points_v);
        let (fw, dw) = fudu(self.points_w);
        let mut out = Vec::with_capacity(self.point_count());
        for w in 0..self.points_w {
            for v in 0..self.points_v {
                for u in 0..self.points_u {
                    out.push(Vec3::new(fu + du * u as f32, fv + dv * v as f32, fw + dw * w as f32));
                }
            }
        }
        out
    }

    /// Change the resolution, resetting points to rest.
    pub fn resize(&mut self, u: u32, v: u32, w: u32) {
        *self = Lattice { interpolation_u: self.interpolation_u, interpolation_v: self.interpolation_v, interpolation_w: self.interpolation_w, ..Lattice::new(u, v, w) };
    }

    /// Deform `coords` (in the deformed object's space). `latmat` takes
    /// object space into lattice space (`lattice_world⁻¹ × object_world`).
    /// `strength` scales the displacement, `weights` (per coordinate) scale
    /// it further (the modifier's vertex group on the deformed mesh). The
    /// lattice's own vertex group (`Lattice.vertex_group`) is not supported.
    pub fn deform(&self, coords: &mut [Vec3], latmat: &Mat4, strength: f32, weights: Option<&[f32]>) {
        let n = self.point_count();
        if self.points.len() != n || n == 0 {
            return;
        }
        let imat = latmat.inverse();
        let rest = self.rest_points();
        // Displacements, in object space.
        let data: Vec<Vec3> = (0..n).map(|i| imat.transform_vector(self.points[i] - rest[i])).collect();
        let (fu, du) = fudu(self.points_u);
        let (fv, dv) = fudu(self.points_v);
        let (fw, dw) = fudu(self.points_w);
        let axis = |x: f32, f: f32, d: f32, res: u32, ty: KeyInterpolation| -> (i64, [f32; 4]) {
            if res > 1 {
                let u = (x - f) / d;
                let ui = u.floor();
                (ui as i64, ty.weights(u - ui))
            } else {
                (0, [0.0, 1.0, 0.0, 0.0])
            }
        };
        let clampi = |i: i64, res: u32| -> u32 { i.clamp(0, res as i64 - 1) as u32 };
        for (ci, co) in coords.iter_mut().enumerate() {
            let w0 = strength * weights.map_or(1.0, |w| w.get(ci).copied().unwrap_or(0.0));
            if w0 == 0.0 {
                continue;
            }
            let vec = latmat.transform_point(*co);
            let (ui, tu) = axis(vec.x, fu, du, self.points_u, self.interpolation_u);
            let (vi, tv) = axis(vec.y, fv, dv, self.points_v, self.interpolation_v);
            let (wi, tw) = axis(vec.z, fw, dw, self.points_w, self.interpolation_w);
            let mut out = *co;
            for ww in 0..4 {
                let w = tw[ww];
                if w == 0.0 {
                    continue;
                }
                let iw = clampi(wi - 1 + ww as i64, self.points_w);
                for vv in 0..4 {
                    let v = w * tv[vv];
                    if v == 0.0 {
                        continue;
                    }
                    let iv = clampi(vi - 1 + vv as i64, self.points_v);
                    for uu in 0..4 {
                        let u = w0 * v * tu[uu];
                        if u == 0.0 {
                            continue;
                        }
                        let iu = clampi(ui - 1 + uu as i64, self.points_u);
                        let idx = self.index(iu, iv, iw);
                        out += data[idx] * u;
                    }
                }
            }
            *co = out;
        }
    }
}
