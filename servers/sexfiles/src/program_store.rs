//! Reads a named program from the disk store into the BOOT image region. Only
//! Shell asks, and the kernel checks whatever arrives; a hash here catches bad disks.
use crate::backends::diskfs::DiskFs;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::program_store as store;
const BLOCK_BYTES: usize = sex_pdx::BLOCK_MAX_XFER as usize;
fn read(lba: u64, bytes: u64, out: &mut [u8], buf_va: u64) -> Result<(), u32> {
    if DiskFs::diskfs_block_read(lba * store::SECTOR as u64, bytes, sex_pdx::SLOT_BUF_LEND) != 0 { return Err(store::IO); }
    let view = unsafe { core::slice::from_raw_parts(buf_va as *const u8, bytes as usize) };
    out[..bytes as usize].copy_from_slice(view);
    Ok(())
}
static mut DIRECTORY: [u8; store::SECTOR * store::MAX_DIRECTORY_SECTORS] = [0; store::SECTOR * store::MAX_DIRECTORY_SECTORS];
pub fn load(name: [u64; 2]) -> u64 {
    // A run with arguments arrives as a ticket for `/bin/NAME`; without that file it is
    // the store's program NAME, still with its arguments.
    match try_load(name,true) { Ok(len) => store::loaded(len), Err(error) => store::failed(error) }
}
/// Image bytes only: never consume an unrelated prepared ticket or loaded latch.
pub fn load_unbound(name:[u64;2])->u64 {
    match try_load(name,false) { Ok(len)=>store::loaded(len),Err(error)=>store::failed(error) }
}
fn try_load(mut name: [u64; 2], remember:bool) -> Result<u32, u32> {
    if !remember && name[0]==store::TICKET { return Err(store::NOT_FOUND); }
    let buf_va = crate::vfs::diskfs_bridge_get_buf_va();
    if buf_va == 0 || buf_va == u64::MAX { return Err(store::IO); }
    let view = sex_pdx::memory_region_view(store::IMAGE_SLOT_FILES, 0, store::IMAGE_BYTES as u64, QUERY_VIEW | READ | WRITE).map_err(|_| store::IO)?;
    let image = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, store::IMAGE_BYTES) };
    // A ticket names a path Spindle prepared. A bare name tries the program directory.
    let mut store_name = None;
    let found = if name[0] == store::TICKET { let (found, fallback) = crate::fs_service::read_ticket(name[1], image); store_name = fallback; found } else {
        let (a, b) = (name[0].to_le_bytes(), name[1].to_le_bytes());
        let mut bytes = [0u8; store::NAME_BYTES];
        bytes[..8].copy_from_slice(&a); bytes[8..].copy_from_slice(&b);
        let len = bytes.iter().position(|b| *b == 0).unwrap_or(store::NAME_BYTES);
        if remember { crate::fs_service::read_named(&bytes[..len], image) }
        else { crate::fs_service::read_named_unbound(&bytes[..len], image) }
    };
    match found {
        crate::fs_service::Program::Loaded(len) => return Ok(len),
        crate::fs_service::Program::Failed(error) => return Err(error),
        crate::fs_service::Program::Absent if name[0] == store::TICKET => match store_name {
            Some(packed) => name = packed,
            None => return Err(store::NOT_FOUND),
        },
        crate::fs_service::Program::Absent => {}
    }
    // The directory: its first sector says how many follow.
    let directory = unsafe { &mut *core::ptr::addr_of_mut!(DIRECTORY) };
    read(store::DIRECTORY_LBA, store::SECTOR as u64, &mut directory[..store::SECTOR], buf_va)?;
    let sectors = store::directory_sectors(&directory[..store::SECTOR]).ok_or(store::NOT_FOUND)?;
    let mut have = 1;
    while have < sectors {
        let take = (sectors - have).min(BLOCK_BYTES / store::SECTOR);
        let at = have * store::SECTOR;
        let mut block = [0u8; BLOCK_BYTES];
        read(store::DIRECTORY_LBA + have as u64, (take * store::SECTOR) as u64, &mut block, buf_va)?;
        directory[at..at + take * store::SECTOR].copy_from_slice(&block[..take * store::SECTOR]);
        have += take;
    }
    let entry = store::find(&directory[..sectors * store::SECTOR], name).ok_or(store::NOT_FOUND)?;
    if entry.len == 0 || entry.len as usize > store::IMAGE_BYTES { return Err(store::TOO_LARGE); }
    if entry.lba < store::DIRECTORY_LBA + sectors as u64 { return Err(store::CORRUPT); }
    let (mut lba, mut done, len) = (entry.lba, 0usize, entry.len as usize);
    while done < len {
        // The drive transfers whole sectors; the last one may be only partly the file's.
        let bytes = (len - done).min(BLOCK_BYTES).next_multiple_of(store::SECTOR).min(BLOCK_BYTES);
        let mut block = [0u8; BLOCK_BYTES];
        read(lba, bytes as u64, &mut block, buf_va)?;
        let used = (len - done).min(bytes);
        image[done..done + used].copy_from_slice(&block[..used]);
        done += used;
        lba += (bytes / store::SECTOR) as u64;
    }
    if store::fnv(&image[..len]) != entry.hash {
        sex_pdx::serial_record!("[files.program.corrupt] bytes={}", len);
        return Err(store::CORRUPT);
    }
    sex_pdx::serial_record!("[files.program.read] bytes={} lba={}", len, entry.lba);
    let packed = [name[0].to_le_bytes(), name[1].to_le_bytes()].concat();
    if remember { crate::fs_service::loaded(&packed[..packed.iter().position(|b| *b == 0).unwrap_or(store::NAME_BYTES)]); }
    Ok(entry.len)
}
