// SPDX-License-Identifier: GPL-3.0-only
//! BitTorrent v2 merkle trees (BEP 52).
//!
//! Every file of a v2 torrent is cut into 16 KiB blocks. Each block is hashed with
//! SHA-256 (the last block may be shorter); those are the leaves. The leaf count is
//! padded to a power of two with all-zero hashes, and pairs are hashed upward until one
//! hash is left: the file's `pieces root`. The layer where one node covers exactly one
//! piece is the "piece layer"; the .torrent carries it (`piece layers`) for files larger
//! than one piece, so a single piece can be checked without the whole file.
//!
//! Padding rules, which are easy to get wrong:
//! - Leaves past the end of the file are 32 zero bytes, so a padding node `h` layers up
//!   is [`pad_hash`]`(h)` (the root of `2^h` zero leaves), not zero.
//! - A file no larger than one piece has a tree of `next_pow2(blocks)` leaves, and its
//!   root is also its only piece hash.
//! - In a larger file the last piece's subtree is padded to a full piece of leaves.
//!
//! Also here: the peer messages `hash request` (21), `hashes` (22) and `hash reject`
//! (23), a [`MerkleTree`] that answers requests, and [`verify_hashes`] that checks an
//! answer (requested hashes plus uncle hashes) against a known root.
use alloc::vec;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// The leaf size: 16 KiB.
pub const BLOCK_SIZE: u64 = 16 * 1024;
/// A SHA-256 node of the tree.
pub type Hash = [u8; 32];
/// A padding leaf.
pub const ZERO: Hash = [0; 32];
/// Largest `length` we ask for or answer (BEP 52: "SHOULD NOT be greater than 512").
pub const MAX_REQUEST_HASHES: u32 = 512;
/// Peer wire message ids (BEP 52).
pub const MSG_HASH_REQUEST: u8 = 21;
pub const MSG_HASHES: u8 = 22;
pub const MSG_HASH_REJECT: u8 = 23;
/// Taller trees than this cannot occur (2^40 blocks is 16 PiB) and are refused.
pub const MAX_HEIGHT: u32 = 40;

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> Hash { Sha256::digest(data).into() }

/// The parent of two nodes.
pub fn hash_pair(l: &Hash, r: &Hash) -> Hash {
    let mut h = Sha256::new();
    h.update(l);
    h.update(r);
    h.finalize().into()
}

/// The root of a subtree of `2^height` zero leaves (`pad_hash(0) == ZERO`).
pub fn pad_hash(height: u32) -> Hash {
    let mut h = ZERO;
    for _ in 0..height.min(MAX_HEIGHT + 8) { h = hash_pair(&h, &h); }
    h
}

/// Blocks in a file of `len` bytes.
pub fn num_blocks(len: u64) -> u64 { len.div_ceil(BLOCK_SIZE) }

/// Leaves of a file's tree: blocks rounded up to a power of two (at least 1).
pub fn num_leaves(len: u64) -> u64 { num_blocks(len).max(1).checked_next_power_of_two().unwrap_or(1 << 63) }

/// Layers above the leaves in a file's tree (0 for a one-block file).
pub fn tree_height(len: u64) -> u32 { num_leaves(len).trailing_zeros() }

/// The layer (above the leaves) of the piece layer, if `piece_length` is valid for v2:
/// a power of two of at least 16 KiB.
pub fn piece_height(piece_length: u64) -> Option<u32> {
    if piece_length < BLOCK_SIZE || !piece_length.is_power_of_two() { return None; }
    let h = (piece_length / BLOCK_SIZE).trailing_zeros();
    if h > MAX_HEIGHT { None } else { Some(h) }
}

/// Leaf hashes of `data`, one per 16 KiB block.
pub fn leaf_hashes(data: &[u8]) -> Vec<Hash> { data.chunks(BLOCK_SIZE as usize).map(sha256).collect() }

