// SPDX-License-Identifier: GPL-3.0-only
//! The kitty graphics protocol: `ESC _ G <control> ; <base64 payload> ESC \`.
//!
//! `vte` 0.15 swallows APC strings without reporting them, so [`ApcScan`] lifts them
//! out of the byte stream before `vte` sees it, holding state across `feed` calls.
//!
//! Understood: transmit (`a=t`), transmit and show (`a=T`), show (`a=p`), query (`a=q`)
//! and delete (`a=d`); formats `f=24`/`f=32` raw and `f=100` PNG, `o=z` zlib, chunks
//! (`m=1`), direct data only (`t=d`: the program and the terminal share no files or
//! shared memory here, so `t=f/t/s` are refused); placement by cells (`c`, `r`), source
//! rectangle (`x y w h`), cell offset (`X Y`), `C=1` and `z`. Replies go through the
//! screen's replies queue unless `q` silences them. Animation (`a=f/a/c`) and Unicode
//! placeholders are not supported and answered with `EINVAL`.

use crate::images::{Base64, CursorMove, Image, PlaceSpec, Placement};
use crate::Screen;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Largest APC string kept; longer ones are dropped (kitty sends 4096-byte chunks).
const MAX_APC: usize = 16 * 1024 * 1024;
/// Largest transmission, decoded (and inflated), in bytes.
const MAX_DATA: usize = crate::images::MEMORY_CAP;

// ---- Lifting APC strings out of the stream ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Scan { #[default] Ground, GroundEsc, Apc, ApcEsc }

/// Finds `ESC _ ... ESC \` across feed boundaries.
#[derive(Default)]
pub struct ApcScan { state: Scan, buf: Vec<u8>, overflow: bool }

/// What to do with the next part of the input.
pub enum Step { Vt(usize), Esc, Apc, Nothing }

impl ApcScan {
    /// Looks at the start of `bytes`; returns how many bytes it took and what they are:
    /// `Vt(n)`, the first `n` bytes go to `vte`; `Esc`, a held ESC goes to `vte` first;
    /// `Apc`, a complete APC string is in [`ApcScan::take`].
    pub fn step(&mut self, bytes: &[u8]) -> (usize, Step) {
        match self.state {
            Scan::Ground => match bytes.iter().position(|&b| b == 0x1b) {
                None => (bytes.len(), Step::Vt(bytes.len())),
                Some(i) if i > 0 => (i, Step::Vt(i)),
                Some(_) => match bytes.get(1) {
                    None => { self.state = Scan::GroundEsc; (1, Step::Nothing) }
                    Some(b'_') => { self.state = Scan::Apc; self.buf.clear(); self.overflow = false; (2, Step::Nothing) }
                    Some(_) => {
                        let n = bytes[2..].iter().position(|&b| b == 0x1b).map_or(bytes.len(), |j| j + 2);
                        (n, Step::Vt(n))
                    }
                },
            },
            Scan::GroundEsc => {
                if bytes[0] == b'_' { self.state = Scan::Apc; self.buf.clear(); self.overflow = false; (1, Step::Nothing) }
                else { self.state = Scan::Ground; (0, Step::Esc) }
            }
            Scan::Apc => {
                let (n, end) = match bytes.iter().position(|&b| b == 0x1b) { Some(i) => (i, true), None => (bytes.len(), false) };
                if self.buf.len() + n > MAX_APC { self.overflow = true; } else { self.buf.extend_from_slice(&bytes[..n]); }
                if end { self.state = Scan::ApcEsc; (n + 1, Step::Nothing) } else { (n, Step::Nothing) }
            }
            Scan::ApcEsc => {
                if bytes[0] == b'\\' {
                    self.state = Scan::Ground;
                    if self.overflow { self.buf.clear(); (1, Step::Nothing) } else { (1, Step::Apc) }
                } else {
                    // Any other escape cancels the string, as in `vte`; the ESC begins anew.
                    self.state = Scan::Ground;
                    self.buf.clear();
                    (0, Step::Esc)
                }
            }
        }
    }
    pub fn take(&mut self) -> Vec<u8> { core::mem::take(&mut self.buf) }
}

