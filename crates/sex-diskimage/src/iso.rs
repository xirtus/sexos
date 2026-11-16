// SPDX-License-Identifier: GPL-3.0-only
//! ISO 9660 (ECMA-119) with Joliet, Rock Ridge (SUSP/RRIP) and El Torito.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::util::{self, le16, le32, lossy, read_exact, read_vec, utf16be};
use crate::{ByteSource, Entry, EntryKind, Error, Node, NodeRef, Result};

const SECTOR: u64 = 2048;
/// Largest directory extent read (a directory of ~100 000 names).
const MAX_DIR: u64 = 16 << 20;
/// Continuation areas followed per record.
const MAX_CE: usize = 32;
pub const BOOT_DIR: &str = "[BOOT]";

/// Which directory tree an ISO is browsed through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    /// Rock Ridge if present, else Joliet, else plain ISO 9660 names.
    Auto,
    RockRidge,
    Joliet,
    Plain,
}

pub(crate) fn detect<S: ByteSource + ?Sized>(src: &S) -> Result<bool> {
    if src.len() < 17 * SECTOR {
        return Ok(false);
    }
    let mut b = [0u8; 6];
    read_exact(src, 16 * SECTOR, &mut b)?;
    Ok(&b[1..6] == b"CD001")
}

#[derive(Clone, Copy)]
struct Root {
    at: u64,
    len: u64,
}

struct Boot {
    catalog: u64,
    entries: Vec<(String, u64, u64)>,
}

/// An ISO 9660 disc.
pub struct Iso<S> {
    src: S,
    block: u64,
    label: String,
    ns: Namespace,
    root: Root,
    /// Rock Ridge: bytes to skip at the start of each system use area.
    rr_skip: Option<usize>,
    has_joliet: bool,
    has_rr: bool,
    boot: Option<Boot>,
}

impl<S: ByteSource> Iso<S> {
    pub fn open(src: S) -> Result<Self> {
        Self::open_with(src, Namespace::Auto)
    }

    pub fn open_with(src: S, want: Namespace) -> Result<Self> {
        if !detect(&src)? {
            return Err(Error::NotRecognized);
        }
        let mut pvd: Option<Vec<u8>> = None;
        let mut joliet: Option<Vec<u8>> = None;
        let mut catalog: Option<u64> = None;
        for i in 16..16 + 64u64 {
            if (i + 1) * SECTOR > src.len() {
                break;
            }
            let d = read_vec(&src, i * SECTOR, SECTOR as usize)?;
            if &d[1..6] != b"CD001" {
                break;
            }
            match d[0] {
                0 if &d[7..30] == b"EL TORITO SPECIFICATION" => catalog = Some(le32(&d, 0x47) as u64),
                1 if pvd.is_none() => pvd = Some(d),
                2 if joliet.is_none() && d[88] == 0x25 && d[89] == 0x2F && matches!(d[90], 0x40 | 0x43 | 0x45) => joliet = Some(d),
                255 => break,
                _ => {}
            }
        }
        let pvd = pvd.ok_or(Error::Corrupt("no primary volume descriptor"))?;
        let block = le16(&pvd, 128) as u64;
        let block = if matches!(block, 512 | 1024 | 2048) { block } else { SECTOR };
        let root_of = |d: &[u8]| Root { at: le32(d, 156 + 2) as u64 * block, len: le32(d, 156 + 10) as u64 };
        let mut iso = Iso {
            block,
            label: String::new(),
            ns: Namespace::Plain,
            root: root_of(&pvd),
            rr_skip: None,
            has_joliet: joliet.is_some(),
            has_rr: false,
            boot: None,
            src,
        };
        // Rock Ridge announces itself with an SP entry in the root's "." record.
        let skip = iso.find_sp()?;
        iso.has_rr = skip.is_some();
        let ns = match want {
            Namespace::Auto if iso.has_rr => Namespace::RockRidge,
            Namespace::Auto if iso.has_joliet => Namespace::Joliet,
            Namespace::Auto => Namespace::Plain,
            Namespace::RockRidge if !iso.has_rr => return Err(Error::Unsupported("disc has no Rock Ridge names")),
            Namespace::Joliet if !iso.has_joliet => return Err(Error::Unsupported("disc has no Joliet names")),
            n => n,
        };
        iso.ns = ns;
        let label_pvd = trim(&lossy(&pvd[40..72]));
        match ns {
            Namespace::Joliet => {
                let j = joliet.as_ref().unwrap();
                iso.root = root_of(j);
                let l = trim(&utf16be(&j[40..72]));
                iso.label = if l.is_empty() { label_pvd } else { l };
            }
            Namespace::RockRidge => {
                iso.rr_skip = skip;
                iso.label = label_pvd;
            }
            _ => iso.label = label_pvd,
        }
        if let Some(c) = catalog {
            iso.boot = iso.read_boot_catalog(c).ok();
        }
        Ok(iso)
    }

    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn namespace(&self) -> Namespace {
        self.ns
    }
    pub fn has_joliet(&self) -> bool {
        self.has_joliet
    }
    pub fn has_rock_ridge(&self) -> bool {
        self.has_rr
    }
    /// The disc carries an El Torito boot catalog (shown as `[BOOT]`).
    pub fn is_bootable(&self) -> bool {
        self.boot.is_some()
    }
    pub fn root(&self) -> Node {
        Node(NodeRef::IsoDir { at: self.root.at, len: self.root.len })
    }
    pub fn source(&self) -> &S {
        &self.src
    }

