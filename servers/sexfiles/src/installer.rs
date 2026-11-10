//! `install`: puts SexOS beside whatever is on the disk (see `sex_disk::install`).
//!
//! Files is the only service with a route to the disk driver, so the installer lives here
//! and reaches the whole disk through the driver's raw requests. Without `go` it only
//! plans and says what it would do; nothing is written.
use crate::backends::diskfs::DiskFs;
use alloc::format;
use alloc::string::String;
use sex_disk::install::{self, Mode, Seeds};
use sex_disk::{BlockDev, Error, RoDev, SECTOR};
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ};
use sexfs::{Fs, Kind};

/// The whole disk through the driver's raw requests, in runs as long as the shared buffer.
pub(crate) struct RawDisk { pub(crate) sectors: u64 }
impl RawDisk {
    fn buffer() -> Result<&'static mut [u8], Error> {
        let va = crate::vfs::diskfs_bridge_get_buf_va();
        let len = crate::vfs::diskfs_bridge_buf_len() as usize;
        if va == 0 || va == u64::MAX || len < 4096 { return Err(Error::Io); }
        Ok(unsafe { core::slice::from_raw_parts_mut(va as *mut u8, len) })
    }
}
impl BlockDev for RawDisk {
    fn sectors(&self) -> u64 { self.sectors }
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), Error> {
        let buf = Self::buffer()?;
        for (i, chunk) in out.chunks_mut(buf.len()).enumerate() {
            let at = lba * SECTOR as u64 + (i * buf.len()) as u64;
            if DiskFs::diskfs_block_call(sex_pdx::BLOCK_RAW_READ, at, chunk.len() as u64, sex_pdx::SLOT_BUF_LEND) != 0 { return Err(Error::Io); }
            chunk.copy_from_slice(&buf[..chunk.len()]);
        }
        Ok(())
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), Error> {
        let buf = Self::buffer()?;
        for (i, chunk) in data.chunks(buf.len()).enumerate() {
            let at = lba * SECTOR as u64 + (i * buf.len()) as u64;
            buf[..chunk.len()].copy_from_slice(chunk);
            if DiskFs::diskfs_block_call(sex_pdx::BLOCK_RAW_WRITE, at, chunk.len() as u64, sex_pdx::SLOT_BUF_LEND) != 0 { return Err(Error::Io); }
        }
        Ok(())
    }
}

/// An image file in Files' own filesystem.
pub(crate) struct FileDev<'a> { pub(crate) fs: &'a mut Fs<'static, crate::fs_service::Disk>, pub(crate) id: u64, pub(crate) sectors: u64 }
impl BlockDev for FileDev<'_> {
    fn sectors(&self) -> u64 { self.sectors }
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), Error> {
        if lba + (out.len() / SECTOR) as u64 > self.sectors { return Err(Error::Invalid); }
        let n = self.fs.read(self.id, lba * SECTOR as u64, out).map_err(|_| Error::Io)?;
        if n != out.len() { return Err(Error::Corrupt); }
        Ok(())
    }
    fn write(&mut self, _: u64, _: &[u8]) -> Result<(), Error> { Err(Error::Io) }
}

/// GUIDs need to be unique, not secret: the time stamp counter, stirred.
pub(crate) fn seeds() -> Seeds {
    let mut x = unsafe { core::arch::x86_64::_rdtsc() } ^ sex_pdx::get_ticks().rotate_left(32);
    let mut next = || { x = x.wrapping_add(0x9E37_79B9_7F4A_7C15); let mut z = x; z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB); z ^ (z >> 31) };
    Seeds(core::array::from_fn(|_| { let mut b = [0u8; 16]; b[..8].copy_from_slice(&next().to_le_bytes()); b[8..].copy_from_slice(&next().to_le_bytes()); b }))
}

pub(crate) fn why(e: Error) -> &'static str {
    match e {
        Error::NoSpace => "Not enough free space. SexOS needs 1.5 GiB that no partition uses; shrink another system's partition first (in that system's own disk tool).",
        Error::Exists => "Something is in the way: SexOS is already on this disk, or another boot menu (limine.cfg) would hide SexOS's.",
        Error::Unsupported => "This disk's layout isn't one SexOS installs beside: it needs a GPT disk (every UEFI PC with Windows 8 or later, or a current Linux).",
        Error::Corrupt => "The disk's partition table or EFI system partition is damaged. Nothing was written.",
        Error::Io => "A disk error stopped the install.",
        Error::Invalid => "The store must be at least 1 GiB.",
        Error::NotFound => "The boot image lacks Limine or its menu.",
        Error::BadSignature => "The update is not signed by this system's release key, or its image is not the one the signature covers.",
        Error::Older => "The update is not newer than the running system.",
        Error::Pending => "An update already waits for a restart. Restart to try it, then update again.",
    }
}

