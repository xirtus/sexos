// SPDX-License-Identifier: GPL-3.0-only
//! Hand-built image fixtures, so the core paths run without any host tools.
#![allow(dead_code)]

pub const S: usize = 2048;

pub fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
pub fn put32be(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}
pub fn put64be(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_be_bytes());
}
/// ISO both-endian 32-bit field.
pub fn both32(b: &mut [u8], at: usize, v: u32) {
    put32(b, at, v);
    put32be(b, at + 4, v);
}
pub fn both16(b: &mut [u8], at: usize, v: u16) {
    put16(b, at, v);
    b[at + 2..at + 4].copy_from_slice(&v.to_be_bytes());
}

/// Deterministic pseudo-random bytes.
pub fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(12345);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

pub fn days(y: i64, m: u32, d: u32) -> i64 {
    // Same algorithm, written independently enough for a check.
    let mut days = 0i64;
    for yy in 1970..y {
        days += if (yy % 4 == 0 && yy % 100 != 0) || yy % 400 == 0 { 366 } else { 365 };
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let ml = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for mm in 0..(m - 1) as usize {
        days += ml[mm];
    }
    days + d as i64 - 1
}

// ------------------------------------------------------------------ ISO 9660

fn su(sig: &[u8; 2], data: &[u8]) -> Vec<u8> {
    let mut v = vec![sig[0], sig[1], (4 + data.len()) as u8, 1];
    v.extend_from_slice(data);
    v
}

fn su_px(mode: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    both32(&mut d, 0, mode);
    both32(&mut d, 8, 1);
    su(b"PX", &d)
}

fn su_tf_modify(t: [u8; 7]) -> Vec<u8> {
    let mut d = vec![0x02];
    d.extend_from_slice(&t);
    su(b"TF", &d)
}

fn su_nm(name: &str) -> Vec<u8> {
    let mut d = vec![0u8];
    d.extend_from_slice(name.as_bytes());
    su(b"NM", &d)
}

fn su_ce(block: u32, off: u32, len: u32) -> Vec<u8> {
    let mut d = vec![0u8; 24];
    both32(&mut d, 0, block);
    both32(&mut d, 8, off);
    both32(&mut d, 16, len);
    su(b"CE", &d)
}

fn dir_rec(extent: u32, size: u32, flags: u8, name: &[u8], date: [u8; 7], sua: &[u8]) -> Vec<u8> {
    let pad = if name.len() % 2 == 0 { 1 } else { 0 };
    let mut len = 33 + name.len() + pad + sua.len();
    len += len % 2;
    let mut r = vec![0u8; len];
    r[0] = len as u8;
    both32(&mut r, 2, extent);
    both32(&mut r, 10, size);
    r[18..25].copy_from_slice(&date);
    r[25] = flags;
    both16(&mut r, 28, 1);
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r[33 + name.len() + pad..33 + name.len() + pad + sua.len()].copy_from_slice(sua);
    r
}

fn ucs2(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

/// Record date 2021-06-15 12:30:45 at UTC+2 (tz byte 8 = 8 * 15 min).
pub const REC_DATE: [u8; 7] = [121, 6, 15, 12, 30, 45, 8];
pub fn rec_date_unix() -> i64 {
    days(2021, 6, 15) * 86400 + 12 * 3600 + 30 * 60 + 45 - 2 * 3600
}
/// Rock Ridge TF date 2023-01-02 03:04:05 UTC.
pub const RR_DATE: [u8; 7] = [123, 1, 2, 3, 4, 5, 0];
pub fn rr_date_unix() -> i64 {
    days(2023, 1, 2) * 86400 + 3 * 3600 + 4 * 60 + 5
}

pub const README: &[u8] = b"hello from a hand-built ISO\n";
pub const INNER: &[u8] = b"inner file\n";
pub fn big_parts() -> (Vec<u8>, Vec<u8>) {
    (pattern(2048, 1), pattern(1000, 2))
}
pub const BOOT_IMG: &[u8] = b"BIOSBOOTIMAGE";
pub const EFI_IMG: &[u8] = b"EFIBOOTIMAGE";

/// A small ISO with Rock Ridge, Joliet, a multi-extent file, a symlink whose
/// name lives in a continuation area, and an El Torito catalog with a BIOS
/// and a UEFI entry.
pub fn build_iso() -> Vec<u8> {
    // Sector map
    const PVD: usize = 16;
    const BOOTREC: usize = 17;
    const SVD: usize = 18;
    const TERM: usize = 19;
    const CATALOG: usize = 20;
    const ROOT: usize = 21;
    const JROOT: usize = 22;
    const SUB: usize = 23;
    const JSUB: usize = 24;
    const CE: usize = 25;
    const README_AT: usize = 26;
    const BIG1: usize = 27;
    const BIG2: usize = 29; // not contiguous with BIG1
    const INNER_AT: usize = 30;
    const BOOT_AT: usize = 31;
    const EFI_AT: usize = 32;
    const RRMOVED: usize = 34;
    const DEEP: usize = 35;
    const TOTAL: usize = 36;
    let mut img = vec![0u8; TOTAL * S];
    let (b1, b2) = big_parts();
    img[README_AT * S..README_AT * S + README.len()].copy_from_slice(README);
    img[BIG1 * S..BIG1 * S + b1.len()].copy_from_slice(&b1);
    img[BIG2 * S..BIG2 * S + b2.len()].copy_from_slice(&b2);
    img[INNER_AT * S..INNER_AT * S + INNER.len()].copy_from_slice(INNER);
    img[BOOT_AT * S..BOOT_AT * S + BOOT_IMG.len()].copy_from_slice(BOOT_IMG);
    img[EFI_AT * S..EFI_AT * S + EFI_IMG.len()].copy_from_slice(EFI_IMG);

    // Rock Ridge continuation area holds the symlink's NM and second SL entry.
    let mut ce = su_nm("link-to-inner");
    // SL part 2: "SubDir" (continues from part 1), then "inner.txt"
    let mut sl2 = vec![0u8];
    sl2.extend_from_slice(&[0, 6]);
    sl2.extend_from_slice(b"SubDir");
    sl2.extend_from_slice(&[0, 9]);
    sl2.extend_from_slice(b"inner.txt");
    ce.extend(su(b"SL", &sl2));
    img[CE * S..CE * S + ce.len()].copy_from_slice(&ce);

    // Root (Rock Ridge + plain)
    let mut root = Vec::new();
    let sp = su(b"SP", &[0xBE, 0xEF, 0]);
    root.extend(dir_rec(ROOT as u32, S as u32, 2, &[0], REC_DATE, &sp));
    root.extend(dir_rec(ROOT as u32, S as u32, 2, &[1], REC_DATE, &[]));
    let mut a = su_nm("readme-with-a-long-name.txt");
    a.extend(su_px(0o100644));
    a.extend(su_tf_modify(RR_DATE));
    root.extend(dir_rec(README_AT as u32, README.len() as u32, 0, b"README.TXT;1", REC_DATE, &a));
    let nm_big = su_nm("big.bin");
    root.extend(dir_rec(BIG1 as u32, b1.len() as u32, 0x80, b"BIG.BIN;1", REC_DATE, &nm_big));
    root.extend(dir_rec(BIG2 as u32, b2.len() as u32, 0, b"BIG.BIN;1", REC_DATE, &nm_big));
    // Symlink: SL part 1 is ".." (continues), name in the CE area.
    let mut l = su_px(0o120777);
    l.extend(su(b"SL", &[1, 0x04, 0]));
    l.extend(su_ce(CE as u32, 0, ce.len() as u32));
    root.extend(dir_rec(0, 0, 0, b"LINK.;1", REC_DATE, &l));
    let mut d = su_nm("SubDir");
    d.extend(su_px(0o40755));
    root.extend(dir_rec(SUB as u32, S as u32, 2, b"SUBDIR", REC_DATE, &d));
    root.extend(dir_rec(RRMOVED as u32, S as u32, 2, b"RR_MOVED", REC_DATE, &su_nm("rr_moved")));
    // Rock Ridge relocation: SubDir/deep really lives in rr_moved.
    let mut mv = Vec::new();
    mv.extend(dir_rec(RRMOVED as u32, S as u32, 2, &[0], REC_DATE, &[]));
    mv.extend(dir_rec(ROOT as u32, S as u32, 2, &[1], REC_DATE, &[]));
    let mut re = su_nm("deep");
    re.extend(su(b"RE", &[]));
    mv.extend(dir_rec(DEEP as u32, S as u32, 2, b"DEEP", REC_DATE, &re));
    img[RRMOVED * S..RRMOVED * S + mv.len()].copy_from_slice(&mv);
    let mut deep = Vec::new();
    deep.extend(dir_rec(DEEP as u32, S as u32, 2, &[0], REC_DATE, &[]));
    deep.extend(dir_rec(SUB as u32, S as u32, 2, &[1], REC_DATE, &[]));
    deep.extend(dir_rec(INNER_AT as u32, INNER.len() as u32, 0, b"LEAF.TXT;1", REC_DATE, &su_nm("leaf.txt")));
    img[DEEP * S..DEEP * S + deep.len()].copy_from_slice(&deep);
    img[ROOT * S..ROOT * S + root.len()].copy_from_slice(&root);

    let mut sub = Vec::new();
    sub.extend(dir_rec(SUB as u32, S as u32, 2, &[0], REC_DATE, &[]));
    sub.extend(dir_rec(ROOT as u32, S as u32, 2, &[1], REC_DATE, &[]));
    sub.extend(dir_rec(INNER_AT as u32, INNER.len() as u32, 0, b"INNER.TXT;1", REC_DATE, &su_nm("inner.txt")));
    let mut cl = su_nm("deep");
    let mut c = vec![0u8; 8];
    both32(&mut c, 0, DEEP as u32);
    cl.extend(su(b"CL", &c));
    sub.extend(dir_rec(0, 0, 0, b"DEEP", REC_DATE, &cl));
    img[SUB * S..SUB * S + sub.len()].copy_from_slice(&sub);

    // Joliet tree
    let mut jroot = Vec::new();
    jroot.extend(dir_rec(JROOT as u32, S as u32, 2, &[0], REC_DATE, &[]));
    jroot.extend(dir_rec(JROOT as u32, S as u32, 2, &[1], REC_DATE, &[]));
    jroot.extend(dir_rec(README_AT as u32, README.len() as u32, 0, &ucs2("Readme – Jöliet.txt;1"), REC_DATE, &[]));
    jroot.extend(dir_rec(BIG1 as u32, b1.len() as u32, 0x80, &ucs2("big.bin;1"), REC_DATE, &[]));
    jroot.extend(dir_rec(BIG2 as u32, b2.len() as u32, 0, &ucs2("big.bin;1"), REC_DATE, &[]));
    jroot.extend(dir_rec(JSUB as u32, S as u32, 2, &ucs2("SubDir"), REC_DATE, &[]));
    img[JROOT * S..JROOT * S + jroot.len()].copy_from_slice(&jroot);
    let mut jsub = Vec::new();
    jsub.extend(dir_rec(JSUB as u32, S as u32, 2, &[0], REC_DATE, &[]));
    jsub.extend(dir_rec(JROOT as u32, S as u32, 2, &[1], REC_DATE, &[]));
    jsub.extend(dir_rec(INNER_AT as u32, INNER.len() as u32, 0, &ucs2("inner.txt;1"), REC_DATE, &[]));
    img[JSUB * S..JSUB * S + jsub.len()].copy_from_slice(&jsub);

    // Volume descriptors
    let vd = |ty: u8| {
        let mut d = vec![0u8; S];
        d[0] = ty;
        d[1..6].copy_from_slice(b"CD001");
        d[6] = 1;
        d
    };
    let mut pvd = vd(1);
    pvd[40..72].fill(b' ');
    pvd[40..48].copy_from_slice(b"TESTDISC");
    both32(&mut pvd, 80, TOTAL as u32);
    both16(&mut pvd, 120, 1);
    both16(&mut pvd, 124, 1);
    both16(&mut pvd, 128, S as u16);
    let rr = dir_rec(ROOT as u32, S as u32, 2, &[0], REC_DATE, &[]);
    pvd[156..190].copy_from_slice(&rr[..34]);
    pvd[881] = 1;
    img[PVD * S..PVD * S + S].copy_from_slice(&pvd);

    let mut br = vd(0);
    br[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
    put32(&mut br, 0x47, CATALOG as u32);
    img[BOOTREC * S..BOOTREC * S + S].copy_from_slice(&br);

    let mut svd = vd(2);
    let label = ucs2("Joliet Disc");
    svd[40..72].fill(0);
    svd[40..40 + label.len()].copy_from_slice(&label);
    svd[88..91].copy_from_slice(&[0x25, 0x2F, 0x45]);
    both32(&mut svd, 80, TOTAL as u32);
    both16(&mut svd, 128, S as u16);
    let jr = dir_rec(JROOT as u32, S as u32, 2, &[0], REC_DATE, &[]);
    svd[156..190].copy_from_slice(&jr[..34]);
    img[SVD * S..SVD * S + S].copy_from_slice(&svd);
    img[TERM * S..TERM * S + S].copy_from_slice(&vd(255));

    // El Torito catalog
    let mut cat = vec![0u8; S];
    cat[0] = 1;
    cat[4..10].copy_from_slice(b"SexOS!");
    cat[30] = 0x55;
    cat[31] = 0xAA;
    let sum = (0..16).fold(0u16, |s, i| s.wrapping_add(u16::from_le_bytes([cat[2 * i], cat[2 * i + 1]])));
    put16(&mut cat, 28, 0u16.wrapping_sub(sum));
    cat[32] = 0x88;
    put16(&mut cat, 32 + 6, 4);
    put32(&mut cat, 32 + 8, BOOT_AT as u32);
    cat[64] = 0x91;
    cat[65] = 0xEF;
    put16(&mut cat, 66, 1);
    cat[96] = 0x88;
    put16(&mut cat, 96 + 6, 8);
    put32(&mut cat, 96 + 8, EFI_AT as u32);
    img[CATALOG * S..CATALOG * S + S].copy_from_slice(&cat);
    img
}

// ------------------------------------------------------------------ UDF

fn udf_tag(b: &mut [u8], id: u16, loc: u32) {
    put16(b, 0, id);
    put16(b, 2, 2);
    put32(b, 12, loc);
    let sum = b[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |s, (_, v)| s.wrapping_add(*v));
    b[4] = sum;
}

fn udf_dstring(field: &mut [u8], s: &str) {
    field[0] = 8;
    field[1..1 + s.len()].copy_from_slice(s.as_bytes());
    let n = field.len();
    field[n - 1] = (1 + s.len()) as u8;
}

fn long_ad(b: &mut [u8], at: usize, len: u32, lbn: u32, part: u16) {
    put32(b, at, len);
    put32(b, at + 4, lbn);
    put16(b, at + 8, part);
}

/// UDF timestamp 2022-02-03 04:05:06 at UTC+1.
fn udf_ts(b: &mut [u8], at: usize) {
    put16(b, at, (1 << 12) | 60);
    put16(b, at + 2, 2022);
    b[at + 4..at + 9].copy_from_slice(&[2, 3, 4, 5, 6]);
}
pub fn udf_ts_unix() -> i64 {
    days(2022, 2, 3) * 86400 + 4 * 3600 + 5 * 60 + 6 - 3600
}

pub struct UdfFe {
    pub extended: bool,
    pub file_type: u8,
    /// 0 short_ad, 1 long_ad, 3 embedded
    pub alloc: u16,
    pub size: u64,
    pub perm: u32,
    pub ads: Vec<u8>,
}

fn file_entry(fe: &UdfFe, loc: u32) -> Vec<u8> {
    let mut b = vec![0u8; S];
    put16(&mut b, 16 + 4, 4);
    put16(&mut b, 16 + 8, 1);
    b[16 + 11] = fe.file_type;
    put16(&mut b, 16 + 18, fe.alloc);
    put32(&mut b, 44, fe.perm);
    put16(&mut b, 48, 1);
    put64(&mut b, 56, fe.size);
    let (mt, ead) = if fe.extended { (92, 216) } else { (84, 176) };
    udf_ts(&mut b, mt);
    let (lea, lad) = if fe.extended { (208, 212) } else { (168, 172) };
    put32(&mut b, lea, 0);
    put32(&mut b, lad, fe.ads.len() as u32);
    b[ead..ead + fe.ads.len()].copy_from_slice(&fe.ads);
    udf_tag(&mut b, if fe.extended { 266 } else { 261 }, loc);
    b
}

fn fid(name: Option<&[u8]>, chars: u8, lbn: u32, part: u16) -> Vec<u8> {
    let fi = name.unwrap_or(&[]);
    let total = (38 + fi.len() + 3) & !3;
    let mut b = vec![0u8; total];
    put16(&mut b, 16, 1);
    b[18] = chars;
    b[19] = fi.len() as u8;
    long_ad(&mut b, 20, S as u32, lbn, part);
    b[38..38 + fi.len()].copy_from_slice(fi);
    udf_tag(&mut b, 257, 0);
    b
}

fn cs0_8(s: &str) -> Vec<u8> {
    let mut v = vec![8];
    v.extend_from_slice(s.as_bytes());
    v
}
fn cs0_16(s: &str) -> Vec<u8> {
    let mut v = vec![16];
    v.extend(ucs2(s));
    v
}

fn short_ad(len: u32, pos: u32, ty: u32) -> Vec<u8> {
    let mut v = vec![0u8; 8];
    put32(&mut v, 0, len | (ty << 30));
    put32(&mut v, 4, pos);
    v
}
fn long_ad_v(len: u32, lbn: u32, part: u16, ty: u32) -> Vec<u8> {
    let mut v = vec![0u8; 16];
    long_ad(&mut v, 0, len | (ty << 30), lbn, part);
    v
}

pub const UDF_EMBEDDED: &[u8] = b"embedded data lives in the file entry";
pub fn udf_short_data() -> Vec<u8> {
    pattern(3000, 7)
}
/// Three runs: 2048 recorded, 2048 not allocated (zeros), 500 recorded.
pub fn udf_long_data() -> Vec<u8> {
    let mut v = pattern(2048, 8);
    v.extend(vec![0u8; 2048]);
    v.extend(pattern(500, 9));
    v
}

/// A UDF volume. `metadata`: put every ICB and folder in a UDF 2.50
/// metadata partition (the Blu-ray layout) instead of the physical one.
/// `continuation`: split the long-file allocation descriptors with an
/// allocation extent descriptor.
pub fn build_udf(metadata: bool, revision: u16) -> Vec<u8> {
    const PART_START: usize = 272;
    const PART_LEN: usize = 128;
    let total = PART_START + PART_LEN + 1;
    let mut img = vec![0u8; total * S];
    for (i, id) in [b"BEA01", if revision >= 0x200 { b"NSR03" } else { b"NSR02" }, b"TEA01"].iter().enumerate() {
        let at = (16 + i) * S;
        img[at + 1..at + 6].copy_from_slice(*id);
        img[at + 6] = 1;
    }
    // Data placement (blocks relative to the physical partition).
    const META_FILE_FE: u32 = 0;
    const META_AREA: u32 = 16; // 32 blocks of metadata
    const SHORT_DATA: u32 = 60;
    const LONG_A: u32 = 64;
    const LONG_B: u32 = 66;
    const AED_BLOCK: u32 = 70;
    // Metadata blocks (relative to the metadata or physical partition).
    let mb = |n: u32| if metadata { n } else { n + 100 };
    let (fsd, root_fe, root_dir, emb_fe, short_fe, sub_fe, sub_dir, link_fe, long_fe, abs_link_fe) =
        (mb(0), mb(1), mb(2), mb(3), mb(4), mb(5), mb(6), mb(7), mb(8), mb(9));
    let mref: u16 = if metadata { 1 } else { 0 };
    let mut part = vec![0u8; PART_LEN * S];
    let mut meta = vec![0u8; 32 * S];
    let mut put_meta = |n: u32, data: &[u8], part: &mut Vec<u8>| {
        if metadata {
            meta[n as usize * S..n as usize * S + data.len()].copy_from_slice(data);
        } else {
            part[n as usize * S..n as usize * S + data.len()].copy_from_slice(data);
        }
    };

    // FSD
    let mut f = vec![0u8; S];
    udf_dstring(&mut f[304..336], "fileset");
    long_ad(&mut f, 400, S as u32, root_fe, mref);
    udf_tag(&mut f, 256, fsd);
    put_meta(fsd, &f, &mut part);

    // Root folder
    let mut dir = Vec::new();
    dir.extend(fid(None, 8 | 2, root_fe, mref));
    dir.extend(fid(Some(&cs0_8("embedded.txt")), 0, emb_fe, mref));
    dir.extend(fid(Some(&cs0_16("日本語.bin")), 0, short_fe, mref));
    dir.extend(fid(Some(&cs0_8("Sub Folder")), 2, sub_fe, mref));
    dir.extend(fid(Some(&cs0_8("gone.txt")), 4, emb_fe, mref)); // deleted
    dir.extend(fid(Some(&cs0_8("link")), 0, link_fe, mref));
    dir.extend(fid(Some(&cs0_8("abs-link")), 1, abs_link_fe, mref)); // hidden
    let root_len = dir.len();
    put_meta(root_dir, &dir, &mut part);
    let fe = file_entry(&UdfFe { extended: revision >= 0x200, file_type: 4, alloc: 0, size: root_len as u64, perm: 0x14A5 | (0x1F << 10), ads: short_ad(root_len as u32, root_dir, 0) }, root_fe);
    put_meta(root_fe, &fe, &mut part);

    // Embedded file
    let fe = file_entry(&UdfFe { extended: false, file_type: 5, alloc: 3, size: UDF_EMBEDDED.len() as u64, perm: (0x5 << 10) | (0x4 << 5) | 0x4, ads: UDF_EMBEDDED.to_vec() }, emb_fe);
    put_meta(emb_fe, &fe, &mut part);

    // short_ad file in the physical partition (via long_ad when metadata is used)
    let sd = udf_short_data();
    part[SHORT_DATA as usize * S..SHORT_DATA as usize * S + sd.len()].copy_from_slice(&sd);
    let (alloc, ads) = if metadata { (1, long_ad_v(sd.len() as u32, SHORT_DATA, 0, 0)) } else { (0, short_ad(sd.len() as u32, SHORT_DATA, 0)) };
    let fe = file_entry(&UdfFe { extended: true, file_type: 5, alloc, size: sd.len() as u64, perm: 0x7FFF, ads }, short_fe);
    put_meta(short_fe, &fe, &mut part);

    // Sub folder with the long_ad file
    let mut sdir = Vec::new();
    sdir.extend(fid(None, 8 | 2, root_fe, mref));
    sdir.extend(fid(Some(&cs0_8("long.dat")), 0, long_fe, mref));
    let sub_len = sdir.len();
    put_meta(sub_dir, &sdir, &mut part);
    let fe = file_entry(&UdfFe { extended: true, file_type: 4, alloc: 1, size: sub_len as u64, perm: 0x7FFF, ads: long_ad_v(sub_len as u32, sub_dir, mref, 0) }, sub_fe);
    put_meta(sub_fe, &fe, &mut part);

    // long_ad file: run A, unallocated run, then (through an AED) run B.
    let ld = udf_long_data();
    part[LONG_A as usize * S..LONG_A as usize * S + 2048].copy_from_slice(&ld[..2048]);
    part[LONG_B as usize * S..LONG_B as usize * S + 500].copy_from_slice(&ld[4096..]);
    let mut ads = long_ad_v(2048, LONG_A, 0, 0);
    ads.extend(long_ad_v(2048, 0, 0, 2));
    ads.extend(long_ad_v(S as u32, AED_BLOCK, 0, 3));
    let mut aed = vec![0u8; S];
    let tail = long_ad_v(500, LONG_B, 0, 0);
    put32(&mut aed, 20, tail.len() as u32);
    aed[24..24 + tail.len()].copy_from_slice(&tail);
    udf_tag(&mut aed, 258, AED_BLOCK);
    part[AED_BLOCK as usize * S..AED_BLOCK as usize * S + S].copy_from_slice(&aed);
    let fe = file_entry(&UdfFe { extended: false, file_type: 5, alloc: 1, size: ld.len() as u64, perm: 0x7FFF, ads }, long_fe);
    put_meta(long_fe, &fe, &mut part);

    // Symlinks (embedded path components)
    let mut pc = vec![3, 0, 0, 0];
    let id = cs0_8("Sub Folder");
    pc.extend_from_slice(&[5, id.len() as u8, 0, 0]);
    pc.extend(&id);
    let id = cs0_8("long.dat");
    pc.extend_from_slice(&[5, id.len() as u8, 0, 0]);
    pc.extend(&id);
    let fe = file_entry(&UdfFe { extended: false, file_type: 12, alloc: 3, size: pc.len() as u64, perm: 0x7FFF, ads: pc }, link_fe);
    put_meta(link_fe, &fe, &mut part);
    let mut pc = vec![2, 0, 0, 0];
    for c in ["etc", "hosts"] {
        let id = cs0_8(c);
        pc.extend_from_slice(&[5, id.len() as u8, 0, 0]);
        pc.extend(&id);
    }
    let fe = file_entry(&UdfFe { extended: false, file_type: 12, alloc: 3, size: pc.len() as u64, perm: 0x7FFF, ads: pc }, abs_link_fe);
    put_meta(abs_link_fe, &fe, &mut part);

    if metadata {
        part[META_AREA as usize * S..(META_AREA as usize + 32) * S].copy_from_slice(&meta);
        // Metadata file (type 250) in the physical partition: one short_ad over the area.
        let fe = file_entry(&UdfFe { extended: true, file_type: 250, alloc: 0, size: (32 * S) as u64, perm: 0, ads: short_ad((32 * S) as u32, META_AREA, 0) }, META_FILE_FE);
        part[META_FILE_FE as usize * S..META_FILE_FE as usize * S + S].copy_from_slice(&fe);
    }
    img[PART_START * S..(PART_START + PART_LEN) * S].copy_from_slice(&part);

    // Volume descriptor sequence at 32..
    let mut pvd = vec![0u8; S];
    udf_dstring(&mut pvd[24..56], "PVDLABEL");
    udf_tag(&mut pvd, 1, 32);
    let mut pd = vec![0u8; S];
    put16(&mut pd, 22, 7); // partition number 7
    pd[25..31].copy_from_slice(b"+NSR03");
    put32(&mut pd, 188, PART_START as u32);
    put32(&mut pd, 192, PART_LEN as u32);
    udf_tag(&mut pd, 5, 33);
    let mut lvd = vec![0u8; S];
    udf_dstring(&mut lvd[84..212], "UDF Test Volume");
    put32(&mut lvd, 212, S as u32);
    lvd[217..236].copy_from_slice(b"*OSTA UDF Compliant");
    put16(&mut lvd, 240, revision);
    long_ad(&mut lvd, 248, S as u32, fsd, mref);
    let mut maps = vec![1u8, 6, 1, 0, 7, 0];
    if metadata {
        let mut m = vec![0u8; 64];
        m[0] = 2;
        m[1] = 64;
        m[5..28].copy_from_slice(b"*UDF Metadata Partition");
        put16(&mut m, 28, revision);
        put16(&mut m, 36, 1);
        put16(&mut m, 38, 7);
        put32(&mut m, 40, META_FILE_FE);
        put32(&mut m, 44, META_FILE_FE);
        put32(&mut m, 48, u32::MAX);
        put32(&mut m, 52, 32);
        put16(&mut m, 56, 32);
        maps.extend(m);
    }
    put32(&mut lvd, 264, maps.len() as u32);
    put32(&mut lvd, 268, if metadata { 2 } else { 1 });
    lvd[440..440 + maps.len()].copy_from_slice(&maps);
    udf_tag(&mut lvd, 6, 34);
    let mut td = vec![0u8; S];
    udf_tag(&mut td, 8, 35);
    for (i, d) in [pvd, pd, lvd, td].iter().enumerate() {
        img[(32 + i) * S..(33 + i) * S].copy_from_slice(d);
    }
    let mut avdp = vec![0u8; S];
    put32(&mut avdp, 16, (4 * S) as u32);
    put32(&mut avdp, 20, 32);
    put32(&mut avdp, 24, (4 * S) as u32);
    put32(&mut avdp, 28, 32);
    udf_tag(&mut avdp, 2, 256);
    img[256 * S..257 * S].copy_from_slice(&avdp);
    img
}

// ------------------------------------------------------------------ disks

/// A FAT16 boot sector (no FATs or data needed for the signature probe).
pub fn fat16_boot() -> Vec<u8> {
    let mut b = vec![0u8; 512];
    b[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    b[3..11].copy_from_slice(b"MSWIN4.1");
    put16(&mut b, 11, 512);
    b[13] = 4;
    put16(&mut b, 14, 4);
    b[16] = 2;
    put16(&mut b, 17, 512);
    put32(&mut b, 32, 40000);
    put16(&mut b, 22, 40);
    b[510] = 0x55;
    b[511] = 0xAA;
    b
}

/// The start of an ext4 filesystem (superblock magic and extents feature).
pub fn ext4_head() -> Vec<u8> {
    let mut b = vec![0u8; 2048];
    put16(&mut b, 1024 + 56, 0xEF53);
    put32(&mut b, 1024 + 92, 4);
    put32(&mut b, 1024 + 96, 0x40 | 0x2);
    b
}

fn mbr_entry(b: &mut [u8], i: usize, boot: bool, ty: u8, start: u32, count: u32) {
    let e = 446 + 16 * i;
    b[e] = if boot { 0x80 } else { 0 };
    b[e + 4] = ty;
    put32(b, e + 8, start);
    put32(b, e + 12, count);
}

/// 8 MiB raw disk: MBR with a bootable FAT16 primary, an extended partition
/// holding an ext4 logical and a swap logical, and a NTFS primary.
pub fn build_mbr_disk() -> Vec<u8> {
    let mut d = vec![0u8; 8 << 20];
    mbr_entry(&mut d, 0, true, 0x06, 2048, 2048);
    mbr_entry(&mut d, 1, false, 0x0F, 4096, 8192);
    mbr_entry(&mut d, 2, false, 0x07, 12288, 4096);
    d[510] = 0x55;
    d[511] = 0xAA;
    d[2048 * 512..2048 * 512 + 512].copy_from_slice(&fat16_boot());
    // EBR 1 at 4096: logical at +64 (1024 sectors), next EBR at ext+2048
    let e1 = 4096 * 512;
    let mut ebr = vec![0u8; 512];
    mbr_entry(&mut ebr, 0, false, 0x83, 64, 1024);
    mbr_entry(&mut ebr, 1, false, 0x05, 2048, 2048);
    ebr[510] = 0x55;
    ebr[511] = 0xAA;
    d[e1..e1 + 512].copy_from_slice(&ebr);
    let l1 = (4096 + 64) * 512;
    d[l1..l1 + 2048].copy_from_slice(&ext4_head());
    let e2 = (4096 + 2048) * 512;
    let mut ebr = vec![0u8; 512];
    mbr_entry(&mut ebr, 0, false, 0x82, 64, 1024);
    ebr[510] = 0x55;
    ebr[511] = 0xAA;
    d[e2..e2 + 512].copy_from_slice(&ebr);
    let l2 = (4096 + 2048 + 64) * 512;
    d[l2 + 4086..l2 + 4096].copy_from_slice(b"SWAPSPACE2");
    let n = 12288 * 512;
    d[n + 3..n + 11].copy_from_slice(b"NTFS    ");
    d
}

// ------------------------------------------------------------------ containers

/// qcow2 v3, 64 KiB clusters, over `disk`. Clusters that are all zeros are
/// left unallocated, every third one is stored compressed, and the zero
/// flag is used once for a cluster whose host data is garbage.
pub fn build_qcow2(disk: &[u8]) -> Vec<u8> {
    let cb = 16u32;
    let cs = 1usize << cb;
    let clusters = disk.len().div_ceil(cs);
    assert!(clusters <= cs / 8);
    let mut img = vec![0u8; 3 * cs];
    img[0..4].copy_from_slice(b"QFI\xfb");
    put32be(&mut img, 4, 3);
    put32be(&mut img, 20, cb);
    put64be(&mut img, 24, disk.len() as u64);
    put32be(&mut img, 36, 1);
    put64be(&mut img, 40, cs as u64);
    put32be(&mut img, 96, 4);
    put32be(&mut img, 100, 104);
    put64be(&mut img, cs, (2 * cs) as u64 | 1 << 63); // L1[0] -> L2 at cluster 2, copied flag
    let mut l2 = vec![0u64; cs / 8];
    let mut zero_flag_done = false;
    for c in 0..clusters {
        let data = &disk[c * cs..((c + 1) * cs).min(disk.len())];
        if data.iter().all(|&b| b == 0) {
            if !zero_flag_done {
                // Allocated garbage that must read as zeros because of the flag.
                img.resize(img.len().next_multiple_of(cs), 0);
                let at = img.len();
                img.extend(vec![0xAAu8; cs]);
                l2[c] = at as u64 | 1;
                zero_flag_done = true;
            }
            continue;
        }
        if c % 3 == 1 {
            let comp = miniz_oxide::deflate::compress_to_vec(data, 6);
            // Start mid-sector to exercise the offset arithmetic.
            while img.len() % 512 != 100 {
                img.push(0);
            }
            let at = img.len() as u64;
            img.extend(&comp);
            let sectors = ((at % 512) as usize + comp.len()).div_ceil(512) as u64;
            let x = 62 - (cb - 8);
            l2[c] = 1 << 62 | (sectors - 1) << x | at;
        } else {
            while img.len() % cs != 0 {
                img.push(0);
            }
            let at = img.len() as u64;
            img.extend(data);
            img.resize(at as usize + cs, 0);
            l2[c] = at | 1 << 63;
        }
    }
    for (i, e) in l2.iter().enumerate() {
        put64be(&mut img, 2 * cs + i * 8, *e);
    }
    img
}

fn vhd_footer(size: u64, ty: u32, data_offset: u64) -> Vec<u8> {
    let mut f = vec![0u8; 512];
    f[0..8].copy_from_slice(b"conectix");
    put32be(&mut f, 8, 2);
    put32be(&mut f, 12, 0x0001_0000);
    put64be(&mut f, 16, data_offset);
    f[28..32].copy_from_slice(b"sxos");
    put64be(&mut f, 40, size);
    put64be(&mut f, 48, size);
    put32be(&mut f, 60, ty);
    let sum: u32 = f.iter().map(|&b| b as u32).sum();
    put32be(&mut f, 64, !sum);
    f
}

pub fn build_vhd_fixed(disk: &[u8]) -> Vec<u8> {
    let mut v = disk.to_vec();
    v.extend(vhd_footer(disk.len() as u64, 2, u64::MAX));
    v
}

/// Dynamic VHD with 64 KiB blocks; all-zero blocks are left out.
pub fn build_vhd_dynamic(disk: &[u8]) -> Vec<u8> {
    let bs = 64 * 1024usize;
    let blocks = disk.len().div_ceil(bs);
    let bat_at = 1536usize;
    let bat_len = (blocks * 4).next_multiple_of(512);
    let mut v = vhd_footer(disk.len() as u64, 3, 512);
    let mut dh = vec![0u8; 1024];
    dh[0..8].copy_from_slice(b"cxsparse");
    put64be(&mut dh, 8, u64::MAX);
    put64be(&mut dh, 16, bat_at as u64);
    put32be(&mut dh, 24, 0x0001_0000);
    put32be(&mut dh, 28, blocks as u32);
    put32be(&mut dh, 32, bs as u32);
    let sum: u32 = dh.iter().map(|&b| b as u32).sum();
    put32be(&mut dh, 36, !sum);
    v.extend(dh);
    v.extend(vec![0xFFu8; bat_len]);
    for b in 0..blocks {
        let data = &disk[b * bs..((b + 1) * bs).min(disk.len())];
        if data.iter().all(|&x| x == 0) {
            continue;
        }
        let sector = (v.len() / 512) as u32;
        put32be(&mut v, bat_at + b * 4, sector);
        v.extend(vec![0xFFu8; 512]); // sector bitmap
        v.extend(data);
        v.resize(v.len() + bs - data.len(), 0);
    }
    v.extend(vhd_footer(disk.len() as u64, 3, 512));
    v
}

pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
        }
    }
    !c
}

const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
}