/// The root over `nodes`, which sit `height` layers above the leaves; the layer is padded
/// to a power of two with [`pad_hash`]`(height)`. An empty layer gives `pad_hash(height)`.
pub fn root_of_layer(nodes: &[Hash], height: u32) -> Hash {
    match nodes.len() {
        0 => return pad_hash(height),
        1 => return nodes[0],
        _ => {}
    }
    let mut layer: Vec<Hash> = nodes.to_vec();
    let mut pad = pad_hash(height);
    while layer.len() > 1 {
        if layer.len() % 2 == 1 { layer.push(pad); }
        let n = layer.len() / 2;
        for i in 0..n { layer[i] = hash_pair(&layer[2 * i], &layer[2 * i + 1]); }
        layer.truncate(n);
        pad = hash_pair(&pad, &pad);
    }
    layer[0]
}

/// Like [`root_of_layer`], but the layer is padded to exactly `width` nodes (a power of
/// two, at least `nodes.len()`), e.g. the last piece of a file padded to a whole piece.
pub fn root_padded(nodes: &[Hash], height: u32, width: usize) -> Hash {
    let mut w = nodes.len().max(1).next_power_of_two();
    let mut h = root_of_layer(nodes, height);
    while w < width {
        h = hash_pair(&h, &pad_hash(height + w.trailing_zeros()));
        w *= 2;
    }
    h
}

/// The merkle root of a whole file's bytes (`None` for an empty file: it has no root).
pub fn file_root(data: &[u8]) -> Option<Hash> {
    if data.is_empty() { None } else { Some(root_of_layer(&leaf_hashes(data), 0)) }
}

/// The piece layer from a file's leaf hashes: one hash per piece, the last piece padded
/// to a whole piece of zero leaves. `None` if `piece_length` is not valid for v2.
pub fn piece_layer(leaves: &[Hash], piece_length: u64) -> Option<Vec<Hash>> {
    let bpp = 1usize << piece_height(piece_length)?;
    Some(leaves.chunks(bpp).map(|c| root_padded(c, 0, bpp)).collect())
}

/// The file root from its piece layer.
pub fn root_from_piece_layer(layer: &[Hash], piece_length: u64) -> Option<Hash> {
    Some(root_of_layer(layer, piece_height(piece_length)?))
}

/// The v2 hash of one piece's bytes, in a file of `file_length` bytes. For a file no
/// larger than a piece this is the file root; otherwise it is the piece layer entry.
pub fn piece_hash(piece: &[u8], piece_length: u64, file_length: u64) -> Option<Hash> {
    let bpp = 1usize << piece_height(piece_length)?;
    let leaves = leaf_hashes(piece);
    if leaves.len() > bpp { return None; }
    Some(if file_length <= piece_length { root_of_layer(&leaves, 0) } else { root_padded(&leaves, 0, bpp) })
}

/// Splits concatenated 32-byte hashes (a `piece layers` value), refusing a ragged tail.
pub fn split_hashes(bytes: &[u8]) -> Option<Vec<Hash>> {
    if bytes.len() % 32 != 0 { return None; }
    Some(bytes.chunks_exact(32).map(|c| { let mut h = ZERO; h.copy_from_slice(c); h }).collect())
}

/// Checks a piece layer for a file of `file_length` bytes against its root: the right
/// number of hashes (one per piece), and they hash up to `root`.
pub fn verify_piece_layer(layer: &[Hash], piece_length: u64, file_length: u64, root: &Hash) -> bool {
    if file_length <= piece_length || piece_height(piece_length).is_none() { return false; }
    layer.len() as u64 == file_length.div_ceil(piece_length) && root_from_piece_layer(layer, piece_length).as_ref() == Some(root)
}

/// What [`FileHasher::finish`] gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHashes {
    pub length: u64,
    /// `None` for an empty file.
    pub root: Option<Hash>,
    /// The piece layer; empty when the file is no larger than one piece.
    pub piece_layer: Vec<Hash>,
}

/// Hashes one file incrementally, keeping only the current piece's leaves and the piece
/// layer (32 bytes per piece) in memory.
#[derive(Clone, Debug)]
pub struct FileHasher {
    piece_length: u64,
    bpp: usize,
    block: Vec<u8>,
    leaves: Vec<Hash>,
    pieces: Vec<Hash>,
    len: u64,
}

impl FileHasher {
    /// `None` if `piece_length` is not a power of two of at least 16 KiB.
    pub fn new(piece_length: u64) -> Option<FileHasher> {
        let bpp = 1usize << piece_height(piece_length)?;
        Some(FileHasher { piece_length, bpp, block: Vec::with_capacity(BLOCK_SIZE as usize), leaves: Vec::new(), pieces: Vec::new(), len: 0 })
    }

