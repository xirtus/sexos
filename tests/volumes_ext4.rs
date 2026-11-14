// SPDX-License-Identifier: GPL-3.0-only
//! ext2/3/4 read only, against images made by Linux's own mke2fs. The fixtures are rebuilt
//! by a generator that writes the same contents `pattern` recomputes here:
//!   ext4.sparse       16 MiB, 4K blocks, default ext4 features, label LINUXDATA: nested
//!                     dirs, a 300 KiB file with a 100 KiB hole, a 10-extent file (extent
//!                     tree of depth 1), an htree dir of 150 entries, a unicode name, a
//!                     symlink, an empty file
//!   ext2.sparse       4 MiB, 1K blocks, no filetype: a sparse 70 MiB file whose data sits
//!                     in direct, single, double and triple indirect blocks
//!   ext4inline.sparse 8 MiB, 1K blocks, inline_data, 64bit, meta_bg: inline files and an
//!                     inline dir, a 400-extent file (extent tree of depth 2)
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
use sv::{Disk, Entry, Error, Format, Kind, Volume};

fn pattern(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9) | 1;
    let mut out = Vec::with_capacity(len + 4);
    while out.len() < len { x ^= x << 13; x ^= x >> 17; x ^= x << 5; out.extend_from_slice(&x.to_le_bytes()); }
    out.truncate(len);
    out
}
/// A file built from (offset, bytes) chunks, zeros elsewhere.
fn sparse(chunks: &[(usize, Vec<u8>)], size: usize) -> Vec<u8> {
    let mut v = vec![0; size];
    for (at, d) in chunks { v[*at..*at + d.len()].copy_from_slice(d); }
    v
}
fn mount(name: &str) -> (common::MemDisk, sv::ext4::Ext4) {
    let mut d = common::fixture(name);
    let v = sv::ext4::Ext4::mount(&mut d).unwrap();
    (d, v)
}
fn read_all(v: &mut dyn Volume, d: &mut dyn Disk, path: &str) -> Vec<u8> {
    let size = v.stat(d, path.as_bytes()).unwrap().size as usize;
    let mut buf = vec![0xAA; size + 100];
    assert_eq!(v.read(d, path.as_bytes(), 0, &mut buf).unwrap(), size, "{path}");
    buf.truncate(size);
    buf
}
/// Equal bytes, or the first difference (not two megabyte dumps).
#[track_caller]
fn same(got: &[u8], want: &[u8], what: &str) {
    if let Some(i) = got.iter().zip(want).position(|(a, b)| a != b) { panic!("{what}: differs at {i}: {:02x?} vs {:02x?}", &got[i..(i + 16).min(got.len())], &want[i..(i + 16).min(want.len())]); }
    assert_eq!(got.len(), want.len(), "{what}: length");
}
fn names(mut l: Vec<Entry>) -> Vec<(String, Kind, u64)> {
    l.sort_by(|a, b| a.name.cmp(&b.name));
    l.into_iter().map(|e| (e.name, e.kind, e.size)).collect()
}

#[test]
fn ext4_root_and_metadata() {
    let (mut d, mut v) = mount("ext4");
    assert_eq!((v.format(), v.label().as_str(), v.read_only()), (Format::Ext4, "LINUXDATA", false));
    let (total, free) = v.statfs(&mut d).unwrap();
    assert_eq!(total, 16 << 20);
    assert!(free > 0 && free < total);
    let f = |n: &str, k, s| (n.to_string(), k, s);
    assert_eq!(names(v.list(&mut d, b"/").unwrap()), vec![
        f("docs", Kind::Dir, 0), f("empty", Kind::File, 0), f("frag.bin", Kind::File, 81920), f("hello.txt", Kind::File, 17),
        f("holey.bin", Kind::File, 307200), f("héllo-世界.txt", Kind::File, 8), f("link", Kind::File, 19),
        f("lost+found", Kind::Dir, 0), f("many", Kind::Dir, 0), f("tail.bin", Kind::File, 70000),
    ]);
    let st = v.stat(&mut d, b"hello.txt").unwrap();
    assert_eq!((st.kind, st.size, st.mtime), (Kind::File, 17, 1_700_000_000));
    assert_eq!(v.stat(&mut d, b"").unwrap().kind, Kind::Dir);
    assert_eq!(v.stat(&mut d, b"/docs/notes").unwrap().kind, Kind::Dir);
}

