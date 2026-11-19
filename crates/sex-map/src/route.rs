// SPDX-License-Identifier: GPL-3.0-only
//! Routes through an OSRM-compatible API (`/route/v1/{profile}/...`), and the encoded
//! polyline format its geometry comes in.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// (latitude, longitude) along the way.
    pub points: Vec<(f64, f64)>,
    /// Metres and seconds.
    pub distance: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error { NotJson, NoRoute(String), Geometry }

pub fn url(base: &str, profile: &str, from: (f64, f64), to: (f64, f64)) -> String {
    format!("{}/route/v1/{}/{:.6},{:.6};{:.6},{:.6}?overview=full&geometries=polyline", base.trim_end_matches('/'), profile, from.1, from.0, to.1, to.0)
}

/// Decodes an encoded polyline (precision 5 is OSRM's default).
pub fn decode_polyline(s: &str, precision: u32) -> Result<Vec<(f64, f64)>, Error> {
    let factor = libm::pow(10.0, precision as f64);
    let b = s.as_bytes();
    let (mut i, mut lat, mut lon) = (0usize, 0i64, 0i64);
    let mut out = Vec::new();
    let next = |i: &mut usize| -> Result<i64, Error> {
        let (mut result, mut shift) = (0i64, 0u32);
        loop {
            let c = *b.get(*i).ok_or(Error::Geometry)? as i64 - 63;
            *i += 1;
            if !(0..64).contains(&c) || shift > 60 { return Err(Error::Geometry); }
            result |= (c & 0x1F) << shift;
            shift += 5;
            if c < 0x20 { break; }
        }
        Ok(if result & 1 != 0 { !(result >> 1) } else { result >> 1 })
    };
    while i < b.len() {
        lat += next(&mut i)?;
        lon += next(&mut i)?;
        out.push((lat as f64 / factor, lon as f64 / factor));
    }
    Ok(out)
}

/// Encodes points as a polyline (for tests and fixtures).
pub fn encode_polyline(points: &[(f64, f64)], precision: u32) -> String {
    let factor = libm::pow(10.0, precision as f64);
    let mut out = String::new();
    let (mut plat, mut plon) = (0i64, 0i64);
    let put = |v: i64, out: &mut String| {
        let mut v = if v < 0 { !(v << 1) } else { v << 1 };
        while v >= 0x20 { out.push((((v & 0x1F) | 0x20) + 63) as u8 as char); v >>= 5; }
        out.push((v + 63) as u8 as char);
    };
    for p in points {
        let (lat, lon) = (libm::round(p.0 * factor) as i64, libm::round(p.1 * factor) as i64);
        put(lat - plat, &mut out);
        put(lon - plon, &mut out);
        plat = lat;
        plon = lon;
    }
    out
}

/// The first route of an answer.
pub fn parse(body: &[u8]) -> Result<Route, Error> {
    let v: Value = serde_json::from_slice(body).map_err(|_| Error::NotJson)?;
    let code = v.get("code").and_then(Value::as_str).unwrap_or("");
    if code != "Ok" {
        let message = v.get("message").and_then(Value::as_str).unwrap_or(code);
        return Err(Error::NoRoute(message.into()));
    }
    let r = v.get("routes").and_then(|r| r.get(0)).ok_or(Error::NoRoute("no routes".into()))?;
    let points = match r.get("geometry") {
        Some(Value::String(s)) => decode_polyline(s, 5)?,
        Some(g) => g.get("coordinates").and_then(Value::as_array).ok_or(Error::Geometry)?.iter()
            .map(|c| Some((c.get(1)?.as_f64()?, c.get(0)?.as_f64()?))).collect::<Option<Vec<_>>>().ok_or(Error::Geometry)?,
        None => return Err(Error::Geometry),
    };
    if points.len() < 2 { return Err(Error::Geometry); }
    Ok(Route { points, distance: r.get("distance").and_then(Value::as_f64).unwrap_or(0.0), duration: r.get("duration").and_then(Value::as_f64).unwrap_or(0.0) })
}
