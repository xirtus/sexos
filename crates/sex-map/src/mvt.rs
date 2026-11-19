// SPDX-License-Identifier: GPL-3.0-only
//! Mapbox Vector Tiles (specification 2.1): layers of features with tags and
//! command-encoded geometry, in tile coordinates (0..extent, y down).
use crate::pbf::{packed, zigzag, Error as PbfError, Field, Reader};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum Value { Str(String), Float(f64), Int(i64), Bool(bool) }

impl Value {
    pub fn as_str(&self) -> Option<&str> { if let Value::Str(s) = self { Some(s) } else { None } }
    pub fn as_f64(&self) -> Option<f64> { match self { Value::Float(f) => Some(*f), Value::Int(i) => Some(*i as f64), _ => None } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeomType { Unknown, Point, Line, Polygon }

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    pub id: Option<u64>,
    pub kind: GeomType,
    pub tags: Vec<(String, Value)>,
    /// Points: one part per point. Lines: one part per line. Polygons: one part per
    /// ring, closed (first point repeated at the end).
    pub parts: Vec<Vec<(i32, i32)>>,
}

impl Feature {
    pub fn get(&self, key: &str) -> Option<&Value> { self.tags.iter().find(|t| t.0 == key).map(|t| &t.1) }
    pub fn str(&self, key: &str) -> Option<&str> { self.get(key).and_then(Value::as_str) }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tile { pub layers: Vec<Layer> }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error { Pbf(PbfError), Geometry, BadTag, BadUtf8 }
impl From<PbfError> for Error { fn from(e: PbfError) -> Self { Error::Pbf(e) } }

impl Tile {
    pub fn layer(&self, name: &str) -> Option<&Layer> { self.layers.iter().find(|l| l.name == name) }
    pub fn features(&self) -> usize { self.layers.iter().map(|l| l.features.len()).sum() }
}

fn utf8(b: &[u8]) -> Result<String, Error> { core::str::from_utf8(b).map(|s| s.to_string()).map_err(|_| Error::BadUtf8) }

/// Decodes a tile. Gzip-compressed tiles must be inflated first (see [`crate::inflate`]).
pub fn decode(bytes: &[u8]) -> Result<Tile, Error> {
    let mut r = Reader::new(bytes);
    let mut tile = Tile::default();
    while let Some(f) = r.next_field()? {
        if let Field::Bytes(3, b) = f { tile.layers.push(layer(b)?); }
    }
    Ok(tile)
}

fn value(bytes: &[u8]) -> Result<Value, Error> {
    let mut r = Reader::new(bytes);
    let mut v = Value::Bool(false);
    while let Some(f) = r.next_field()? {
        v = match f {
            Field::Bytes(1, s) => Value::Str(utf8(s)?),
            Field::Fixed32(2, x) => Value::Float(f32::from_bits(x) as f64),
            Field::Fixed64(3, x) => Value::Float(f64::from_bits(x)),
            Field::Varint(4, x) | Field::Varint(5, x) => Value::Int(x as i64),
            Field::Varint(6, x) => Value::Int(zigzag(x)),
            Field::Varint(7, x) => Value::Bool(x != 0),
            _ => continue,
        };
    }
    Ok(v)
}

fn layer(bytes: &[u8]) -> Result<Layer, Error> {
    let mut r = Reader::new(bytes);
    let (mut name, mut extent) = (String::new(), 4096u32);
    let (mut keys, mut values, mut raw) = (Vec::new(), Vec::new(), Vec::new());
    while let Some(f) = r.next_field()? {
        match f {
            Field::Bytes(1, s) => name = utf8(s)?,
            Field::Bytes(2, b) => raw.push(b),
            Field::Bytes(3, s) => keys.push(utf8(s)?),
            Field::Bytes(4, b) => values.push(value(b)?),
            Field::Varint(5, e) => extent = (e as u32).max(1),
            _ => {}
        }
    }
    let mut features = Vec::with_capacity(raw.len());
    for b in raw {
        let mut r = Reader::new(b);
        let (mut id, mut kind, mut tags, mut geom) = (None, GeomType::Unknown, Vec::new(), Vec::new());
        while let Some(f) = r.next_field()? {
            match f {
                Field::Varint(1, v) => id = Some(v),
                Field::Bytes(2, b) => tags = packed(b)?,
                Field::Varint(3, t) => kind = match t { 1 => GeomType::Point, 2 => GeomType::Line, 3 => GeomType::Polygon, _ => GeomType::Unknown },
                Field::Bytes(4, b) => geom = packed(b)?,
                _ => {}
            }
        }
        if tags.len() % 2 != 0 { return Err(Error::BadTag); }
        let tags = tags.chunks(2).map(|kv| {
            let k = keys.get(kv[0] as usize).ok_or(Error::BadTag)?;
            let v = values.get(kv[1] as usize).ok_or(Error::BadTag)?;
            Ok((k.clone(), v.clone()))
        }).collect::<Result<Vec<_>, Error>>()?;
        features.push(Feature { id, kind, tags, parts: geometry(&geom, kind)? });
    }
    Ok(Layer { name, extent, features })
}

/// Runs the geometry commands: MoveTo (1), LineTo (2), ClosePath (7).
pub fn geometry(cmds: &[u64], _kind: GeomType) -> Result<Vec<Vec<(i32, i32)>>, Error> {
    let (mut x, mut y) = (0i32, 0i32);
    let mut parts: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut i = 0;
    while i < cmds.len() {
        let c = cmds[i];
        i += 1;
        let (id, count) = ((c & 7) as u8, (c >> 3) as usize);
        match id {
            1 | 2 => {
                if i + 2 * count > cmds.len() { return Err(Error::Geometry); }
                for k in 0..count {
                    x = x.wrapping_add(zigzag(cmds[i + 2 * k]) as i32);
                    y = y.wrapping_add(zigzag(cmds[i + 2 * k + 1]) as i32);
                    // Each MoveTo starts a part (a point of a multipoint, a line, a ring).
                    if id == 1 { parts.push(Vec::new()); }
                    parts.last_mut().ok_or(Error::Geometry)?.push((x, y));
                }
                i += 2 * count;
            }
            7 => {
                let ring = parts.last_mut().ok_or(Error::Geometry)?;
                if let Some(&first) = ring.first() { ring.push(first); }
            }
            _ => return Err(Error::Geometry),
        }
    }
    Ok(parts)
}

/// Twice the signed area of a ring (positive is the specification's exterior ring).
pub fn ring_area(ring: &[(i32, i32)]) -> i64 {
    ring.windows(2).map(|w| w[0].0 as i64 * w[1].1 as i64 - w[1].0 as i64 * w[0].1 as i64).sum()
}
