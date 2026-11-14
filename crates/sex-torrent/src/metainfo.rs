// SPDX-License-Identifier: GPL-3.0-only
//! Reading .torrent files: v1 (BEP 3), v2 (BEP 52) and hybrids that are both.
//!
//! [`Metainfo::parse`] takes the bytes of a .torrent file; [`Metainfo::from_info_bytes`]
//! takes just the `info` dictionary, as fetched from peers for a magnet link (BEP 9).
//! The info-hashes are computed over the `info` dictionary exactly as it appears in the
//! input (SHA-1 for v1, SHA-256 for v2), never over a re-encoding.
//!
//! The result is one flat list of [`FileEntry`] in torrent order, each with its byte
//! offset in the torrent's "piece space". For v1 the files are back to back; for v2 every
//! file starts on a piece boundary; a hybrid uses BEP 47 padding files so that both
//! agree, and we check that they do (same files, same order, same alignment). Padding
//! files stay in the list, flagged, so the v1 offsets are right; storage skips them.
//!
//! File names come from the network and are hostile until proven otherwise: each path
//! component is sanitized (`..`, `.` and empty components dropped, `/`, `\`, NUL and other
//! control characters replaced by `_`, invalid UTF-8 replaced), a path that ends up
//! empty is refused, as are duplicate paths and a file that is also a directory.
//!
//! Also read: trackers (`announce`, `announce-list` tiers, BEP 12), web seeds
//! (`url-list`, BEP 19), DHT `nodes` (BEP 5), `private` (BEP 27), comment, creator and
//! date, `similar` and `collections` (BEP 38), BEP 47 attributes (`p` pad, `x`
//! executable, `h` hidden, `l` symlink, with `symlink path`) and per-file `sha1`.
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::ops::Range;

use sha1::{Digest, Sha1};

use crate::bencode::{self, Value};
use crate::merkle::{self, Hash};
use crate::{InfoHashV1, InfoHashV2, InfoHashes};

/// Most files in one torrent.
pub const MAX_FILES: usize = 500_000;
/// Most components in one path (directory depth plus the file name).
pub const MAX_PATH_DEPTH: usize = 48;
/// Longest path component kept, in bytes (longer ones are cut at a character boundary).
pub const MAX_COMPONENT_BYTES: usize = 255;
/// Largest piece length accepted (512 MiB).
pub const MAX_PIECE_LENGTH: u64 = 1 << 29;
/// Caps on the lists outside the file list.
pub const MAX_TRACKERS: usize = 256;
pub const MAX_WEB_SEEDS: usize = 256;
pub const MAX_NODES: usize = 256;
pub const MAX_HINTS: usize = 1024;

/// Why a torrent was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Bencode(bencode::Error),
    /// The file or the `info` value is not a dictionary.
    NotDict,
    MissingField(&'static str),
    /// A field has the wrong type or an impossible value.
    BadField(&'static str),
    /// `meta version` other than 2.
    UnsupportedVersion(i64),
    /// Piece length zero, too large, or (v2) not a power of two of at least 16 KiB.
    BadPieceLength,
    TooManyFiles,
    PathTooDeep,
    /// A path with nothing left after sanitizing.
    BadPath,
    /// Two files with the same path, or a file that is also a directory.
    DuplicatePath,
    /// No data at all.
    Empty,
    /// Sizes overflow, or more pieces than fit in a `u32`.
    TooLarge,
    /// `pieces` does not have 20 bytes per piece.
    PieceCount,
    /// The v2 `file tree` is malformed.
    BadFileTree,
    /// The v1 and v2 halves of a hybrid describe different content.
    HybridMismatch,
    /// A piece layer is missing, has the wrong length or does not hash to its root.
    PieceLayer,
}

impl From<bencode::Error> for Error { fn from(e: bencode::Error) -> Self { Error::Bencode(e) } }

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "invalid torrent: {:?}", self) }
}

/// BEP 47 file attributes (`attr`). Unknown letters are ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileAttr {
    /// `p`: a padding file of zeros, never written to disk.
    pub pad: bool,
    /// `x`
    pub executable: bool,
    /// `h`
    pub hidden: bool,
    /// `l`: a symlink to [`FileEntry::symlink_target`].
    pub symlink: bool,
}

impl FileAttr {
    pub fn parse(s: &[u8]) -> FileAttr {
        FileAttr { pad: s.contains(&b'p'), executable: s.contains(&b'x'), hidden: s.contains(&b'h'), symlink: s.contains(&b'l') }
    }
    /// The `attr` string, empty when no flag is set.
    pub fn encode(&self) -> String {
        let mut s = String::new();
        for (on, c) in [(self.pad, 'p'), (self.executable, 'x'), (self.hidden, 'h'), (self.symlink, 'l')] { if on { s.push(c); } }
        s
    }
}

/// One file of the torrent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Sanitized path components, relative to the torrent's directory (see
    /// [`Metainfo::multi_file`]).
    pub path: Vec<String>,
    pub length: u64,
    /// Where the file starts in the piece space.
    pub offset: u64,
    pub attr: FileAttr,
    /// For a symlink: its target, relative to the torrent root (sanitized).
    pub symlink_target: Option<Vec<String>>,
    /// BEP 47 `sha1` of the file alone (a hint, not for verification).
    pub sha1: Option<[u8; 20]>,
    /// The v2 merkle root (non-empty files of v2 and hybrid torrents).
    pub pieces_root: Option<Hash>,
    /// `mtime`, if given.
    pub mtime: Option<i64>,
}

/// A run of bytes of one file inside one piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSlice {
    /// Index into [`Metainfo::files`].
    pub file: usize,
    /// Offset within the file.
    pub offset: u64,
    pub length: u64,
}