    /// Bytes fed so far.
    pub fn len(&self) -> u64 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// Feeds the next bytes of the file.
    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.saturating_add(data.len() as u64);
        while !data.is_empty() {
            let take = (BLOCK_SIZE as usize - self.block.len()).min(data.len());
            if take == BLOCK_SIZE as usize && self.block.is_empty() {
                self.push_leaf(sha256(&data[..take]));
            } else {
                self.block.extend_from_slice(&data[..take]);
                if self.block.len() == BLOCK_SIZE as usize {
                    let h = sha256(&self.block);
                    self.block.clear();
                    self.push_leaf(h);
                }
            }
            data = &data[take..];
        }
    }

    fn push_leaf(&mut self, h: Hash) {
        self.leaves.push(h);
        if self.leaves.len() == self.bpp {
            self.pieces.push(root_of_layer(&self.leaves, 0));
            self.leaves.clear();
        }
    }

    /// The root and piece layer of everything fed.
    pub fn finish(mut self) -> FileHashes {
        if !self.block.is_empty() {
            let h = sha256(&self.block);
            self.leaves.push(h);
        }
        let length = self.len;
        if length == 0 { return FileHashes { length, root: None, piece_layer: Vec::new() }; }
        if length <= self.piece_length {
            let root = if self.pieces.len() == 1 { self.pieces[0] } else { root_of_layer(&self.leaves, 0) };
            return FileHashes { length, root: Some(root), piece_layer: Vec::new() };
        }
        if !self.leaves.is_empty() { self.pieces.push(root_padded(&self.leaves, 0, self.bpp)); }
        let root = root_from_piece_layer(&self.pieces, self.piece_length);
        FileHashes { length, root, piece_layer: self.pieces }
    }
}

/// A `hash request` (and `hash reject`) payload: which hashes of which file's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashRequest {
    /// The file, named by its root.
    pub pieces_root: Hash,
    /// Layers above the leaves where the requested hashes sit (0 = leaves).
    pub base_layer: u32,
    /// The first requested hash in that layer; a multiple of `length`.
    pub index: u32,
    /// How many hashes; a power of two, at least 2.
    pub length: u32,
    /// Ancestor layers to prove; the first `log2(length) - 1` are implied by the hashes.
    pub proof_layers: u32,
}

impl HashRequest {
    /// Payload size (after the message id).
    pub const LEN: usize = 48;

    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        b[..32].copy_from_slice(&self.pieces_root);
        for (i, v) in [self.base_layer, self.index, self.length, self.proof_layers].iter().enumerate() {
            b[32 + 4 * i..36 + 4 * i].copy_from_slice(&v.to_be_bytes());
        }
        b
    }

    /// Parses a payload of exactly 48 bytes.
    pub fn parse(p: &[u8]) -> Option<HashRequest> {
        if p.len() != Self::LEN { return None; }
        HashRequest::parse_prefix(p)
    }

    fn parse_prefix(p: &[u8]) -> Option<HashRequest> {
        let mut pieces_root = ZERO;
        pieces_root.copy_from_slice(p.get(..32)?);
        let u = |i: usize| p.get(32 + 4 * i..36 + 4 * i).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
        Some(HashRequest { pieces_root, base_layer: u(0)?, index: u(1)?, length: u(2)?, proof_layers: u(3)? })
    }

    /// How many uncle hashes the answer carries (BEP 52: proof layers minus the
    /// `log2(length) - 1` layers implied by the hashes themselves).
    pub fn uncle_count(&self) -> u32 {
        let k = self.length.max(1).trailing_zeros();
        (self.proof_layers + 1).saturating_sub(k)
    }

    /// Whether this request is well formed for a file of `file_length` bytes: length a
    /// power of two in 2..=512, index a multiple of it, the range inside the (padded)
    /// layer, and no proof above the root.
    pub fn is_valid_for(&self, file_length: u64) -> bool {
        if file_length == 0 || self.length < 2 || self.length > MAX_REQUEST_HASHES || !self.length.is_power_of_two() { return false; }
        if self.index % self.length != 0 { return false; }
        let h = tree_height(file_length);
        if h > MAX_HEIGHT || self.base_layer >= h { return false; }
        let layer_size = num_leaves(file_length) >> self.base_layer;
        if self.index as u64 + self.length as u64 > layer_size { return false; }
        self.proof_layers < h - self.base_layer
    }
}

