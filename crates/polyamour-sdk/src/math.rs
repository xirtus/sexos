// SPDX-License-Identifier: GPL-3.0-only
//! Pure-Rust `#![no_std]` math types replacing Blender's `mathutils` (`Vector`,
//! `Euler`, `Color`, `Quat`) for Polyamour WASM add-ons and procedural scripts.

use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub const fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0, z: 1.0 };
    pub const X: Self = Self { x: 1.0, y: 0.0, z: 0.0 };
    pub const Y: Self = Self { x: 0.0, y: 1.0, z: 0.0 };
    pub const Z: Self = Self { x: 0.0, y: 0.0, z: 1.0 };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }

    pub const fn from_array(a: [f32; 3]) -> Self {
        Self { x: a[0], y: a[1], z: a[2] }
    }

    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        libm::sqrtf(self.length_squared())
    }

    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > 0.0 {
            self / len
        } else {
            self
        }
    }

    pub fn approx_eq(self, other: Self, eps: f32) -> bool {
        libm::fabsf(self.x - other.x) <= eps
            && libm::fabsf(self.y - other.y) <= eps
            && libm::fabsf(self.z - other.z) <= eps
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl SubAssign for Vec3 {
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Div<f32> for Vec3 {
    type Output = Self;
    fn div(self, s: f32) -> Self {
        Self::new(self.x / s, self.y / s, self.z / s)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// XYZ Euler angles in radians (Blender's default rotation order).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Euler {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Euler {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

/// Linear RGBA color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Self::rgba(0.8, 0.8, 0.8, 1.0)
    }
}

impl Color {
    pub const WHITE: Self = Self::rgba(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgba(0.0, 0.0, 0.0, 1.0);

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn to_rgb_array(self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }

    pub const fn to_rgba_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Unit quaternion stored in `(w, x, y, z)` convention at the `polyops` boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Self = Self { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    pub const fn from_wxyz(w: f32, x: f32, y: f32, z: f32) -> Self {
        Self { w, x, y, z }
    }

    pub fn normalize(self) -> Self {
        let n = libm::sqrtf(self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z);
        if n > 0.0 {
            Self { w: self.w / n, x: self.x / n, y: self.y / n, z: self.z / n }
        } else {
            Self::IDENTITY
        }
    }

    pub fn from_euler_xyz(e: Euler) -> Self {
        let (sx, cx) = libm::sincosf(e.x * 0.5);
        let (sy, cy) = libm::sincosf(e.y * 0.5);
        let (sz, cz) = libm::sincosf(e.z * 0.5);
        Self {
            w: cx * cy * cz + sx * sy * sz,
            x: sx * cy * cz - cx * sy * sz,
            y: cx * sy * cz + sx * cy * sz,
            z: cx * cy * sz - sx * sy * cz,
        }
        .normalize()
    }

    pub fn to_euler_xyz(self) -> Euler {
        let q = self.normalize();
        let (w, x, y, z) = (q.w, q.x, q.y, q.z);
        let r00 = 1.0 - 2.0 * (y * y + z * z);
        let r10 = 2.0 * (x * y + w * z);
        let r20 = 2.0 * (x * z - w * y);
        let r21 = 2.0 * (y * z + w * x);
        let r22 = 1.0 - 2.0 * (x * x + y * y);
        let sy = (-r20).clamp(-1.0, 1.0);
        if libm::fabsf(sy) > 0.99999 {
            let r01 = 2.0 * (x * y - w * z);
            let r11 = 1.0 - 2.0 * (x * x + z * z);
            let sign = if sy >= 0.0 { 1.0 } else { -1.0 };
            return Euler::new(libm::atan2f(sign * r01, r11), libm::asinf(sy), 0.0);
        }
        Euler::new(libm::atan2f(r21, r22), libm::asinf(sy), libm::atan2f(r10, r00))
    }

    pub const fn to_wxyz_array(self) -> [f32; 4] {
        [self.w, self.x, self.y, self.z]
    }
}