#[test]
fn ext4_contents() {
    let (mut d, mut v) = mount("ext4");
    assert_eq!(read_all(&mut v, &mut d, "hello.txt"), b"Hello from Linux\n");
    assert_eq!(read_all(&mut v, &mut d, "empty"), b"");
    assert_eq!(read_all(&mut v, &mut d, "/docs/notes/deep.txt"), pattern(1, 5000));
    assert_eq!(read_all(&mut v, &mut d, "héllo-世界.txt"), b"unicode\n");
    assert_eq!(read_all(&mut v, &mut d, "link"), b"docs/notes/deep.txt");
    same(&read_all(&mut v, &mut d, "tail.bin"), &pattern(4, 70000), "tail");
    let holey = sparse(&[(0, pattern(2, 102400)), (204800, pattern(3, 102400))], 307200);
    same(&read_all(&mut v, &mut d, "holey.bin"), &holey, "holey");
    let frag = sparse(&(0..10).map(|i| (i * 8192, pattern(10 + i as u32, 4096))).collect::<Vec<_>>(), 81920);
    same(&read_all(&mut v, &mut d, "frag.bin"), &frag, "frag");
    // Odd offsets and lengths, across extent and hole edges and past the end.
    for (file, want) in [("holey.bin", &holey), ("frag.bin", &frag)] {
        for (off, len) in [(1usize, 1usize), (4095, 3), (102399, 2), (102300, 102600), (8191, 8194), (307190, 50), (81919, 9), (5000, 77777)] {
            let mut buf = vec![0xEE; len];
            let n = v.read(&mut d, file.as_bytes(), off as u64, &mut buf).unwrap();
            let end = (off + len).min(want.len()).max(off.min(want.len()));
            same(&buf[..n], &want[off.min(want.len())..end], &format!("{file} {off}+{len}"));
        }
    }
    assert_eq!(v.read(&mut d, b"hello.txt", 17, &mut [0; 4]).unwrap(), 0);
    assert_eq!(v.read(&mut d, b"hello.txt", 1 << 40, &mut [0; 4]).unwrap(), 0);
}

#[test]
fn ext4_htree_dir() {
    let (mut d, mut v) = mount("ext4");
    let l = names(v.list(&mut d, b"many").unwrap());
    assert_eq!(l.len(), 150);
    for (i, (n, k, s)) in l.iter().enumerate() {
        assert_eq!((n.as_str(), *k, *s), (format!("a-rather-long-file-name-number-{i:03}").as_str(), Kind::File, 9));
    }
    let p = b"many/a-rather-long-file-name-number-123";
    assert_eq!(read_all(&mut v, &mut d, std::str::from_utf8(p).unwrap()), b"file 123\n");
}

#[test]
fn ext4_errors_and_read_only() {
    let (mut d, mut v) = mount("ext4");
    assert_eq!(v.stat(&mut d, b"nope").unwrap_err(), Error::NotFound);
    assert_eq!(v.stat(&mut d, b"HELLO.TXT").unwrap_err(), Error::NotFound);
    assert_eq!(v.stat(&mut d, b"hello.txt/x").unwrap_err(), Error::NotDir);
    assert_eq!(v.list(&mut d, b"hello.txt").unwrap_err(), Error::NotDir);
    assert_eq!(v.read(&mut d, b"docs", 0, &mut [0; 8]).unwrap_err(), Error::IsDir);
    assert_eq!(v.stat(&mut d, b"link/x").unwrap_err(), Error::Unsupported);
    assert_eq!(v.stat(&mut d, b"docs/../hello.txt").unwrap_err(), Error::Name);
    // New names don't go into an htree directory; ext2 (no extents) stays read only.
    assert_eq!(v.create(&mut d, b"many/new").unwrap_err(), Error::Unsupported);
    let (mut d2, mut old) = mount("ext2");
    assert!(old.read_only());
    assert_eq!(old.write(&mut d2, b"small.txt", 0, b"x").unwrap_err(), Error::ReadOnly);
    assert_eq!(old.create(&mut d2, b"new").unwrap_err(), Error::ReadOnly);
    assert_eq!(old.mkdir(&mut d2, b"newdir").unwrap_err(), Error::ReadOnly);
    assert_eq!(old.remove(&mut d2, b"small.txt").unwrap_err(), Error::ReadOnly);
}

