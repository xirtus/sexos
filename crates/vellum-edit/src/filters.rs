//! Stream decoding: enough to read cross-reference and object streams.

use crate::cos::{Dict, Object, Stream};
use crate::Error;

/// Decodes a stream's data through its `/Filter` chain.
pub fn decode(stream: &Stream) -> Result<Vec<u8>, Error> {
    let filters: Vec<Vec<u8>> = match stream.dict.get(b"Filter") {
        None => return Ok(stream.data.clone()),
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a.iter().filter_map(|o| o.as_name().map(<[u8]>::to_vec)).collect(),
        Some(_) => return Err(Error::Malformed("bad /Filter")),
    };
    let parms: Vec<Option<&Dict>> = match stream.dict.get(b"DecodeParms") {
        Some(Object::Dict(d)) => vec![Some(d)],
        Some(Object::Array(a)) => a.iter().map(|o| o.as_dict()).collect(),
        _ => vec![],
    };
    let mut data = stream.data.clone();
    for (i, f) in filters.iter().enumerate() {
        let parm = parms.get(i).copied().flatten();
        data = match f.as_slice() {
            b"FlateDecode" | b"Fl" => predict(inflate(&data)?, parm)?,
            b"ASCIIHexDecode" | b"AHx" => ascii_hex(&data),
            b"ASCII85Decode" | b"A85" => ascii85(&data)?,
            _ => return Err(Error::Unsupported("stream filter")),
        };
    }
    Ok(data)
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    use miniz_oxide::inflate::{decompress_to_vec_zlib, decompress_to_vec};
    decompress_to_vec_zlib(data)
        .or_else(|e| if e.output.is_empty() { decompress_to_vec(data).map_err(|_| ()) } else { Ok(e.output) })
        .map_err(|_| Error::Malformed("bad deflate data"))
}


fn predict(data: Vec<u8>, parm: Option<&Dict>) -> Result<Vec<u8>, Error> {
    let Some(p) = parm else { return Ok(data) };
    let get = |k: &[u8], d: i64| p.get(k).and_then(Object::as_int).unwrap_or(d);
    let predictor = get(b"Predictor", 1);
    if predictor < 10 {
        if predictor == 2 { return Err(Error::Unsupported("TIFF predictor")); }
        return Ok(data);
    }
    let colors = get(b"Colors", 1).max(1) as usize;
    let bpc = get(b"BitsPerComponent", 8).max(1) as usize;
    let columns = get(b"Columns", 1).max(1) as usize;
    let bpp = (colors * bpc).div_ceil(8);
    let row = (columns * colors * bpc).div_ceil(8);
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        if chunk.len() < 2 { break; }
        let kind = chunk[0];
        let mut cur = chunk[1..].to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] } else { 0 } as i16;
            let b = prev[i] as i16;
            let c = if i >= bpp { prev[i - bpp] } else { 0 } as i16;
            let add = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                }
                _ => return Err(Error::Malformed("bad PNG predictor row")),
            };
            cur[i] = cur[i].wrapping_add(add as u8);
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Ok(out)
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut hi = None;
    for &b in data {
        if b == b'>' { break; }
        let v = match b { b'0'..=b'9' => b - b'0', b'a'..=b'f' => b - b'a' + 10, b'A'..=b'F' => b - b'A' + 10, _ => continue };
        match hi.take() { Some(h) => out.push(h << 4 | v), None => hi = Some(v) }
    }
    if let Some(h) = hi { out.push(h << 4); }
    out
}

fn ascii85(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut group = [0u32; 5];
    let mut n = 0;
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &b in body {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = (b - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |acc, &d| acc * 85 + d as u64);
                    out.extend_from_slice(&(v as u32).to_be_bytes());
                    n = 0;
                }
            }
            _ if crate::parse::is_white(b) => {}
            _ => return Err(Error::Malformed("bad ASCII85 data")),
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) { *g = 84; }
        let v = group.iter().fold(0u64, |acc, &d| acc * 85 + d as u64);
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    Ok(out)
}