/// Plans, and with `go` installs. Writes what happened into `out` as lines of text and
/// returns their length. `image` is a file path, or empty for the running system.
pub fn run(fs: Option<&mut Fs<'static, crate::fs_service::Disk>>, image: &[u8], store_mib: u64, go: bool, out: &mut [u8]) -> Result<u32, sexfs::Error> {
    let disk_sectors = DiskFs::diskfs_block_call(sex_pdx::BLOCK_LAYOUT, 0, 0, 0) >> 8;
    let mut text = String::new();
    let mut disk = RawDisk { sectors: disk_sectors };
    let layout = sex_disk::probe::classify(&mut disk);
    let mode = match layout {
        Ok(sex_disk::probe::Layout::Blank) | Ok(sex_disk::probe::Layout::LegacySexos) => Mode::WholeDisk,
        _ => Mode::FreeSpace(if store_mib == 0 { None } else { Some(store_mib * 2048) }),
    };
    let finish = |text: String, out: &mut [u8]| { let n = text.len().min(out.len()); out[..n].copy_from_slice(&text.as_bytes()[..n]); Ok(n as u32) };
    let mut plan = match install::plan(&mut disk, mode, &seeds()) {
        Ok(p) => p,
        Err(e) => {
            sex_pdx::serial_record!("[files.install.refused] error={:?} layout={:?}", e, layout);
            text.push_str("Not installed. "); text.push_str(why(e)); text.push('\n');
            return finish(text, out);
        }
    };
    if mode == Mode::WholeDisk { text.push_str("This disk is blank or holds only an old SexOS: it becomes SexOS's alone.\n"); }
    for line in install::describe(&plan) { text.push_str(&line); text.push('\n'); }
    sex_pdx::serial_record!("[files.install.plan] whole={} esp_new={} store_sectors={} others={}", plan.new_table as u8, plan.new_esp as u8, plan.store.sectors, plan.others.len());
    if !go {
        text.push_str("Nothing written yet. To install: install go");
        if store_mib != 0 { text.push_str(&format!(" {}", store_mib / 1024)); }
        text.push('\n');
        return finish(text, out);
    }
    // The image: the running system's, or a file.
    let mut progress = |step: &str| sex_pdx::serial_record!("[files.install.step] {}", step);
    let done = if image.is_empty() {
        let info = sex_pdx::memory_region_info(w::BOOT_IMAGE_SLOT).map_err(|_| sexfs::Error::NotFound)?;
        let view = sex_pdx::memory_region_view(w::BOOT_IMAGE_SLOT, 0, info.length, QUERY_VIEW | READ).map_err(|_| sexfs::Error::NotFound)?;
        let bytes = unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, view.length as usize) };
        let bytes = &bytes[..bytes.len() & !(SECTOR - 1)];
        install::apply(&mut disk, &mut plan, &mut RoDev(bytes), &mut progress)
    } else {
        let fs = fs.ok_or(sexfs::Error::NotFound)?;
        let st = fs.open(image)?;
        if st.kind == Kind::Dir { return Err(sexfs::Error::IsDir); }
        install::apply(&mut disk, &mut plan, &mut FileDev { fs, id: st.id, sectors: st.size / SECTOR as u64 }, &mut progress)
    };
    // Only what reached the disk counts.
    let done = done.and_then(|d| if DiskFs::diskfs_block_sync() == 0 { Ok(d) } else { Err(Error::Io) });
    match done {
        Ok(d) => {
            sex_pdx::serial_record!("[files.install.done] bytes={} slot_a={} store={} menu_lba={}", d.image_sectors * SECTOR as u64, plan.slot_a.first, plan.store.first, d.menu.first_lba);
            text.push_str("Installed. Restart and pick SexOS in the boot menu.\n");
            if !plan.fallback_free {
                text.push_str("This PC's firmware starts the other system by default: choose \\EFI\\SEXOS\\BOOTX64.EFI once in its boot menu or setup (often F12, F8 or Esc at power on) and make it first.\n");
            }
        }
        Err(e) => {
            sex_pdx::serial_record!("[files.install.failed] error={:?}", e);
            text.push_str("Install stopped. "); text.push_str(why(e));
            text.push_str(" Unless the disk could not confirm its last writes, the partition table was not changed, so every system starts as before.\n");
        }
    }
    text.push_str("The running system stops writing to this disk until it restarts.\n");
    finish(text, out)
}
