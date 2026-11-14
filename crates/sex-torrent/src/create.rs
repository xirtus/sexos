// SPDX-License-Identifier: GPL-3.0-only
//! Making torrents: v1 (BEP 3), v2 (BEP 52) or hybrid, without touching a file.
//!
//! The program lists the files (paths and sizes), builds a [`Creator`], then reads the
//! files itself and hands their bytes to [`Creator::feed`] one after another in the
//! order of [`Creator::files`] (sorted by path, which is the order of the v2 file tree;
//! empty files and symlinks get no bytes). [`Creator::finish`] returns the .torrent.
//!
//! Choices made here:
//! - Piece length: given, or [`auto_piece_length`] picks a power of two from 16 KiB to
//!   16 MiB giving at most 2000 pieces (so between 1000 and 2000 unless clamped).
//! - A multi-file hybrid gets a BEP 47 padding file (`attr` `p`, path `.pad/<n>`) after
//!   every non-empty file whose end is not on a piece boundary, the last one included
//!   (as libtorrent and the BEP 52 reference creator do), so v1 and v2 pieces line up.
//! - A torrent is single-file when it has exactly one file whose path is `[name]`.
//! - v2 output always has a `piece layers` dictionary, empty if no file needs one.
//! - The creation date is whatever the caller passes (we have no clock).
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use sha1::{Digest, Sha1};

use crate::bencode::{dict, Item};
use crate::merkle::{FileHasher, FileHashes, BLOCK_SIZE};
use crate::metainfo::{sanitize_component, FileAttr, MAX_FILES, MAX_PATH_DEPTH, MAX_PIECE_LENGTH};
use crate::InfoHashV1;

/// Largest piece length [`auto_piece_length`] picks.
pub const AUTO_MAX_PIECE: u64 = 16 << 20;
/// Most pieces [`auto_piece_length`] aims for.
pub const AUTO_TARGET_PIECES: u64 = 2000;

/// Which kind of torrent to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version { V1, V2, Hybrid }

impl Version {
    fn v1(self) -> bool { self != Version::V2 }
    fn v2(self) -> bool { self != Version::V1 }
}

/// One file to put in the torrent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSpec {
    /// Path components below the torrent's directory (or `[name]` for a single file).
    pub path: Vec<String>,
    pub length: u64,
    /// Executable and hidden flags are written; `pad` and `symlink` are ignored here.
    pub attr: FileAttr,
    /// Makes this entry a symlink (length 0) to this path, relative to the torrent root.
    pub symlink_target: Option<Vec<String>>,
}

impl FileSpec {
    pub fn new(path: &[&str], length: u64) -> FileSpec {
        FileSpec { path: path.iter().map(|c| c.to_string()).collect(), length, attr: FileAttr::default(), symlink_target: None }
    }
    fn is_data(&self) -> bool { self.symlink_target.is_none() && self.length > 0 }
}

/// Everything about the torrent except the files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub name: String,
    pub version: Version,
    /// `None` picks one with [`auto_piece_length`].
    pub piece_length: Option<u64>,
    /// Tracker tiers (BEP 12).
    pub trackers: Vec<Vec<String>>,
    /// `url-list` (BEP 19).
    pub web_seeds: Vec<String>,
    /// DHT nodes (BEP 5).
    pub nodes: Vec<(String, u16)>,
    /// BEP 27.
    pub private: bool,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    /// Seconds since the Unix epoch, from the program's clock.
    pub creation_date: Option<i64>,
    /// BEP 38 hints.
    pub similar: Vec<InfoHashV1>,
    pub collections: Vec<String>,
}

