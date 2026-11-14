// SPDX-License-Identifier: GPL-3.0-only
//! Offline regions: a named box and zoom range whose tiles are kept as files in the
//! program's folder (`offline/Z/X/Y.mvt` or `.png`), listed in `regions.txt`.
use crate::geo::tile_at;
use crate::TileId;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The most tiles one region may hold.
pub const MAX_TILES: usize = 4000;

#[derive(Debug, Clone, PartialEq)]
pub struct Region { pub name: String, pub north: f64, pub west: f64, pub south: f64, pub east: f64, pub min_zoom: u8, pub max_zoom: u8 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Vector, Raster }

impl Kind { pub fn ext(self) -> &'static str { match self { Kind::Vector => "mvt", Kind::Raster => "png" } } }

pub fn tile_path(t: TileId, kind: Kind) -> String { format!("offline/{}/{}/{}.{}", t.z, t.x, t.y, kind.ext()) }
pub fn tile_dir(t: TileId) -> String { format!("offline/{}/{}", t.z, t.x) }

impl Region {
    /// Every tile of the region, lowest zoom first; None if over [`MAX_TILES`].
    pub fn tiles(&self) -> Option<Vec<TileId>> {
        let mut out = Vec::new();
        for z in self.min_zoom..=self.max_zoom {
            let a = tile_at(self.north, self.west, z);
            let b = tile_at(self.south, self.east, z);
            let count = (b.x.abs_diff(a.x) as usize + 1) * (b.y.abs_diff(a.y) as usize + 1);
            if out.len() + count > MAX_TILES { return None; }
            for y in a.y.min(b.y)..=a.y.max(b.y) { for x in a.x.min(b.x)..=a.x.max(b.x) { out.push(TileId { z, x, y }); } }
        }
        Some(out)
    }
    pub fn contains(&self, lat: f64, lon: f64) -> bool { lat <= self.north && lat >= self.south && lon >= self.west && lon <= self.east }
}

/// `regions.txt`: one region per line, tab-separated: name, north, west, south, east,
/// min zoom, max zoom.
pub fn parse(text: &str) -> Vec<Region> {
    text.lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() != 7 { return None; }
        Some(Region { name: f[0].to_string(), north: f[1].parse().ok()?, west: f[2].parse().ok()?, south: f[3].parse().ok()?, east: f[4].parse().ok()?, min_zoom: f[5].parse().ok()?, max_zoom: f[6].parse().ok()? })
    }).collect()
}

pub fn serialize(regions: &[Region]) -> String {
    let mut s = String::new();
    for r in regions {
        s.push_str(&format!("{}\t{:.6}\t{:.6}\t{:.6}\t{:.6}\t{}\t{}\n", r.name.replace(['\t', '\n'], " "), r.north, r.west, r.south, r.east, r.min_zoom, r.max_zoom));
    }
    s
}
