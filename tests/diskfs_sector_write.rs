//! Production sector write body, with only the authenticated block transport mocked.
#![allow(dead_code)]
use std::sync::Mutex;
const BLOCK_SECTOR_SIZE: u64 = 512;
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
}
static STATE: Mutex<Option<State>> = Mutex::new(None);
static TEST_LOCK: Mutex<()> = Mutex::new(());
static VA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn hot_log()->bool{false}
struct DiskFs;
impl DiskFs {
    fn diskfs_block_read(offset: u64, size: u64, slot: u64) -> u64 {
        assert_eq!((size, slot), (512, 17));
        let mut s = STATE.lock().unwrap();
        let s = s.as_mut().unwrap();
        s.reads += 1;
        if s.read_error != 0 {
            return s.read_error;
        }
        let va = VA.load(std::sync::atomic::Ordering::Relaxed) as *mut u8;
        unsafe {
            std::ptr::copy_nonoverlapping(s.bytes[offset as usize..].as_ptr(), va, 512);
        }
        0
    }
    fn diskfs_block_write(offset: u64, size: u64, slot: u64) -> u64 {
        assert_eq!((size, slot), (512, 17));
        let mut s = STATE.lock().unwrap();
        let s = s.as_mut().unwrap();
        s.writes += 1;
        if s.write_error != 0 {
            return s.write_error;
        }
        let va = VA.load(std::sync::atomic::Ordering::Relaxed) as *const u8;
        unsafe {
            std::ptr::copy_nonoverlapping(va, s.bytes[offset as usize..].as_mut_ptr(), 512);
        }
        0
    }
}
include!(env!("DISKFS_SECTOR_WRITE_SOURCE"));
fn setup(buf: &mut [u8; 512]) {
    VA.store(
        buf.as_mut_ptr() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    *STATE.lock().unwrap() = Some(State {
        bytes: vec![0xA5; 8192],
        reads: 0,
        writes: 0,
        read_error: 0,
        write_error: 0,
    });
}
#[test]
fn full_sectors_skip_reads_and_partial_boundaries_preserve_neighbors() {
    let _g = TEST_LOCK.lock().unwrap();
    let mut buf = [0; 512];
    setup(&mut buf);
    let entry = DiskManifestEntryV1 {
        start_lba: 1,
        len_bytes: 4096,
    };
    let data = vec![0x37; 1024];
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 0, &data, buf.as_mut_ptr() as u64),
        Ok(1024)
    );
    {
        let s = STATE.lock().unwrap();
        let s = s.as_ref().unwrap();
        assert_eq!((s.reads, s.writes), (0, 2));
        assert_eq!(&s.bytes[512..1536], &data);
        assert_eq!(s.bytes[1536], 0xA5);
    }
    setup(&mut buf);
    let data = vec![0x42; 1024];
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 100, &data, buf.as_mut_ptr() as u64),
        Ok(1024)
    );
    let s = STATE.lock().unwrap();
    let s = s.as_ref().unwrap();
    assert_eq!((s.reads, s.writes), (2, 3));
    assert_eq!(&s.bytes[612..1636], &data);
    assert_eq!(s.bytes[611], 0xA5);
    assert_eq!(s.bytes[1636], 0xA5);
}
#[test]
fn errors_and_out_of_bounds_never_report_success() {
    let _g = TEST_LOCK.lock().unwrap();
    let mut buf = [0; 512];
    setup(&mut buf);
    let entry = DiskManifestEntryV1 {
        start_lba: 1,
        len_bytes: 4096,
    };
    STATE.lock().unwrap().as_mut().unwrap().read_error = 91;
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 1, &[1; 16], buf.as_mut_ptr() as u64),
        Err(91)
    );
    assert_eq!(STATE.lock().unwrap().as_ref().unwrap().writes, 0);
    // A full-sector overwrite ignores an irrelevant read failure but still
    // propagates its real write error, without claiming a durable receipt.
    STATE.lock().unwrap().as_mut().unwrap().write_error = 92;
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 0, &[1; 512], buf.as_mut_ptr() as u64),
        Err(92)
    );
    assert!(
        DiskFs::diskfs_write_object_entry(entry, 4090, &[1; 16], buf.as_mut_ptr() as u64).is_err()
    );
}
