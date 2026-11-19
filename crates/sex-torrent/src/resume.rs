// SPDX-License-Identifier: GPL-3.0-only
//! Resume data: what Swingers saves per torrent so a restart picks up where it left off
//! without rehashing everything ("fast resume").
//!
//! It is a bencoded dictionary with a `version` field. Decoding is tolerant: a missing or
//! badly typed field takes its default, unknown fields are ignored, and a newer version
//! is still read for the fields we know, so a damaged or future file costs at most a
//! recheck, never the torrent. Counts and sizes are capped against hostile files.
use alloc::string::String;
use alloc::vec::Vec;

use crate::bencode::{self, Item};
use crate::{Ip, PeerAddr};

/// The format version we write.
pub const VERSION: i64 = 1;
const MAX_PEERS: usize = 1000;
const MAX_TRACKERS: usize = 512;
const MAX_TAGS: usize = 256;
const MAX_UNFINISHED: usize = 1 << 16;
const MAX_STRING: usize = 4096;

/// Everything kept across restarts.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ResumeData {
    pub version: i64,
    /// The info-hashes (20 and/or 32 bytes), to match the file to its torrent.
    pub info_hash_v1: Option<[u8; 20]>,
    pub info_hash_v2: Option<[u8; 32]>,
    pub name: String,
    pub save_path: String,
    /// The pieces we have, as a BEP 3 bitfield, and how many pieces there are.
    pub have: Vec<u8>,
    pub num_pieces: u32,
    pub file_priorities: Vec<u8>,
    /// Received blocks of pieces not yet complete: (piece, one bool per block).
    pub unfinished: Vec<(u32, Vec<bool>)>,
    /// (piece, file) pairs held in the part region (deselected files).
    pub part_slices: Vec<(u32, u32)>,
    pub uploaded: u64,
    pub downloaded: u64,
    /// Unix seconds.
    pub added_time: u64,
    pub completed_time: u64,
    pub peers: Vec<PeerAddr>,
    /// Tracker tiers (BEP 12).
    pub trackers: Vec<Vec<String>>,
    pub category: String,
    pub tags: Vec<String>,
    pub labels: Vec<String>,
    pub sequential: bool,
    pub paused: bool,
    /// The raw info dictionary for magnet links that finished fetching it.
    pub info: Option<Vec<u8>>,
}

fn s(x: &str) -> Item { Item::Bytes(x.as_bytes().to_vec()) }
fn int(x: u64) -> Item { Item::Int(x.min(i64::MAX as u64) as i64) }
fn strings(v: &[String]) -> Item { Item::List(v.iter().map(|x| s(x)).collect()) }

impl ResumeData {
    pub fn encode(&self) -> Vec<u8> {
        let mut d = alloc::collections::BTreeMap::new();
        let mut put = |k: &str, v: Item| { d.insert(k.as_bytes().to_vec(), v); };
        put("version", Item::Int(VERSION));
        if let Some(h) = self.info_hash_v1 { put("info-hash", h.to_vec().into()); }
        if let Some(h) = self.info_hash_v2 { put("info-hash2", h.to_vec().into()); }
        put("name", s(&self.name));
        put("save_path", s(&self.save_path));
        put("pieces", self.have.clone().into());
        put("num_pieces", int(self.num_pieces as u64));
        put("file_priority", Item::List(self.file_priorities.iter().map(|p| Item::Int(*p as i64)).collect()));
        put("unfinished", Item::List(self.unfinished.iter().map(|(p, blocks)| {
            let mut bits = alloc::vec![0u8; blocks.len().div_ceil(8)];
            for (i, b) in blocks.iter().enumerate() { if *b { bits[i / 8] |= 0x80 >> (i % 8); } }
            bencode::dict([(b"piece", int(*p as u64)), (b"blocks", int(blocks.len() as u64)), (b"bitmask", bits.into())])
        }).collect()));
        put("parts", Item::List(self.part_slices.iter().map(|(p, f)| Item::List(alloc::vec![int(*p as u64), int(*f as u64)])).collect()));
        put("total_uploaded", int(self.uploaded));
        put("total_downloaded", int(self.downloaded));
        put("added_time", int(self.added_time));
        put("completed_time", int(self.completed_time));
        let (mut v4, mut v6) = (Vec::new(), Vec::new());
        for p in &self.peers { match p.ip { Ip::V4(_) => v4.extend(p.compact()), Ip::V6(_) => v6.extend(p.compact()) } }
        put("peers", v4.into());
        put("peers6", v6.into());
        put("trackers", Item::List(self.trackers.iter().map(|t| strings(t)).collect()));
        put("category", s(&self.category));
        put("tags", strings(&self.tags));
        put("labels", strings(&self.labels));
        put("sequential_download", Item::Int(self.sequential as i64));
        put("paused", Item::Int(self.paused as i64));
        if let Some(i) = &self.info { put("info", i.clone().into()); }
        Item::Dict(d).encode()
    }

