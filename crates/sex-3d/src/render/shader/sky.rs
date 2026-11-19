// SPDX-License-Identifier: GPL-3.0-only
//! Sky Texture.
//!
//! * **Preetham**: the analytic Perez model with Preetham's zenith fits, as in
//!   Cycles' `sky_radiance_preetham`.
//! * **Hosek / Wilkie**: approximated with the Preetham model (no fitted
//!   dataset is bundled).
//! * **Nishita**: approximated by single scattering through a Rayleigh + Mie
//!   + ozone atmosphere, integrated numerically into a table over view
//!   elevation and azimuth when the program is compiled, plus the sun disc.
//!   Cycles precomputes a multiple-scattering-free table the same way but with
//!   spectral sampling; here the integration runs on three wavelengths, so
//!   colours and absolute levels are close but not identical.
use crate::math::Vec3;
use core::f32::consts::{FRAC_PI_2, PI, TAU};

#[derive(Clone, Debug)]
pub(crate) enum Sky {
    Preetham(Preetham),
    Nishita(Box<Nishita>),
}

impl Sky {
    pub fn eval(&self, d: Vec3) -> [f32; 3] {
        match self {
            Sky::Preetham(p) => p.eval(d),
            Sky::Nishita(n) => n.eval(d),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Preetham {
    sun_theta: f32,
    sun_phi: f32,
    radiance: [f32; 3],
    config: [[f32; 5]; 3],
}

fn perez(lam: &[f32; 5], theta: f32, gamma: f32) -> f32 {
    let ct = theta.cos();
    let cg = gamma.cos();
    (1.0 + lam[0] * (lam[1] / ct).exp()) * (1.0 + lam[2] * (lam[3] * gamma).exp() + lam[4] * cg * cg)
}

fn spherical(d: Vec3) -> (f32, f32) {
    (d.z.clamp(-1.0, 1.0).acos(), d.y.atan2(d.x))
}

fn angle_between(t1: f32, p1: f32, t2: f32, p2: f32) -> f32 {
    let c = t1.sin() * t2.sin() * (p1 - p2).cos() + t1.cos() * t2.cos();
    c.clamp(-1.0, 1.0).acos()
}

fn xyz_to_rgb(x: f32, y: f32, z: f32) -> [f32; 3] {
    super::color::xyz_to_rgb([x, y, z])
}

impl Preetham {
    pub fn new(sun_dir: Vec3, turbidity: f32) -> Preetham {
        let d = if sun_dir.length_squared() > 0.0 { sun_dir.normalize() } else { Vec3::Z };
        let (theta, phi) = spherical(d);
        let t = turbidity.clamp(1.0, 30.0);
        let chi = (4.0 / 9.0 - t / 120.0) * (PI - 2.0 * theta);
        let y_zenith = ((4.0453 * t - 4.9710) * chi.tan() - 0.2155 * t + 2.4192) * 0.06;
        let (t2, th, th2, th3) = (t * t, theta, theta * theta, theta * theta * theta);
        let x = t2 * (0.00166 * th3 - 0.00375 * th2 + 0.00209 * th) + t * (-0.02903 * th3 + 0.06377 * th2 - 0.03202 * th + 0.00394) + (0.11693 * th3 - 0.21196 * th2 + 0.06052 * th + 0.25886);
        let y = t2 * (0.00275 * th3 - 0.00610 * th2 + 0.00317 * th) + t * (-0.04214 * th3 + 0.08970 * th2 - 0.04153 * th + 0.00516) + (0.15346 * th3 - 0.26756 * th2 + 0.06670 * th + 0.26688);
        let cy = [0.1787 * t - 1.4630, -0.3554 * t + 0.4275, -0.0227 * t + 5.3251, 0.1206 * t - 2.5771, -0.0670 * t + 0.3703];
        let cx = [-0.0193 * t - 0.2592, -0.0665 * t + 0.0008, -0.0004 * t + 0.2125, -0.0641 * t - 0.8989, -0.0033 * t + 0.0452];
        let cyy = [-0.0167 * t - 0.2608, -0.0950 * t + 0.0092, -0.0079 * t + 0.2102, -0.0441 * t - 1.6537, -0.0109 * t + 0.0529];
        let radiance = [x / perez(&cx, 0.0, theta), y / perez(&cyy, 0.0, theta), y_zenith / perez(&cy, 0.0, theta)];
        Preetham { sun_theta: theta, sun_phi: phi, radiance, config: [cx, cyy, cy] }
    }

    pub fn eval(&self, d: Vec3) -> [f32; 3] {
        let (theta, phi) = spherical(d);
        let gamma = angle_between(theta, phi, self.sun_theta, self.sun_phi);
        let theta = theta.min(FRAC_PI_2 - 0.001);
        let x = self.radiance[0] * perez(&self.config[0], theta, gamma);
        let y = self.radiance[1] * perez(&self.config[1], theta, gamma);
        let yy = self.radiance[2] * perez(&self.config[2], theta, gamma);
        if y <= 0.0 {
            return [0.0; 3];
        }
        let c = xyz_to_rgb(x / y * yy, yy, (1.0 - x - y) / y * yy);
        c.map(|v| v.max(0.0))
    }
}

const EARTH_R: f32 = 6_360_000.0;
const ATMOS_R: f32 = 6_420_000.0;
const RAYLEIGH_H: f32 = 8000.0;
const MIE_H: f32 = 1200.0;
/// Rayleigh scattering at 680, 550, 440 nm (per metre).
const BETA_R: [f32; 3] = [5.8e-6, 13.5e-6, 33.1e-6];
const BETA_M: f32 = 21e-6;
const BETA_OZONE: [f32; 3] = [0.65e-6, 1.88e-6, 0.085e-6];
const MIE_G: f32 = 0.76;
const TABLE_E: usize = 64;
const TABLE_A: usize = 64;
/// Overall radiance scale so a clear midday sky is comparable to Cycles'.
const SKY_SCALE: f32 = 0.08;

#[derive(Clone, Debug)]
pub(crate) struct Nishita {
    sun: Vec3,
    sun_rotation: f32,
    /// Sky radiance over (elevation 0..pi/2, azimuth relative to the sun 0..pi).
    table: Vec<[f32; 3]>,
    sun_disc: bool,
    sun_cos: f32,
    sun_radiance: [f32; 3],
    ground: [f32; 3],
}

struct Atmosphere {
    altitude: f32,
    air: f32,
    dust: f32,
    ozone: f32,
}

fn ray_sphere_exit(o: Vec3, d: Vec3, r: f32) -> f32 {
    let b = o.dot(d);
    let c = o.dot(o) - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return 0.0;
    }
    (-b + disc.sqrt()).max(0.0)
}

fn ray_hits_earth(o: Vec3, d: Vec3) -> bool {
    let b = o.dot(d);
    let c = o.dot(o) - EARTH_R * EARTH_R;
    b < 0.0 && b * b - c > 0.0
}

impl Atmosphere {
    fn density(&self, h: f32) -> (f32, f32, f32) {
        let r = (-h / RAYLEIGH_H).exp() * self.air;
        let m = (-h / MIE_H).exp() * self.dust;
        // Ozone: a tent around 25 km.
        let o = (1.0 - (h - 25_000.0).abs() / 15_000.0).max(0.0) * self.ozone;
        (r, m, o)
    }

    fn optical_depth(&self, o: Vec3, d: Vec3, steps: usize) -> [f32; 3] {
        let len = ray_sphere_exit(o, d, ATMOS_R);
        let ds = len / steps as f32;
        let (mut r, mut m, mut oz) = (0.0, 0.0, 0.0);
        for i in 0..steps {
            let p = o + d * ((i as f32 + 0.5) * ds);
            let (a, b, c) = self.density(p.length() - EARTH_R);
            r += a * ds;
            m += b * ds;
            oz += c * ds;
        }
        core::array::from_fn(|k| BETA_R[k] * r + 1.1 * BETA_M * m + BETA_OZONE[k] * oz)
    }

    /// Single-scattered radiance along `d` for unit sun irradiance from `sun`.
    fn single_scatter(&self, d: Vec3, sun: Vec3) -> [f32; 3] {
        let o = Vec3::new(0.0, 0.0, EARTH_R + self.altitude.max(1.0));
        let len = ray_sphere_exit(o, d, ATMOS_R);
        if len <= 0.0 {
            return [0.0; 3];
        }
        let steps = 24;
        let ds = len / steps as f32;
        let mu = d.dot(sun);
        let phase_r = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
        let g = MIE_G;
        let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g * g) * (1.0 + mu * mu)) / ((2.0 + g * g) * (1.0 + g * g - 2.0 * g * mu).powf(1.5));
        let mut tau = [0.0f32; 3];
        let mut sum = [0.0f32; 3];
        for i in 0..steps {
            let p = o + d * ((i as f32 + 0.5) * ds);
            let (dr, dm, doz) = self.density(p.length() - EARTH_R);
            for k in 0..3 {
                tau[k] += (BETA_R[k] * dr + 1.1 * BETA_M * dm + BETA_OZONE[k] * doz) * ds;
            }
            if ray_hits_earth(p, sun) {
                continue;
            }
            let ts = self.optical_depth(p, sun, 8);
            for k in 0..3 {
                let att = (-(tau[k] + ts[k])).exp();
                sum[k] += att * (BETA_R[k] * dr * phase_r + BETA_M * dm * phase_m) * ds;
            }
        }
        sum
    }
}

