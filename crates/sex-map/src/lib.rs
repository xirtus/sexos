// SPDX-License-Identifier: GPL-3.0-only
//! Maps for SexOS (Rendezvous, apps/rendezvous): OpenStreetMap vector tiles
//! (Mapbox Vector Tile protobuf, read by the small reader in [`pbf`]) drawn with
//! tiny-skia, raster PNG tiles as a fallback, place search through a
//! Nominatim-compatible JSON API, routes through an OSRM-compatible API, and offline
//! regions kept as tile files in the program's folder.
//!
//! * [`geo`]: Web Mercator: degrees to tile and pixel coordinates and back.
//! * [`View`]: the visible map: centre, zoom, size; pan, zoom about a point, which
//!   tiles cover it.
//! * [`mvt`], [`render`]: decode and draw a tile (water, parks, buildings, roads,
//!   labels) into a pixmap.
//! * [`search`], [`route`]: request URLs and answers of the two web APIs, and the
//!   encoded polyline format.
//! * [`http`]: URLs and HTTP/1.1 responses for the app's plain socket client.
//! * [`offline`], [`Servers`]: tile files and regions on disk; configurable servers.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub mod pbf;
pub mod mvt;
pub mod geo;
pub mod render;
pub mod search;
pub mod route;
pub mod http;
pub mod offline;

/// One tile's address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId { pub z: u8, pub x: u32, pub y: u32 }

impl core::fmt::Display for TileId {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { write!(f, "{}/{}/{}", self.z, self.x, self.y) }
}

/// Inflates a gzip or zlib tile body; other bytes come back unchanged.
pub fn inflate(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    use zune_inflate::{DeflateDecoder, DeflateOptions};
    let opts = DeflateOptions::default().set_limit(32 << 20).set_size_hint(bytes.len() * 4);
    if bytes.starts_with(&[0x1F, 0x8B]) { return DeflateDecoder::new_with_options(bytes, opts).decode_gzip().map_err(|_| ()); }
    if bytes.len() > 2 && bytes[0] & 0x0F == 8 && (u16::from(bytes[0]) << 8 | u16::from(bytes[1])) % 31 == 0 {
        if let Ok(v) = DeflateDecoder::new_with_options(bytes, opts).decode_zlib() { return Ok(v); }
    }
    Ok(bytes.to_vec())
}

/// The visible map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Centre, in world pixels at zoom 0 (0..256).
    pub cx: f64,
    pub cy: f64,
    /// Fractional zoom.
    pub zoom: f64,
    pub width: u32,
    pub height: u32,
}

pub const MIN_ZOOM: f64 = 1.0;
pub const MAX_ZOOM: f64 = 19.0;