/// VHDX with 1 MiB blocks: all-zero blocks alternate between NOT_PRESENT and
/// ZERO states. Two headers, the second with the higher sequence number.
pub fn build_vhdx(disk: &[u8]) -> Vec<u8> {
    const MB: usize = 1 << 20;
    let bs = MB;
    let blocks = disk.len().div_ceil(bs);
    let mut v = vec![0u8; 3 * MB];
    v[0..8].copy_from_slice(b"vhdxfile");
    for (i, seq) in [(64 * 1024, 1u64), (128 * 1024, 2)] {
        let mut h = vec![0u8; 4096];
        h[0..4].copy_from_slice(b"head");
        put64(&mut h, 8, seq);
        put16(&mut h, 66, 1);
        put32(&mut h, 68, MB as u32);
        put64(&mut h, 72, MB as u64);
        let c = crc32c(&h);
        put32(&mut h, 4, c);
        v[i..i + 4096].copy_from_slice(&h);
    }
    let mut r = vec![0u8; 64 * 1024];
    r[0..4].copy_from_slice(b"regi");
    put32(&mut r, 8, 2);
    let bat_guid = guid(0x2DC27766, 0xF623, 0x4200, [0x9D, 0x64, 0x11, 0x5E, 0x9B, 0xFD, 0x4A, 0x08]);
    let meta_guid = guid(0x8B7CA206, 0x4790, 0x4B9A, [0xB8, 0xFE, 0x57, 0x5F, 0x05, 0x0F, 0x88, 0x6E]);
    r[16..32].copy_from_slice(&bat_guid);
    put64(&mut r, 32, 2 * MB as u64);
    put32(&mut r, 40, MB as u32);
    put32(&mut r, 44, 1);
    r[48..64].copy_from_slice(&meta_guid);
    put64(&mut r, 64, MB as u64);
    put32(&mut r, 72, MB as u32);
    put32(&mut r, 76, 1);
    let c = crc32c(&r);
    put32(&mut r, 4, c);
    // Region table copy 1 is damaged on purpose; copy 2 is good.
    let mut bad = r.clone();
    bad[100] ^= 1;
    v[192 * 1024..256 * 1024].copy_from_slice(&bad);
    v[256 * 1024..320 * 1024].copy_from_slice(&r);
    // Metadata
    let m = MB;
    v[m..m + 8].copy_from_slice(b"metadata");
    put16(&mut v, m + 10, 3);
    let items: [([u8; 16], Vec<u8>); 3] = [
        (guid(0xCAA16737, 0xFA36, 0x4D43, [0xB3, 0xB6, 0x33, 0xF0, 0xAA, 0x44, 0xE7, 0x6B]), {
            let mut x = vec![0u8; 8];
            put32(&mut x, 0, bs as u32);
            x
        }),
        (guid(0x2FA54224, 0xCD1B, 0x4876, [0xB2, 0x11, 0x5D, 0xBE, 0xD8, 0x3B, 0xF4, 0xB8]), (disk.len() as u64).to_le_bytes().to_vec()),
        (guid(0x8141BF1D, 0xA96F, 0x4709, [0xBA, 0x47, 0xF2, 0x33, 0xA8, 0xFA, 0xAB, 0x5F]), 512u32.to_le_bytes().to_vec()),
    ];
    for (i, (id, data)) in items.iter().enumerate() {
        let e = m + 32 + i * 32;
        v[e..e + 16].copy_from_slice(id);
        let off = 65536 + i * 64;
        put32(&mut v, e + 16, off as u32);
        put32(&mut v, e + 20, data.len() as u32);
        put32(&mut v, e + 24, 4);
        v[m + off..m + off + data.len()].copy_from_slice(data);
    }
    // BAT at 2 MiB; data from 3 MiB.
    let chunk_ratio = (1usize << 23) * 512 / bs;
    for b in 0..blocks {
        let data = &disk[b * bs..((b + 1) * bs).min(disk.len())];
        let idx = b + b / chunk_ratio;
        let entry = if data.iter().all(|&x| x == 0) {
            if b % 2 == 0 { 0 } else { 2 }
        } else {
            let at = v.len();
            v.extend(data);
            v.resize(at + bs, 0);
            at as u64 | 6
        };
        put64(&mut v, 2 * MB + idx * 8, entry);
    }
    v
}

