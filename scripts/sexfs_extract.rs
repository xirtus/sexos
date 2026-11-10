//! Read one file from the actual SexFS image; extraction never writes the disk.
#[path = "../crates/sexfs/src/lib.rs"]
mod sexfs;
use sexfs::{Device, BLOCK};
#[path = "../crates/sex-pdx/src/fs.rs"]
mod file_protocol;
mod app_catalog {
    pub const SPINDLE_TARGETS: [u32; 0] = [];
}
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};
struct Disk {
    file: File,
    base: u64,
    blocks: u64,
}
impl Device for Disk {
    fn blocks(&self) -> u64 {
        self.blocks
    }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        if block >= self.blocks {
            return Err(());
        }
        self.file
            .seek(SeekFrom::Start(self.base + block * BLOCK as u64))
            .map_err(|_| ())?;
        self.file.read_exact(out).map_err(|_| ())
    }
    fn write(&mut self, _: u64, _: &[u8; BLOCK]) -> Result<(), ()> {
        Err(())
    }
    fn flush(&mut self) -> Result<(), ()> {
        Ok(())
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    assert_eq!(a.len(), 4, "DISK GUEST_PATH OUTPUT");
    let file = File::open(&a[1]).expect("image read-only");
    let base = file_protocol::BASE_LBA * 512;
    let bytes = file.metadata().unwrap().len();
    assert!(bytes > base);
    let blocks = (bytes - base) / BLOCK as u64;
    assert!(blocks <= 16 * 1024 * 1024, "bounded extraction image");
    let mut cache = vec![[0; BLOCK]; 128];
    let mut slots = vec![sexfs::Slot::EMPTY; 128];
    let mut used = vec![0; (blocks as usize + 63) / 64];
    let mut pending = vec![0; used.len()];
    let mut scratch = [0; BLOCK];
    let memory = sexfs::Memory {
        cache: &mut cache,
        slots: &mut slots,
        used: &mut used,
        pending: &mut pending,
        scratch: &mut scratch,
    };
    let mut fs = sexfs::Fs::mount(Disk { file, base, blocks }, memory).expect("mount SexFS");
    let stat = fs.open(a[2].as_bytes()).expect("guest file");
    assert_eq!(stat.kind, sexfs::Kind::File);
    assert!(stat.size <= 32 * 1024 * 1024, "bounded output");
    let mut out = vec![0; stat.size as usize];
    let mut at = 0;
    while at < out.len() {
        let n = fs
            .read(stat.id, at as u64, &mut out[at..])
            .expect("read guest file");
        assert!(n > 0, "unexpected EOF");
        at += n;
    }
    std::fs::write(&a[3], out).expect("write extracted copy");
}