/// A parsed torrent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metainfo {
    pub info_hashes: InfoHashes,
    /// Display name, and the directory name of a multi-file torrent (sanitized).
    pub name: String,
    pub piece_length: u64,
    /// Bytes of real content (padding files not counted).
    pub total_length: u64,
    /// End of the piece space: v1 total including padding, or the end of the last v2 file.
    pub space_length: u64,
    pub piece_count: u32,
    /// Whether files live under a directory called [`Metainfo::name`]. A single-file
    /// torrent has one file whose path is the file name.
    pub multi_file: bool,
    pub files: Vec<FileEntry>,
    /// SHA-1 per piece (v1 and hybrid).
    pub v1_pieces: Vec<[u8; 20]>,
    /// v2 piece layers by pieces root; for files larger than one piece. Every entry has
    /// been checked against its root.
    pub piece_layers: BTreeMap<Hash, Vec<Hash>>,
    /// Tracker tiers (BEP 12). From `announce-list` when present, else `announce`.
    pub trackers: Vec<Vec<String>>,
    /// `url-list` web seeds (BEP 19).
    pub web_seeds: Vec<String>,
    /// DHT bootstrap nodes: host and port (BEP 5).
    pub nodes: Vec<(String, u16)>,
    pub private: bool,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    /// Seconds since the Unix epoch.
    pub creation_date: Option<i64>,
    /// BEP 38 `similar` info-hashes (inside and outside `info`, merged).
    pub similar: Vec<InfoHashV1>,
    /// BEP 38 `collections`.
    pub collections: Vec<String>,
    /// The `info` dictionary exactly as received: what ut_metadata serves.
    pub info_bytes: Vec<u8>,
}

impl Metainfo {
    /// Parses a .torrent file. v2 piece layers must be present for every file larger
    /// than one piece, and must match the roots.
    pub fn parse(torrent: &[u8]) -> Result<Metainfo, Error> {
        let top = bencode::decode(torrent)?;
        top.dict().ok_or(Error::NotDict)?;
        let info = top.get(b"info").ok_or(Error::MissingField("info"))?;
        let mut m = parse_info(info)?;
        m.read_outer(&top);
        if let Some(layers) = top.get(b"piece layers") {
            for (root, v) in layers.dict().ok_or(Error::BadField("piece layers"))? {
                let (Ok(root), Some(bytes)) = (<[u8; 32]>::try_from(*root), v.bytes()) else { return Err(Error::PieceLayer) };
                // Layers for roots that are not ours (or need none) are ignored.
                if m.needs_layer(&root) { m.set_piece_layer(root, merkle::split_hashes(bytes).ok_or(Error::PieceLayer)?)?; }
            }
        }
        if !m.has_all_piece_layers() { return Err(Error::PieceLayer); }
        Ok(m)
    }

    /// Builds a torrent from a bare `info` dictionary (metadata fetched for a magnet).
    /// No trackers, and no v2 piece layers yet: fetch them with hash requests and add
    /// them with [`Metainfo::set_piece_layer`].
    pub fn from_info_bytes(info: &[u8]) -> Result<Metainfo, Error> {
        parse_info(&bencode::decode(info)?)
    }

    pub fn is_v1(&self) -> bool { self.info_hashes.v1().is_some() }
    pub fn is_v2(&self) -> bool { self.info_hashes.v2().is_some() }
    pub fn is_hybrid(&self) -> bool { matches!(self.info_hashes, InfoHashes::Hybrid(..)) }

    fn needs_layer(&self, root: &Hash) -> bool {
        self.files.iter().any(|f| f.pieces_root.as_ref() == Some(root) && f.length > self.piece_length)
    }

    /// Whether every v2 file larger than a piece has its piece layer.
    pub fn has_all_piece_layers(&self) -> bool {
        self.files.iter().all(|f| f.length <= self.piece_length || f.pieces_root.map_or(true, |r| self.piece_layers.contains_key(&r)))
    }

    /// Adds a piece layer (e.g. fetched with hash requests) after checking it against
    /// the root of a file of this torrent.
    pub fn set_piece_layer(&mut self, root: Hash, layer: Vec<Hash>) -> Result<(), Error> {
        let f = self.files.iter().find(|f| f.pieces_root == Some(root) && f.length > self.piece_length).ok_or(Error::PieceLayer)?;
        if !merkle::verify_piece_layer(&layer, self.piece_length, f.length, &root) { return Err(Error::PieceLayer); }
        self.piece_layers.insert(root, layer);
        Ok(())
    }

    /// The byte range of a piece in the piece space (for v2-only torrents a file's last
    /// piece ends with the file).
    pub fn piece_range(&self, piece: u32) -> Option<Range<u64>> {
        if piece >= self.piece_count { return None; }
        let start = piece as u64 * self.piece_length;
        let mut end = start.saturating_add(self.piece_length).min(self.space_length);
        if !self.is_v1() {
            let f = &self.files[self.file_at(start)?];
            end = end.min(f.offset + f.length);
        }
        Some(start..end)
    }

    /// The size of a piece in bytes.
    pub fn piece_size(&self, piece: u32) -> Option<u64> { self.piece_range(piece).map(|r| r.end - r.start) }

    /// Index of the non-empty file covering byte `pos` of the piece space.
    fn file_at(&self, pos: u64) -> Option<usize> {
        let i = self.files.partition_point(|f| f.offset + f.length <= pos);
        let f = self.files.get(i)?;
        (f.offset <= pos && f.length > 0).then_some(i)
    }

    /// The file pieces a piece is made of, in order (padding files included).
    pub fn piece_files(&self, piece: u32) -> Vec<FileSlice> {
        let Some(r) = self.piece_range(piece) else { return Vec::new() };
        let mut out = Vec::new();
        let mut i = self.files.partition_point(|f| f.offset + f.length <= r.start);
        while let Some(f) = self.files.get(i) {
            if f.offset >= r.end { break; }
            if f.length > 0 {
                let a = r.start.max(f.offset);
                let b = r.end.min(f.offset + f.length);
                out.push(FileSlice { file: i, offset: a - f.offset, length: b - a });
            }
            i += 1;
        }
        out
    }