#[test]
fn ext2_indirect_blocks() {
    let (mut d, mut v) = mount("ext2");
    assert_eq!(v.label(), "OLDDISK");
    assert_eq!(v.statfs(&mut d).unwrap().0, 4 << 20);
    assert_eq!(names(v.list(&mut d, b"").unwrap()), vec![
        ("big.bin".into(), Kind::File, 70 << 20), ("lost+found".into(), Kind::Dir, 0),
        ("small.txt".into(), Kind::File, 13), ("sub".into(), Kind::Dir, 0),
    ]);
    assert_eq!(read_all(&mut v, &mut d, "small.txt"), b"ext2 says hi\n");
    assert_eq!(read_all(&mut v, &mut d, "sub/dir/x.txt"), pattern(24, 1500));
    let chunks = [(0, pattern(20, 3000)), (200 << 10, pattern(21, 2000)), (1 << 20, pattern(22, 1500)), ((66 << 20) + 123, pattern(23, 2500))];
    let big = sparse(&chunks, 70 << 20);
    same(&read_all(&mut v, &mut d, "big.bin"), &big, "big");
    for (off, len) in [(2999usize, 10usize), ((200 << 10) - 7, 2010), ((1 << 20) + 1499, 2), ((66 << 20) + 100, 3000), ((70 << 20) - 5, 10)] {
        let mut buf = vec![0xEE; len];
        let n = v.read(&mut d, b"big.bin", off as u64, &mut buf).unwrap();
        same(&buf[..n], &big[off..(off + len).min(big.len())], "big range");
    }
}

#[test]
fn ext4_inline_and_deep_extents() {
    let (mut d, mut v) = mount("ext4inline");
    assert_eq!(v.label(), "INLINE");
    assert_eq!(read_all(&mut v, &mut d, "tiny.txt"), b"tiny inline file\n");
    assert_eq!(read_all(&mut v, &mut d, "small.txt"), pattern(30, 100));
    let mut buf = [0; 30];
    assert_eq!(v.read(&mut d, b"small.txt", 50, &mut buf).unwrap(), 30);
    assert_eq!(&buf[..], &pattern(30, 100)[50..80]);
    assert_eq!(names(v.list(&mut d, b"smalldir").unwrap()), vec![("a".into(), Kind::File, 2), ("b".into(), Kind::File, 3)]);
    assert_eq!(read_all(&mut v, &mut d, "smalldir/b"), b"bb\n");
    let mut deep = vec![0u8; 399 * 2048 + 8];
    for i in 0..400u32 { let at = i as usize * 2048; deep[at..at + 4].copy_from_slice(&0xD00Du32.to_le_bytes()); deep[at + 4..at + 8].copy_from_slice(&i.to_le_bytes()); }
    same(&read_all(&mut v, &mut d, "deep.bin"), &deep, "deep");
    let mut buf = vec![0; 5000];
    assert_eq!(v.read(&mut d, b"deep.bin", 600 * 1024 + 3, &mut buf).unwrap(), 5000);
    same(&buf, &deep[600 * 1024 + 3..600 * 1024 + 5003], "deep range");
}

