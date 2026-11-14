// SPDX-License-Identifier: GPL-3.0-only
//! Disk usage maps: a tree of sizes, laid out as a squarified treemap in whole pixels and
//! as sunburst rings in whole millidegrees.
//!
//! Both layouts are exact partitions: the children of a folder cover its tile (or arc)
//! with no gap and no overlap, because every edge is a rounded *cumulative* share of the
//! parent rather than a rounded share of its own. Sizes far apart can still round a tiny
//! child to nothing; it then gets no tile and its space goes to its neighbours' rounding.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A file or folder. Folders' `total` is the sum of everything under them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node { pub name: Vec<u8>, pub dir: bool, pub own: u64, pub total: u64, pub files: u64, pub parent: Option<usize>, pub children: Vec<usize> }

/// The tree, root at index 0. Build with [`Tree::add`], then call [`Tree::finish`].
#[derive(Clone, Debug, Default)]
pub struct Tree { pub nodes: Vec<Node>, index: BTreeMap<(usize, Vec<u8>), usize> }

pub const ROOT: usize = 0;

impl Tree {
    pub fn new(root_name: &[u8]) -> Tree {
        Tree { nodes: alloc::vec![Node { name: root_name.to_vec(), dir: true, own: 0, total: 0, files: 0, parent: None, children: Vec::new() }], index: BTreeMap::new() }
    }

    fn child(&mut self, parent: usize, name: &[u8], dir: bool) -> usize {
        if let Some(&i) = self.index.get(&(parent, name.to_vec())) { return i; }
        let i = self.nodes.len();
        self.nodes.push(Node { name: name.to_vec(), dir, own: 0, total: 0, files: 0, parent: Some(parent), children: Vec::new() });
        self.nodes[parent].children.push(i);
        self.index.insert((parent, name.to_vec()), i);
        i
    }

    /// Adds a file at `path` (relative to the root, `/`-separated), making its folders.
    pub fn add(&mut self, path: &[u8], size: u64) -> usize {
        let mut at = ROOT;
        let parts: Vec<&[u8]> = path.split(|b| *b == b'/').filter(|p| !p.is_empty()).collect();
        for (k, p) in parts.iter().enumerate() { at = self.child(at, p, k + 1 < parts.len()); }
        if at != ROOT { self.nodes[at].own += size; }
        at
    }

    /// Adds an (empty) folder.
    pub fn add_dir(&mut self, path: &[u8]) -> usize {
        let mut at = ROOT;
        for p in path.split(|b| *b == b'/').filter(|p| !p.is_empty()) { at = self.child(at, p, true); }
        at
    }

    /// Sums totals bottom-up and sorts children largest first (then by name).
    pub fn finish(&mut self) {
        for n in &mut self.nodes { n.total = n.own; n.files = if n.dir { 0 } else { 1 }; }
        // Children always come after their parent.
        for i in (1..self.nodes.len()).rev() {
            let (t, f) = (self.nodes[i].total, self.nodes[i].files);
            let p = self.nodes[i].parent.unwrap_or(ROOT);
            self.nodes[p].total += t;
            self.nodes[p].files += f;
        }
        for i in 0..self.nodes.len() {
            let mut c = core::mem::take(&mut self.nodes[i].children);
            c.sort_by(|a, b| self.nodes[*b].total.cmp(&self.nodes[*a].total).then_with(|| self.nodes[*a].name.cmp(&self.nodes[*b].name)));
            self.nodes[i].children = c;
        }
    }

    /// The path of a node from the root.
    pub fn path(&self, mut i: usize) -> Vec<u8> {
        let mut parts = Vec::new();
        while let Some(p) = self.nodes[i].parent { parts.push(i); i = p; }
        let mut out = Vec::new();
        for (k, n) in parts.iter().rev().enumerate() {
            if k > 0 { out.push(b'/'); }
            out.extend_from_slice(&self.nodes[*n].name);
        }
        out
    }