// ---- The protocol ----

#[derive(Clone, Copy, Debug)]
struct Control {
    a: u8, t: u8, f: u32, o: u8, i: u32, n: u32, p: u32, m: u32, q: u32, s: u32, v: u32,
    c: u32, r: u32, x: u32, y: u32, w: u32, h: u32, ox: u32, oy: u32, no_move: bool, z: i32, d: u8,
    /// `S` (size) and `O` (offset) into the data.
    size: u32, offset: u32,
}
impl Default for Control {
    fn default() -> Self {
        Control { a: b't', t: b'd', f: 32, o: 0, i: 0, n: 0, p: 0, m: 0, q: 0, s: 0, v: 0, c: 0, r: 0, x: 0, y: 0, w: 0, h: 0,
            ox: 0, oy: 0, no_move: false, z: 0, d: b'a', size: 0, offset: 0 }
    }
}

fn parse(control: &[u8]) -> Control {
    let mut k = Control::default();
    for kv in control.split(|&b| b == b',') {
        let [key, b'=', value @ ..] = kv else { continue };
        let num = || -> u32 { core::str::from_utf8(value).ok().and_then(|s| s.parse().ok()).unwrap_or(0) };
        let ch = value.first().copied().unwrap_or(0);
        match key {
            b'a' => k.a = ch, b't' => k.t = ch, b'f' => k.f = num(), b'o' => k.o = ch, b'i' => k.i = num(), b'I' => k.n = num(),
            b'p' => k.p = num(), b'm' => k.m = num(), b'q' => k.q = num(), b's' => k.s = num(), b'v' => k.v = num(),
            b'c' => k.c = num(), b'r' => k.r = num(), b'x' => k.x = num(), b'y' => k.y = num(), b'w' => k.w = num(),
            b'h' => k.h = num(), b'X' => k.ox = num(), b'Y' => k.oy = num(), b'C' => k.no_move = num() == 1,
            b'z' => k.z = core::str::from_utf8(value).ok().and_then(|s| s.parse().ok()).unwrap_or(0),
            b'd' => k.d = ch, b'S' => k.size = num(), b'O' => k.offset = num(),
            _ => {}
        }
    }
    k
}

/// A chunked transmission in progress.
pub struct Pending { ctrl: Control, data: Vec<u8>, b64: Base64, bad: Option<&'static str> }

impl Screen {
    /// One APC string from the stream.
    pub(crate) fn kitty_apc(&mut self, apc: &[u8]) {
        let Some(rest) = apc.strip_prefix(b"G") else { return };
        let (control, payload) = match rest.iter().position(|&b| b == b';') { Some(i) => (&rest[..i], &rest[i + 1..]), None => (rest, &[][..]) };
        if let Some(mut pending) = self.images.kitty.take() {
            // A later chunk: only `m` (and `q`) count.
            let k = parse(control);
            pending.add(payload);
            if k.q != 0 { pending.ctrl.q = k.q; }
            if k.m == 1 { self.images.kitty = Some(pending); } else { self.kitty_transmitted(pending); }
            return;
        }
        let k = parse(control);
        match k.a {
            b't' | b'T' | b'q' => {
                let mut pending = Pending { ctrl: k, data: Vec::new(), b64: Base64::default(), bad: None };
                if k.t != b'd' { pending.bad = Some("EINVAL:only direct (t=d) transmission is supported"); }
                pending.add(payload);
                if k.m == 1 { self.images.kitty = Some(pending); } else { self.kitty_transmitted(pending); }
            }
            b'p' => {
                let key = if k.i != 0 { self.images.key_of(k.i) } else { self.images.key_of_number(k.n) };
                match key {
                    Some(key) => { let id = self.images.get(key).map_or(0, |i| i.id); self.kitty_place(key, id, &k); self.kitty_reply(&k, id, None); }
                    None => self.kitty_reply(&k, k.i, Some("ENOENT:no such image")),
                }
            }
            b'd' => self.kitty_delete(&k),
            _ => self.kitty_reply(&k, k.i, Some("EINVAL:unsupported action")),
        }
    }

