// SPDX-License-Identifier: GPL-3.0-only
//! Disk usage (Ctrl+U): what takes the room under the folder being looked at, as a map of
//! coloured rectangles, each as big as what it holds (a squarified treemap), folders
//! showing what is inside them. Arrows pick one of the biggest, Enter goes into a folder,
//! Backspace comes back out, Esc returns to the list. The folders are measured in the
//! background, one folder per piece of work.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loom_core::usage::{treemap, Rect, Tile, Tree, TreemapOptions, ROOT};
use sex_app::files::direct;
use sex_app::{Nav, Window};
use sex_font::Font;

use crate::{at, human, join, shown, App, BACKSPACE, ENTER, ESC};

const HEAD: usize = 34;
const FOOT: usize = 40;
const BG: u32 = 0x1B1D22;
const TEXT: u32 = 0xE6E8EC;
const DIM: u32 = 0x9AA0AA;
/// One colour per top-level item, in turn.
const PALETTE: [u32; 8] = [0x3D6FD6, 0xD6803D, 0x4FA35B, 0xB04FB0, 0xC9B23A, 0x3AA6B9, 0xC9504F, 0x7A7FD6];

pub struct State {
    pub base: Vec<u8>,
    tree: Tree,
    dirs: VecDeque<Vec<u8>>,
    scanning: bool,
    /// The folder of the tree being shown, and which of its children is picked.
    view: usize,
    pick: usize,
    started: u64,
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    cover: Vec<u8>,
}
impl State {
    pub fn busy(&self) -> bool { self.scanning }
}

fn blend(under: u32, over: u32, a: u32) -> u32 {
    let mix = |s: u32| ((((under >> s) & 0xFF) * (255 - a) + ((over >> s) & 0xFF) * a) / 255) << s;
    mix(16) | mix(8) | mix(0)
}
fn fill(px: &mut [u32], stride: usize, r: Rect, color: u32) {
    let rows = px.len() / stride;
    let (x0, y0) = (r.x.max(0) as usize, r.y.max(0) as usize);
    let (x1, y1) = (((r.x + r.w).max(0) as usize).min(stride), ((r.y + r.h).max(0) as usize).min(rows));
    for y in y0..y1 { if x0 < x1 { px[y * stride + x0..y * stride + x1].fill(color); } }
}
fn frame(px: &mut [u32], stride: usize, r: Rect, t: i32, color: u32) {
    fill(px, stride, Rect { x: r.x, y: r.y, w: r.w, h: t }, color);
    fill(px, stride, Rect { x: r.x, y: r.y + r.h - t, w: r.w, h: t }, color);
    fill(px, stride, Rect { x: r.x, y: r.y, w: t, h: r.h }, color);
    fill(px, stride, Rect { x: r.x + r.w - t, y: r.y, w: t, h: r.h }, color);
}

const GLYPH_W: usize = 22;
const GLYPH_H: usize = 20;
const SIZE: u32 = 13 * 64;

