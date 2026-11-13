// SPDX-License-Identifier: GPL-3.0-only
//! `sexfs-fsck [--repair] [--deep] [--offset SECTORS] IMAGE`
//!
//! Checks the SexOS filesystem in a disk image (or a block device) and, with `--repair`,
//! repairs it the way Files does at start when a disk fails to mount. `--offset` is where
//! the filesystem starts, in 512-byte sectors (default 131072, where SexOS puts it on a
//! whole disk; 0 for an image of the store partition alone). `--deep` also reads every
//! data block against its checksum. Exit status: 0 clean, 1 damage found (and, with
//! `--repair`, repaired), 2 could not read or repair.
use sexfs::check::{check, repair, Report};
use sexfs::{Device, Memory, Slot, BLOCK};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

struct Image { file: File, base: u64, blocks: u64 }
impl Device for Image {
    fn blocks(&self) -> u64 { self.blocks }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        self.file.seek(SeekFrom::Start(self.base + block * BLOCK as u64)).map_err(|_| ())?;
        self.file.read_exact(out).map_err(|_| ())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        self.file.seek(SeekFrom::Start(self.base + block * BLOCK as u64)).map_err(|_| ())?;
        self.file.write_all(data).map_err(|_| ())
    }
    fn flush(&mut self) -> Result<(), ()> { self.file.sync_data().map_err(|_| ()) }
}

fn show(r: &Report) {
    println!("generation {}: {} files, {} folders, {} tree nodes", r.generation, r.files, r.dirs, r.nodes);
    let lines = [
        (r.bad_superblocks, "damaged superblock(s)"), (r.bad_nodes, "unreadable tree node(s)"), (r.from_older, "file(s) or folder(s) taken back from the commit before"),
        (r.bad_inodes, "damaged inode(s)"), (r.dangling_entries, "folder entr(ies) naming nothing, or a second name"), (r.duplicate_names, "duplicate name(s)"),
        (r.wrong_kinds, "entr(ies) of the wrong kind"), (r.orphans, "unnamed file(s) or folder(s), for /lost+found"),
        (r.stray_blocks, "block record(s) outside their file or the disk"), (r.shared_blocks, "block(s) claimed twice"), (r.bad_data, "data block(s) failing their checksum"),
    ];
    for (n, what) in lines { if n > 0 { println!("  {} {}", n, what); } }
    if r.clean() { println!("clean"); }
}

fn main() {
    let (mut fix, mut deep, mut offset, mut path) = (false, false, 131072u64, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--repair" => fix = true,
            "--deep" => deep = true,
            "--offset" => offset = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| { eprintln!("--offset needs a number of sectors"); std::process::exit(2) }),
            _ if path.is_none() => path = Some(a),
            _ => { eprintln!("usage: sexfs-fsck [--repair] [--deep] [--offset SECTORS] IMAGE"); std::process::exit(2) }
        }
    }
    let Some(path) = path else { eprintln!("usage: sexfs-fsck [--repair] [--deep] [--offset SECTORS] IMAGE"); std::process::exit(2) };
    let file = OpenOptions::new().read(true).write(fix).open(&path).unwrap_or_else(|e| { eprintln!("{}: {}", path, e); std::process::exit(2) });
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let base = offset * 512;
    let blocks = len.saturating_sub(base) / BLOCK as u64;
    let mut dev = Image { file, base, blocks };
    if !fix {
        match check(&mut dev, deep) {
            Ok(r) => { show(&r); std::process::exit(if r.clean() { 0 } else { 1 }) }
            Err(e) => { eprintln!("no readable SexOS filesystem: {:?}", e); std::process::exit(2) }
        }
    }
    let words = (blocks as usize).div_ceil(64);
    let mut cache = vec![[0u8; BLOCK]; 256];
    let mut slots = vec![Slot::EMPTY; 256];
    let (mut used, mut pending) = (vec![0u64; words], vec![0u64; words]);
    let mut scratch = Box::new([0u8; BLOCK]);
    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    match repair(dev, mem, deep) {
        Ok((_, r)) => { show(&r); if !r.clean() { println!("repaired"); } std::process::exit(if r.clean() { 0 } else { 1 }) }
        Err(e) => { eprintln!("could not repair: {:?}", e); std::process::exit(2) }
    }
}