    pub fn find(&self, path: &[u8]) -> Option<usize> {
        let mut at = ROOT;
        for p in path.split(|b| *b == b'/').filter(|p| !p.is_empty()) { at = *self.index.get(&(at, p.to_vec()))?; }
        Some(at)
    }

    /// The `n` largest files, largest first.
    pub fn largest(&self, n: usize) -> Vec<usize> {
        let mut f: Vec<usize> = (0..self.nodes.len()).filter(|&i| !self.nodes[i].dir).collect();
        f.sort_by(|a, b| self.nodes[*b].total.cmp(&self.nodes[*a].total).then(a.cmp(b)));
        f.truncate(n);
        f
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect { Rect { x, y, w, h } }
    pub fn area(&self) -> i64 { self.w.max(0) as i64 * self.h.max(0) as i64 }
    pub fn contains(&self, x: i32, y: i32) -> bool { x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    pub fn inset(&self, d: i32) -> Rect { Rect { x: self.x + d, y: self.y + d, w: (self.w - 2 * d).max(0), h: (self.h - 2 * d).max(0) } }
}

/// `a * b / c`, rounded to nearest, in wide integers.
fn share(a: u64, b: i64, c: u64) -> i64 {
    if c == 0 { return 0; }
    ((a as u128 * b as u128 + c as u128 / 2) / c as u128) as i64
}

/// Lays out `items` (weight, already largest first; zero weights get nothing) in `r` with
/// the squarified algorithm. Returns (item index, rect) for every item that got pixels.
pub fn squarify(weights: &[u64], r: Rect) -> Vec<(usize, Rect)> {
    let mut out = Vec::new();
    let items: Vec<usize> = (0..weights.len()).filter(|&i| weights[i] > 0).collect();
    let mut rest: u64 = items.iter().map(|&i| weights[i]).sum();
    let mut r = r;
    let mut i = 0;
    while i < items.len() && r.w > 0 && r.h > 0 {
        let wide = r.w >= r.h;
        let (long, short) = if wide { (r.w as f64, r.h as f64) } else { (r.h as f64, r.w as f64) };
        let area = long * short;
        let scale = area / rest as f64;
        // Grow the row while its worst aspect ratio improves.
        let worst = |sum: f64, max: f64, min: f64| {
            let s = sum * scale;
            let l2 = short * short;
            let a = l2 * max * scale / (s * s);
            let b = s * s / (l2 * min * scale);
            if a > b { a } else { b }
        };
        let mut j = i + 1;
        let first = weights[items[i]] as f64;
        let (mut sum, mut max, mut min) = (first, first, first);
        let mut best = worst(sum, max, min);
        while j < items.len() {
            let w = weights[items[j]] as f64;
            let (s2, mx, mn) = (sum + w, if w > max { w } else { max }, if w < min { w } else { min });
            let next = worst(s2, mx, mn);
            if next > best { break; }
            sum = s2; max = mx; min = mn; best = next;
            j += 1;
        }
        let row = &items[i..j];
        let row_sum: u64 = row.iter().map(|&k| weights[k]).sum();
        let extent = if wide { r.w } else { r.h };
        let thick = if j == items.len() { extent } else { share(row_sum, extent as i64, rest) as i32 };
        let along = if wide { r.h } else { r.w };
        let mut cum = 0u64;
        let mut prev = 0i32;
        for &k in row {
            cum += weights[k];
            let edge = share(cum, along as i64, row_sum) as i32;
            let tile = if wide { Rect::new(r.x, r.y + prev, thick, edge - prev) } else { Rect::new(r.x + prev, r.y, edge - prev, thick) };
            if tile.w > 0 && tile.h > 0 { out.push((k, tile)); }
            prev = edge;
        }
        r = if wide { Rect::new(r.x + thick, r.y, r.w - thick, r.h) } else { Rect::new(r.x, r.y + thick, r.w, r.h - thick) };
        rest -= row_sum;
        i = j;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreemapOptions {
    /// Levels below the root to draw (the root is depth 0).
    pub max_depth: u32,
    /// Pixels a folder's children sit inside its edge (room for a frame).
    pub inset: i32,
    /// Folders smaller than this on either side aren't opened up.
    pub min_side: i32,
}
impl Default for TreemapOptions { fn default() -> Self { TreemapOptions { max_depth: 32, inset: 0, min_side: 4 } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile { pub node: usize, pub rect: Rect, pub depth: u32 }

/// A treemap of `root` in `rect`: the root's tile first, then each folder's children after it
/// (parents always before children, so drawing in order paints folders under their contents).
pub fn treemap(tree: &Tree, root: usize, rect: Rect, opts: &TreemapOptions) -> Vec<Tile> {
    let mut out = Vec::new();
    if tree.nodes[root].total == 0 || rect.w <= 0 || rect.h <= 0 { return out; }
    out.push(Tile { node: root, rect, depth: 0 });
    let mut k = 0;
    while k < out.len() {
        let t = out[k];
        k += 1;
        let n = &tree.nodes[t.node];
        if !n.dir || t.depth >= opts.max_depth { continue; }
        let inner = t.rect.inset(opts.inset);
        if inner.w < opts.min_side || inner.h < opts.min_side { continue; }
        let weights: Vec<u64> = n.children.iter().map(|&c| tree.nodes[c].total).collect();
        for (i, r) in squarify(&weights, inner) { out.push(Tile { node: n.children[i], rect: r, depth: t.depth + 1 }); }
    }
    out
}

/// The deepest tile under a point.
pub fn tile_at(tiles: &[Tile], x: i32, y: i32) -> Option<&Tile> {
    tiles.iter().filter(|t| t.rect.contains(x, y)).max_by_key(|t| t.depth)
}

/// A full turn in sunburst angle units (millidegrees).
pub const TURN: u32 = 360_000;

/// One ring segment: `depth` 1 is the ring around the centre disc, angles clockwise from 12 o'clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arc { pub node: usize, pub depth: u32, pub start: u32, pub sweep: u32 }

/// Sunburst rings for `root` (the centre disc), `max_depth` rings out; arcs narrower than
/// `min_sweep` are left out with everything under them.
pub fn sunburst(tree: &Tree, root: usize, max_depth: u32, min_sweep: u32) -> Vec<Arc> {
    let mut out = Vec::new();
    let mut stack = alloc::vec![(root, 0u32, 0u32, TURN)];
    while let Some((n, depth, start, sweep)) = stack.pop() {
        let node = &tree.nodes[n];
        if depth >= max_depth || node.total == 0 { continue; }
        let sum: u64 = node.children.iter().map(|&c| tree.nodes[c].total).sum();
        let mut cum = 0u64;
        let mut prev = 0u32;
        let mut ring = Vec::new();
        for &c in &node.children {
            cum += tree.nodes[c].total;
            let edge = share(cum, sweep as i64, sum) as u32;
            if edge - prev >= min_sweep.max(1) { ring.push(Arc { node: c, depth: depth + 1, start: start + prev, sweep: edge - prev }); }
            prev = edge;
        }
        for a in ring.iter().rev() { if tree.nodes[a.node].dir { stack.push((a.node, a.depth, a.start, a.sweep)); } }
        out.extend(ring);
    }
    out.sort_by_key(|a| (a.depth, a.start));
    out
}

/// The arc at a ring and angle.
pub fn arc_at(arcs: &[Arc], depth: u32, angle: u32) -> Option<&Arc> {
    arcs.iter().find(|a| a.depth == depth && angle >= a.start && angle < a.start + a.sweep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> Tree {
        let mut t = Tree::new(b"/");
        let mut x = 12345u32;
        for d in 0..6 {
            for s in 0..(3 + d * 2) {
                for f in 0..(1 + (s % 4)) {
                    x ^= x << 13; x ^= x >> 17; x ^= x << 5;
                    let size = (x % 5_000_000) as u64 + 1;
                    t.add(format!("dir{d}/sub{s}/file{f}.bin").as_bytes(), size);
                }
            }
        }
        t.add(b"big.iso", 40_000_000);
        t.add(b"tiny", 1);
        t.add_dir(b"emptydir");
        t.finish();
        t
    }

    #[test]
    fn tree_sums() {
        let mut t = Tree::new(b"");
        t.add(b"a/b/c", 10);
        t.add(b"a/b/d", 5);
        t.add(b"a/e", 1);
        t.add(b"f", 100);
        t.add(b"a/b/c", 10); // added twice: counts twice
        t.finish();
        assert_eq!(t.nodes[ROOT].total, 126);
        assert_eq!(t.nodes[ROOT].files, 4);
        let a = t.find(b"a").unwrap();
        assert_eq!(t.nodes[a].total, 26);
        assert_eq!(t.nodes[ROOT].children[0], t.find(b"f").unwrap());
        assert_eq!(t.path(t.find(b"a/b/d").unwrap()), b"a/b/d");
        assert_eq!(t.largest(2), vec![t.find(b"f").unwrap(), t.find(b"a/b/c").unwrap()]);
    }

    #[test]
    fn squarify_golden() {
        // The example from Bruls, Huizing and van Wijk's paper, scaled to 600x400 pixels.
        let r = squarify(&[6, 6, 4, 3, 2, 2, 1], Rect::new(0, 0, 600, 400));
        let rects: Vec<Rect> = r.iter().map(|x| x.1).collect();
        assert_eq!(rects, vec![
            Rect::new(0, 0, 300, 200), Rect::new(0, 200, 300, 200),
            Rect::new(300, 0, 171, 233), Rect::new(471, 0, 129, 233),
            Rect::new(300, 233, 120, 167), Rect::new(420, 233, 120, 167), Rect::new(540, 233, 60, 167),
        ]);
        assert_eq!(rects.iter().map(Rect::area).sum::<i64>(), 240_000);
    }

    fn check_partition(tiles: &[Tile], tree: &Tree, opts: &TreemapOptions) {
        for parent in tiles {
            let kids: Vec<&Tile> = tiles.iter().filter(|t| tree.nodes[t.node].parent == Some(parent.node) && t.depth == parent.depth + 1).collect();
            if kids.is_empty() { continue; }
            let inner = parent.rect.inset(opts.inset);
            let area: i64 = kids.iter().map(|k| k.rect.area()).sum();
            assert_eq!(area, inner.area(), "children fill their folder");
            let total: u64 = tree.nodes[parent.node].children.iter().map(|&c| tree.nodes[c].total).sum();
            for (i, a) in kids.iter().enumerate() {
                assert!(a.rect.x >= inner.x && a.rect.y >= inner.y && a.rect.x + a.rect.w <= inner.x + inner.w && a.rect.y + a.rect.h <= inner.y + inner.h);
                for b in &kids[i + 1..] { assert!(!a.rect.intersects(&b.rect), "{:?} overlaps {:?}", a.rect, b.rect); }
                // Area within rounding of its share: each edge is off by at most half a pixel.
                let want = inner.area() as f64 * tree.nodes[a.node].total as f64 / total as f64;
                let slack = (inner.w + inner.h) as f64;
                assert!((a.rect.area() as f64 - want).abs() <= slack, "area {} want {}", a.rect.area(), want);
            }
        }
    }

    #[test]
    fn treemap_invariants() {
        let t = sample();
        for (rect, inset) in [(Rect::new(0, 0, 1024, 768), 0), (Rect::new(10, 20, 333, 1000), 2), (Rect::new(0, 0, 1920, 1080), 1)] {
            let opts = TreemapOptions { inset, ..TreemapOptions::default() };
            let tiles = treemap(&t, ROOT, rect, &opts);
            assert_eq!(tiles[0].rect, rect);
            check_partition(&tiles, &t, &opts);
            // Leaves alone tile the whole area when nothing is inset.
            if inset == 0 {
                let leaves: i64 = tiles.iter().filter(|x| !t.nodes[x.node].dir).map(|x| x.rect.area()).sum();
                assert!(leaves <= rect.area());
            }
            // Deterministic.
            assert_eq!(tiles, treemap(&t, ROOT, rect, &opts));
            // Every tile within the root, and hit testing finds the deepest one.
            let big = t.find(b"big.iso").unwrap();
            let bt = tiles.iter().find(|x| x.node == big).unwrap();
            assert_eq!(tile_at(&tiles, bt.rect.x + 1, bt.rect.y + 1).map(|x| x.node), Some(big));
        }
        // Squarified tiles stay roughly square: the big file isn't a sliver.
        let tiles = treemap(&t, ROOT, Rect::new(0, 0, 1000, 1000), &TreemapOptions::default());
        let big = tiles.iter().find(|x| x.node == t.find(b"big.iso").unwrap()).unwrap().rect;
        assert!(big.w * 3 > big.h && big.h * 3 > big.w);
        let depth1 = TreemapOptions { max_depth: 1, ..TreemapOptions::default() };
        assert!(treemap(&t, ROOT, Rect::new(0, 0, 100, 100), &depth1).iter().all(|x| x.depth <= 1));
        assert!(treemap(&t, ROOT, Rect::new(0, 0, 0, 100), &depth1).is_empty());
    }

    #[test]
    fn sunburst_invariants() {
        let t = sample();
        let arcs = sunburst(&t, ROOT, 3, 1);
        assert!(arcs.iter().all(|a| a.depth >= 1 && a.depth <= 3 && a.sweep > 0));
        // The first ring covers the full turn, in order, with no gap.
        let ring1: Vec<&Arc> = arcs.iter().filter(|a| a.depth == 1).collect();
        assert_eq!(ring1[0].start, 0);
        for w in ring1.windows(2) { assert_eq!(w[0].start + w[0].sweep, w[1].start); }
        assert_eq!(ring1.last().map(|a| a.start + a.sweep), Some(TURN));
        // Children exactly cover their parent's arc.
        for p in &arcs {
            let kids: Vec<&Arc> = arcs.iter().filter(|a| a.depth == p.depth + 1 && t.nodes[a.node].parent == Some(p.node)).collect();
            if kids.is_empty() { continue; }
            assert_eq!(kids.iter().map(|a| a.sweep).sum::<u32>(), p.sweep);
            assert_eq!(kids[0].start, p.start);
        }
        // Golden: big.iso takes its share of the turn.
        let big = arcs.iter().find(|a| a.node == t.find(b"big.iso").unwrap()).unwrap();
        let want = (40_000_000u128 * TURN as u128 + t.nodes[ROOT].total as u128 / 2) / t.nodes[ROOT].total as u128;
        assert!(big.sweep as u128 + 1 >= want && big.sweep as u128 <= want + 1, "{} vs {}", big.sweep, want);
        assert_eq!(arc_at(&arcs, 1, big.start + 1).map(|a| a.node), Some(big.node));
        assert_eq!(arc_at(&arcs, 1, big.start + big.sweep).map(|a| a.node == big.node), Some(false));
        // Narrow arcs drop out with what is under them.
        let coarse = sunburst(&t, ROOT, 6, 5_000);
        assert!(coarse.iter().all(|a| a.sweep >= 5_000));
        assert!(coarse.len() < sunburst(&t, ROOT, 6, 1).len());
    }

    #[test]
    fn sunburst_golden_small() {
        let mut t = Tree::new(b"");
        t.add(b"a/x", 3);
        t.add(b"a/y", 1);
        t.add(b"b", 4);
        t.finish();
        let arcs = sunburst(&t, ROOT, 2, 1);
        let got: Vec<(Vec<u8>, u32, u32, u32)> = arcs.iter().map(|a| (t.path(a.node), a.depth, a.start, a.sweep)).collect();
        assert_eq!(got, vec![
            (b"a".to_vec(), 1, 0, 180_000), (b"b".to_vec(), 1, 180_000, 180_000),
            (b"a/x".to_vec(), 2, 0, 135_000), (b"a/y".to_vec(), 2, 135_000, 45_000),
        ]);
    }
}