    /// The pieces a file touches (empty for empty files).
    pub fn file_pieces(&self, file: usize) -> Range<u32> {
        let Some(f) = self.files.get(file) else { return 0..0 };
        let first = (f.offset / self.piece_length) as u32;
        if f.length == 0 { return first..first; }
        first..((f.offset + f.length).div_ceil(self.piece_length) as u32)
    }

    /// The SHA-1 of a piece (v1 and hybrid).
    pub fn v1_piece_hash(&self, piece: u32) -> Option<[u8; 20]> { self.v1_pieces.get(piece as usize).copied() }

    /// For v2: the file a piece belongs to and the piece's index within that file.
    pub fn v2_piece(&self, piece: u32) -> Option<(usize, u32)> {
        if !self.is_v2() || piece >= self.piece_count { return None; }
        let i = self.file_at(piece as u64 * self.piece_length)?;
        let f = &self.files[i];
        if f.attr.pad || f.pieces_root.is_none() { return None; }
        Some((i, piece - (f.offset / self.piece_length) as u32))
    }

    /// The v2 hash this piece must have: the file root for a single-piece file, else the
    /// piece layer entry (`None` while the layer is unknown).
    pub fn v2_piece_hash(&self, piece: u32) -> Option<Hash> {
        let (i, k) = self.v2_piece(piece)?;
        let f = &self.files[i];
        let root = f.pieces_root?;
        if f.length <= self.piece_length { return Some(root); }
        self.piece_layers.get(&root)?.get(k as usize).copied()
    }

    /// Checks a piece's bytes (the whole piece range, padding zeros included) with every
    /// hash we know: `Some(true)` only if all known hashes match, `None` if no hash is
    /// known yet (a v2 file whose piece layer has not been fetched).
    pub fn verify_piece(&self, piece: u32, data: &[u8]) -> Option<bool> {
        if Some(data.len() as u64) != self.piece_size(piece) { return Some(false); }
        let mut known = None;
        if let Some(h) = self.v1_piece_hash(piece) {
            if <[u8; 20]>::from(Sha1::digest(data)) != h { return Some(false); }
            known = Some(true);
        }
        if let (Some((i, k)), Some(want)) = (self.v2_piece(piece), self.v2_piece_hash(piece)) {
            let f = &self.files[i];
            let n = (f.length - k as u64 * self.piece_length).min(self.piece_length) as usize;
            let got = merkle::piece_hash(data.get(..n)?, self.piece_length, f.length)?;
            if got != want { return Some(false); }
            known = Some(true);
        }
        known
    }

    /// Re-encodes as a .torrent file, with the `info` dictionary byte for byte as
    /// received (so the info-hashes stay the same). For saving a magnet's metadata.
    pub fn to_torrent(&self) -> Vec<u8> {
        use bencode::Item;
        let s = |x: &str| Item::from(x);
        let mut entries: BTreeMap<&[u8], Vec<u8>> = BTreeMap::new();
        if let Some(first) = self.trackers.iter().flatten().next() { entries.insert(b"announce", s(first).encode()); }
        if self.trackers.iter().map(|t| t.len()).sum::<usize>() > 1 {
            let tiers = self.trackers.iter().map(|t| Item::List(t.iter().map(|u| s(u)).collect())).collect();
            entries.insert(b"announce-list", Item::List(tiers).encode());
        }
        if let Some(c) = &self.comment { entries.insert(b"comment", s(c).encode()); }
        if let Some(c) = &self.created_by { entries.insert(b"created by", s(c).encode()); }
        if let Some(d) = self.creation_date { entries.insert(b"creation date", Item::Int(d).encode()); }
        entries.insert(b"info", self.info_bytes.clone());
        if !self.piece_layers.is_empty() {
            let d = self.piece_layers.iter().map(|(r, l)| (r.to_vec(), Item::Bytes(l.concat()))).collect();
            entries.insert(b"piece layers", Item::Dict(d).encode());
        }
        if !self.web_seeds.is_empty() { entries.insert(b"url-list", Item::List(self.web_seeds.iter().map(|u| s(u)).collect()).encode()); }
        if !self.nodes.is_empty() {
            let n = self.nodes.iter().map(|(h, p)| Item::List(alloc::vec![s(h), Item::Int(*p as i64)])).collect();
            entries.insert(b"nodes", Item::List(n).encode());
        }
        let mut out = alloc::vec![b'd'];
        for (k, v) in entries {
            out.extend_from_slice(k.len().to_string().as_bytes());
            out.push(b':');
            out.extend_from_slice(k);
            out.extend_from_slice(&v);
        }
        out.push(b'e');
        out
    }

    /// Reads the fields outside `info`.
    fn read_outer(&mut self, top: &Value) {
        if let Some(list) = top.get(b"announce-list").and_then(|v| v.list()) {
            let mut seen = BTreeSet::new();
            for tier in list.iter().filter_map(|t| t.list()) {
                let urls: Vec<String> = tier.iter().filter_map(|u| u.str()).map(|u| u.trim().to_string())
                    .filter(|u| !u.is_empty() && seen.len() < MAX_TRACKERS && seen.insert(u.clone())).collect();
                if !urls.is_empty() { self.trackers.push(urls); }
            }
        }
        if self.trackers.is_empty() {
            if let Some(u) = top.get(b"announce").and_then(|v| v.str()).map(str::trim).filter(|u| !u.is_empty()) {
                self.trackers.push(alloc::vec![u.to_string()]);
            }
        }
        match top.get(b"url-list") {
            Some(v) if v.bytes().is_some() => self.web_seeds.extend(v.str().filter(|u| !u.is_empty()).map(String::from)),
            Some(v) => self.web_seeds.extend(v.list().unwrap_or(&[]).iter().filter_map(|u| u.str()).filter(|u| !u.is_empty()).take(MAX_WEB_SEEDS).map(String::from)),
            None => {}
        }
        for n in top.get(b"nodes").and_then(|v| v.list()).unwrap_or(&[]).iter().take(MAX_NODES) {
            let l = n.list().unwrap_or(&[]);
            if let (Some(h), Some(p)) = (l.first().and_then(|h| h.str()), l.get(1).and_then(|p| p.int())) {
                if let Ok(p) = u16::try_from(p) { if !h.is_empty() { self.nodes.push((h.to_string(), p)); } }
            }
        }
        self.comment = text(top, b"comment.utf-8").or_else(|| text(top, b"comment"));
        self.created_by = text(top, b"created by.utf-8").or_else(|| text(top, b"created by"));
        self.creation_date = top.get(b"creation date").and_then(|v| v.int());
        read_hints(top, &mut self.similar, &mut self.collections);
    }
}

