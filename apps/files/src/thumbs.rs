// SPDX-License-Identifier: GPL-3.0-only
//! Thumbnails in Files: Ctrl+G shows the folder as a grid of pictures instead of a list.
//!
//! Pictures (PNG, JPEG, GIF, WebP) are made small in the background, one per piece of
//! work between keys, and kept in Files' own folder (`thumbs/`), named by path, size and
//! modification time; the folder opened again draws from there without decoding anything.
//! Other files show a tile with their kind (PDF, VIDEO, TEXT, ...).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::{self, Entry};
use sex_app::Window;
use sex_font::Font;

use crate::{at, join};

/// Largest side of a thumbnail, in pixels.
pub const SIDE: usize = 112;
const CELL_W: usize = 136;
const CELL_H: usize = SIDE + 30;
const HEAD: usize = 34;
const FOOT: usize = 40;
const DIR: &[u8] = b"thumbs";
/// Pictures larger than this are not read for a thumbnail.
const FILE_MAX: u64 = 48 * 1024 * 1024;

const BG: u32 = 0x1B1D22;
const TILE: u32 = 0x2A2D34;
const PICKED: u32 = 0x3D6FD6;
const TEXT: u32 = 0xE6E8EC;
const DIM: u32 = 0x9AA0AA;

pub enum Thumb { Picture { w: usize, h: usize, px: Vec<u32> }, Kind(&'static str) }

/// What one folder's thumbnails took.
#[derive(Default, Clone, Copy)]
pub struct Tally { pub pictures: usize, pub made: usize, pub cached: usize, pub failed: usize, pub started: u64 }

pub struct Thumbs {
    pub on: bool,
    made: BTreeMap<Vec<u8>, Thumb>,
    queue: VecDeque<(Vec<u8>, u64)>,
    pub tally: Tally,
    /// The folder whose thumbnails are being made, logged when the queue empties.
    folder: Vec<u8>,
    dir_ready: bool,
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    cover: Vec<u8>,
}

pub fn kind(name: &[u8], dir: bool) -> Option<&'static str> {
    if dir { return Some("FOLDER"); }
    let ext = match name.iter().rposition(|b| *b == b'.') { Some(i) => &name[i + 1..], None => return Some("FILE") };
    let mut e = [0u8; 5];
    if ext.len() > e.len() { return Some("FILE"); }
    for (o, b) in e.iter_mut().zip(ext) { *o = b.to_ascii_lowercase(); }
    Some(match &e[..ext.len()] {
        b"png" | b"jpg" | b"jpeg" | b"gif" | b"webp" => return None,
        b"pdf" => "PDF",
        b"mp4" | b"webm" | b"mkv" | b"mov" | b"avi" | b"m4v" => "VIDEO",
        b"mp3" | b"ogg" | b"opus" | b"flac" | b"wav" | b"m4a" => "AUDIO",
        b"txt" | b"md" | b"rs" | b"toml" | b"json" | b"csv" | b"log" | b"html" | b"css" | b"js" => "TEXT",
        b"zip" | b"tar" | b"gz" | b"xz" | b"zst" => "ARCHIVE",
        _ => "FILE",
    })
}

fn fnv(bytes: &[u8]) -> u64 { bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)) }