impl State {
    fn font(&mut self) -> bool {
        if self.font.is_none() {
            self.font = Font::new(sex_font::GEIST_SANS);
            self.acc = vec![0; sex_font::raster::scratch_len(GLYPH_W, GLYPH_H)];
            self.cover = vec![0; GLYPH_W * GLYPH_H];
        }
        self.font.is_some()
    }
    /// One line of text at (`x`, `y`) top left, cut at `max` pixels wide.
    fn text(&mut self, px: &mut [u32], stride: usize, x: usize, y: usize, max: usize, s: &str, color: u32) {
        if !self.font() { return; }
        let (font, acc, cover) = (self.font.as_ref().unwrap(), &mut self.acc, &mut self.cover);
        let rows = px.len() / stride;
        let mut pen = 0i32;
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
    /// The child of the view a tile belongs to, by its place among the view's children.
    fn top_of(&self, mut node: usize) -> Option<usize> {
        loop {
            let p = self.tree.nodes[node].parent?;
            if p == self.view { return self.tree.nodes[self.view].children.iter().position(|&c| c == node); }
            node = p;
        }
    }
    fn tiles(&self, width: usize, height: usize) -> Vec<Tile> {
        let r = Rect { x: 8, y: HEAD as i32, w: width as i32 - 16, h: (height - HEAD - FOOT) as i32 };
        treemap(&self.tree, self.view, r, &TreemapOptions { max_depth: 3, inset: 3, min_side: 12 })
    }
}

impl App {
    pub(crate) fn usage_start(&mut self) {
        if self.found.is_some() { self.status = "Esc leaves the search results first.".into(); return; }
        let base = self.cwd.clone();
        sex_pdx::serial_record!("[files.app.usage.start] base={}", shown(&base));
        self.loom.mode = crate::loom::Mode::Usage(State { tree: Tree::new(crate::base(&base)), dirs: VecDeque::from([base.clone()]), base, scanning: true,
            view: ROOT, pick: 0, started: sex_pdx::get_ticks(), font: None, acc: Vec::new(), cover: Vec::new() });
        self.last_drawn = 0;
    }
    /// Measures one folder. True when the window should be drawn again.
    pub(crate) fn usage_step(&mut self) -> bool {
        let crate::loom::Mode::Usage(st) = &mut self.loom.mode else { return false };
        if !st.scanning { return false; }
        if let Some(dir) = st.dirs.pop_front() {
            let rel = dir[st.base.len()..].strip_prefix(b"/").unwrap_or(&dir[st.base.len()..]).to_vec();
            for e in direct::list(&at(&dir)).unwrap_or_default() {
                if e.name == crate::TRASH || e.name == crate::meta::FILE || e.name.first() == Some(&b'.') && e.name.ends_with(b".part") { continue; }
                let mut r = rel.clone();
                if !r.is_empty() { r.push(b'/'); }
                r.extend_from_slice(&e.name);
                if e.dir { st.tree.add_dir(&r); st.dirs.push_back(join(&dir, &e.name)); } else { st.tree.add(&r, e.size); }
            }
            let now = sex_pdx::get_ticks();
            if now.saturating_sub(self.last_drawn) >= 250 { self.last_drawn = now; return true; }
            return false;
        }
        st.tree.finish();
        st.scanning = false;
        let root = &st.tree.nodes[ROOT];
        sex_pdx::serial_record!("[files.app.usage] base={} files={} nodes={} bytes={} ms={}", shown(&st.base), root.files, st.tree.nodes.len(), root.total, sex_pdx::get_ticks().saturating_sub(st.started));
        self.usage_log();
        true
    }
    fn usage_log(&self) {
        let crate::loom::Mode::Usage(st) = &self.loom.mode else { return };
        let n = &st.tree.nodes[st.view];
        let top: Vec<String> = n.children.iter().take(5).map(|&c| format!("{}:{}", shown(&st.tree.nodes[c].name), st.tree.nodes[c].total)).collect();
        sex_pdx::serial_record!("[files.app.usage.view] path=/{} total={} items={} biggest={}", shown(&st.tree.path(st.view)), n.total, n.children.len(), top.join(","));
    }
    pub(crate) fn usage_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let crate::loom::Mode::Usage(st) = &mut self.loom.mode else { return };
        if key.code == ESC { self.loom.mode = crate::loom::Mode::None; self.load(window); return; }
        if st.scanning { return; }
        let count = st.tree.nodes[st.view].children.len();
        match (key.code, sex_app::nav(key)) {
            (_, Some(Nav::Left | Nav::Up)) => st.pick = st.pick.saturating_sub(1),
            (_, Some(Nav::Right | Nav::Down)) => st.pick = (st.pick + 1).min(count.saturating_sub(1)),
            (_, Some(Nav::Home)) => st.pick = 0,
            (_, Some(Nav::End)) => st.pick = count.saturating_sub(1),
            (ENTER, _) => {
                let Some(&c) = st.tree.nodes[st.view].children.get(st.pick) else { return };
                let n = &st.tree.nodes[c];
                if n.dir && n.total > 0 { st.view = c; st.pick = 0; self.usage_log(); }
                else { self.status = format!("{}: {}.", shown(&n.name), human(n.total)); }
            }
            (BACKSPACE, _) => if let Some(p) = st.tree.nodes[st.view].parent {
                let from = st.view;
                st.view = p;
                st.pick = st.tree.nodes[p].children.iter().position(|&c| c == from).unwrap_or(0);
                self.usage_log();
            },
            _ => {}
        }
    }
    /// Draws the map into the window's pixels.
    pub(crate) fn usage_render(&mut self, window: &mut Window) -> bool {
        let status = self.status.clone();
        let crate::loom::Mode::Usage(st) = &mut self.loom.mode else { return false };
        let (width, height) = window.size();
        let (width, height) = (width.max(320), height.max(HEAD + FOOT + 120));
        let px = window.pixels();
        let px = &mut px[..width * height];
        px.fill(BG);
        let path = st.tree.path(st.view);
        if st.scanning {
            let n = st.tree.nodes.len();
            st.text(px, width, 10, 8, width - 20, &format!("Measuring {}: {} items so far.  Esc stops.", shown(&st.base), n), TEXT);
        } else {
            let (total, files_n, picked) = { let v = &st.tree.nodes[st.view]; (v.total, v.files, v.children.get(st.pick).copied()) };
            let head = format!("Disk usage  {}/{}   {} in {} files", shown(&st.base), shown(&path), human(total), files_n);
            st.text(px, width, 10, 8, width - 20, &head, TEXT);
            let tiles = st.tiles(width, height);
            for t in tiles.iter().skip(1) {
                let Some(top) = st.top_of(t.node) else { continue };
                let base = PALETTE[top % PALETTE.len()];
                // Deeper is lighter; folders frame what is inside them.
                let c = blend(base, 0xFFFFFF, 40 * (t.depth - 1).min(4));
                fill(px, width, t.rect, if st.tree.nodes[t.node].dir { blend(c, 0x000000, 60) } else { c });
                if !st.tree.nodes[t.node].dir || t.depth == 3 { frame(px, width, t.rect, 1, BG); }
            }
            // Labels on the biggest, then the frame of the picked one.
            for t in tiles.iter().skip(1).filter(|t| t.depth == 1) {
                if t.rect.w >= 60 && t.rect.h >= 22 {
                    let label = { let n = &st.tree.nodes[t.node]; format!("{}{}  {}", shown(&n.name), if n.dir { "/" } else { "" }, human(n.total)) };
                    fill(px, width, Rect { x: t.rect.x + 3, y: t.rect.y + 3, w: (t.rect.w - 6).min(label.len() as i32 * 8 + 8), h: 18 }, BG);
                    st.text(px, width, t.rect.x as usize + 6, t.rect.y as usize + 2, (t.rect.w - 10) as usize, &label, TEXT);
                }
            }
            if let Some(c) = picked {
                if let Some(t) = tiles.iter().find(|t| t.node == c) { frame(px, width, t.rect, 3, 0xFFFFFF); }
                let n = &st.tree.nodes[c];
                let foot = format!("{}{}  {} ({} files)   Arrows pick  Enter opens a folder  Backspace up  Esc list  {}", shown(&n.name), if n.dir { "/" } else { "" }, human(n.total), n.files, status);
                st.text(px, width, 10, height - FOOT + 12, width - 20, &foot, DIM);
            } else {
                st.text(px, width, 10, height - FOOT + 12, width - 20, "Nothing takes room here.   Backspace up  Esc list", DIM);
            }
        }
        window.show_pixels(width, height);
        self.showing_pixels = true;
        true
    }
}