    fn find_sp(&self) -> Result<Option<usize>> {
        let len = self.root.len.min(SECTOR) as usize;
        if len < 34 {
            return Ok(None);
        }
        let d = read_vec(&self.src, self.root.at, len)?;
        let rl = d[0] as usize;
        if rl < 34 || rl > d.len() {
            return Ok(None);
        }
        let su = &d[34..rl];
        if su.len() >= 7 && &su[0..2] == b"SP" && su[4] == 0xBE && su[5] == 0xEF {
            return Ok(Some(su[6] as usize));
        }
        Ok(None)
    }

    fn read_boot_catalog(&self, lba: u64) -> Result<Boot> {
        let at = lba * self.block;
        let cat = read_vec(&self.src, at, SECTOR as usize)?;
        if cat[0] != 1 || cat[30] != 0x55 || cat[31] != 0xAA {
            return Err(Error::Corrupt("bad El Torito validation entry"));
        }
        let sum = (0..16).fold(0u16, |s, i| s.wrapping_add(le16(&cat, i * 2)));
        if sum != 0 {
            return Err(Error::Corrupt("bad El Torito checksum"));
        }
        let mut entries = Vec::new();
        let mut platform = cat[1];
        let add = |e: &[u8], platform: u8, entries: &mut Vec<(String, u64, u64)>| {
            if e[0] != 0x88 {
                return;
            }
            let count = le16(e, 6) as u64;
            let len = match e[1] & 0x0F {
                1 => 1_228_800,
                2 => 1_474_560,
                3 => 2_949_120,
                _ => count.max(1) * 512,
            };
            let plat = match platform {
                0 => "BIOS",
                1 => "PPC",
                2 => "Mac",
                0xEF => "UEFI",
                _ => "Other",
            };
            let name = format!("Boot-{}-{}.img", entries.len() + 1, plat);
            entries.push((name, le32(e, 8) as u64 * self.block, len));
        };
        add(&cat[32..64], platform, &mut entries);
        let mut pos = 64;
        while pos + 32 <= cat.len() {
            let h = cat[pos];
            if h != 0x90 && h != 0x91 {
                break;
            }
            platform = cat[pos + 1];
            let n = le16(&cat, pos + 2) as usize;
            pos += 32;
            for _ in 0..n {
                if pos + 32 > cat.len() {
                    break;
                }
                add(&cat[pos..pos + 32], platform, &mut entries);
                pos += 32;
            }
            if h == 0x91 {
                break;
            }
        }
        Ok(Boot { catalog: at, entries })
    }

    pub fn file_len(&self, file: &Node) -> Result<u64> {
        match &file.0 {
            NodeRef::Extents(r) => Ok(r.iter().map(|x| x.1).sum()),
            _ => Err(Error::NotAFile),
        }
    }

    pub fn read(&self, file: &Node, offset: u64, buf: &mut [u8]) -> Result<usize> {
        match &file.0 {
            NodeRef::Extents(r) => {
                // Clamp each run to the image so a lying length reads short, not wrong.
                util::read_runs(&self.src, r, offset, buf)
            }
            _ => Err(Error::NotAFile),
        }
    }

    pub fn list_dir(&self, dir: &Node) -> Result<Vec<Entry>> {
        match dir.0 {
            NodeRef::IsoDir { at, len } => self.list(at, len),
            NodeRef::BootDir => Ok(self.boot_entries()),
            _ => Err(Error::NotADirectory),
        }
    }

    fn boot_entries(&self) -> Vec<Entry> {
        let Some(b) = &self.boot else { return Vec::new() };
        let mut out = vec![synthetic("boot.catalog", b.catalog, SECTOR)];
        for (n, at, len) in &b.entries {
            out.push(synthetic(n, *at, *len));
        }
        out
    }

