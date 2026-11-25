// SPDX-License-Identifier: GPL-3.0-only
//! Actual Files/store loaders, with explicit finite host disk, FS and Region doubles.
#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
extern crate self as sha2;
// Files' crash-log view: a live Shell and Files at epoch 1.
pub mod crash_log {
    pub struct PeerState { pub epoch: u64 }
    impl PeerState { pub fn live(&self) -> bool { true } }
    pub fn peer_state(_: u64) -> Result<PeerState, u64> { Ok(PeerState { epoch: 1 }) }
    pub fn self_epoch() -> Result<u64, u64> { Ok(1) }
}
// A real SHA-256 so program_store's pin test keeps its meaning without the crate.
pub struct Sha256;
pub trait Digest { fn digest(data: impl AsRef<[u8]>) -> [u8; 32]; }
impl Digest for Sha256 {
    fn digest(data: impl AsRef<[u8]>) -> [u8; 32] {
        const K: [u32; 64] = [0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2];
        let mut h: [u32; 8] = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
        let mut m = data.as_ref().to_vec();
        let bits = (m.len() as u64) * 8;
        m.push(0x80);
        while m.len() % 64 != 56 { m.push(0); }
        m.extend_from_slice(&bits.to_be_bytes());
        for block in m.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 { w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap()); }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let mut v = h;
            for i in 0..64 {
                let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
                let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
                let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
                let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
                let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
                let t2 = s0.wrapping_add(maj);
                v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
            }
            for i in 0..8 { h[i] = h[i].wrapping_add(v[i]); }
        }
        let mut out = [0u8; 32];
        for i in 0..8 { out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes()); }
        out
    }
}
use std::sync::Mutex;
#[path = "../servers/sexfiles/src/program_store.rs"]
mod production_store;
#[path = "../crates/sex-pdx/src/program_store.rs"]
pub mod program_store;
// Finite single-writer backend: this fixture exercises image loading, not admission.
mod restricted_image {
    pub struct Writer;
    pub fn writer() -> Option<Writer> { Some(Writer) }
    impl Writer { pub fn seal_ordinary(&mut self, _: u64, _: u64) -> Option<u32> { Some(1) } }
    pub fn release_unobserved_before_load() {}
    pub fn shell_epoch() -> u64 { 1 }
}
#[macro_export]
macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}};}
pub const BLOCK_MAX_XFER: u64 = 512;
pub const SLOT_BUF_LEND: u64 = 7;
pub mod memory_region {
    pub const QUERY_VIEW: u32 = 1;
    pub const READ: u32 = 2;
    pub const WRITE: u32 = 4;
    pub struct View {
        pub virtual_address: u64,
    }
}
struct World {
    image: Vec<u8>,
    bridge: Vec<u8>,
    disk: Vec<u8>,
    file: Option<Vec<u8>>,
    opened: Vec<Vec<u8>>,
    reads: usize,
}
static WORLD: Mutex<Option<World>> = Mutex::new(None);
pub fn memory_region_view(_: u64, _: u64, len: u64, _: u32) -> Result<memory_region::View, ()> {
    let w = WORLD.lock().unwrap();
    let w = w.as_ref().unwrap();
    assert_eq!(len as usize, w.image.len());
    Ok(memory_region::View {
        virtual_address: w.image.as_ptr() as u64,
    })
}
mod vfs {
    pub fn diskfs_bridge_get_buf_va() -> u64 {
        crate::WORLD
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .bridge
            .as_ptr() as u64
    }
    pub fn diskfs_bridge_buf_len() -> u64 {
        crate::WORLD.lock().unwrap().as_ref().unwrap().bridge.len() as u64
    }
}
mod backends {
    pub mod diskfs {
        pub struct DiskFs;
        impl DiskFs {
            pub fn diskfs_block_read(offset: u64, len: u64, _: u64) -> u64 {
                let mut w = crate::WORLD.lock().unwrap();
                let w = w.as_mut().unwrap();
                let start = offset as usize;
                let end = start + len as usize;
                if end > w.disk.len() || len as usize > w.bridge.len() {
                    return 1;
                }
                let data = w.disk[start..end].to_vec();
                w.bridge[..data.len()].copy_from_slice(&data);
                w.reads += 1;
                0
            }
        }
    }
}
mod console_relay {
    pub const SLOTS: usize = 8;
    pub fn not_started(_: usize) {}
}
mod fs_service {
    pub mod w {
        pub const PATH_BYTES: usize = 1024;
        pub const ARGS_MAX: usize = 256;
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Kind {
        File,
        Dir,
    }
    #[derive(Clone, Copy)]
    enum Error {
        NotFound,
        NotDir,
        Corrupt,
        Io,
    }
    struct Stat {
        kind: Kind,
        size: u64,
        id: u64,
    }
    struct FakeFs;
    impl FakeFs {
        fn open(&mut self, path: &[u8]) -> Result<Stat, Error> {
            let mut w = crate::WORLD.lock().unwrap();
            let w = w.as_mut().unwrap();
            w.opened.push(path.to_vec());
            let file = w.file.as_ref().ok_or(Error::NotFound)?;
            Ok(Stat {
                kind: Kind::File,
                size: file.len() as u64,
                id: 1,
            })
        }
        fn read(&mut self, _: u64, offset: u64, out: &mut [u8]) -> Result<usize, Error> {
            let w = crate::WORLD.lock().unwrap();
            let file = w.as_ref().unwrap().file.as_ref().ok_or(Error::NotFound)?;
            let start = offset as usize;
            let len = out.len().min(file.len() - start);
            out[..len].copy_from_slice(&file[start..start + len]);
            Ok(len)
        }
    }
    enum State {
        Untried,
        Absent,
        Ready(FakeFs),
    }
    static mut STATE: State = State::Untried;
    /// No program rows here: every ticket is Spindle's (requester 0).
    fn program_index(_: u32) -> Option<usize> { None }
    fn current_ticket_owner(_: u32) -> Option<TicketOwner> { None }
    fn bring_up() -> State {
        State::Ready(FakeFs)
    }
    include!(env!("RECORDING_IMAGE_LOADER_SOURCE"));
    pub fn seed() {
        unsafe {
            STATE = State::Untried;
            LOADED = ([0; 32], 9);
            LOADED.0[..9].copy_from_slice(b"unrelated");
            LOADED_ARGS = ([0; w::ARGS_MAX], 7);
            LOADED_ARGS.0[..7].copy_from_slice(b"keep me");
            PREPARED = ([0; w::PATH_BYTES], 13, 77);
            PREPARED.0[..13].copy_from_slice(b"/bin/ordinary");
            PREPARED_ARGS = ([0; w::ARGS_MAX], 5);
            PREPARED_ARGS.0[..5].copy_from_slice(b"alpha");
        }
    }
    pub fn snapshot() -> Vec<u8> {
        unsafe {
            let mut out = vec![];
            out.extend_from_slice(&LOADED.0);
            out.extend_from_slice(&LOADED.1.to_le_bytes());
            out.extend_from_slice(&LOADED_ARGS.0);
            out.extend_from_slice(&LOADED_ARGS.1.to_le_bytes());
            out.extend_from_slice(&PREPARED.0);
            out.extend_from_slice(&PREPARED.1.to_le_bytes());
            out.extend_from_slice(&PREPARED.2.to_le_bytes());
            out.extend_from_slice(&PREPARED_ARGS.0);
            out.extend_from_slice(&PREPARED_ARGS.1.to_le_bytes());
            out
        }
    }
    pub fn loaded_name() -> Vec<u8> {
        unsafe { LOADED.0[..LOADED.1].to_vec() }
    }
    pub fn loaded_arguments() -> Vec<u8> {
        unsafe { LOADED_ARGS.0[..LOADED_ARGS.1].to_vec() }
    }
    pub fn prepared_length() -> usize {
        unsafe { PREPARED.1 }
    }
}
fn reset(file: bool, corrupt: bool) {
    let blob = b"host image bytes; executable identity is checked elsewhere".to_vec();
    let start = (program_store::DIRECTORY_LBA as usize + 1) * 512;
    let mut disk = vec![0u8; start + 512];
    let base = program_store::DIRECTORY_LBA as usize * 512;
    disk[base..base + 8].copy_from_slice(&program_store::MAGIC);
    disk[base + 8..base + 12].copy_from_slice(&1u32.to_le_bytes());
    disk[base + 16..base + 22].copy_from_slice(b"record");
    disk[base + 32..base + 36]
        .copy_from_slice(&((program_store::DIRECTORY_LBA + 1) as u32).to_le_bytes());
    disk[base + 36..base + 40].copy_from_slice(&(blob.len() as u32).to_le_bytes());
    let hash = program_store::fnv(&blob) ^ u64::from(corrupt);
    disk[base + 40..base + 48].copy_from_slice(&hash.to_le_bytes());
    disk[start..start + blob.len()].copy_from_slice(&blob);
    *WORLD.lock().unwrap() = Some(World {
        image: vec![0; program_store::IMAGE_BYTES],
        bridge: vec![0; 512],
        disk,
        file: file.then_some(blob),
        opened: vec![],
        reads: 0,
    });
    fs_service::seed();
}
#[test]
fn unbound_fs_image_preserves_unrelated_loaded_name_arguments_and_ticket() {
    reset(true, false);
    let before = fs_service::snapshot();
    let word = production_store::load_unbound(program_store::pack_name(b"record").unwrap());
    assert!(matches!(program_store::decode_reply(word),Some(Ok(n))if n>0));
    assert_eq!(fs_service::snapshot(), before);
    assert_eq!(
        WORLD.lock().unwrap().as_ref().unwrap().opened,
        [b"/bin/record".to_vec()]
    );
}
#[test]
fn unbound_directory_fallback_and_corruption_leave_loader_bookkeeping_untouched() {
    for corrupt in [false, true] {
        reset(false, corrupt);
        let before = fs_service::snapshot();
        let word = production_store::load_unbound(program_store::pack_name(b"record").unwrap());
        if corrupt {
            assert_eq!(
                program_store::decode_reply(word),
                Some(Err(program_store::CORRUPT))
            );
        } else {
            assert!(matches!(program_store::decode_reply(word),Some(Ok(n))if n>0));
        }
        assert_eq!(fs_service::snapshot(), before);
        assert!(WORLD.lock().unwrap().as_ref().unwrap().reads >= 2);
    }
}
#[test]
fn unbound_mode_refuses_tickets_without_consuming_or_reading_anything() {
    reset(true, false);
    let before = fs_service::snapshot();
    let word = production_store::load_unbound([program_store::TICKET, 77]);
    assert_eq!(
        program_store::decode_reply(word),
        Some(Err(program_store::NOT_FOUND))
    );
    assert_eq!(fs_service::snapshot(), before);
    let w = WORLD.lock().unwrap();
    let w = w.as_ref().unwrap();
    assert_eq!(w.reads, 0);
    assert!(w.opened.is_empty());
}
#[test]
fn ordinary_named_and_ticket_loads_retain_their_existing_bookkeeping() {
    reset(true, false);
    let word = production_store::load(program_store::pack_name(b"ordinary").unwrap(), 0);
    assert!(matches!(program_store::decode_reply(word), Some(Ok(_))));
    assert_eq!(fs_service::loaded_name(), b"ordinary");
    assert_eq!(fs_service::loaded_arguments(), b"");
    assert_eq!(fs_service::prepared_length(), 13);
    reset(true, false);
    let word = production_store::load([program_store::TICKET, 77], 0);
    assert!(matches!(program_store::decode_reply(word), Some(Ok(_))));
    assert_eq!(fs_service::loaded_name(), b"ordinary");
    assert_eq!(fs_service::loaded_arguments(), b"alpha");
    assert_eq!(fs_service::prepared_length(), 0);
}
#[test]
fn unbound_bare_names_reject_paths_empty_and_oversize_without_loader_mutation() {
    reset(true, false);
    let before = fs_service::snapshot();
    let mut image = [0u8; 512];
    for name in [
        b"".as_slice(),
        b"../record",
        b"/record",
        b"record\0suffix",
        b"this-name-is-much-too-long",
    ] {
        assert!(matches!(
            fs_service::read_named_unbound(name, &mut image),
            fs_service::Program::Failed(program_store::NOT_A_PROGRAM)
        ));
    }
    assert_eq!(fs_service::snapshot(), before);
    assert!(WORLD.lock().unwrap().as_ref().unwrap().opened.is_empty());
}