    fn kitty_transmitted(&mut self, mut pending: Pending) {
        pending.b64.finish(&mut pending.data);
        let k = pending.ctrl;
        if let Some(e) = pending.bad { self.kitty_reply(&k, k.i, Some(e)); return; }
        let image = match decode(&k, pending.data) {
            Ok(img) => img,
            Err(e) => { self.kitty_reply(&k, k.i, Some(e)); return; }
        };
        if k.a == b'q' { self.kitty_reply(&k, k.i, None); return; }
        // An image sent with only a number gets a fresh id, which the reply names.
        let id = if k.i == 0 && k.n != 0 { self.images.fresh_id() } else { k.i };
        let Some(key) = self.add_image(Image { id, number: k.n, ..image }) else {
            self.kitty_reply(&k, id, Some("EFBIG:image too large")); return;
        };
        if k.a == b'T' { self.kitty_place(key, id, &k); }
        self.kitty_reply(&k, id, None);
    }

    fn kitty_place(&mut self, key: u64, id: u32, k: &Control) {
        let spec = PlaceSpec {
            image_id: id, placement_id: k.p, src: (k.x as usize, k.y as usize, k.w as usize, k.h as usize),
            cols: (k.c != 0).then_some(k.c as usize), rows: (k.r != 0).then_some(k.r as usize),
            offset: (k.ox as usize, k.oy as usize), z: k.z,
        };
        self.place_at_cursor(key, spec, if k.no_move { CursorMove::None } else { CursorMove::Kitty });
    }

    /// `a=d`: lower case removes placements, upper case also frees the images' data
    /// when no placement shows them any more.
    fn kitty_delete(&mut self, k: &Control) {
        let free = k.d.is_ascii_uppercase();
        let alt = self.alternate();
        // The absolute line of screen row 0; a placement's screen row is `p.line - origin`.
        let origin = self.screen_line();
        let self_row = |p: &Placement, origin: i64| p.line - origin;
        let (cx, cy) = (self.x.min(self.cols - 1) as i64, self.y as i64);
        let rows = self.rows as i64;
        let ids_before: Vec<u64> = self.placements_iter().map(|p| p.key).collect();
        let (x1, y1) = (k.x as i64 - 1, k.y as i64 - 1);
        let mut keys: Vec<u64> = Vec::new();
        match k.d.to_ascii_lowercase() {
            b'a' => { self.drop_placements(|p| p.alt == alt && { let r = self_row(p, origin); r + p.rows as i64 > 0 && r < rows }); }
            b'i' => {
                if let Some(key) = self.images.key_of(k.i) { keys.push(key); }
                let (id, pid) = (k.i, k.p);
                self.drop_placements(|p| p.image_id == id && (pid == 0 || p.id == pid));
            }
            b'n' => {
                if let Some(key) = self.images.key_of_number(k.n) {
                    keys.push(key);
                    let pid = k.p;
                    self.drop_placements(|p| p.key == key && (pid == 0 || p.id == pid));
                }
            }
            b'c' => { self.drop_placements(|p| p.alt == alt && hit_at(p, self_row(p, origin), cy, cx)); }
            b'p' => { self.drop_placements(|p| p.alt == alt && hit_at(p, self_row(p, origin), y1, x1)); }
            b'q' => { let z = k.z; self.drop_placements(|p| p.alt == alt && p.z == z && hit_at(p, self_row(p, origin), y1, x1)); }
            b'x' => { self.drop_placements(|p| p.alt == alt && x1 >= p.col as i64 && x1 < (p.col + p.cols) as i64); }
            b'y' => { self.drop_placements(|p| p.alt == alt && { let r = self_row(p, origin); y1 >= r && y1 < r + p.rows as i64 }); }
            b'z' => { let z = k.z; self.drop_placements(|p| p.alt == alt && p.z == z); }
            b'r' => { let (lo, hi) = (k.x, k.y); keys.extend(self.images.keys_in_range(lo, hi)); self.drop_placements(|p| p.image_id >= lo && p.image_id <= hi); }
            _ => { self.kitty_reply(k, k.i, Some("EINVAL:unsupported delete")); return; }
        }
        if free {
            keys.extend(ids_before);
            keys.sort_unstable();
            keys.dedup();
            self.free_unplaced(&keys);
        }
    }