impl Thumbs {
    pub const fn new() -> Self {
        Thumbs { on: false, made: BTreeMap::new(), queue: VecDeque::new(), tally: Tally { pictures: 0, made: 0, cached: 0, failed: 0, started: 0 }, folder: Vec::new(),
                 dir_ready: false, font: None, acc: Vec::new(), cover: Vec::new() }
    }
    pub fn busy(&self) -> bool { self.on && !self.queue.is_empty() }
    /// Queues the pictures of a freshly listed folder that have no thumbnail yet.
    pub fn folder(&mut self, cwd: &[u8], entries: &[Entry]) {
        self.queue.clear();
        self.folder = cwd.to_vec();
        self.tally = Tally { started: sex_pdx::get_ticks(), ..Tally::default() };
        for e in entries {
            if kind(&e.name, e.dir).is_some() { continue; }
            self.tally.pictures += 1;
            let path = join(cwd, &e.name);
            if !self.made.contains_key(&path) { self.queue.push_back((path, e.size)); }
        }
        if self.on && self.queue.is_empty() && self.tally.pictures > 0 { self.report(); }
    }
    fn report(&self) {
        let t = &self.tally;
        sex_pdx::serial_record!("[files.app.thumbs] dir={} pictures={} made={} cached={} failed={} ms={}",
            crate::shown(&self.folder), t.pictures, t.made, t.cached, t.failed, sex_pdx::get_ticks().saturating_sub(t.started));
    }
    /// Makes one thumbnail. True when the grid should be drawn again.
    pub fn step(&mut self, window: &mut Window) -> bool {
        let Some((path, size)) = self.queue.pop_front() else { return false };
        let mtime = files::stat(window, &at(&path)).map(|s| s.mtime).unwrap_or(0);
        let name = alloc::format!("thumbs/{:016x}-{}-{}", fnv(&path), size, mtime);
        let thumb = match self.cached(window, name.as_bytes()) {
            Some(t) => { self.tally.cached += 1; t }
            None => match picture(window, &path, size) {
                Some((w, h, px)) => {
                    self.tally.made += 1;
                    self.keep(window, name.as_bytes(), w, h, &px);
                    Thumb::Picture { w, h, px }
                }
                None => { self.tally.failed += 1; Thumb::Kind("PICTURE?") }
            },
        };
        self.made.insert(path, thumb);
        if self.queue.is_empty() { self.report(); }
        true
    }
    fn cached(&self, window: &mut Window, name: &[u8]) -> Option<Thumb> {
        let mut raw = vec![0u8; 4 + SIDE * SIDE * 3];
        let n = files::read(window, name, &mut raw).ok()?;
        let (w, h) = (u16::from_le_bytes([raw[0], raw[1]]) as usize, u16::from_le_bytes([raw[2], raw[3]]) as usize);
        if w == 0 || h == 0 || w > SIDE || h > SIDE || n != 4 + w * h * 3 { return None; }
        let px = raw[4..n].chunks_exact(3).map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
        Some(Thumb::Picture { w, h, px })
    }
    fn keep(&mut self, window: &mut Window, name: &[u8], w: usize, h: usize, px: &[u32]) {
        if !self.dir_ready { let _ = files::mkdir(window, DIR); self.dir_ready = true; }
        let mut out = Vec::with_capacity(4 + px.len() * 3);
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
        let _ = files::write(window, name, &out);
    }

    /// How many tiles fit across a window this wide.
    pub fn columns(width: usize) -> usize { (width.saturating_sub(8) / CELL_W).max(1) }
    /// How many rows of tiles fit.
    fn rows(height: usize) -> usize { (height.saturating_sub(HEAD + FOOT) / CELL_H).max(1) }

    /// Draws the grid into the window's pixels; `top` is moved to keep the cursor seen.
    pub fn draw(&mut self, window: &mut Window, head: &str, foot: &str, cwd: &[u8], entries: &[Entry], cursor: usize, top: &mut usize) {
        let (width, height) = window.size();
        let (width, height) = (width.max(CELL_W + 8), height.max(HEAD + FOOT + CELL_H));
        let px = window.pixels();
        let px = &mut px[..width * height];
        px.fill(BG);
        let cols = Self::columns(width);
        let rows = Self::rows(height);
        let row = cursor / cols;
        if row < *top { *top = row; }
        if row >= *top + rows { *top = row + 1 - rows; }
        self.text(px, width, 10, 8, width - 20, head, TEXT);
        let first = *top * cols;
        for (i, e) in entries.iter().enumerate().skip(first).take(rows * cols) {
            let (c, r) = ((i - first) % cols, (i - first) / cols);
            let (x0, y0) = (8 + c * CELL_W, HEAD + r * CELL_H);
            let box_w = CELL_W - 8;
            fill(px, width, x0, y0, box_w, CELL_H - 6, if i == cursor { PICKED } else { TILE });
            let inner = (x0 + (box_w - SIDE) / 2, y0 + 4);
            match (kind(&e.name, e.dir), self.made.get(&join(cwd, &e.name))) {
                (None, Some(Thumb::Picture { w, h, px: t })) => {
                    let (ox, oy) = (inner.0 + (SIDE - w) / 2, inner.1 + (SIDE - h) / 2);
                    for y in 0..*h { px[(oy + y) * width + ox..(oy + y) * width + ox + w].copy_from_slice(&t[y * w..y * w + w]); }
                }
                (k, made) => {
                    let label = k.or(match made { Some(Thumb::Kind(k)) => Some(*k), _ => None }).unwrap_or("...");
                    fill(px, width, inner.0 + 20, inner.1 + 16, SIDE - 40, SIDE - 32, BG);
                    let lw = self.measure(label);
                    self.text(px, width, inner.0 + (SIDE.saturating_sub(lw)) / 2, inner.1 + SIDE / 2 - 8, SIDE, label, DIM);
                }
            }
            let mut name = String::from(crate::shown(&e.name));
            if e.dir { name.push('/'); }
            self.text(px, width, x0 + 4, y0 + SIDE + 6, box_w - 8, &name, TEXT);
        }
        let more = entries.len().saturating_sub(first + rows * cols);
        let foot = if more > 0 { alloc::format!("{} more below.  {}", more, foot) } else { String::from(foot) };
        self.text(px, width, 10, height - FOOT + 12, width - 20, &foot, DIM);
        window.show_pixels(width, height);
    }