impl View {
    pub fn new(lat: f64, lon: f64, zoom: f64, width: u32, height: u32) -> View {
        let (cx, cy) = geo::world(lat, lon);
        View { cx, cy, zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM), width, height }
    }
    pub fn scale(&self) -> f64 { geo::pow2(self.zoom) }
    pub fn center(&self) -> (f64, f64) { geo::lat_lon(self.cx, self.cy) }
    /// A place's position in the window (pixels from its top left).
    pub fn to_screen(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (wx, wy) = geo::world(lat, lon);
        self.world_to_screen(wx, wy)
    }
    pub fn world_to_screen(&self, wx: f64, wy: f64) -> (f64, f64) {
        let s = self.scale();
        ((wx - self.cx) * s + self.width as f64 / 2.0, (wy - self.cy) * s + self.height as f64 / 2.0)
    }
    pub fn to_lat_lon(&self, x: f64, y: f64) -> (f64, f64) {
        let s = self.scale();
        geo::lat_lon(self.cx + (x - self.width as f64 / 2.0) / s, self.cy + (y - self.height as f64 / 2.0) / s)
    }
    /// Moves the map by a drag of (`dx`, `dy`) pixels.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let s = self.scale();
        let x = libm::fmod(self.cx - dx / s, 256.0);
        self.cx = if x < 0.0 { x + 256.0 } else { x };
        self.cy = (self.cy - dy / s).clamp(0.0, 256.0);
    }
    /// Zooms by `by` levels keeping the point under (`x`, `y`) where it is.
    pub fn zoom_at(&mut self, by: f64, x: f64, y: f64) {
        let before = self.to_lat_lon(x, y);
        self.zoom = (self.zoom + by).clamp(MIN_ZOOM, MAX_ZOOM);
        let (wx, wy) = geo::world(before.0, before.1);
        let s = self.scale();
        self.cx = wx - (x - self.width as f64 / 2.0) / s;
        self.cy = wy - (y - self.height as f64 / 2.0) / s;
    }
    /// The tile zoom used: the view's, at most `max` (deeper views stretch tiles).
    pub fn tile_zoom(&self, max: u8) -> u8 { (geo::floor(self.zoom + 0.25) as i64).clamp(0, max as i64) as u8 }
    /// The tiles covering the view at tile zoom `z`, nearest the centre first, with
    /// each tile's top-left position in the window and its size in pixels.
    pub fn tiles(&self, z: u8) -> Vec<(TileId, f64, f64, f64)> {
        let n = 1u64 << z;
        let size = 256.0 * self.scale() / n as f64;
        let (x0, y0) = self.world_to_screen(0.0, 0.0);
        let first_x = geo::floor(-x0 / size) as i64;
        let first_y = geo::floor(-y0 / size) as i64;
        let last_x = geo::floor((self.width as f64 - x0) / size) as i64;
        let last_y = geo::floor((self.height as f64 - y0) / size) as i64;
        let mut out = Vec::new();
        for ty in first_y.max(0)..=last_y.min(n as i64 - 1) {
            for tx in first_x..=last_x {
                let wrapped = tx.rem_euclid(n as i64) as u32;
                out.push((TileId { z, x: wrapped, y: ty as u32 }, x0 + tx as f64 * size, y0 + ty as f64 * size, size));
            }
        }
        let (mx, my) = (self.width as f64 / 2.0, self.height as f64 / 2.0);
        out.sort_by(|a, b| {
            let d = |t: &(TileId, f64, f64, f64)| { let (x, y) = (t.1 + t.3 / 2.0 - mx, t.2 + t.3 / 2.0 - my); x * x + y * y };
            d(a).partial_cmp(&d(b)).unwrap_or(core::cmp::Ordering::Equal)
        });
        out
    }
}

/// Where tiles, search and routes come from. Each URL may hold `{z}`, `{x}`, `{y}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Servers {
    pub vector: String,
    pub raster: String,
    pub search: String,
    pub route: String,
    /// The deepest zoom the tile servers have.
    pub max_zoom: u8,
}

impl Servers {
    /// Everything from one base URL laid out as the gate's fixture server (and a
    /// self-hosted tile server) lays it out.
    pub fn at(base: &str) -> Servers {
        let b = base.trim_end_matches('/');
        Servers { vector: format!("{b}/tiles/{{z}}/{{x}}/{{y}}.mvt"), raster: format!("{b}/raster/{{z}}/{{x}}/{{y}}.png"), search: b.to_string(), route: b.to_string(), max_zoom: 14 }
    }
    /// `key=value` lines: vector, raster, search, route, max_zoom; `base=` sets all.
    pub fn parse(text: &str) -> Servers {
        let mut s = Servers::at("http://127.0.0.1:8080");
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "base" => { let m = s.max_zoom; s = Servers::at(v); s.max_zoom = m; }
                "vector" => s.vector = v.into(),
                "raster" => s.raster = v.into(),
                "search" => s.search = v.trim_end_matches('/').into(),
                "route" => s.route = v.trim_end_matches('/').into(),
                "max_zoom" => if let Ok(z) = v.parse::<u8>() { s.max_zoom = z.min(22) },
                _ => {}
            }
        }
        s
    }
    pub fn serialize(&self) -> String {
        format!("vector={}\nraster={}\nsearch={}\nroute={}\nmax_zoom={}\n", self.vector, self.raster, self.search, self.route, self.max_zoom)
    }
    pub fn tile_url(template: &str, t: TileId) -> String {
        template.replace("{z}", &t.z.to_string()).replace("{x}", &t.x.to_string()).replace("{y}", &t.y.to_string())
    }
}