/// A `hashes` payload: the request it answers, then the base-layer hashes, then the
/// uncle hashes, the one closest to the root last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hashes {
    pub request: HashRequest,
    pub hashes: Vec<Hash>,
}

impl Hashes {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(48 + 32 * self.hashes.len());
        v.extend_from_slice(&self.request.encode());
        for h in &self.hashes { v.extend_from_slice(h); }
        v
    }

    /// Parses a payload; the hash count is capped at 512 + 64.
    pub fn parse(p: &[u8]) -> Option<Hashes> {
        let request = HashRequest::parse_prefix(p)?;
        let rest = p.get(HashRequest::LEN..)?;
        if rest.len() > 32 * (MAX_REQUEST_HASHES as usize + 64) { return None; }
        Some(Hashes { request, hashes: split_hashes(rest)? })
    }

    /// The requested hashes (empty if the message is short).
    pub fn base(&self) -> &[Hash] { self.hashes.get(..self.request.length as usize).unwrap_or(&[]) }
    /// The uncle hashes.
    pub fn uncles(&self) -> &[Hash] { self.hashes.get(self.request.length as usize..).unwrap_or(&[]) }
}

/// Hashes the requested hashes up to their subtree root, then climbs with the uncles.
/// Returns `(layer, index, node)` of the highest node reached, which the caller compares
/// with a node it already trusts (the root, or a piece layer entry). `None` if the
/// message is malformed for a file of `file_length` bytes.
pub fn climb(h: &Hashes, file_length: u64) -> Option<(u32, u64, Hash)> {
    let r = &h.request;
    if !r.is_valid_for(file_length) { return None; }
    let uncles = r.uncle_count() as usize;
    if h.hashes.len() != r.length as usize + uncles { return None; }
    let k = r.length.trailing_zeros();
    let node = root_of_layer(h.base(), r.base_layer);
    let mut layer = r.base_layer + k;
    let mut index = (r.index / r.length) as u64;
    let mut node = node;
    for u in h.uncles() {
        node = if index % 2 == 0 { hash_pair(&node, u) } else { hash_pair(u, &node) };
        index /= 2;
        layer += 1;
    }
    Some((layer, index, node))
}

/// Verifies a `hashes` message that proves all the way to the file root.
pub fn verify_hashes(h: &Hashes, file_length: u64, root: &Hash) -> bool {
    match climb(h, file_length) {
        Some((layer, 0, node)) => layer == tree_height(file_length) && node == *root,
        _ => false,
    }
}

/// The proof layers to ask for so that the answer proves up to the root.
pub fn proof_layers_to_root(file_length: u64, base_layer: u32) -> u32 {
    tree_height(file_length).saturating_sub(base_layer).saturating_sub(1)
}

/// The known layers of one file's tree, from some lowest layer up to the root: built
/// from the leaves (a seed with the data) or from the piece layer (a peer that has the
/// .torrent). Padding nodes are computed, not stored. Answers hash requests.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    file_length: u64,
    lowest: u32,
    /// `layers[i]` holds the real nodes of layer `lowest + i`.
    layers: Vec<Vec<Hash>>,
}

impl MerkleTree {
    /// From all leaf hashes of a file of `file_length` bytes.
    pub fn from_leaves(leaves: Vec<Hash>, file_length: u64) -> Option<MerkleTree> {
        if file_length == 0 || leaves.len() as u64 != num_blocks(file_length) { return None; }
        Some(Self::build(leaves, 0, file_length))
    }

    /// From a file's piece layer (files larger than one piece).
    pub fn from_piece_layer(layer: Vec<Hash>, piece_length: u64, file_length: u64) -> Option<MerkleTree> {
        let ph = piece_height(piece_length)?;
        if file_length <= piece_length || layer.len() as u64 != file_length.div_ceil(piece_length) { return None; }
        Some(Self::build(layer, ph, file_length))
    }

