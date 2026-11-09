// SPDX-License-Identifier: GPL-3.0-only
//! Shared by the sex-volumes host tests: a disk in memory, fixture images, and the
//! system's own mkfs/fsck tools when the host has them.
#![allow(dead_code)]
use sex_volumes::{self as sv, Disk, SECTOR};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct MemDisk { pub bytes: Vec<u8>, pub flushes: usize }
impl MemDisk {
    pub fn new(sectors: u64) -> Self { Self { bytes: vec![0; sectors as usize * SECTOR], flushes: 0 } }
    pub fn from_file(path: &Path) -> Self { Self { bytes: std::fs::read(path).unwrap(), flushes: 0 } }
}
impl Disk for MemDisk {
    fn sectors(&self) -> u64 { (self.bytes.len() / SECTOR) as u64 }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> sv::Result<()> {
        let at = lba as usize * SECTOR;
        if buf.len() % SECTOR != 0 || at + buf.len() > self.bytes.len() { return Err(sv::Error::Io); }
        buf.copy_from_slice(&self.bytes[at..at + buf.len()]); Ok(())
    }
    fn write(&mut self, lba: u64, buf: &[u8]) -> sv::Result<()> {
        let at = lba as usize * SECTOR;
        if buf.len() % SECTOR != 0 || at + buf.len() > self.bytes.len() { return Err(sv::Error::Io); }
        self.bytes[at..at + buf.len()].copy_from_slice(buf); Ok(())
    }
    fn flush(&mut self) -> sv::Result<()> { self.flushes += 1; Ok(()) }
}



/// `tests/volumes/NAME.sparse`: "SXSPARSE", total sectors (u64 LE), then for each
/// sector that isn't all zero its LBA (u64 LE) and its 512 bytes.
pub fn fixture(name: &str) -> MemDisk {
    let path = Path::new(file!()).parent().unwrap().join("volumes").join(format!("{name}.sparse"));
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&raw[..8], b"SXSPARSE");
    let total = u64::from_le_bytes(raw[8..16].try_into().unwrap());
    let mut disk = MemDisk::new(total);
    let mut at = 16;
    while at < raw.len() {
        let lba = u64::from_le_bytes(raw[at..at + 8].try_into().unwrap()) as usize;
        disk.bytes[lba * SECTOR..(lba + 1) * SECTOR].copy_from_slice(&raw[at + 8..at + 8 + SECTOR]);
        at += 8 + SECTOR;
    }
    disk
}

/// A host tool, if installed (and found in /sbin too).
pub fn tool(name: &str) -> Option<PathBuf> {
    for dir in ["/usr/sbin", "/sbin", "/usr/bin", "/bin", "/usr/local/sbin", "/usr/local/bin"] {
        let p = Path::new(dir).join(name);
        if p.exists() { return Some(p); }
    }
    None
}
pub fn run(tool: &Path, args: &[&str]) -> std::process::Output {
    Command::new(tool).args(args).output().unwrap()
}
pub fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sexos-volumes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}
