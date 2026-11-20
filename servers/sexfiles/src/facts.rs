//! Tatami's Updates, Storage and About pages (`sex_pdx::facts`): what Files knows of the
//! system slots, the disk and the boot image, as `key=value` lines. Read only; the caller
//! has checked the run is the system's `tatami`.
use crate::backends::diskfs::DiskFs;
use crate::fs_service::Disk;
use alloc::format;
use alloc::string::String;
use sex_pdx::facts as f;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ};
use sexfs::Fs;

/// SHA-256 of the boot image this run came with, once per Files run: it never changes.
static mut IMAGE: Option<(u64, [u8; 32])> = None;
fn image() -> Option<(u64, [u8; 32])> {
    if let Some(known) = unsafe { IMAGE } { return Some(known); }
    let info = sex_pdx::memory_region_info(w::BOOT_IMAGE_SLOT).ok()?;
    let view = sex_pdx::memory_region_view(w::BOOT_IMAGE_SLOT, 0, info.length, QUERY_VIEW | READ).ok()?;
    let bytes = unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, view.length as usize) };
    let len = (bytes.len() & !(sex_disk::SECTOR - 1)) as u64;
    let hash = sex_disk::update::hash(&mut sex_disk::RoDev(&bytes[..len as usize]), 0, len).ok()?;
    unsafe { IMAGE = Some((len, hash)); }
    Some((len, hash))
}

/// The lines for `topic` into `out`; the reply's status and length.
pub fn answer(mut fs: Option<&mut Fs<'static, Disk>>, topic: u64, out: &mut [u8]) -> (u8, u32) {
    let mut t = String::new();
    match topic {
        f::UPDATES => {
            crate::updater::facts(&mut t);
            t.push_str(&format!("release_key={}\n", crate::updater::has_release_key() as u8));
            if let Some(fs) = fs { crate::packages::waiting(fs, &mut t); }
        }
        f::STORAGE => {
            let layout = DiskFs::diskfs_block_call(sex_pdx::BLOCK_LAYOUT, 0, 0, 0);
            let kind = ["unknown", "whole", "store", "foreign", "locked"].get((layout & 0xFF) as usize).copied().unwrap_or("unknown");
            t.push_str(&format!("disk={} {}\n", layout >> 8, kind));
            if let Some(fs) = fs.as_deref_mut() {
                let s = fs.statfs();
                let home = match crate::home::state(fs) { sex_pdx::home_lock::STATE_UNLOCKED => "unlocked", sex_pdx::home_lock::STATE_LOCKED => "locked", _ => "none" };
                t.push_str(&format!("filesystem=sexfs {} {}\nhome={}\n", s.total, s.free, home));
            }
            crate::volumes::facts(fs, &mut t);
            // No driver reads an NVMe health log (SMART) yet: say so rather than guess.
            t.push_str("health=none\n");
        }
        f::ABOUT => {
            crate::updater::facts(&mut t);
            if let Some((len, hash)) = image() {
                t.push_str(&format!("image_bytes={}\nimage_sha256=", len));
                for b in hash { t.push_str(&format!("{:02x}", b)); }
                t.push('\n');
            }
        }
        _ => return (w::INVALID, 0),
    }
    sex_pdx::serial_record!("[files.facts] topic={} bytes={}", topic, t.len());
    let n = t.len().min(out.len());
    out[..n].copy_from_slice(&t.as_bytes()[..n]);
    (w::OK, n as u32)
}