fn text(d: &Value, key: &[u8]) -> Option<String> {
    d.get(key)?.bytes().map(|b| String::from_utf8_lossy(b).into_owned())
}

fn read_hints(d: &Value, similar: &mut Vec<InfoHashV1>, collections: &mut Vec<String>) {
    for v in d.get(b"similar").and_then(|v| v.list()).unwrap_or(&[]) {
        if let Some(Ok(h)) = v.bytes().map(<[u8; 20]>::try_from) {
            if similar.len() < MAX_HINTS && !similar.contains(&InfoHashV1(h)) { similar.push(InfoHashV1(h)); }
        }
    }
    for v in d.get(b"collections").and_then(|v| v.list()).unwrap_or(&[]) {
        if let Some(s) = v.str() {
            if collections.len() < MAX_HINTS && !collections.iter().any(|c| c == s) { collections.push(s.to_string()); }
        }
    }
}

/// Sanitizes one path component; `None` means "drop it" (empty, `.` or `..`).
pub fn sanitize_component(raw: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(s.len().min(MAX_COMPONENT_BYTES));
    for c in s.chars() {
        let c = if c.is_control() || c == '/' || c == '\\' { '_' } else { c };
        if out.len() + c.len_utf8() > MAX_COMPONENT_BYTES { break; }
        out.push(c);
    }
    if out.is_empty() || out == "." || out == ".." { None } else { Some(out) }
}

/// Sanitizes a list of components; refuses a path with nothing left or too deep.
fn sanitize_path(list: &[Value]) -> Result<Vec<String>, Error> {
    if list.len() > MAX_PATH_DEPTH { return Err(Error::PathTooDeep); }
    let mut out = Vec::with_capacity(list.len());
    for c in list { out.extend(sanitize_component(c.bytes().ok_or(Error::BadField("path"))?)); }
    if out.is_empty() { Err(Error::BadPath) } else { Ok(out) }
}

fn length_of(d: &Value) -> Result<u64, Error> {
    let n = d.get(b"length").ok_or(Error::MissingField("length"))?.int().ok_or(Error::BadField("length"))?;
    u64::try_from(n).map_err(|_| Error::BadField("length"))
}

fn sha1_of(d: &Value) -> Option<[u8; 20]> { d.get(b"sha1")?.bytes()?.try_into().ok() }

fn symlink_of(d: &Value, attr: &FileAttr) -> Result<Option<Vec<String>>, Error> {
    if !attr.symlink { return Ok(None); }
    let l = d.get(b"symlink path").and_then(|v| v.list()).ok_or(Error::MissingField("symlink path"))?;
    sanitize_path(l).map(Some)
}

fn entry(path: Vec<String>, length: u64, d: &Value, attr: FileAttr) -> Result<FileEntry, Error> {
    Ok(FileEntry { path, length, offset: 0, attr, symlink_target: symlink_of(d, &attr)?, sha1: sha1_of(d), pieces_root: None, mtime: d.get(b"mtime").and_then(|v| v.int()) })
}

/// The v1 file list, and whether it is a multi-file torrent.
fn v1_files(info: &Value, name: &str) -> Result<(Vec<FileEntry>, bool), Error> {
    let Some(files) = info.get(b"files") else {
        let attr = FileAttr::parse(info.get(b"attr").and_then(|v| v.bytes()).unwrap_or(b""));
        let length = if attr.symlink && info.get(b"length").is_none() { 0 } else { length_of(info)? };
        return Ok((alloc::vec![entry(alloc::vec![name.to_string()], length, info, attr)?], false));
    };
    let list = files.list().ok_or(Error::BadField("files"))?;
    if list.len() > MAX_FILES { return Err(Error::TooManyFiles); }
    let mut out = Vec::with_capacity(list.len());
    for f in list {
        f.dict().ok_or(Error::BadField("files"))?;
        let mut attr = FileAttr::parse(f.get(b"attr").and_then(|v| v.bytes()).unwrap_or(b""));
        let length = if attr.symlink && f.get(b"length").is_none() { 0 } else { length_of(f)? };
        let raw = f.get(b"path.utf-8").and_then(|v| v.list()).or_else(|| f.get(b"path").and_then(|v| v.list()));
        // Old BitComet padding files are recognised by name.
        if raw.and_then(|p| p.last()).and_then(|c| c.bytes()).is_some_and(|c| c.starts_with(b"_____padding_file_")) { attr.pad = true; }
        let path = match raw {
            Some(p) if !attr.pad => sanitize_path(p)?,
            Some(p) => sanitize_path(p).unwrap_or_else(|_| alloc::vec![".pad".into(), length.to_string()]),
            None if attr.pad => alloc::vec![".pad".into(), length.to_string()],
            None => return Err(Error::MissingField("path")),
        };
        out.push(entry(path, length, f, attr)?);
    }
    if out.is_empty() { return Err(Error::Empty); }
    Ok((out, true))
}

