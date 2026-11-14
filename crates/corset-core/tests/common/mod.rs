#![allow(dead_code)]
use corset_core::{BlockDevice, Error, MemDisk, SECTOR};
use std::path::PathBuf;
use std::process::Command;

pub const MIB: u64 = 2048;

/// Deterministic "random" bytes: tests must not depend on the host's entropy.
pub fn rng(seed: u64) -> impl FnMut(&mut [u8]) {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    move |out: &mut [u8]| for b in out.iter_mut() { s ^= s << 13; s ^= s >> 7; s ^= s << 17; *b = s as u8; }
}

pub fn quiet(_: corset_core::Progress<'_>) {}

pub fn tool(name: &str) -> bool {
    Command::new(name).arg("--help").output().is_ok() || Command::new(name).arg("-V").output().is_ok()
}

pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("corset-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `sgdisk -v` on the disk, if sgdisk is installed: true when it finds no problems.
pub fn sgdisk_ok(d: &MemDisk, name: &str) -> Option<String> {
    if Command::new("sgdisk").arg("--version").output().is_err() { return None; }
    let dir = scratch(name);
    let img = dir.join("disk.img");
    std::fs::write(&img, &d.0).unwrap();
    let v = Command::new("sgdisk").arg("-v").arg(&img).output().unwrap();
    let p = Command::new("sgdisk").arg("-p").arg(&img).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = String::from_utf8_lossy(&v.stdout).into_owned() + &String::from_utf8_lossy(&p.stdout);
    assert!(text.contains("No problems found"), "sgdisk -v: {text}");
    Some(text)
}

/// `fsck.fat -n` on sectors `first..first + sectors`, if installed.
pub fn fsck_fat_ok(d: &MemDisk, first: u64, sectors: u64, name: &str) -> Option<String> {
    if Command::new("fsck.fat").arg("--help").output().is_err() { return None; }
    let dir = scratch(name);
    let img = dir.join("part.img");
    let a = first as usize * SECTOR;
    std::fs::write(&img, &d.0[a..a + sectors as usize * SECTOR]).unwrap();
    let out = Command::new("fsck.fat").arg("-n").arg("-v").arg(&img).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "fsck.fat -n: {text}");
    Some(text)
}

/// A device for sex-volumes, as Files lends one.
pub struct Sv<'a>(pub &'a mut dyn BlockDevice);
impl sex_volumes::Disk for Sv<'_> {
    fn sectors(&self) -> u64 { self.0.sectors() }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> sex_volumes::Result<()> { self.0.read(lba, buf).map_err(|_| sex_volumes::Error::Io) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> sex_volumes::Result<()> { self.0.write(lba, buf).map_err(|_| sex_volumes::Error::Io) }
    fn flush(&mut self) -> sex_volumes::Result<()> { Ok(()) }
}

/// A disk that loses power after `left` more writes: that write and all later ones fail.
pub struct Cut<'a> { pub d: &'a mut MemDisk, pub left: usize, pub writes: usize }
impl BlockDevice for Cut<'_> {
    fn sectors(&self) -> u64 { self.d.sectors() }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> { self.d.read(lba, buf) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error> {
        self.writes += 1;
        if self.left == 0 { return Err(Error::Io); }
        self.left -= 1;
        self.d.write(lba, buf)
    }
    fn flush(&mut self) -> Result<(), Error> { Ok(()) }
}
