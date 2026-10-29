// Shared authenticated block-transport model; real entry I/O bodies are included.
use std::sync::Mutex;
const BLOCK_SECTOR_SIZE: u64 = 512;
const BLOCK_MAX_XFER:u64=4096;
pub const DISKFS_V4_POOL_BASE_LBA:u64=400;
pub const DISKFS_V4_POOL_SECTORS:u64=1408;
const MODEL_BYTES:usize=1024*1024;
const SLOT_BUF_LEND: u64 = 17;
const DISKFS_WRITE_PROOF_LBA: u64 = 2047;
mod messages {
    pub const ERR_OVERFLOW: i64 = -4;
}
macro_rules! serial_println { ($($x:tt)*)=>{{let _=format_args!($($x)*);}}; }
#[derive(Clone, Copy)]
struct DiskManifestEntryV1 {
    start_lba: u64,
    len_bytes: u32,
}
struct State {
    bytes: Vec<u8>,
    reads: usize,
    writes: usize,
    read_error: u64,
    write_error: u64,
    fail_read_at:usize,
    fail_write_at:usize,
    uncertain_write_prefix:usize,
    calls:Vec<(bool,u64,u64)>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);
static TEST_LOCK: Mutex<()> = Mutex::new(());
static VA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn hot_log()->bool{false}
struct DiskFs;
impl DiskFs {
    fn diskfs_block_read(offset: u64, size: u64, slot: u64) -> u64 {
        assert_eq!(slot,17);
        assert!(size>=512 && size<=4096 && size%512==0 && offset%512==0);
        let mut s = STATE.lock().unwrap();
        let s = s.as_mut().unwrap();
        s.reads += 1;
        s.calls.push((false,offset,size));
        if s.fail_read_at==s.reads{return 91;}
        if s.read_error != 0 {
            return s.read_error;
        }
        let va = VA.load(std::sync::atomic::Ordering::Relaxed) as *mut u8;
        unsafe {
            std::ptr::copy_nonoverlapping(s.bytes[offset as usize..].as_ptr(), va, size as usize);
        }
        0
    }
    fn diskfs_block_write(offset: u64, size: u64, slot: u64) -> u64 {
        assert_eq!(slot,17);
        assert!(size>=512 && size<=4096 && size%512==0 && offset%512==0);
        if size>512{assert!(offset>=DISKFS_V4_POOL_BASE_LBA*512 && offset+size<=(DISKFS_V4_POOL_BASE_LBA+DISKFS_V4_POOL_SECTORS)*512,"legacy routes remain sector-only");}
        let mut s = STATE.lock().unwrap();
        let s = s.as_mut().unwrap();
        s.writes += 1;
        s.calls.push((true,offset,size));
        if s.fail_write_at==s.writes {
            let prefix=s.uncertain_write_prefix.min(size as usize);
            let va=VA.load(std::sync::atomic::Ordering::Relaxed)as *const u8;
            unsafe{std::ptr::copy_nonoverlapping(va,s.bytes[offset as usize..].as_mut_ptr(),prefix);}
            return 92;
        }
        if s.write_error != 0 {
            return s.write_error;
        }
        let va = VA.load(std::sync::atomic::Ordering::Relaxed) as *const u8;
        unsafe {
            std::ptr::copy_nonoverlapping(va, s.bytes[offset as usize..].as_mut_ptr(), size as usize);
        }
        0
    }
}
include!(env!("DISKFS_SECTOR_WRITE_SOURCE"));
fn setup(buf: &mut [u8; 4096]) {
    VA.store(
        buf.as_mut_ptr() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    *STATE.lock().unwrap() = Some(State {
        bytes: vec![0xA5; MODEL_BYTES],
        reads: 0,
        writes: 0,
        read_error: 0,
        write_error: 0,
        fail_read_at:0, fail_write_at:0, uncertain_write_prefix:0, calls:Vec::new(),
    });
}
