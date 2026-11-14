// SPDX-License-Identifier: GPL-3.0-only
//! Joins the parsed torrent ([`Metainfo`]) to the engine: the file layout the session
//! writes through, and piece verification by every hash the torrent carries (SHA-1 for
//! v1, the BEP 52 merkle piece layers for v2, both for a hybrid).
use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::metainfo::Metainfo;
use crate::session::PieceVerifier;
use crate::storage::{LayoutError, LayoutFile, TorrentLayout};

impl Metainfo {
    /// The layout for the session. Paths start with the torrent's name for a
    /// multi-file torrent (its folder); a single-file torrent is just its name. Pad
    /// files stay in, flagged, so they are never written.
    pub fn layout(&self) -> Result<TorrentLayout, LayoutError> {
        let piece_length = u32::try_from(self.piece_length).map_err(|_| LayoutError::BadPieceLength)?;
        let files: Vec<LayoutFile> = self.files.iter().map(|f| {
            let mut path = Vec::with_capacity(f.path.len() + 1);
            if self.multi_file { path.push(self.name.clone()); }
            path.extend(f.path.iter().cloned());
            if path.is_empty() { path.push(self.name.clone()); }
            LayoutFile { path, length: f.length, offset: f.offset, pad: f.attr.pad }
        }).collect();
        let total_length = files.iter().map(|f| f.offset + f.length).max().unwrap_or(0);
        let l = TorrentLayout { piece_length, total_length, files, v2_aligned: self.is_v2() && !self.is_v1() };
        l.validate()?;
        Ok(l)
    }

    /// A verifier for the session that checks every hash it knows. Pieces whose hashes
    /// are not known yet (v2 piece layers still missing) fail, so they are fetched again.
    pub fn verifier(&self) -> Box<dyn PieceVerifier> { Box::new(MetaVerifier(self.clone())) }
}

struct MetaVerifier(Metainfo);
impl PieceVerifier for MetaVerifier {
    fn verify(&self, piece: u32, data: &[u8]) -> bool { self.0.verify_piece(piece, data) == Some(true) }
}

#[cfg(test)]
mod tests {
    use crate::create::{Creator, FileSpec, Options, Version};
    use crate::metainfo::Metainfo;

    fn payload(n: usize, seed: u8) -> alloc::vec::Vec<u8> { (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect() }

    #[test]
    fn layouts_and_verifiers_for_every_version() {
        for v in [Version::V1, Version::V2, Version::Hybrid] {
            let a = payload(70_000, 1);
            let b = payload(20_000, 2);
            let mut opts = Options::new("pack", v);
            opts.piece_length = Some(32 * 1024);
            let mut c = Creator::new(opts, alloc::vec![FileSpec::new(&["a.bin"], a.len() as u64), FileSpec::new(&["b.bin"], b.len() as u64)]).unwrap();
            let order: alloc::vec::Vec<alloc::string::String> = c.files().iter().map(|f| f.path.join("/")).collect();
            for name in &order { c.feed(if name == "a.bin" { &a } else { &b }).unwrap(); }
            let m = Metainfo::parse(&c.finish().unwrap()).unwrap();
            let l = m.layout().unwrap();
            assert!(l.files.iter().filter(|f| !f.pad).all(|f| f.path[0] == "pack"));
            // Rebuild each piece from the layout and check it verifies; a flipped byte fails.
            let ver = m.verifier();
            let mut space = alloc::vec![0u8; l.total_length as usize];
            for f in &l.files { if f.pad { continue; } let src = if f.path[1] == "a.bin" { &a } else { &b }; space[f.offset as usize..(f.offset + f.length) as usize].copy_from_slice(src); }
            for p in 0..l.num_pieces() {
                let s = p as usize * 32 * 1024;
                let e = (s + 32 * 1024).min(space.len());
                let mut piece = space[s..e].to_vec();
                if m.is_v2() && !m.is_v1() { let n = m.piece_size(p).unwrap() as usize; piece.truncate(n); }
                assert!(ver.verify(p, &piece), "{:?} piece {}", v, p);
                piece[0] ^= 1;
                assert!(!ver.verify(p, &piece));
            }
        }
    }
}