    fn list(&self, at: u64, len: u64) -> Result<Vec<Entry>> {
        if len > MAX_DIR {
            return Err(Error::Unsupported("folder too large"));
        }
        let data = read_vec(&self.src, at, len as usize)?;
        let mut out: Vec<Entry> = Vec::new();
        // A multi-extent file in progress: its entry and whether more extents follow.
        let mut pending: Option<Entry> = None;
        let mut pos = 0usize;
        let block = self.block as usize;
        let is_root = at == self.root.at;
        while pos < data.len() {
            let rl = data[pos] as usize;
            let block_end = (pos / block + 1) * block;
            if rl == 0 || rl < 34 || pos + rl > block_end.min(data.len()) {
                pos = block_end;
                continue;
            }
            let rec = &data[pos..pos + rl];
            pos += rl;
            let name_len = rec[32] as usize;
            if 33 + name_len > rl {
                continue;
            }
            let raw_name = &rec[33..33 + name_len];
            if name_len == 1 && raw_name[0] <= 1 {
                continue; // "." and ".."
            }
            let flags = rec[25];
            let xattr = rec[1] as u64;
            let extent = (le32(rec, 2) as u64 + xattr) * self.block;
            let size = le32(rec, 10) as u64;
            let mut su_at = 33 + name_len + (1 - name_len % 2);
            let rr = match self.rr_skip {
                Some(skip) => {
                    su_at += skip;
                    Some(self.rock_ridge(rec.get(su_at.min(rl)..).unwrap_or(&[]))?)
                }
                None => None,
            };
            if rr.as_ref().is_some_and(|r| r.relocated) {
                continue; // shown where its CL entry points
            }
            if is_root && flags & 2 != 0 && rr.is_some() && matches!(rr.as_ref().unwrap().name.as_deref(), Some("rr_moved" | ".rr_moved")) {
                continue; // the relocation folder mkisofs makes for deep trees
            }
            let name = match (&rr, self.ns) {
                (Some(r), _) if r.name.is_some() => r.name.clone().unwrap(),
                (_, Namespace::Joliet) => strip_version(&utf16be(raw_name)),
                _ => strip_version(&lossy(raw_name)),
            };
            let mtime = rr.as_ref().and_then(|r| r.mtime).or_else(|| iso_time7(&rec[18..25]));
            let mode = rr.as_ref().and_then(|r| r.mode).map(|m| m & 0o7777);
            let hidden = flags & 1 != 0;

            if let Some(mut p) = pending.take() {
                if p.name == name && flags & 2 == 0 {
                    if let NodeRef::Extents(r) = &mut p.node.0 {
                        r.push((extent, size));
                    }
                    p.size += size;
                    if flags & 0x80 != 0 {
                        pending = Some(p);
                    } else {
                        out.push(p);
                    }
                    continue;
                }
                out.push(p); // a broken chain: keep what we have
            }

            let rr_link = rr.as_ref().and_then(|r| r.symlink.clone());
            let child = rr.as_ref().and_then(|r| r.child_link);
            let e = if let Some(target) = rr_link {
                Entry { name, kind: EntryKind::Symlink(target), size: 0, mtime, mode, hidden, synthetic: false, node: Node(NodeRef::Extents(Vec::new())) }
            } else if let Some(cl) = child {
                // Rock Ridge relocated directory: its real extent is the CL target's ".".
                let at = cl * self.block;
                let len = read_vec(&self.src, at, 34).map(|d| le32(&d, 10) as u64).unwrap_or(0);
                Entry { name, kind: EntryKind::Dir, size: 0, mtime, mode, hidden, synthetic: false, node: Node(NodeRef::IsoDir { at, len }) }
            } else if flags & 2 != 0 {
                Entry { name, kind: EntryKind::Dir, size: 0, mtime, mode, hidden, synthetic: false, node: Node(NodeRef::IsoDir { at: extent, len: size }) }
            } else {
                Entry { name, kind: EntryKind::File, size, mtime, mode, hidden, synthetic: false, node: Node(NodeRef::Extents(vec![(extent, size)])) }
            };
            if e.kind == EntryKind::File && flags & 0x80 != 0 {
                pending = Some(e);
            } else {
                out.push(e);
            }
        }
        if let Some(p) = pending {
            out.push(p);
        }
        if is_root && self.boot.is_some() && !out.iter().any(|e| e.name == BOOT_DIR) {
            out.push(Entry {
                name: BOOT_DIR.into(),
                kind: EntryKind::Dir,
                size: 0,
                mtime: None,
                mode: None,
                hidden: false,
                synthetic: true,
                node: Node(NodeRef::BootDir),
            });
        }
        Ok(out)
    }