    fn build(base: Vec<Hash>, lowest: u32, file_length: u64) -> MerkleTree {
        let height = tree_height(file_length);
        let mut layers = vec![base];
        let mut l = lowest;
        while l < height {
            let below = &layers[layers.len() - 1];
            let pad = pad_hash(l);
            let up: Vec<Hash> = below.chunks(2).map(|c| hash_pair(&c[0], c.get(1).unwrap_or(&pad))).collect();
            layers.push(up);
            l += 1;
        }
        MerkleTree { file_length, lowest, layers }
    }

    pub fn root(&self) -> Hash { self.node(tree_height(self.file_length), 0).unwrap_or(ZERO) }

    /// A node, padding included; `None` below the lowest known layer or past the layer.
    pub fn node(&self, layer: u32, index: u64) -> Option<Hash> {
        let nodes = self.layers.get(layer.checked_sub(self.lowest)? as usize)?;
        if index >= num_leaves(self.file_length) >> layer { return None; }
        Some(nodes.get(usize::try_from(index).ok()?).copied().unwrap_or_else(|| pad_hash(layer)))
    }

    /// Answers a request for this file, or `None` (send `hash reject`).
    pub fn answer(&self, r: &HashRequest) -> Option<Hashes> {
        if r.pieces_root != self.root() || !r.is_valid_for(self.file_length) { return None; }
        let mut hashes = Vec::with_capacity(r.length as usize + r.uncle_count() as usize);
        for i in 0..r.length as u64 { hashes.push(self.node(r.base_layer, r.index as u64 + i)?); }
        let mut layer = r.base_layer + r.length.trailing_zeros();
        let mut index = (r.index / r.length) as u64;
        for _ in 0..r.uncle_count() {
            hashes.push(self.node(layer, index ^ 1)?);
            index /= 2;
            layer += 1;
        }
        Some(Hashes { request: *r, hashes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_hex;

    fn h(s: &str) -> Hash { from_hex::<32>(s).unwrap() }

    // Roots from libtorrent's test torrents (v2.torrent, v2_multipiece_file.torrent),
    // whose files are all zeros (their v1 piece hash is SHA-1 of 64 KiB of zeros).
    const ROOT_64K_ZEROS: &str = "60aae9c7b428f87e0713e88229e18f0adf12cd7b22a0dd8a92bb2485eb7af242";
    const ROOT_1M_ZEROS: &str = "515ea9181744b817744ded9d2e8e9dc6a8450c0b0c52e24b5077f302ffbd9008";

    #[test]
    fn golden_zero_files() {
        let z = vec![0u8; 1 << 20];
        assert_eq!(file_root(&z[..65536]).unwrap(), h(ROOT_64K_ZEROS));
        assert_eq!(file_root(&z).unwrap(), h(ROOT_1M_ZEROS));
        let layer = piece_layer(&leaf_hashes(&z), 65536).unwrap();
        assert_eq!(layer.len(), 16);
        assert!(layer.iter().all(|x| *x == h(ROOT_64K_ZEROS)));
        assert!(verify_piece_layer(&layer, 65536, 1 << 20, &h(ROOT_1M_ZEROS)));
        assert_eq!(piece_hash(&z[..65536], 65536, 1 << 20).unwrap(), h(ROOT_64K_ZEROS));
    }

    #[test]
    fn hand_built_tree() {
        // 3 blocks: a full 16 KiB of 1s, a full one of 2s, then 5 bytes of 3s.
        let mut data = vec![1u8; 16384];
        data.extend(vec![2u8; 16384]);
        data.extend([3u8; 5]);
        let (a, b, c) = (sha256(&[1u8; 16384]), sha256(&[2u8; 16384]), sha256(&[3u8; 5]));
        let want = hash_pair(&hash_pair(&a, &b), &hash_pair(&c, &ZERO));
        assert_eq!(file_root(&data).unwrap(), want);
        assert_eq!(tree_height(data.len() as u64), 2);
        // One block: the root is the block hash itself.
        assert_eq!(file_root(&[9u8; 100]).unwrap(), sha256(&[9u8; 100]));
        assert_eq!(file_root(&[]), None);
        // A file of 3 blocks in a 64 KiB piece is a single-piece file: 4 leaves, not 4+.
        assert_eq!(piece_hash(&data, 65536, data.len() as u64).unwrap(), want);
    }

    #[test]
    fn last_piece_zero_padding() {
        // 5 blocks, 32 KiB pieces: pieces [b0 b1] [b2 b3] [b4 Z]; the piece layer is
        // padded with the root of 2 zero leaves, not with zero.
        let data: Vec<u8> = (0..5 * 16384u32).map(|i| (i / 16384 * 7 + i % 251) as u8).collect();
        let l = leaf_hashes(&data);
        let p = [hash_pair(&l[0], &l[1]), hash_pair(&l[2], &l[3]), hash_pair(&l[4], &ZERO)];
        assert_eq!(piece_layer(&l, 32768).unwrap(), p.to_vec());
        let pad1 = hash_pair(&ZERO, &ZERO);
        let root = hash_pair(&hash_pair(&p[0], &p[1]), &hash_pair(&p[2], &pad1));
        assert_eq!(file_root(&data).unwrap(), root);
        assert_eq!(root_from_piece_layer(&p, 32768).unwrap(), root);
        assert_eq!(piece_hash(&data[65536..], 32768, data.len() as u64).unwrap(), p[2]);
        assert_ne!(root_of_layer(&p, 1), root_of_layer(&p, 0));
        // Short last block inside the last piece.
        let short = &data[..2 * 16384 + 10];
        let l2 = leaf_hashes(short);
        assert_eq!(piece_layer(&l2, 32768).unwrap()[1], hash_pair(&l2[2], &ZERO));
        assert_eq!(pad_hash(3), root_of_layer(&[ZERO; 8], 0));
    }

    #[test]
    fn incremental_hasher_matches() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 253) as u8).collect();
        for &pl in &[16384u64, 65536, 262144, 1 << 20] {
            for &chunk in &[1usize, 999, 16384, 70000] {
                let mut fh = FileHasher::new(pl).unwrap();
                for c in data.chunks(chunk) { fh.update(c); }
                let out = fh.finish();
                assert_eq!(out.root, file_root(&data), "pl {} chunk {}", pl, chunk);
                if (data.len() as u64) > pl {
                    assert_eq!(out.piece_layer, piece_layer(&leaf_hashes(&data), pl).unwrap());
                    assert!(verify_piece_layer(&out.piece_layer, pl, data.len() as u64, &out.root.unwrap()));
                } else {
                    assert!(out.piece_layer.is_empty());
                }
            }
        }
        // Exactly one piece.
        let mut fh = FileHasher::new(32768).unwrap();
        fh.update(&data[..32768]);
        assert_eq!(fh.finish().root, file_root(&data[..32768]));
        assert_eq!(FileHasher::new(16384).unwrap().finish().root, None);
        assert!(FileHasher::new(20000).is_none());
        assert!(FileHasher::new(8192).is_none());
    }

