// SPDX-License-Identifier: GPL-3.0-only
//! Web Mercator (EPSG:3857) as tile servers use it: the world is a 256 pixel square
//! at zoom 0, doubling each level; y grows southward.
use crate::TileId;
use core::f64::consts::PI;

pub const MAX_LAT: f64 = 85.051_128_779_806_59;

pub fn floor(x: f64) -> f64 { libm::floor(x) }
pub fn pow2(x: f64) -> f64 { libm::exp2(x) }

/// World pixels at zoom 0 for a place.
pub fn world(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-MAX_LAT, MAX_LAT) * PI / 180.0;
    let x = (lon + 180.0) / 360.0 * 256.0;
    let y = (1.0 - libm::log(libm::tan(lat) + 1.0 / libm::cos(lat)) / PI) / 2.0 * 256.0;
    (x, y)
}

/// The place at world pixels (zoom 0).
pub fn lat_lon(x: f64, y: f64) -> (f64, f64) {
    let lon = x / 256.0 * 360.0 - 180.0;
    let n = PI - 2.0 * PI * y / 256.0;
    let lat = libm::atan(libm::sinh(n)) * 180.0 / PI;
    (lat, lon)
}

/// The tile holding a place at zoom `z`.
pub fn tile_at(lat: f64, lon: f64, z: u8) -> TileId {
    let (x, y) = world(lat, lon);
    let n = (1u64 << z) as f64;
    let max = (1u32 << z) - 1;
    TileId { z, x: ((x / 256.0 * n) as i64).clamp(0, max as i64) as u32, y: ((y / 256.0 * n) as i64).clamp(0, max as i64) as u32 }
}

/// A tile's corners: (north, west, south, east) in degrees.
pub fn tile_bounds(t: TileId) -> (f64, f64, f64, f64) {
    let size = 256.0 / (1u64 << t.z) as f64;
    let (n, w) = lat_lon(t.x as f64 * size, t.y as f64 * size);
    let (s, e) = lat_lon((t.x + 1) as f64 * size, (t.y + 1) as f64 * size);
    (n, w, s, e)
}

/// Great-circle distance in metres.
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let r = PI / 180.0;
    let (la1, la2) = (a.0 * r, b.0 * r);
    let h = libm::pow(libm::sin((la2 - la1) / 2.0), 2.0) + libm::cos(la1) * libm::cos(la2) * libm::pow(libm::sin((b.1 - a.1) * r / 2.0), 2.0);
    2.0 * 6_371_008.8 * libm::asin(libm::sqrt(h.min(1.0)))
}