    fn rock_ridge(&self, first: &[u8]) -> Result<Rr> {
        let mut rr = Rr::default();
        let mut area: Vec<u8> = first.to_vec();
        let mut name: Option<String> = None;
        let mut link = String::new();
        let mut in_link = false;
        let mut link_sep = false;
        for _ in 0..MAX_CE {
            let mut next: Option<(u64, u64, u64)> = None;
            let mut p = 0;
            while p + 4 <= area.len() {
                let sig = [area[p], area[p + 1]];
                let l = area[p + 2] as usize;
                if l < 4 || p + l > area.len() {
                    break;
                }
                let e = &area[p..p + l];
                match &sig {
                    b"NM" if l >= 5 => {
                        if e[4] & 6 == 0 {
                            name.get_or_insert_with(String::new).push_str(&lossy(&e[5..]));
                        }
                    }
                    b"PX" if l >= 8 => rr.mode = Some(le32(e, 4)),
                    b"TF" if l >= 5 => rr.mtime = tf_mtime(e),
                    b"SL" if l >= 5 => {
                        in_link = true;
                        let mut c = 5;
                        while c + 2 <= l {
                            let f = e[c];
                            let cl = e[c + 1] as usize;
                            let text = e.get(c + 2..c + 2 + cl).unwrap_or(&[]);
                            if f & 8 != 0 {
                                link.push('/');
                                link_sep = false;
                            } else {
                                if link_sep {
                                    link.push('/');
                                }
                                if f & 2 != 0 {
                                    link.push('.');
                                } else if f & 4 != 0 {
                                    link.push_str("..");
                                } else {
                                    link.push_str(&lossy(text));
                                }
                                link_sep = f & 1 == 0;
                            }
                            c += 2 + cl;
                        }
                    }
                    b"CL" if l >= 8 => rr.child_link = Some(le32(e, 4) as u64),
                    b"RE" => rr.relocated = true,
                    b"CE" if l >= 28 => next = Some((le32(e, 4) as u64, le32(e, 12) as u64, le32(e, 20) as u64)),
                    b"ST" => break,
                    _ => {}
                }
                p += l;
            }
            match next {
                Some((blk, off, len)) if len > 0 && len <= 64 * 1024 => {
                    area = read_vec(&self.src, blk * self.block + off, len as usize)?;
                }
                _ => break,
            }
        }
        rr.name = name.filter(|n| !n.is_empty());
        if in_link {
            rr.symlink = Some(link);
        }
        Ok(rr)
    }
}

fn synthetic(name: &str, at: u64, len: u64) -> Entry {
    Entry {
        name: name.into(),
        kind: EntryKind::File,
        size: len,
        mtime: None,
        mode: None,
        hidden: false,
        synthetic: true,
        node: Node(NodeRef::Extents(vec![(at, len)])),
    }
}

#[derive(Default)]
struct Rr {
    name: Option<String>,
    mode: Option<u32>,
    mtime: Option<i64>,
    symlink: Option<String>,
    child_link: Option<u64>,
    relocated: bool,
}

fn trim(s: &str) -> String {
    s.trim_end_matches([' ', '\0']).into()
}

/// `NAME.EXT;1` -> `NAME.EXT`, `NAME.;1` -> `NAME`.
fn strip_version(s: &str) -> String {
    let base = match s.rfind(';') {
        Some(i) if s[i + 1..].bytes().all(|b| b.is_ascii_digit()) => &s[..i],
        _ => s,
    };
    let base = base.strip_suffix('.').unwrap_or(base);
    base.into()
}

fn iso_time7(b: &[u8]) -> Option<i64> {
    if b.len() < 7 || b[1] == 0 {
        return None;
    }
    let tz = (b[6] as i8) as i64 * 15;
    util::unix_time(1900 + b[0] as i64, b[1] as u32, b[2] as u32, b[3] as u32, b[4] as u32, b[5] as u32, tz)
}

fn iso_time17(b: &[u8]) -> Option<i64> {
    if b.len() < 17 {
        return None;
    }
    let num = |r: core::ops::Range<usize>| -> Option<u32> {
        let mut v = 0u32;
        for &c in &b[r] {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as u32;
        }
        Some(v)
    };
    let y = num(0..4)?;
    if y == 0 {
        return None;
    }
    let tz = (b[16] as i8) as i64 * 15;
    util::unix_time(y as i64, num(4..6)?, num(6..8)?, num(8..10)?, num(10..12)?, num(12..14)?, tz)
}

fn tf_mtime(e: &[u8]) -> Option<i64> {
    let flags = e[4];
    let size = if flags & 0x80 != 0 { 17 } else { 7 };
    let mut at = 5;
    if flags & 1 != 0 {
        at += size; // creation
    }
    if flags & 2 == 0 {
        return None;
    }
    let b = e.get(at..at + size)?;
    if size == 17 {
        iso_time17(b)
    } else {
        iso_time7(b)
    }
}