    #[test]
    fn request_round_trip_and_rules() {
        let r = HashRequest { pieces_root: [7; 32], base_layer: 2, index: 4, length: 4, proof_layers: 3 };
        assert_eq!(HashRequest::parse(&r.encode()), Some(r));
        assert_eq!(HashRequest::parse(&r.encode()[..47]), None);
        assert_eq!(r.uncle_count(), 2);
        assert_eq!(HashRequest { length: 2, proof_layers: 0, ..r }.uncle_count(), 0);
        assert_eq!(HashRequest { length: 2, proof_layers: 1, ..r }.uncle_count(), 1);
        assert_eq!(HashRequest { length: 8, proof_layers: 1, ..r }.uncle_count(), 0);
        let len = 64 * 16384; // 64 leaves, height 6
        assert!(HashRequest { base_layer: 0, index: 0, length: 2, proof_layers: 5, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: 0, index: 0, length: 2, proof_layers: 6, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: 0, index: 1, length: 2, proof_layers: 0, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: 0, index: 0, length: 3, proof_layers: 0, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: 0, index: 0, length: 1024, proof_layers: 0, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: 5, index: 2, length: 2, proof_layers: 0, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: 6, index: 0, length: 2, proof_layers: 0, ..r }.is_valid_for(len));
        assert!(!HashRequest { base_layer: u32::MAX, index: u32::MAX, length: 2, proof_layers: u32::MAX, ..r }.is_valid_for(len));
        let hs = Hashes { request: r, hashes: vec![[1; 32], [2; 32]] };
        assert_eq!(Hashes::parse(&hs.encode()), Some(hs.clone()));
        assert_eq!(Hashes::parse(&hs.encode()[..60]), None);
        assert_eq!(Hashes::parse(&[0u8; 10]), None);
        assert_eq!(Hashes::parse(&vec![0u8; 48 + 32 * 1000]), None);
    }