impl Options {
    pub fn new(name: &str, version: Version) -> Options {
        Options { name: name.to_string(), version, piece_length: None, trackers: Vec::new(), web_seeds: Vec::new(), nodes: Vec::new(), private: false, comment: None, created_by: None, creation_date: None, similar: Vec::new(), collections: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoFiles,
    TooManyFiles,
    /// An empty path or name, or a component that is `.`, `..`, empty, too long, or has
    /// `/`, `\` or control characters.
    BadPath,
    DuplicatePath,
    /// Not a power of two from 16 KiB to 512 MiB.
    BadPieceLength,
    TooLarge,
    /// More bytes fed than the files hold.
    TooMuchData,
    /// `finish` before all bytes were fed; this many are missing.
    Incomplete(u64),
}

/// The piece length for `total` bytes: the smallest power of two from 16 KiB that gives
/// at most [`AUTO_TARGET_PIECES`] pieces, capped at 16 MiB.
pub fn auto_piece_length(total: u64) -> u64 {
    let mut pl = BLOCK_SIZE;
    while pl < AUTO_MAX_PIECE && total.div_ceil(pl) > AUTO_TARGET_PIECES { pl *= 2; }
    pl
}

fn clean(path: &[String]) -> bool {
    !path.is_empty() && path.len() <= MAX_PATH_DEPTH && path.iter().all(|c| sanitize_component(c.as_bytes()).as_deref() == Some(c.as_str()))
}

/// Builds a torrent from file bytes fed in order.
#[derive(Clone, Debug)]
pub struct Creator {
    opts: Options,
    files: Vec<FileSpec>,
    single: bool,
    piece_length: u64,
    /// The file being fed, and its bytes still to come.
    cur: usize,
    left: u64,
    remaining: u64,
    v1: Option<Sha1>,
    v1_in_piece: u64,
    v1_pieces: Vec<u8>,
    v2: Option<FileHasher>,
    v2_done: Vec<Option<FileHashes>>,
}

impl Creator {
    /// Checks the file list and options. Files are sorted by path.
    pub fn new(opts: Options, mut files: Vec<FileSpec>) -> Result<Creator, Error> {
        if files.is_empty() { return Err(Error::NoFiles); }
        if files.len() > MAX_FILES { return Err(Error::TooManyFiles); }
        if !clean(core::slice::from_ref(&opts.name)) { return Err(Error::BadPath); }
        for f in files.iter_mut() {
            if !clean(&f.path) || f.symlink_target.as_ref().is_some_and(|t| !clean(t)) { return Err(Error::BadPath); }
            f.attr.pad = false;
            f.attr.symlink = f.symlink_target.is_some();
            if f.attr.symlink { f.length = 0; }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        // Sorted, so a duplicate or a file-inside-a-file comes right after its twin.
        if files.windows(2).any(|w| w[1].path.starts_with(&w[0].path)) { return Err(Error::DuplicatePath); }
        let total = files.iter().try_fold(0u64, |a, f| a.checked_add(f.length)).ok_or(Error::TooLarge)?;
        if total == 0 { return Err(Error::NoFiles); }
        let piece_length = opts.piece_length.unwrap_or_else(|| auto_piece_length(total));
        if piece_length < BLOCK_SIZE || piece_length > MAX_PIECE_LENGTH || !piece_length.is_power_of_two() { return Err(Error::BadPieceLength); }
        // The piece space must fit: v2 alignment adds less than a piece per file.
        let pieces = files.iter().try_fold(0u64, |a, f| a.checked_add(f.length.div_ceil(piece_length))).ok_or(Error::TooLarge)?;
        if pieces > u32::MAX as u64 || pieces.checked_mul(piece_length).is_none() { return Err(Error::TooLarge); }
        let single = files.len() == 1 && files[0].path.len() == 1 && files[0].path[0] == opts.name && files[0].symlink_target.is_none();
        let v = opts.version;
        let mut c = Creator {
            single, piece_length, cur: 0, left: 0, remaining: total,
            v1: v.v1().then(Sha1::new), v1_in_piece: 0, v1_pieces: Vec::new(),
            v2: None, v2_done: vec![None; files.len()], files, opts,
        };
        c.left = c.files[0].length;
        c.start_file();
        Ok(c)
    }

    /// The files, in the order their bytes must be fed.
    pub fn files(&self) -> &[FileSpec] { &self.files }
    pub fn piece_length(&self) -> u64 { self.piece_length }
    /// Bytes still to be fed.
    pub fn remaining(&self) -> u64 { self.remaining }
    /// The index (in [`Creator::files`]) of the file whose bytes come next.
    pub fn current_file(&self) -> Option<usize> { (self.remaining > 0).then_some(self.cur) }

    /// Skips files with no bytes and sets up hashing of the next one.
    fn start_file(&mut self) {
        while self.left == 0 && self.cur + 1 < self.files.len() && self.remaining > 0 {
            self.cur += 1;
            self.left = if self.files[self.cur].is_data() { self.files[self.cur].length } else { 0 };
        }
        if self.opts.version.v2() && self.left > 0 { self.v2 = FileHasher::new(self.piece_length); }
    }

    fn v1_update(&mut self, mut data: &[u8]) {
        let Some(h) = self.v1.as_mut() else { return };
        while !data.is_empty() {
            let take = ((self.piece_length - self.v1_in_piece) as usize).min(data.len());
            h.update(&data[..take]);
            self.v1_in_piece += take as u64;
            data = &data[take..];
            if self.v1_in_piece == self.piece_length {
                self.v1_pieces.extend_from_slice(&h.finalize_reset());
                self.v1_in_piece = 0;
            }
        }
    }

    fn pad_after(&self, f: &FileSpec) -> u64 {
        if self.single || self.opts.version != Version::Hybrid || !f.is_data() { return 0; }
        (self.piece_length - f.length % self.piece_length) % self.piece_length
    }

    /// Feeds the next bytes; they may span files.
    pub fn feed(&mut self, mut data: &[u8]) -> Result<(), Error> {
        while !data.is_empty() {
            if self.remaining == 0 { return Err(Error::TooMuchData); }
            let take = (self.left.min(data.len() as u64)) as usize;
            self.v1_update(&data[..take]);
            if let Some(h) = self.v2.as_mut() { h.update(&data[..take]); }
            self.left -= take as u64;
            self.remaining -= take as u64;
            data = &data[take..];
            if self.left == 0 {
                if let Some(h) = self.v2.take() { self.v2_done[self.cur] = Some(h.finish()); }
                let mut pad = self.pad_after(&self.files[self.cur]);
                const ZEROS: [u8; 4096] = [0; 4096];
                while pad > 0 { let n = pad.min(4096); self.v1_update(&ZEROS[..n as usize]); pad -= n; }
                self.start_file();
            }
        }
        Ok(())
    }

    /// The .torrent file.
    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        if self.remaining > 0 { return Err(Error::Incomplete(self.remaining)); }
        if let Some(h) = self.v1.take() {
            if self.v1_in_piece > 0 { self.v1_pieces.extend_from_slice(&h.finalize()); }
        }
        let o = &self.opts;
        let strs = |l: &[String]| Item::List(l.iter().map(|c| Item::from(c.as_str())).collect());
        let mut info = BTreeMap::new();
        info.insert(b"name".to_vec(), Item::from(o.name.as_str()));
        info.insert(b"piece length".to_vec(), Item::Int(self.piece_length as i64));
        if o.private { info.insert(b"private".to_vec(), Item::Int(1)); }
        if !o.similar.is_empty() { info.insert(b"similar".to_vec(), Item::List(o.similar.iter().map(|h| Item::from(&h.0[..])).collect())); }
        if !o.collections.is_empty() { info.insert(b"collections".to_vec(), strs(&o.collections)); }
        let props = |f: &FileSpec, with_path: bool| {
            let mut d = BTreeMap::new();
            d.insert(b"length".to_vec(), Item::Int(f.length as i64));
            let a = f.attr.encode();
            if !a.is_empty() { d.insert(b"attr".to_vec(), Item::from(a.as_str())); }
            if let Some(t) = &f.symlink_target { d.insert(b"symlink path".to_vec(), strs(t)); }
            if with_path { d.insert(b"path".to_vec(), strs(&f.path)); }
            d
        };
        if o.version.v1() {
            info.insert(b"pieces".to_vec(), Item::Bytes(core::mem::take(&mut self.v1_pieces)));
            if self.single {
                for (k, v) in props(&self.files[0], false) { info.insert(k, v); }
            } else {
                let mut list = Vec::new();
                for f in &self.files {
                    list.push(Item::Dict(props(f, true)));
                    let pad = self.pad_after(f);
                    if pad > 0 {
                        list.push(dict([(b"attr", "p".into()), (b"length", (pad as i64).into()), (b"path", Item::List(vec![".pad".into(), pad.to_string().as_str().into()]))]));
                    }
                }
                info.insert(b"files".to_vec(), Item::List(list));
            }
        }
        let mut layers = BTreeMap::new();
        if o.version.v2() {
            info.insert(b"meta version".to_vec(), Item::Int(2));
            let mut tree = BTreeMap::new();
            for (f, h) in self.files.iter().zip(&self.v2_done) {
                let mut p = props(f, false);
                if let Some(FileHashes { root: Some(root), piece_layer, .. }) = h {
                    p.insert(b"pieces root".to_vec(), Item::from(&root[..]));
                    if !piece_layer.is_empty() { layers.insert(root.to_vec(), Item::Bytes(piece_layer.concat())); }
                }
                insert(&mut tree, &f.path, Item::Dict(p));
            }
            info.insert(b"file tree".to_vec(), Item::Dict(tree));
        }
        let mut top = BTreeMap::new();
        let urls: Vec<&String> = o.trackers.iter().flatten().collect();
        if let Some(first) = urls.first() { top.insert(b"announce".to_vec(), Item::from(first.as_str())); }
        if urls.len() > 1 {
            top.insert(b"announce-list".to_vec(), Item::List(o.trackers.iter().filter(|t| !t.is_empty()).map(|t| strs(t)).collect()));
        }
        if let Some(c) = &o.comment { top.insert(b"comment".to_vec(), Item::from(c.as_str())); }
        if let Some(c) = &o.created_by { top.insert(b"created by".to_vec(), Item::from(c.as_str())); }
        if let Some(d) = o.creation_date { top.insert(b"creation date".to_vec(), Item::Int(d)); }
        if !o.web_seeds.is_empty() { top.insert(b"url-list".to_vec(), strs(&o.web_seeds)); }
        if !o.nodes.is_empty() {
            top.insert(b"nodes".to_vec(), Item::List(o.nodes.iter().map(|(h, p)| Item::List(vec![h.as_str().into(), Item::Int(*p as i64)])).collect()));
        }
        if o.version.v2() { top.insert(b"piece layers".to_vec(), Item::Dict(layers)); }
        top.insert(b"info".to_vec(), Item::Dict(info));
        Ok(Item::Dict(top).encode())
    }
}

/// Puts a file's properties at `path` in a v2 file tree. Depth is bounded by
/// [`MAX_PATH_DEPTH`] (checked in [`Creator::new`]).
fn insert(tree: &mut BTreeMap<Vec<u8>, Item>, path: &[String], props: Item) {
    let Some((first, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        tree.insert(first.as_bytes().to_vec(), dict([(b"", props)]));
    } else if let Item::Dict(d) = tree.entry(first.as_bytes().to_vec()).or_insert_with(|| Item::Dict(BTreeMap::new())) {
        insert(d, rest, props);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle;
    use crate::metainfo::Metainfo;

    fn data(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2654435761) | 1;
        (0..len).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }).collect()
    }

    /// Feeds `bytes` (in creator order) in odd chunk sizes and parses the result.
    fn make(opts: Options, specs: Vec<FileSpec>, contents: &BTreeMap<Vec<String>, Vec<u8>>, chunk: usize) -> (Vec<u8>, Metainfo) {
        let mut c = Creator::new(opts, specs).unwrap();
        let mut stream = Vec::new();
        for f in c.files() { if f.symlink_target.is_none() { stream.extend_from_slice(&contents[&f.path]); } }
        for part in stream.chunks(chunk) { c.feed(part).unwrap(); }
        let t = c.finish().unwrap();
        let m = Metainfo::parse(&t).unwrap();
        (t, m)
    }

    /// Reads every piece from the layout (padding as zeros) and checks it.
    fn check_all_pieces(m: &Metainfo, contents: &BTreeMap<Vec<String>, Vec<u8>>) {
        for p in 0..m.piece_count {
            let mut buf = Vec::new();
            for s in m.piece_files(p) {
                let f = &m.files[s.file];
                if f.attr.pad { buf.extend(core::iter::repeat(0).take(s.length as usize)); continue; }
                buf.extend_from_slice(&contents[&f.path][s.offset as usize..(s.offset + s.length) as usize]);
            }
            assert_eq!(buf.len() as u64, m.piece_size(p).unwrap());
            assert_eq!(m.verify_piece(p, &buf), Some(true), "piece {}", p);
            if !buf.is_empty() { buf[0] ^= 1; assert_eq!(m.verify_piece(p, &buf), Some(false)); }
        }
    }

    // libtorrent's test torrents v2.torrent and v2_multipiece_file.torrent were made from
    // files of zeros; we make the same torrents and must produce the same bytes.
    #[test]
    fn golden_matches_libtorrent() {
        for (name, len, fixture) in [("test64K", 65536usize, &include_bytes!("../tests/fixtures/v2.torrent")[..]), ("test1MB", 1 << 20, &include_bytes!("../tests/fixtures/v2_multipiece_file.torrent")[..])] {
            let mut o = Options::new(name, Version::Hybrid);
            o.piece_length = Some(65536);
            o.trackers = vec![vec!["http://example.com/announce".into()]];
            let mut c = Creator::new(o, vec![FileSpec::new(&[name], len as u64)]).unwrap();
            c.feed(&vec![0u8; len]).unwrap();
            assert_eq!(c.finish().unwrap(), fixture);
        }
    }

    #[test]
    fn hybrid_multi_file() {
        let sizes: [(&[&str], usize); 7] = [(&["b", "big.bin"], 200_000), (&["a.txt"], 1), (&["empty"], 0), (&["b", "exact"], 65536), (&["c", "d", "mid"], 50_000), (&["z"], 16384), (&["b", "one"], 70_000)];
        let mut contents = BTreeMap::new();
        let mut specs = Vec::new();
        for (i, (p, n)) in sizes.iter().enumerate() {
            let mut s = FileSpec::new(p, *n as u64);
            s.attr.executable = i == 1;
            contents.insert(s.path.clone(), data(*n, i as u32 + 1));
            specs.push(s);
        }
        let mut link = FileSpec::new(&["link"], 0);
        link.symlink_target = Some(vec!["b".into(), "one".into()]);
        specs.push(link);
        let mut o = Options::new("pack", Version::Hybrid);
        o.piece_length = Some(32768);
        o.trackers = vec![vec!["http://t1/".into(), "http://t2/".into()], vec!["udp://t3:1".into()]];
        o.web_seeds = vec!["https://seed/".into()];
        o.private = true;
        o.comment = Some("hi".into());
        o.created_by = Some("Swingers".into());
        o.creation_date = Some(1_700_000_000);
        o.collections = vec!["c".into()];
        let (t1, m) = make(o.clone(), specs.clone(), &contents, 7777);
        let (t2, _) = make(o.clone(), specs.clone(), &contents, 1 << 20);
        assert_eq!(t1, t2);
        assert!(m.is_hybrid() && m.multi_file && m.private);
        assert_eq!(m.name, "pack");
        assert_eq!(m.trackers, o.trackers);
        assert_eq!(m.web_seeds, o.web_seeds);
        assert_eq!((m.comment.as_deref(), m.created_by.as_deref(), m.creation_date), (Some("hi"), Some("Swingers"), Some(1_700_000_000)));
        assert_eq!(m.collections, vec![String::from("c")]);
        let real: Vec<&crate::metainfo::FileEntry> = m.files.iter().filter(|f| !f.attr.pad).collect();
        let paths: Vec<String> = real.iter().map(|f| f.path.join("/")).collect();
        assert_eq!(paths, ["a.txt", "b/big.bin", "b/exact", "b/one", "c/d/mid", "empty", "link", "z"]);
        assert!(real[0].attr.executable);
        assert_eq!(real[6].symlink_target, Some(vec![String::from("b"), String::from("one")]));
        for f in &real {
            assert_eq!(f.pieces_root, contents.get(&f.path).and_then(|d| merkle::file_root(d)));
            if f.length > 0 { assert_eq!(f.offset % 32768, 0); }
        }
        assert_eq!(m.total_length, sizes.iter().map(|s| s.1 as u64).sum::<u64>());
        assert!(m.files.iter().any(|f| f.attr.pad && f.path[0] == ".pad"));
        assert_eq!(m.piece_count as u64, sizes.iter().map(|s| (s.1 as u64).div_ceil(32768)).sum::<u64>());
        assert_eq!(m.piece_layers.len(), 4);
        check_all_pieces(&m, &contents);
    }

    #[test]
    fn v1_and_v2_only() {
        let mut contents = BTreeMap::new();
        contents.insert(vec![String::from("x")], data(100_000, 9));
        contents.insert(vec![String::from("y")], data(40_000, 10));
        let specs = vec![FileSpec::new(&["y"], 40_000), FileSpec::new(&["x"], 100_000)];
        for v in [Version::V1, Version::V2] {
            let mut o = Options::new("two", v);
            o.piece_length = Some(16384);
            let (_, m) = make(o, specs.clone(), &contents, 3000);
            assert_eq!((m.is_v1(), m.is_v2(), m.is_hybrid()), (v == Version::V1, v == Version::V2, false));
            assert!(m.files.iter().all(|f| !f.attr.pad));
            check_all_pieces(&m, &contents);
            if v == Version::V1 {
                assert_eq!(m.files[1].offset, 100_000);
                assert_eq!(m.piece_count, 140_000u32.div_ceil(16384));
            } else {
                assert_eq!(m.files[1].offset, 7 * 16384);
                assert_eq!(m.piece_count, 7 + 3);
            }
        }
        // A single-file v1 torrent.
        let mut c = Creator::new(Options::new("x", Version::V1), vec![FileSpec::new(&["x"], 100_000)]).unwrap();
        assert_eq!(c.piece_length(), 16384);
        c.feed(&contents[&vec![String::from("x")]]).unwrap();
        let m = Metainfo::parse(&c.finish().unwrap()).unwrap();
        assert!(!m.multi_file);
        check_all_pieces(&m, &contents);
    }

    #[test]
    fn hybrid_halves_must_agree() {
        let mut o = Options::new("h", Version::Hybrid);
        o.piece_length = Some(16384);
        let mut c = Creator::new(o, vec![FileSpec::new(&["a"], 100), FileSpec::new(&["b"], 100)]).unwrap();
        c.feed(&[1u8; 200]).unwrap();
        let t = c.finish().unwrap();
        let top = crate::bencode::decode(&t).unwrap();
        let info = top.get(b"info").unwrap().to_item();
        let Item::Dict(mut d) = info else { panic!() };
        // Drop the padding files: the v1 layout no longer matches the v2 one.
        if let Some(Item::List(files)) = d.get_mut(&b"files"[..]) { files.retain(|f| f.get(b"attr").is_none()); }
        d.insert(b"pieces".to_vec(), Item::Bytes(vec![0u8; 20]));
        assert_eq!(Metainfo::from_info_bytes(&Item::Dict(d.clone()).encode()), Err(crate::metainfo::Error::HybridMismatch));
        // Rename a v1 file: mismatch too.
        let ok = Metainfo::from_info_bytes(top.get(b"info").unwrap().raw).unwrap();
        assert!(ok.is_hybrid());
        let mut renamed = top.get(b"info").unwrap().raw.to_vec();
        let at = renamed.windows(6).position(|w| w == b"l1:aee").unwrap();
        renamed[at + 3] = b'q';
        assert_eq!(Metainfo::from_info_bytes(&renamed), Err(crate::metainfo::Error::HybridMismatch));
    }

    #[test]
    fn auto_piece_lengths() {
        assert_eq!(auto_piece_length(0), 16384);
        assert_eq!(auto_piece_length(10 << 20), 16384);
        assert_eq!(auto_piece_length(1 << 30), 1 << 20);
        assert_eq!(auto_piece_length(4_700_000_000), 4 << 20);
        assert_eq!(auto_piece_length(1 << 50), 16 << 20);
        for total in [100u64 << 20, 3 << 30, 20 << 30] {
            let n = total.div_ceil(auto_piece_length(total));
            assert!(n > 1000 && n <= 2000, "{} -> {}", total, n);
        }
    }

    #[test]
    fn refusals() {
        let o = Options::new("t", Version::Hybrid);
        assert_eq!(Creator::new(o.clone(), vec![]).err(), Some(Error::NoFiles));
        assert_eq!(Creator::new(o.clone(), vec![FileSpec::new(&["a"], 0)]).err(), Some(Error::NoFiles));
        for bad in [&["..", "a"][..], &["a/b"], &[""], &["a\0"], &[]] {
            assert_eq!(Creator::new(o.clone(), vec![FileSpec::new(bad, 5)]).err(), Some(Error::BadPath));
        }
        assert_eq!(Creator::new(Options::new("../x", Version::V1), vec![FileSpec::new(&["a"], 5)]).err(), Some(Error::BadPath));
        assert_eq!(Creator::new(o.clone(), vec![FileSpec::new(&["a"], 5), FileSpec::new(&["a"], 6)]).err(), Some(Error::DuplicatePath));
        assert_eq!(Creator::new(o.clone(), vec![FileSpec::new(&["a", "b"], 5), FileSpec::new(&["a"], 6)]).err(), Some(Error::DuplicatePath));
        assert_eq!(Creator::new(o.clone(), vec![FileSpec::new(&["a"], u64::MAX), FileSpec::new(&["b"], 6)]).err(), Some(Error::TooLarge));
        let mut p = o.clone();
        p.piece_length = Some(20000);
        assert_eq!(Creator::new(p, vec![FileSpec::new(&["a"], 5)]).err(), Some(Error::BadPieceLength));
        let mut c = Creator::new(o.clone(), vec![FileSpec::new(&["a"], 5)]).unwrap();
        c.feed(b"abc").unwrap();
        assert_eq!(c.remaining(), 2);
        assert_eq!(c.clone().finish().err(), Some(Error::Incomplete(2)));
        assert_eq!(c.feed(b"defg"), Err(Error::TooMuchData));
    }
}