/// VMDK monolithicSparse (or streamOptimized with zlib grains), 64 KiB grains.
pub fn build_vmdk(disk: &[u8], stream: bool, parent: bool) -> Vec<u8> {
    let grain = 64 * 1024usize;
    let grains = disk.len().div_ceil(grain);
    let gtes = 512usize;
    let gts = grains.div_ceil(gtes);
    let mut h = vec![0u8; 512];
    h[0..4].copy_from_slice(b"KDMV");
    put32(&mut h, 4, if stream { 3 } else { 1 });
    put32(&mut h, 8, 3 | if stream { 3 << 16 } else { 0 });
    put64(&mut h, 12, (disk.len() / 512) as u64);
    put64(&mut h, 20, (grain / 512) as u64);
    put64(&mut h, 28, 1);
    put64(&mut h, 36, 2);
    put32(&mut h, 44, gtes as u32);
    h[73] = b'\n';
    h[74] = b' ';
    h[75] = b'\r';
    h[76] = b'\n';
    if stream {
        put16(&mut h, 77, 1);
    }
    let mut v = h.clone();
    let desc = format!(
        "# Disk DescriptorFile\nversion=1\nCID=12345678\nparentCID={}\ncreateType=\"{}\"\n",
        if parent { "abcdef01" } else { "ffffffff" },
        if stream { "streamOptimized" } else { "monolithicSparse" }
    );
    let mut d = desc.into_bytes();
    d.resize(1024, 0);
    v.extend(d);
    // Grain tables follow (sector aligned), then grains.
    let gd_sector = v.len() / 512;
    v.resize(v.len() + (gts * 4).next_multiple_of(512), 0);
    let gt_sector = v.len() / 512;
    v.resize(v.len() + gts * gtes * 4, 0);
    for (i, _) in (0..gts).enumerate() {
        put32(&mut v, gd_sector * 512 + i * 4, (gt_sector + i * gtes * 4 / 512) as u32);
    }
    for g in 0..grains {
        let data = &disk[g * grain..((g + 1) * grain).min(disk.len())];
        if data.iter().all(|&x| x == 0) {
            continue;
        }
        let sector = (v.len() / 512) as u32;
        if stream {
            let comp = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
            let mut m = vec![0u8; 12];
            put64(&mut m, 0, (g * grain / 512) as u64);
            put32(&mut m, 8, comp.len() as u32);
            v.extend(m);
            v.extend(comp);
        } else {
            v.extend(data);
            v.resize(v.len() + grain - data.len(), 0);
        }
        v.resize(v.len().next_multiple_of(512), 0);
        put32(&mut v, gt_sector * 512 + g * 4, sector);
    }
    if stream {
        put64(&mut v, 56, u64::MAX);
        // footer marker, footer header, end-of-stream marker
        let mut fm = vec![0u8; 512];
        put32(&mut fm, 12, 3);
        v.extend(fm);
        let mut fh = h.clone();
        put64(&mut fh, 56, gd_sector as u64);
        v.extend(fh);
        v.extend(vec![0u8; 512]);
    } else {
        put64(&mut v, 56, gd_sector as u64);
    }
    v
}

/// A 6 MiB disk with a GPT written by `sex-disk`: an EFI System partition
/// holding a FAT16 boot sector and a named Linux partition holding ext4.
pub fn build_gpt_disk() -> Vec<u8> {
    use sex_disk::gpt::{Gpt, Guid, TYPE_ESP};
    let mut dev = sex_disk::MemDev(vec![0u8; 6 << 20]);
    let linux = Guid::new(0x0FC63DAF, 0x8483, 0x4772, [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4]);
    let mut g = Gpt::new_empty(6 << 11, Guid::random([1; 16])).unwrap();
    g.add(TYPE_ESP, Guid::random([2; 16]), 2048, 2048, "EFI system").unwrap();
    g.add(linux, Guid::random([3; 16]), 4096, 4096, "Wurzel – root").unwrap();
    g.write(&mut dev).unwrap();
    let mut d = dev.0;
    d[2048 * 512..2048 * 512 + 512].copy_from_slice(&fat16_boot());
    d[4096 * 512..4096 * 512 + 2048].copy_from_slice(&ext4_head());
    d
}