    /// A downloader and a seed exchanging hash requests over an in-memory "network".
    #[test]
    fn seed_answers_and_downloader_verifies() {
        let data: Vec<u8> = (0..(37 * 16384 + 100) as u32).map(|i| (i % 241) as u8 ^ (i >> 14) as u8).collect();
        let len = data.len() as u64;
        let root = file_root(&data).unwrap();
        let seed = MerkleTree::from_leaves(leaf_hashes(&data), len).unwrap();
        assert_eq!(seed.root(), root);
        let pl = 4 * 16384;
        let layer = piece_layer(&leaf_hashes(&data), pl).unwrap();
        let from_layer = MerkleTree::from_piece_layer(layer.clone(), pl, len).unwrap();
        assert_eq!(from_layer.root(), root);
        let height = tree_height(len);
        for base in 0..height {
            let width = (num_leaves(len) >> base) as u32;
            let mut length = 2;
            while length <= width.min(16) {
                for index in (0..width).step_by(length as usize) {
                    let req = HashRequest { pieces_root: root, base_layer: base, index, length, proof_layers: proof_layers_to_root(len, base) };
                    let wire = seed.answer(&req).unwrap().encode();
                    let got = Hashes::parse(&wire).unwrap();
                    assert!(verify_hashes(&got, len, &root), "base {} index {} len {}", base, index, length);
                    if base >= 2 { assert_eq!(from_layer.answer(&req).unwrap(), got); } else { assert!(from_layer.answer(&req).is_none()); }
                    let mut bad = got.clone();
                    bad.hashes[0][0] ^= 1;
                    assert!(!verify_hashes(&bad, len, &root));
                    let mut bad = got.clone();
                    bad.hashes.pop();
                    assert!(!verify_hashes(&bad, len, &root));
                }
                length *= 2;
            }
        }
        // The piece layer fetched via a request at the piece layer equals the real one.
        let req = HashRequest { pieces_root: root, base_layer: 2, index: 0, length: 16, proof_layers: proof_layers_to_root(len, 2) };
        let got = seed.answer(&req).unwrap();
        assert_eq!(&got.base()[..layer.len()], &layer[..]);
        assert_eq!(got.base()[layer.len()], pad_hash(2));
        // A partial proof climbs to a node the downloader already trusts.
        let req = HashRequest { pieces_root: root, base_layer: 0, index: 4, length: 2, proof_layers: 1 };
        let got = seed.answer(&req).unwrap();
        let (l, i, n) = climb(&got, len).unwrap();
        assert_eq!((l, i), (2, 1));
        assert_eq!(n, layer[1]);
        // Wrong root: rejected.
        assert!(seed.answer(&HashRequest { pieces_root: [0; 32], ..req }).is_none());
    }

    #[test]
    fn hostile_messages_do_not_panic() {
        let len = 1u64 << 30;
        for seed in 0..2000u32 {
            let mut p = vec![0u8; 48 + 32 * (seed as usize % 7)];
            let mut x = seed.wrapping_mul(2654435761);
            for b in p.iter_mut() { x ^= x << 13; x ^= x >> 17; x ^= x << 5; *b = x as u8; }
            for i in [32usize, 36, 40, 44] { p[i] = 0; p[i + 1] = 0; }
            if let Some(h) = Hashes::parse(&p) { let _ = climb(&h, len); let _ = verify_hashes(&h, len, &[0; 32]); }
        }
        assert_eq!(piece_hash(&vec![0u8; 3 * 16384], 32768, 1 << 20), None);
        assert!(MerkleTree::from_leaves(vec![], 0).is_none());
        assert!(MerkleTree::from_leaves(vec![ZERO], 20000).is_none());
    }
}