    fn font(&mut self) -> Option<&Font<'static>> {
        if self.font.is_none() {
            self.font = Font::new(sex_font::GEIST_SANS);
            self.acc = vec![0; sex_font::raster::scratch_len(GLYPH_W, GLYPH_H)];
            self.cover = vec![0; GLYPH_W * GLYPH_H];
        }
        self.font.as_ref()
    }
    fn measure(&mut self, s: &str) -> usize {
        let Some(font) = self.font() else { return 0 };
        s.chars().filter_map(|c| font.glyph(c as u32)).map(|g| font.advance(g, SIZE) as usize).sum::<usize>() / 256
    }
    /// Draws one line of text at (`x`, `y`) top left, cut at `max` pixels wide.
    fn text(&mut self, px: &mut [u32], stride: usize, x: usize, y: usize, max: usize, s: &str, color: u32) {
        if self.font().is_none() { return; }
        let (font, acc, cover) = (self.font.as_ref().unwrap(), &mut self.acc, &mut self.cover);
        let rows = px.len() / stride;
        let mut pen = 0i32; // 1/256 pixel
        for c in s.chars() {
            let Some(g) = font.glyph(c as u32).or_else(|| font.glyph('?' as u32)) else { continue };
            let adv = font.advance(g, SIZE);
            if (pen + adv) / 256 > max as i32 { break; }
            let (whole, frac) = (pen / 256, pen % 256);
            if c != ' ' && font.draw(g, SIZE, 2 * 256 + frac, 15 * 256, GLYPH_W, GLYPH_H, acc, cover) {
                for gy in 0..GLYPH_H {
                    let py = y + gy;
                    if py >= rows { break; }
                    for gx in 0..GLYPH_W {
                        let a = cover[gy * GLYPH_W + gx] as u32;
                        if a == 0 { continue; }
                        let Some(pxx) = (x as i32 + whole + gx as i32 - 2).try_into().ok().filter(|v: &usize| *v < stride) else { continue };
                        let d = &mut px[py * stride + pxx];
                        *d = blend(*d, color, a);
                    }
                }
            }
            pen += adv;
        }
    }
}

const GLYPH_W: usize = 22;
const GLYPH_H: usize = 20;
/// 13 px, in 1/64 pixel.
const SIZE: u32 = 13 * 64;

fn blend(under: u32, over: u32, a: u32) -> u32 {
    let mix = |s: u32| ((((under >> s) & 0xFF) * (255 - a) + ((over >> s) & 0xFF) * a) / 255) << s;
    mix(16) | mix(8) | mix(0)
}
fn fill(px: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    for row in y..(y + h).min(px.len() / stride) { px[row * stride + x..row * stride + (x + w).min(stride)].fill(color); }
}

/// Reads and shrinks one picture: (width, height, 0x00RRGGBB pixels over the tile).
fn picture(window: &mut Window, path: &[u8], size: u64) -> Option<(usize, usize, Vec<u32>)> {
    if size == 0 || size > FILE_MAX { return None; }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size as usize).ok()?;
    bytes.resize(size as usize, 0);
    let n = files::read(window, &at(path), &mut bytes).ok()?;
    bytes.truncate(n);
    let image = sex_image::decode(&bytes, &sex_image::Limits::PHOTO).ok()?;
    drop(bytes);
    let small = image.fit_within(SIDE, SIDE).ok()?;
    let px = small.pixels.iter().map(|p| {
        let a = p >> 24;
        blend(TILE, p & 0xFF_FFFF, a)
    }).collect();
    Some((small.width, small.height, px))
}