/// Walks the v2 file tree in order. Recursion is bounded by [`MAX_PATH_DEPTH`].
fn walk_tree(node: &Value, path: &mut Vec<String>, depth: usize, out: &mut Vec<FileEntry>) -> Result<(), Error> {
    let d = node.dict().ok_or(Error::BadFileTree)?;
    if depth >= MAX_PATH_DEPTH { return Err(Error::PathTooDeep); }
    for (key, child) in d {
        if key.is_empty() { return Err(Error::BadFileTree); }
        let pushed = match sanitize_component(key) { Some(c) => { path.push(c); true } None => false };
        match child.get(b"") {
            Some(props) => {
                if child.dict().map_or(0, |d| d.len()) != 1 || props.dict().is_none() { return Err(Error::BadFileTree); }
                let attr = FileAttr::parse(props.get(b"attr").and_then(|v| v.bytes()).unwrap_or(b""));
                if attr.pad { return Err(Error::BadFileTree); }
                let length = if attr.symlink && props.get(b"length").is_none() { 0 } else { length_of(props)? };
                if path.is_empty() { return Err(Error::BadPath); }
                let mut e = entry(path.clone(), length, props, attr)?;
                if length > 0 && !attr.symlink {
                    let root = props.get(b"pieces root").and_then(|v| v.bytes()).ok_or(Error::MissingField("pieces root"))?;
                    e.pieces_root = Some(root.try_into().map_err(|_| Error::BadField("pieces root"))?);
                }
                if out.len() >= MAX_FILES { return Err(Error::TooManyFiles); }
                out.push(e);
            }
            None => walk_tree(child, path, depth + 1, out)?,
        }
        if pushed { path.pop(); }
    }
    Ok(())
}

fn parse_info(info: &Value) -> Result<Metainfo, Error> {
    info.dict().ok_or(Error::NotDict)?;
    let v2 = match info.get(b"meta version") {
        None => false,
        Some(v) => match v.int() { Some(2) => true, Some(n) => return Err(Error::UnsupportedVersion(n)), None => return Err(Error::BadField("meta version")) },
    };
    let pieces = info.get(b"pieces");
    let v1 = pieces.is_some();
    if !v1 && !v2 { return Err(Error::MissingField("pieces")); }
    let pl = info.get(b"piece length").ok_or(Error::MissingField("piece length"))?.int().ok_or(Error::BadField("piece length"))?;
    let pl = u64::try_from(pl).map_err(|_| Error::BadPieceLength)?;
    if pl == 0 || pl > MAX_PIECE_LENGTH || (v2 && merkle::piece_height(pl).is_none()) { return Err(Error::BadPieceLength); }
    let raw_name = info.get(b"name.utf-8").or_else(|| info.get(b"name")).ok_or(Error::MissingField("name"))?;
    let name = sanitize_component(raw_name.bytes().ok_or(Error::BadField("name"))?).unwrap_or_else(|| "_".into());

    let v1_part = if v1 { Some(v1_files(info, &name)?) } else { None };
    let v2_part = if v2 {
        let tree = info.get(b"file tree").ok_or(Error::MissingField("file tree"))?;
        let mut files = Vec::new();
        walk_tree(tree, &mut Vec::new(), 0, &mut files)?;
        if files.is_empty() { return Err(Error::Empty); }
        let multi = !(files.len() == 1 && files[0].path.len() == 1 && tree.dict().map_or(0, |d| d.len()) == 1);
        // Align: each non-empty file starts on a piece boundary.
        let mut next: u64 = 0;
        for f in files.iter_mut() {
            f.offset = next.checked_mul(pl).ok_or(Error::TooLarge)?;
            next = next.checked_add(f.length.div_ceil(pl)).ok_or(Error::TooLarge)?;
        }
        next.checked_mul(pl).ok_or(Error::TooLarge)?;
        Some((files, multi, next))
    } else { None };

    let (files, multi_file, piece_count, space_length) = match (v1_part, v2_part) {
        (Some((mut files, multi)), v2p) => {
            let mut off: u64 = 0;
            for f in files.iter_mut() { f.offset = off; off = off.checked_add(f.length).ok_or(Error::TooLarge)?; }
            let count = off.div_ceil(pl);
            if let Some((v2files, v2multi, v2count)) = v2p {
                let real: Vec<&mut FileEntry> = files.iter_mut().filter(|f| !f.attr.pad).collect();
                if v2multi != multi || v2count != count || real.len() != v2files.len() { return Err(Error::HybridMismatch); }
                for (a, b) in real.into_iter().zip(v2files) {
                    if a.path != b.path || a.length != b.length || a.attr.symlink != b.attr.symlink || (a.length > 0 && a.offset != b.offset) {
                        return Err(Error::HybridMismatch);
                    }
                    a.pieces_root = b.pieces_root;
                }
            }
            (files, multi, count, off)
        }
        (None, Some((files, multi, count))) => {
            let end = files.iter().map(|f| f.offset + f.length).max().unwrap_or(0);
            (files, multi, count, end)
        }
        (None, None) => return Err(Error::MissingField("pieces")),
    };
    if piece_count == 0 { return Err(Error::Empty); }
    let piece_count = u32::try_from(piece_count).map_err(|_| Error::TooLarge)?;
    check_paths(&files)?;

    let v1_pieces = match pieces {
        Some(p) => {
            let p = p.bytes().ok_or(Error::BadField("pieces"))?;
            if p.len() as u64 != piece_count as u64 * 20 { return Err(Error::PieceCount); }
            p.chunks_exact(20).map(|c| { let mut h = [0u8; 20]; h.copy_from_slice(c); h }).collect()
        }
        None => Vec::new(),
    };
    let h1 = v1.then(|| InfoHashV1(Sha1::digest(info.raw).into()));
    let h2 = v2.then(|| InfoHashV2(merkle::sha256(info.raw)));
    let info_hashes = match (h1, h2) {
        (Some(a), Some(b)) => InfoHashes::Hybrid(a, b),
        (Some(a), None) => InfoHashes::V1(a),
        (None, Some(b)) => InfoHashes::V2(b),
        (None, None) => return Err(Error::MissingField("pieces")),
    };
    let mut m = Metainfo {
        info_hashes, name, piece_length: pl,
        total_length: files.iter().filter(|f| !f.attr.pad).map(|f| f.length).sum(),
        space_length, piece_count, multi_file, files, v1_pieces,
        piece_layers: BTreeMap::new(), trackers: Vec::new(), web_seeds: Vec::new(), nodes: Vec::new(),
        private: info.get(b"private").and_then(|v| v.int()) == Some(1),
        comment: None, created_by: None, creation_date: None, similar: Vec::new(), collections: Vec::new(),
        info_bytes: info.raw.to_vec(),
    };
    read_hints(info, &mut m.similar, &mut m.collections);
    Ok(m)
}