impl Nishita {
    #[allow(clippy::too_many_arguments)]
    pub fn new(sun_elevation: f32, sun_rotation: f32, sun_size: f32, sun_intensity: f32, sun_disc: bool, altitude: f32, air: f32, dust: f32, ozone: f32) -> Nishita {
        let atm = Atmosphere { altitude: altitude.clamp(1.0, 59_999.0), air: air.max(0.0), dust: dust.max(0.0), ozone: ozone.max(0.0) };
        let el = sun_elevation.clamp(-FRAC_PI_2, FRAC_PI_2);
        // Sun in the table frame (azimuth 0).
        let sun_local = Vec3::new(el.cos(), 0.0, el.sin());
        let mut table = Vec::with_capacity(TABLE_E * TABLE_A);
        for ei in 0..TABLE_E {
            // Denser near the horizon, like Cycles' table.
            let x = ei as f32 / (TABLE_E - 1) as f32;
            let e = x * x * FRAC_PI_2;
            for ai in 0..TABLE_A {
                let a = ai as f32 / (TABLE_A - 1) as f32 * PI;
                let d = Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin());
                let s = atm.single_scatter(d, sun_local);
                table.push(s.map(|v| v * SKY_SCALE * 20.0 * sun_intensity.max(0.0)));
            }
        }
        // Sun disc: transmitted solar radiance.
        let o = Vec3::new(0.0, 0.0, EARTH_R + atm.altitude);
        let tr = if ray_hits_earth(o, sun_local) { [0.0; 3] } else { atm.optical_depth(o, sun_local, 32).map(|t| (-t).exp()) };
        let half = (sun_size * 0.5).max(1e-4);
        let solid = TAU * (1.0 - half.cos());
        let sun_irr = SKY_SCALE * 20.0 * sun_intensity.max(0.0) * 0.6;
        let sun_radiance = tr.map(|t| t * sun_irr / solid.max(1e-8));
        let sun = Vec3::new(el.cos() * sun_rotation.sin(), -el.cos() * sun_rotation.cos(), el.sin());
        // The ground: the horizon colour, darkened.
        let h = Vec3::new(1.0, 0.0, 0.0);
        let ground = atm.single_scatter(Vec3::new(h.x, 0.0, 0.01), sun_local).map(|v| v * SKY_SCALE * 20.0 * sun_intensity.max(0.0) * 0.3);
        Nishita { sun, sun_rotation, table, sun_disc, sun_cos: half.cos(), sun_radiance, ground }
    }

    pub fn eval(&self, d: Vec3) -> [f32; 3] {
        let d = if d.length_squared() > 0.0 { d.normalize() } else { Vec3::Z };
        let mut c = if d.z < 0.0 {
            // Below the horizon: fade to the ground colour.
            self.ground
        } else {
            let e = d.z.clamp(0.0, 1.0).asin();
            // Azimuth relative to the sun.
            let az = d.y.atan2(d.x);
            let sun_az = self.sun.y.atan2(self.sun.x);
            let mut a = (az - sun_az).rem_euclid(TAU);
            if a > PI {
                a = TAU - a;
            }
            let _ = self.sun_rotation;
            let x = (e / FRAC_PI_2).sqrt() * (TABLE_E - 1) as f32;
            let y = a / PI * (TABLE_A - 1) as f32;
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(TABLE_E - 1), (y0 + 1).min(TABLE_A - 1));
            let (fx, fy) = (x - x0 as f32, y - y0 as f32);
            let t = |e: usize, a: usize| self.table[e * TABLE_A + a];
            core::array::from_fn(|k| (t(x0, y0)[k] * (1.0 - fy) + t(x0, y1)[k] * fy) * (1.0 - fx) + (t(x1, y0)[k] * (1.0 - fy) + t(x1, y1)[k] * fy) * fx)
        };
        if self.sun_disc && d.dot(self.sun) >= self.sun_cos {
            for k in 0..3 {
                c[k] += self.sun_radiance[k];
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preetham_is_blue_and_bright_near_sun() {
        let s = Preetham::new(Vec3::new(0.0, 1.0, 1.0), 2.2);
        let zen = s.eval(Vec3::Z);
        assert!(zen[2] > zen[0], "{zen:?}");
        let near = s.eval(Vec3::new(0.0, 1.0, 1.05).normalize());
        let far = s.eval(Vec3::new(0.0, -1.0, 0.3).normalize());
        assert!(near[1] > far[1]);
    }

    #[test]
    fn nishita_day_sky() {
        let s = Nishita::new(0.8, 0.0, 0.0095, 1.0, true, 1.0, 1.0, 1.0, 1.0);
        let zen = s.eval(Vec3::Z);
        assert!(zen[2] > zen[0] && zen[2] > 0.0, "{zen:?}");
        let sun = s.eval(s.sun);
        assert!(sun[0] > 100.0 * zen[0]);
    }
}