#[test]
fn ext_scan_bare_and_gpt() {
    let mut bare = common::fixture("ext2");
    let found = sv::scan(&mut bare).unwrap();
    assert_eq!((found.len(), found[0].start, found[0].volume.format()), (1, 0, Format::Ext4));
    // The same volume as the only partition of a hand-made GPT disk.
    let len = bare.sectors();
    let mut disk = common::MemDisk::new(2048 + len + 34);
    disk.bytes[2048 * 512..(2048 + len as usize) * 512].copy_from_slice(&bare.bytes);
    let b = &mut disk.bytes;
    b[446 + 4] = 0xEE;
    b[446 + 8..446 + 12].copy_from_slice(&1u32.to_le_bytes());
    b[510] = 0x55; b[511] = 0xAA;
    b[512..520].copy_from_slice(b"EFI PART");
    b[512 + 72..512 + 80].copy_from_slice(&2u64.to_le_bytes());
    b[512 + 80..512 + 84].copy_from_slice(&128u32.to_le_bytes());
    b[512 + 84..512 + 88].copy_from_slice(&128u32.to_le_bytes());
    let e = 1024;
    b[e..e + 16].copy_from_slice(&[0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4]);
    b[e + 16] = 1;
    b[e + 32..e + 40].copy_from_slice(&2048u64.to_le_bytes());
    b[e + 40..e + 48].copy_from_slice(&(2048 + len - 1).to_le_bytes());
    let mut found = sv::scan(&mut disk).unwrap();
    assert_eq!((found.len(), found[0].start, found[0].len), (1, 2048, len));
    let mut slice = sv::Slice { disk: &mut disk, start: 2048, len };
    assert_eq!(found[0].volume.label(), "OLDDISK");
    assert_eq!(read_all(&mut *found[0].volume, &mut slice, "small.txt"), b"ext2 says hi\n");
}

#[test]
fn ext_refuses_unknown_features_and_garbage() {
    let mut d = common::fixture("ext4");
    // INCOMPAT_COMPRESSION.
    d.bytes[1024 + 0x60] |= 1;
    assert_eq!(sv::ext4::Ext4::mount(&mut d).err(), Some(Error::Unsupported));
    // Random bytes over the metadata: errors are fine, panics are not.
    let (mut d, mut v) = mount("ext4");
    let clean = d.bytes.clone();
    for seed in 0..300u32 {
        d.bytes.copy_from_slice(&clean);
        let noise = pattern(1000 + seed, 64);
        // Scatter into superblock, descriptors, inode table and directory blocks.
        for (i, x) in noise.chunks(4).enumerate() {
            let at = [1024 + 0x18, 4096 + 8, 4096 * 2 + i * 32, 4096 * 3, 0][i % 4] + (u32::from_le_bytes(x.try_into().unwrap()) as usize % (64 << 10));
            d.bytes[at % clean.len()] = x[0];
        }
        if let Ok(mut v2) = sv::ext4::Ext4::mount(&mut d) {
            let _ = v2.list(&mut d, b"");
            let _ = v2.list(&mut d, b"many");
            let mut buf = [0; 9000];
            let _ = v2.read(&mut d, b"frag.bin", 3, &mut buf);
            let _ = v2.read(&mut d, b"holey.bin", 100000, &mut buf);
        }
    }
    d.bytes.copy_from_slice(&clean);
    assert!(v.list(&mut d, b"").is_ok());
}

/// With mke2fs on the host: random trees written by Linux, read back byte for byte.
#[test]
fn ext_random_trees_from_mke2fs() {
    check_trees_from_mke2fs(None);
}

/// Replay the CI seed whose final zero byte was dropped by mke2fs's inline-data
/// importer. The image's logical size is explicit, so the reader must return that
/// zero from a sparse final block, both in a whole-file read and across EOF.
#[test]
fn ext4_zero_tail_from_mke2fs() {
    check_trees_from_mke2fs(Some(0x2fa6_92b7));
}