    fn kitty_reply(&mut self, k: &Control, id: u32, error: Option<&str>) {
        if id == 0 && k.n == 0 { return; }
        match error { None if k.q >= 1 => return, Some(_) if k.q >= 2 => return, _ => {} }
        let mut r = String::from("\x1b_G");
        if id != 0 { r.push_str(&format!("i={}", id)); }
        if k.n != 0 { if id != 0 { r.push(','); } r.push_str(&format!("I={}", k.n)); }
        if k.p != 0 { r.push_str(&format!(",p={}", k.p)); }
        r.push(';');
        r.push_str(error.unwrap_or("OK"));
        r.push_str("\x1b\\");
        self.replies.extend_from_slice(r.as_bytes());
    }
}

fn hit_at(p: &Placement, r: i64, row: i64, col: i64) -> bool {
    row >= r && row < r + p.rows as i64 && col >= p.col as i64 && col < (p.col + p.cols) as i64
}

impl Pending {
    fn add(&mut self, payload: &[u8]) {
        if self.bad.is_some() { return; }
        if self.data.len() + payload.len() / 4 * 3 > MAX_DATA { self.bad = Some("EFBIG:transmission too large"); self.data = Vec::new(); return; }
        if !self.b64.decode(payload, &mut self.data) { self.bad = Some("EINVAL:bad base64"); self.data = Vec::new(); }
    }
}

/// Turns transmitted bytes into pixels.
fn decode(k: &Control, data: Vec<u8>) -> Result<Image, &'static str> {
    let data = match k.o {
        0 => data,
        b'z' => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, MAX_DATA).map_err(|_| "EINVAL:zlib data could not be inflated")?,
        _ => return Err("EINVAL:unknown compression"),
    };
    // `S` and `O` pick a part of the data.
    let data = if k.offset != 0 || k.size != 0 {
        let from = (k.offset as usize).min(data.len());
        let to = if k.size == 0 { data.len() } else { (from + k.size as usize).min(data.len()) };
        data[from..to].to_vec()
    } else { data };
    let (w, h) = (k.s as usize, k.v as usize);
    let pixels = match k.f {
        24 | 32 => {
            let bpp = if k.f == 24 { 3 } else { 4 };
            if w == 0 || h == 0 { return Err("EINVAL:s and v are required for raw pixels"); }
            if w > 10_000 || h > 10_000 || w * h > crate::png::MAX_PIXELS { return Err("EFBIG:image too large"); }
            if data.len() < w * h * bpp { return Err("ENODATA:insufficient image data"); }
            data.chunks_exact(bpp).take(w * h).map(|c| {
                let a = if bpp == 4 { c[3] as u32 } else { 255 };
                a << 24 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
            }).collect()
        }
        100 => {
            let (pw, ph, px) = crate::png::decode(&data).map_err(|e| match e {
                crate::png::PngError::TooLarge => "EFBIG:image too large",
                _ => "EBADPNG:the PNG could not be decoded",
            })?;
            return Ok(Image { id: 0, number: 0, width: pw, height: ph, pixels: px });
        }
        _ => return Err("EINVAL:unknown format"),
    };
    Ok(Image { id: 0, number: 0, width: w, height: h, pixels })
}
