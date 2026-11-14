// SPDX-License-Identifier: GPL-3.0-only
//! Small linear algebra: `f32` vectors, unit quaternions, column-major 4x4 matrices.
use core::ops::{Add, AddAssign, Div, Index, Mul, MulAssign, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn lerp(self, o: Vec2, t: f32) -> Vec2 {
        Vec2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
}
impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    pub const ONE: Vec3 = Vec3 { x: 1.0, y: 1.0, z: 1.0 };
    pub const X: Vec3 = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
    pub const Y: Vec3 = Vec3 { x: 0.0, y: 1.0, z: 0.0 };
    pub const Z: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    /// Unit vector, or zero for a zero vector.
    pub fn normalize(self) -> Vec3 {
        let l = self.length();
        if l > 1e-20 { self / l } else { Vec3::ZERO }
    }
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    pub fn mul_elem(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x * o.x, self.y * o.y, self.z * o.z)
    }
    pub fn abs(self) -> Vec3 {
        Vec3::new(self.x.abs(), self.y.abs(), self.z.abs())
    }
    pub fn max_elem(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }
    pub fn distance(self, o: Vec3) -> f32 {
        (self - o).length()
    }
    pub fn approx_eq(self, o: Vec3, eps: f32) -> bool {
        (self - o).abs().max_elem() <= eps
    }
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
    pub fn from_array(a: [f32; 3]) -> Vec3 {
        Vec3::new(a[0], a[1], a[2])
    }
    /// Any unit vector perpendicular to this (unit) vector.
    pub fn any_orthonormal(self) -> Vec3 {
        let a = if self.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
        self.cross(a).normalize()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Mul<Vec3> for f32 {
    type Output = Vec3;
    fn mul(self, v: Vec3) -> Vec3 {
        v * self
    }
}
impl MulAssign<f32> for Vec3 {
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}
impl Div<f32> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f32) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}
impl Index<usize> for Vec3 {
    type Output = f32;
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            _ => &self.z,
        }
    }
}

/// Unit quaternion (x, y, z vector part, w scalar part).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
    pub fn from_axis_angle(axis: Vec3, radians: f32) -> Quat {
        let a = axis.normalize();
        let (s, c) = (radians * 0.5).sin_cos();
        Quat { x: a.x * s, y: a.y * s, z: a.z * s, w: c }
    }
    /// Blender's default XYZ Euler order: rotate about X, then Y, then Z.
    pub fn from_euler_xyz(x: f32, y: f32, z: f32) -> Quat {
        Quat::from_axis_angle(Vec3::Z, z) * Quat::from_axis_angle(Vec3::Y, y) * Quat::from_axis_angle(Vec3::X, x)
    }
    pub fn normalize(self) -> Quat {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if l < 1e-20 {
            return Quat::IDENTITY;
        }
        Quat { x: self.x / l, y: self.y / l, z: self.z / l, w: self.w / l }
    }
    pub fn conjugate(self) -> Quat {
        Quat { x: -self.x, y: -self.y, z: -self.z, w: self.w }
    }
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = 2.0 * q.cross(v);
        v + self.w * t + q.cross(t)
    }
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }
    /// Spherical interpolation along the shortest arc.
    pub fn slerp(self, mut o: Quat, t: f32) -> Quat {
        let mut d = self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w;
        if d < 0.0 {
            o = Quat { x: -o.x, y: -o.y, z: -o.z, w: -o.w };
            d = -d;
        }
        if d > 0.9995 {
            return Quat {
                x: self.x + (o.x - self.x) * t,
                y: self.y + (o.y - self.y) * t,
                z: self.z + (o.z - self.z) * t,
                w: self.w + (o.w - self.w) * t,
            }
            .normalize();
        }
        let th = d.acos();
        let s = th.sin();
        let (a, b) = (((1.0 - t) * th).sin() / s, (t * th).sin() / s);
        Quat { x: a * self.x + b * o.x, y: a * self.y + b * o.y, z: a * self.z + b * o.z, w: a * self.w + b * o.w }
    }
}

