// SPDX-License-Identifier: GPL-3.0-only
//! Place search through a Nominatim-compatible JSON API (`/search?format=jsonv2`).
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// What it is: `amenity/fountain`, `railway/station`, ...
    pub kind: String,
}

impl Place {
    /// The first part of its display name: "Fountain Square" of
    /// "Fountain Square, Old Town, Sometown".
    pub fn short(&self) -> &str { self.name.split(',').next().unwrap_or(&self.name).trim() }
}

/// Percent-encodes a query string value.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn url(base: &str, query: &str, limit: u8) -> String {
    format!("{}/search?q={}&format=jsonv2&limit={}", base.trim_end_matches('/'), encode(query.trim()), limit.clamp(1, 40))
}

fn number(v: &Value) -> Option<f64> {
    match v { Value::String(s) => s.trim().parse().ok(), Value::Number(n) => n.as_f64(), _ => None }
}

/// The places in an answer, in its order. Entries without coordinates are skipped.
pub fn parse(body: &[u8]) -> Result<Vec<Place>, ()> {
    let v: Value = serde_json::from_slice(body).map_err(|_| ())?;
    let list = v.as_array().ok_or(())?;
    Ok(list.iter().filter_map(|e| {
        let lat = number(e.get("lat")?)?;
        let lon = number(e.get("lon")?)?;
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) { return None; }
        let name = e.get("display_name").and_then(Value::as_str).or_else(|| e.get("name").and_then(Value::as_str)).unwrap_or("?").to_string();
        let cat = e.get("category").or_else(|| e.get("class")).and_then(Value::as_str).unwrap_or("");
        let ty = e.get("type").and_then(Value::as_str).unwrap_or("");
        Some(Place { name, lat, lon, kind: format!("{cat}/{ty}") })
    }).collect())
}