    /// Decodes tolerantly. Only input that is not a bencoded dictionary at all fails.
    pub fn decode(bytes: &[u8]) -> Option<ResumeData> {
        let v = bencode::decode(bytes).ok()?;
        v.dict()?;
        let g = |k: &str| v.get(k.as_bytes());
        let n = |k: &str| g(k).and_then(|x| x.int()).filter(|x| *x >= 0).map(|x| x as u64).unwrap_or(0);
        let st = |x: &bencode::Value| x.bytes().filter(|b| b.len() <= MAX_STRING).map(|b| String::from_utf8_lossy(b).into_owned());
        let text = |k: &str| g(k).and_then(st).unwrap_or_default();
        let list_of = |k: &str, cap: usize| -> Vec<String> { g(k).and_then(|l| l.list()).map(|l| l.iter().filter_map(st).take(cap).collect()).unwrap_or_default() };
        let mut r = ResumeData {
            version: g("version").and_then(|x| x.int()).unwrap_or(0),
            info_hash_v1: g("info-hash").and_then(|x| x.bytes()).and_then(|b| b.try_into().ok()),
            info_hash_v2: g("info-hash2").and_then(|x| x.bytes()).and_then(|b| b.try_into().ok()),
            name: text("name"),
            save_path: text("save_path"),
            num_pieces: n("num_pieces").min(u32::MAX as u64) as u32,
            uploaded: n("total_uploaded"),
            downloaded: n("total_downloaded"),
            added_time: n("added_time"),
            completed_time: n("completed_time"),
            category: text("category"),
            tags: list_of("tags", MAX_TAGS),
            labels: list_of("labels", MAX_TAGS),
            sequential: n("sequential_download") != 0,
            paused: n("paused") != 0,
            info: g("info").and_then(|x| x.bytes()).map(|b| b.to_vec()),
            ..ResumeData::default()
        };
        let have = g("pieces").and_then(|x| x.bytes()).unwrap_or(&[]);
        if have.len() as u64 == (r.num_pieces as u64).div_ceil(8) { r.have = have.to_vec(); } else { r.num_pieces = 0; }
        r.file_priorities = g("file_priority").and_then(|l| l.list()).map(|l| l.iter().map(|p| p.int().unwrap_or(1).clamp(0, 7) as u8).collect()).unwrap_or_default();
        if let Some(l) = g("unfinished").and_then(|l| l.list()) {
            for e in l.iter().take(MAX_UNFINISHED) {
                let (Some(p), Some(nb), Some(mask)) = (e.get(b"piece").and_then(|x| x.int()), e.get(b"blocks").and_then(|x| x.int()), e.get(b"bitmask").and_then(|x| x.bytes())) else { continue };
                if p < 0 || p >= r.num_pieces as i64 || nb <= 0 || (nb as u64).div_ceil(8) != mask.len() as u64 { continue; }
                r.unfinished.push((p as u32, (0..nb as usize).map(|i| mask[i / 8] & (0x80 >> (i % 8)) != 0).collect()));
            }
        }
        if let Some(l) = g("parts").and_then(|l| l.list()) {
            for e in l.iter().take(MAX_UNFINISHED) {
                let Some([p, f]) = e.list().and_then(|x| <&[bencode::Value; 2]>::try_from(x).ok()) else { continue };
                if let (Some(p), Some(f)) = (p.int(), f.int()) {
                    if (0..=u32::MAX as i64).contains(&p) && (0..=u32::MAX as i64).contains(&f) { r.part_slices.push((p as u32, f as u32)); }
                }
            }
        }
        for (k, six) in [("peers", false), ("peers6", true)] {
            if let Some(b) = g(k).and_then(|x| x.bytes()) {
                let room = MAX_PEERS - r.peers.len();
                r.peers.extend(PeerAddr::parse_compact(b, six).into_iter().filter(|a| a.port != 0).take(room));
            }
        }
        if let Some(l) = g("trackers").and_then(|l| l.list()) {
            let mut total = 0;
            for tier in l {
                let urls: Vec<String> = tier.list().unwrap_or(&[]).iter().filter_map(st).take(MAX_TRACKERS - total).collect();
                total += urls.len();
                if !urls.is_empty() { r.trackers.push(urls); }
                if total >= MAX_TRACKERS { break; }
            }
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> ResumeData {
        ResumeData {
            version: VERSION,
            info_hash_v1: Some([1; 20]),
            info_hash_v2: Some([2; 32]),
            name: "Big Buck Bunny".into(),
            save_path: "/home/andreas/Downloads".into(),
            have: vec![0b1010_0000],
            num_pieces: 3,
            file_priorities: vec![1, 0, 7],
            unfinished: vec![(1, vec![true, false, true, true, false, false, false, false, true])],
            part_slices: vec![(1, 1)],
            uploaded: 1 << 40,
            downloaded: 12345,
            added_time: 1_700_000_000,
            completed_time: 0,
            peers: vec![PeerAddr { ip: Ip::V4([10, 0, 0, 1]), port: 6881 }, PeerAddr { ip: Ip::V6([9; 16]), port: 51413 }],
            trackers: vec![vec!["udp://tracker.example:1337/announce".into()], vec!["https://b.example/announce".into(), "https://c.example/announce".into()]],
            category: "Movies".into(),
            tags: vec!["4k".into(), "open-movie".into()],
            labels: vec!["blender".into()],
            sequential: true,
            paused: false,
            info: Some(b"d4:name1:xe".to_vec()),
        }
    }

    #[test]
    fn round_trip() {
        let r = sample();
        let enc = r.encode();
        assert!(bencode::decode(&enc).unwrap().canonical());
        assert_eq!(ResumeData::decode(&enc), Some(r));
    }

    #[test]
    fn tolerant_decode() {
        assert_eq!(ResumeData::decode(b"not bencode"), None);
        assert_eq!(ResumeData::decode(b"le"), None);
        let r = ResumeData::decode(b"de").unwrap();
        assert_eq!(r, ResumeData::default());
        // Wrong types, a future version, a bitfield of the wrong size, bad unfinished entries.
        let r = ResumeData::decode(b"d7:versioni99e4:name3:abc10:num_piecesi16e6:pieces1:x14:total_uploadedi-5e13:file_priorityli9ei-1e3:abce10:unfinishedld5:piecei99e6:blocksi1e7:bitmask1:\x80ei3e3:fooe5:peers7:\x01\x02\x03\x04\x00\x01\x05e").unwrap();
        assert_eq!(r.version, 99);
        assert_eq!(r.name, "abc");
        assert_eq!(r.num_pieces, 0, "bitfield of the wrong size: recheck");
        assert_eq!(r.uploaded, 0);
        assert_eq!(r.file_priorities, vec![7, 0, 1]);
        assert!(r.unfinished.is_empty());
        assert_eq!(r.peers, vec![PeerAddr { ip: Ip::V4([1, 2, 3, 4]), port: 1 }]);
    }

    #[test]
    fn caps() {
        let mut r = ResumeData::default();
        r.peers = (0..3000u32).map(|i| PeerAddr { ip: Ip::V4(i.to_be_bytes()), port: 1 }).collect();
        r.trackers = (0..1000).map(|i| vec![alloc::format!("udp://t{i}")]).collect();
        let d = ResumeData::decode(&r.encode()).unwrap();
        assert_eq!(d.peers.len(), MAX_PEERS);
        assert_eq!(d.trackers.len(), MAX_TRACKERS);
    }
}