impl Mul for Quat {
    type Output = Quat;
    fn mul(self, b: Quat) -> Quat {
        let a = self;
        Quat {
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
            x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        }
    }
}

/// Column-major 4x4 matrix: `cols[c][r]`, like glTF and OpenGL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub cols: [[f32; 4]; 4],
}

impl Default for Mat4 {
    fn default() -> Self {
        Mat4::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4 { cols: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]] };

    pub fn from_cols_array(a: [f32; 16]) -> Mat4 {
        let mut cols = [[0.0; 4]; 4];
        for c in 0..4 {
            for r in 0..4 {
                cols[c][r] = a[c * 4 + r];
            }
        }
        Mat4 { cols }
    }
    pub fn to_cols_array(&self) -> [f32; 16] {
        let mut a = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                a[c * 4 + r] = self.cols[c][r];
            }
        }
        a
    }
    pub fn translation(t: Vec3) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.cols[3] = [t.x, t.y, t.z, 1.0];
        m
    }
    /// Translation * rotation * scale.
    pub fn from_trs(t: Vec3, r: Quat, s: Vec3) -> Mat4 {
        let x = r.rotate(Vec3::X) * s.x;
        let y = r.rotate(Vec3::Y) * s.y;
        let z = r.rotate(Vec3::Z) * s.z;
        Mat4 { cols: [[x.x, x.y, x.z, 0.0], [y.x, y.y, y.z, 0.0], [z.x, z.y, z.z, 0.0], [t.x, t.y, t.z, 1.0]] }
    }
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        let c = &self.cols;
        let w = c[0][3] * p.x + c[1][3] * p.y + c[2][3] * p.z + c[3][3];
        let v = Vec3::new(
            c[0][0] * p.x + c[1][0] * p.y + c[2][0] * p.z + c[3][0],
            c[0][1] * p.x + c[1][1] * p.y + c[2][1] * p.z + c[3][1],
            c[0][2] * p.x + c[1][2] * p.y + c[2][2] * p.z + c[3][2],
        );
        if (w - 1.0).abs() > 1e-6 && w.abs() > 1e-20 { v / w } else { v }
    }
    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        let c = &self.cols;
        Vec3::new(
            c[0][0] * v.x + c[1][0] * v.y + c[2][0] * v.z,
            c[0][1] * v.x + c[1][1] * v.y + c[2][1] * v.z,
            c[0][2] * v.x + c[1][2] * v.y + c[2][2] * v.z,
        )
    }
    pub fn transpose(&self) -> Mat4 {
        let mut m = Mat4 { cols: [[0.0; 4]; 4] };
        for c in 0..4 {
            for r in 0..4 {
                m.cols[c][r] = self.cols[r][c];
            }
        }
        m
    }
    pub fn determinant(&self) -> f32 {
        let i = self.inverse_unscaled();
        i.1
    }
    /// General inverse; identity if singular.
    pub fn inverse(&self) -> Mat4 {
        let (m, det) = self.inverse_unscaled();
        if det.abs() < 1e-30 {
            return Mat4::IDENTITY;
        }
        let mut out = m;
        for c in 0..4 {
            for r in 0..4 {
                out.cols[c][r] /= det;
            }
        }
        out
    }
    fn inverse_unscaled(&self) -> (Mat4, f32) {
        let m = self.to_cols_array();
        let mut inv = [0.0f32; 16];
        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
        let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        (Mat4::from_cols_array(inv), det)
    }
    /// Right-handed look-at view matrix (camera looks down -Z).
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
        let f = (target - eye).normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);
        Mat4 {
            cols: [
                [s.x, u.x, -f.x, 0.0],
                [s.y, u.y, -f.y, 0.0],
                [s.z, u.z, -f.z, 0.0],
                [-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0],
            ],
        }
    }
    /// Right-handed perspective projection to OpenGL clip space (z in -1..1).
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fov_y * 0.5).tan();
        let nf = 1.0 / (near - far);
        Mat4 { cols: [[f / aspect, 0.0, 0.0, 0.0], [0.0, f, 0.0, 0.0], [0.0, 0.0, (far + near) * nf, -1.0], [0.0, 0.0, 2.0 * far * near * nf, 0.0]] }
    }
    /// Split into translation, rotation and scale (assumes no shear).
    pub fn to_trs(&self) -> (Vec3, Quat, Vec3) {
        let c = &self.cols;
        let t = Vec3::new(c[3][0], c[3][1], c[3][2]);
        let mut x = Vec3::new(c[0][0], c[0][1], c[0][2]);
        let y = Vec3::new(c[1][0], c[1][1], c[1][2]);
        let z = Vec3::new(c[2][0], c[2][1], c[2][2]);
        let mut s = Vec3::new(x.length(), y.length(), z.length());
        if x.cross(y).dot(z) < 0.0 {
            s.x = -s.x;
            x = -x;
        }
        let (x, y, z) = (if s.x != 0.0 { x / s.x.abs() } else { Vec3::X }, if s.y != 0.0 { y / s.y } else { Vec3::Y }, if s.z != 0.0 { z / s.z } else { Vec3::Z });
        // Rotation matrix to quaternion (Shepperd).
        let (m00, m11, m22) = (x.x, y.y, z.z);
        let tr = m00 + m11 + m22;
        let q = if tr > 0.0 {
            let s4 = (tr + 1.0).sqrt() * 2.0;
            Quat { w: 0.25 * s4, x: (y.z - z.y) / s4, y: (z.x - x.z) / s4, z: (x.y - y.x) / s4 }
        } else if m00 > m11 && m00 > m22 {
            let s4 = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            Quat { w: (y.z - z.y) / s4, x: 0.25 * s4, y: (y.x + x.y) / s4, z: (z.x + x.z) / s4 }
        } else if m11 > m22 {
            let s4 = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            Quat { w: (z.x - x.z) / s4, x: (y.x + x.y) / s4, y: 0.25 * s4, z: (z.y + y.z) / s4 }
        } else {
            let s4 = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            Quat { w: (x.y - y.x) / s4, x: (z.x + x.z) / s4, y: (z.y + y.z) / s4, z: 0.25 * s4 }
        };
        (t, q.normalize(), s)
    }
}