/// Refuses duplicate paths and a file that is also a directory (padding files exempt).
fn check_paths(files: &[FileEntry]) -> Result<(), Error> {
    let mut seen: BTreeSet<&[String]> = BTreeSet::new();
    for f in files.iter().filter(|f| !f.attr.pad) {
        if !seen.insert(&f.path) { return Err(Error::DuplicatePath); }
    }
    for f in files.iter().filter(|f| !f.attr.pad) {
        for n in 1..f.path.len() { if seen.contains(&f.path[..n]) { return Err(Error::DuplicatePath); } }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bencode::{dict, Item};
    use crate::{from_hex, to_hex};
    use alloc::vec;

    const BASE: &[u8] = include_bytes!("../tests/fixtures/base.torrent");
    const PAD_FILE: &[u8] = include_bytes!("../tests/fixtures/pad_file.torrent");
    const SAMPLE: &[u8] = include_bytes!("../tests/fixtures/sample.torrent");
    const V2: &[u8] = include_bytes!("../tests/fixtures/v2.torrent");
    const V2_MULTIPIECE: &[u8] = include_bytes!("../tests/fixtures/v2_multipiece_file.torrent");
    const V2_SYMLINKS: &[u8] = include_bytes!("../tests/fixtures/v2_symlinks.torrent");
    const V2_NO_POW2: &[u8] = include_bytes!("../tests/fixtures/v2_no_power2_piece.torrent");
    const V2_ZERO_ROOT: &[u8] = include_bytes!("../tests/fixtures/v2_zero_root.torrent");
    const V2_BAD_NAME: &[u8] = include_bytes!("../tests/fixtures/v2_invalid_filename.torrent");
    const V2_BAD_PAD: &[u8] = include_bytes!("../tests/fixtures/v2_invalid_pad_file.torrent");

    fn s(x: &str) -> String { x.to_string() }

    // Fixtures are libtorrent's test torrents; the expected hashes were computed
    // independently (SHA-1/SHA-256 of the info span).
    #[test]
    fn v1_single_file() {
        let m = Metainfo::parse(BASE).unwrap();
        assert_eq!(m.info_hashes, InfoHashes::V1(InfoHashV1(from_hex("c0fda1edafdbdbb96443424e0b3899af7159d10e").unwrap())));
        assert_eq!((m.name.as_str(), m.piece_length, m.total_length, m.piece_count, m.multi_file), ("temp", 16384, 425, 1, false));
        assert_eq!(m.files[0].path, vec![s("temp")]);
        assert_eq!(m.created_by.as_deref(), Some("libtorrent"));
        assert_eq!(m.creation_date, Some(1359599503));
        assert_eq!(to_hex(&m.v1_pieces[0]), "829ebc8c26bec74a579b7ddc4134752cb7bc9187");
        assert_eq!(m.piece_files(0), vec![FileSlice { file: 0, offset: 0, length: 425 }]);
        assert_eq!(m.to_torrent(), BASE);
    }

    #[test]
    fn v1_padding_and_attrs() {
        let m = Metainfo::parse(SAMPLE).unwrap();
        assert_eq!(to_hex(&m.info_hashes.v1().unwrap().0), "58d8d15a4eb3bd9afabc9cee2564f78192777edb");
        assert_eq!(m.files.len(), 3);
        assert!(m.files[1].attr.pad && !m.files[0].attr.pad);
        assert_eq!(m.files[2].offset, 16384);
        assert_eq!(m.files[2].sha1, Some(*b"abababababababababab"));
        assert_eq!((m.total_length, m.space_length, m.piece_count), (45, 16404, 2));
        assert_eq!(m.trackers, vec![vec![s("udp://tracker.opentracker.com:80/announce")], vec![s("tracker.publicbt.com:80/announce")]]);
        assert_eq!(m.comment.as_deref(), Some("sample comment"));
        assert_eq!(m.piece_files(0).len(), 2);
        assert_eq!(m.piece_files(1), vec![FileSlice { file: 2, offset: 0, length: 20 }]);
        assert_eq!(m.piece_size(1), Some(20));
        assert_eq!(m.file_pieces(2), 1..2);
        assert_eq!(Metainfo::parse(&m.to_torrent()).unwrap(), m);
        // A BitComet-style padding file is recognised by its name.
        let p = Metainfo::parse(PAD_FILE).unwrap();
        assert!(p.files[1].attr.pad);
        assert_eq!(p.total_length, 45);
    }

    #[test]
    fn hybrid_single_files() {
        let m = Metainfo::parse(V2).unwrap();
        assert!(m.is_hybrid());
        assert_eq!(to_hex(&m.info_hashes.v1().unwrap().0), "c14199bbec64d0e9e439aa3b6b7639e666b86eca");
        assert_eq!(to_hex(&m.info_hashes.v2().unwrap().0), "597b180c1a170a585dfc5e85d834d69013ceda174b8f357d5bb1a0ca509faf0a");
        assert_eq!(m.files[0].path, vec![s("test64K")]);
        assert!(!m.multi_file);
        let zeros = vec![0u8; 65536];
        assert_eq!(m.verify_piece(0, &zeros), Some(true));
        assert_eq!(m.verify_piece(0, &[0u8; 100]), Some(false));

        let m = Metainfo::parse(V2_MULTIPIECE).unwrap();
        assert_eq!(to_hex(&m.info_hashes.v2().unwrap().0), "108ac2c3718ce722e6896edc56c4afa98f1d711ecaace7aad74fca418ebd03de");
        assert_eq!(m.piece_count, 16);
        assert_eq!(m.piece_layers.len(), 1);
        for p in 0..16 { assert_eq!(m.verify_piece(p, &zeros), Some(true)); }
        let mut bad = zeros.clone();
        bad[70] = 1;
        assert_eq!(m.verify_piece(3, &bad), Some(false));
        // From the bare info dictionary (a magnet): no layers yet, so v1 decides.
        let mut bare = Metainfo::from_info_bytes(&m.info_bytes).unwrap();
        assert_eq!(bare.info_hashes, m.info_hashes);
        assert_eq!(bare.v2_piece_hash(1), None);
        assert!(!bare.has_all_piece_layers());
        let (root, layer) = m.piece_layers.iter().next().unwrap();
        let mut wrong = layer.clone();
        wrong[0][0] ^= 1;
        assert_eq!(bare.set_piece_layer(*root, wrong), Err(Error::PieceLayer));
        bare.set_piece_layer(*root, layer.clone()).unwrap();
        assert_eq!(bare.v2_piece_hash(1), m.v2_piece_hash(1));
        assert_eq!(Metainfo::parse(&m.to_torrent()).unwrap(), m);
    }

    #[test]
    fn v2_symlinks_and_tree() {
        let m = Metainfo::parse(V2_SYMLINKS).unwrap();
        assert!(m.is_v2() && !m.is_v1());
        assert_eq!(to_hex(&m.info_hashes.v2().unwrap().0), "8e4db680dbf49d2a08a15b47ecefc1556f8a68a1e6f8a1fe5db56caf52554bee");
        assert!(m.multi_file);
        let paths: Vec<String> = m.files.iter().map(|f| f.path.join("/")).collect();
        assert_eq!(paths, ["Headers", "Resources", "SDL2", "Versions/Current/Headers/SDL.h", "Versions/Current/Resources/Info.plist", "Versions/Current/SDL2"]);
        assert!(m.files[0].attr.symlink && m.files[0].attr.executable);
        assert_eq!(m.files[0].symlink_target, Some(vec![s("Versions"), s("Current"), s("Headers")]));
        // Alignment: SDL.h is 1 piece, Info.plist 1, SDL2 80 pieces.
        assert_eq!(m.files[3].offset, 0);
        assert_eq!(m.files[4].offset, 16384);
        assert_eq!(m.files[5].offset, 32768);
        assert_eq!(m.piece_count, 2 + 1307232u64.div_ceil(16384) as u32);
        assert_eq!(m.piece_size(0), Some(4210));
        assert_eq!(m.piece_size(1), Some(1313));
        assert_eq!(m.piece_files(0), vec![FileSlice { file: 3, offset: 0, length: 4210 }]);
        assert_eq!(m.v2_piece(5), Some((5, 3)));
        assert_eq!(m.piece_size(m.piece_count - 1), Some(1307232 % 16384));
        assert_eq!(m.file_pieces(0), 0..0);
        assert!(m.v2_piece_hash(1).is_some() && m.v2_piece_hash(40).is_some());
    }

    #[test]
    fn invalid_fixtures() {
        assert_eq!(Metainfo::parse(V2_NO_POW2), Err(Error::BadPieceLength));
        assert_eq!(Metainfo::parse(V2_ZERO_ROOT), Err(Error::PieceLayer));
        assert_eq!(Metainfo::parse(V2_BAD_PAD), Err(Error::BadFileTree));
        // A '/' and invalid UTF-8 inside a name are sanitized, not trusted.
        let m = Metainfo::parse(V2_BAD_NAME).unwrap();
        assert_eq!(m.files[0].path, vec![s("\u{fffd}est_MB")]);
        assert_eq!(m.name, "\u{fffd}est_MB");
    }

    fn v1_multi(files: Vec<Item>) -> Vec<u8> {
        let n: i64 = files.iter().map(|f| match f.get(b"length") { Some(Item::Int(n)) => *n, _ => 0 }).sum();
        let pieces = (n.max(1) as u64).div_ceil(16384) as usize;
        let info = dict([(b"files", Item::List(files)), (b"name", "top".into()), (b"piece length", 16384.into()), (b"pieces", vec![0u8; 20 * pieces].into())]);
        dict([(b"info", info)]).encode()
    }
    fn file(len: i64, path: &[&str]) -> Item {
        dict([(b"length", len.into()), (b"path", Item::List(path.iter().map(|c| Item::from(*c)).collect()))])
    }

    #[test]
    fn hostile_paths() {
        let m = Metainfo::parse(&v1_multi(vec![file(5, &["..", "..", "etc", "passwd"]), file(1, &["/abs", ".", "", "a\0b\nc"])])).unwrap();
        assert_eq!(m.files[0].path, vec![s("etc"), s("passwd")]);
        assert_eq!(m.files[1].path, vec![s("_abs"), s("a_b_c")]);
        assert_eq!(Metainfo::parse(&v1_multi(vec![file(5, &["..", "."])])), Err(Error::BadPath));
        assert_eq!(Metainfo::parse(&v1_multi(vec![file(5, &["a"]), file(5, &["a"])])), Err(Error::DuplicatePath));
        assert_eq!(Metainfo::parse(&v1_multi(vec![file(5, &["a"]), file(5, &["a", "b"])])), Err(Error::DuplicatePath));
        assert_eq!(Metainfo::parse(&v1_multi(vec![file(5, &["x", "..", "a"]), file(5, &["x", "a"])])), Err(Error::DuplicatePath));
        assert_eq!(Metainfo::parse(&v1_multi(vec![file(-5, &["a"])])), Err(Error::BadField("length")));
        let deep: Vec<&str> = core::iter::repeat("d").take(MAX_PATH_DEPTH + 1).collect();
        assert_eq!(Metainfo::parse(&v1_multi(vec![file(5, &deep)])), Err(Error::PathTooDeep));
        let long = "x".repeat(1000);
        let m = Metainfo::parse(&v1_multi(vec![file(5, &[&long])])).unwrap();
        assert_eq!(m.files[0].path[0].len(), MAX_COMPONENT_BYTES);
        assert_eq!(Metainfo::parse(&v1_multi(vec![])), Err(Error::Empty));
    }

    #[test]
    fn hostile_structure() {
        for bad in [&b""[..], b"i1e", b"d4:infoi1ee", b"d4:infod4:name1:x12:piece lengthi0e6:pieces0:6:lengthi1eee", b"de", b"d4:infodee"] {
            assert!(Metainfo::parse(bad).is_err());
        }
        // Wrong pieces count, overflowing lengths, unsupported version.
        let info = dict([(b"length", 20000.into()), (b"name", "a".into()), (b"piece length", 16384.into()), (b"pieces", vec![0u8; 20].into())]);
        assert_eq!(Metainfo::from_info_bytes(&info.encode()), Err(Error::PieceCount));
        let files = Item::List(vec![file(i64::MAX, &["a"]), file(i64::MAX, &["b"]), file(i64::MAX, &["c"])]);
        let info = dict([(b"files", files), (b"name", "a".into()), (b"piece length", 16384.into()), (b"pieces", vec![0u8; 20].into())]);
        assert_eq!(Metainfo::from_info_bytes(&info.encode()), Err(Error::TooLarge));
        let files = Item::List(vec![file(i64::MAX, &["a"]), file(i64::MAX, &["b"])]);
        let info = dict([(b"files", files), (b"name", "a".into()), (b"piece length", 16384.into()), (b"pieces", vec![0u8; 20].into())]);
        assert_eq!(Metainfo::from_info_bytes(&info.encode()), Err(Error::TooLarge));
        let info = dict([(b"meta version", 3.into()), (b"name", "a".into()), (b"piece length", 16384.into())]);
        assert_eq!(Metainfo::from_info_bytes(&info.encode()), Err(Error::UnsupportedVersion(3)));
        // A v2 tree whose root is a file, and a file node with siblings.
        let leaf = dict([(b"", dict([(b"length", 5.into()), (b"pieces root", vec![1u8; 32].into())]))]);
        for tree in [leaf.clone(), Item::Dict([(b"a".to_vec(), Item::Dict([(b"".to_vec(), leaf.get(b"").unwrap().clone()), (b"x".to_vec(), leaf.clone())].into_iter().collect()))].into_iter().collect())] {
            let info = dict([(b"file tree", tree), (b"meta version", 2.into()), (b"name", "a".into()), (b"piece length", 16384.into())]);
            assert_eq!(Metainfo::from_info_bytes(&info.encode()), Err(Error::BadFileTree));
        }
        // Random mutations of real torrents never panic.
        for fixture in [SAMPLE, V2_SYMLINKS, V2_MULTIPIECE] {
            let mut x: u32 = 12345;
            for _ in 0..3000 {
                let mut t = fixture.to_vec();
                for _ in 0..3 {
                    x ^= x << 13; x ^= x >> 17; x ^= x << 5;
                    let i = x as usize % t.len();
                    t[i] = (x >> 8) as u8;
                }
                if let Ok(m) = Metainfo::parse(&t) {
                    for p in 0..m.piece_count.min(4) { let _ = m.piece_files(p); let _ = m.verify_piece(p, &[0; 10]); }
                }
                let cut = x as usize % fixture.len();
                let _ = Metainfo::parse(&fixture[..cut]);
            }
        }
    }

    #[test]
    fn outer_fields() {
        let info = dict([(b"length", 5.into()), (b"name", "f".into()), (b"piece length", 16384.into()), (b"pieces", vec![0u8; 20].into()), (b"private", 1.into()),
            (b"similar", Item::List(vec![vec![7u8; 20].into(), vec![1u8; 3].into()])), (b"collections", Item::List(vec!["c1".into()]))]);
        let t = dict([
            (b"announce", "http://ignored/".into()),
            (b"announce-list", Item::List(vec![Item::List(vec!["http://a/".into(), "http://b/".into(), "http://a/".into()]), Item::List(vec![]), Item::List(vec!["udp://c:1".into(), 5.into()])])),
            (b"url-list", "http://seed/".into()),
            (b"nodes", Item::List(vec![Item::List(vec!["router.example".into(), 6881.into()]), Item::List(vec!["bad".into(), 70000.into()])])),
            (b"collections", Item::List(vec!["c1".into(), "c2".into()])),
            (b"comment.utf-8", "hi".into()),
            (b"info", info),
        ]).encode();
        let m = Metainfo::parse(&t).unwrap();
        assert_eq!(m.trackers, vec![vec![s("http://a/"), s("http://b/")], vec![s("udp://c:1")]]);
        assert_eq!(m.web_seeds, vec![s("http://seed/")]);
        assert_eq!(m.nodes, vec![(s("router.example"), 6881)]);
        assert!(m.private);
        assert_eq!(m.similar, vec![InfoHashV1([7; 20])]);
        assert_eq!(m.collections, vec![s("c1"), s("c2")]);
        assert_eq!(m.comment.as_deref(), Some("hi"));
        let again = Metainfo::parse(&m.to_torrent()).unwrap();
        assert_eq!((again.trackers, again.nodes, again.info_hashes), (m.trackers.clone(), m.nodes.clone(), m.info_hashes));
    }
}