fn check_trees_from_mke2fs(replay: Option<u32>) {
    let Some(mkfs) = common::tool("mke2fs") else { eprintln!("mke2fs missing, skipped"); return };
    let Some(debugfs) = common::tool("debugfs") else { eprintln!("debugfs missing, skipped"); return };
    let tag = if replay.is_some() { "replay" } else { "random" };
    let variants: [&[&str]; 4] = [
        &["-t", "ext4", "-b", "4096"],
        &["-t", "ext2", "-b", "1024"],
        &["-t", "ext3", "-b", "2048"],
        &["-t", "ext4", "-b", "1024", "-I", "256", "-O", "inline_data,^flex_bg"],
    ];
    let mut saw_zero_tail = false;
    for (k, opts) in variants.iter().enumerate() {
        let seed = replay.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() ^ k as u32);
        let mut rng = seed | 1;
        let mut next = move |n: u32| { rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5; rng % n };
        let src = common::temp(&format!("ext-src-{tag}-{k}"));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        let mut files = Vec::new();
        let mut dirs = vec![String::new()];
        for i in 0..40 {
            let parent = dirs[next(dirs.len() as u32) as usize].clone();
            let name = format!("{parent}/n{i}-{}", "x".repeat(next(40) as usize));
            if next(4) == 0 { std::fs::create_dir(src.join(&name[1..])).unwrap(); dirs.push(name); continue; }
            let size = [0, 1, 59, 61, 4095, 4097, 70_000, 300_000][next(8) as usize] + next(100) as usize;
            let mut data = pattern(next(1 << 30), size);
            if size > 10_000 && next(2) == 0 { let h = size / 3; data[h..2 * h].fill(0); }
            std::fs::write(src.join(&name[1..]), &data).unwrap();
            files.push((name, data));
        }
        let img = common::temp(&format!("ext-{tag}-{k}.img"));
        let _ = std::fs::remove_file(&img);
        let mut args: Vec<&str> = vec!["-q", "-F"];
        args.extend_from_slice(opts);
        let (s, i) = (src.to_str().unwrap(), img.to_str().unwrap());
        args.extend_from_slice(&["-d", s, i, "12M"]);
        let out = common::run(&mkfs, &args);
        assert!(out.status.success(), "{opts:?}: {}", String::from_utf8_lossy(&out.stderr));
        // mke2fs -d with inline_data can lose an all-zero final partial block
        // in an extent-backed inode (4097 -> 4096 in the replay seed).
        // debugfs independently confirms the shortened i_size. Declare the intended
        // logical sizes explicitly; missing tail blocks remain sparse zeros. Keep
        // comparing every byte with the original source, including the zero tail.
        let sizes = common::temp(&format!("ext-{tag}-{k}-sizes.txt"));
        let commands: String = files.iter().map(|(name, data)|
            format!("set_inode_field {name} size {}\n", data.len())).collect();
        std::fs::write(&sizes, commands).unwrap();
        let out = common::run(&debugfs, &["-w", "-f", sizes.to_str().unwrap(), i]);
        assert!(out.status.success(), "{opts:?}: {}", String::from_utf8_lossy(&out.stderr));
        let mut d = common::MemDisk::from_file(&img);
        let mut v = sv::ext4::Ext4::mount(&mut d).unwrap_or_else(|e| panic!("{opts:?}: {e:?}"));
        for (name, data) in &files {
            same(&read_all(&mut v, &mut d, name), data, &format!("{opts:?} seed {seed:#x} {name}"));
            if replay.is_some() && k == 3 && name == "/n12-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/n23-xxxxxxxxxxxxxxxxxxxxxxx" {
                saw_zero_tail = true;
                assert_eq!(data.len(), 4097);
                assert_eq!(data[4096], 0, "the regression must keep its sparse zero tail");
                let mut tail = [0xAA; 4];
                let n = v.read(&mut d, name.as_bytes(), 4095, &mut tail).unwrap();
                assert_eq!(n, 2);
                assert_eq!(&tail[..n], &data[4095..]);
                assert_eq!(&tail[n..], &[0xAA; 2]);
            }
        }
        for dir in &dirs {
            let want = std::fs::read_dir(src.join(dir.trim_start_matches('/'))).unwrap().count();
            assert_eq!(v.list(&mut d, dir.as_bytes()).unwrap().len(), want + if dir.is_empty() { 1 } else { 0 }, "{opts:?} {dir}");
        }
    }
    assert!(replay.is_none() || saw_zero_tail, "replay must exercise the zero tail");
}