impl Mul for Mat4 {
    type Output = Mat4;
    fn mul(self, b: Mat4) -> Mat4 {
        let mut m = Mat4 { cols: [[0.0; 4]; 4] };
        for c in 0..4 {
            for r in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += self.cols[k][r] * b.cols[c][k];
                }
                m.cols[c][r] = s;
            }
        }
        m
    }
}

/// Axis-aligned bounding box. `Aabb::EMPTY` grows with [`Aabb::grow`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };
    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    pub fn union(self, o: Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    pub fn surface_area(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let d = self.size();
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quat_rotates_and_matrix_round_trips() {
        let q = Quat::from_axis_angle(Vec3::Z, core::f32::consts::FRAC_PI_2);
        assert!(q.rotate(Vec3::X).approx_eq(Vec3::Y, 1e-6));
        let m = Mat4::from_trs(Vec3::new(1.0, 2.0, 3.0), q, Vec3::new(2.0, 3.0, 4.0));
        let (t, r, s) = m.to_trs();
        assert!(t.approx_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5));
        assert!(s.approx_eq(Vec3::new(2.0, 3.0, 4.0), 1e-5));
        assert!(r.rotate(Vec3::X).approx_eq(Vec3::Y, 1e-5));
        let p = Vec3::new(0.3, -0.7, 1.1);
        assert!(m.inverse().transform_point(m.transform_point(p)).approx_eq(p, 1e-5));
    }

    #[test]
    fn euler_order_matches_blender() {
        // X 90 then Z 90: +Y -> +Z (after X) -> stays +Z.
        let q = Quat::from_euler_xyz(core::f32::consts::FRAC_PI_2, 0.0, core::f32::consts::FRAC_PI_2);
        assert!(q.rotate(Vec3::Y).approx_eq(Vec3::Z, 1e-6));
        assert!(q.rotate(Vec3::X).approx_eq(Vec3::Y, 1e-6));
    }
}
