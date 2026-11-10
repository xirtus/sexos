// SPDX-License-Identifier: GPL-3.0-only
//! Actual Files/store loaders, with explicit finite host disk, FS and Region doubles.
#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
use std::sync::Mutex;
#[path = "../servers/sexfiles/src/program_store.rs"]
mod production_store;
#[path = "../crates/sex-pdx/src/program_store.rs"]
pub mod program_store;
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
    let word = production_store::load(program_store::pack_name(b"ordinary").unwrap());
    assert!(matches!(program_store::decode_reply(word), Some(Ok(_))));
    assert_eq!(fs_service::loaded_name(), b"ordinary");
    assert_eq!(fs_service::loaded_arguments(), b"");
    assert_eq!(fs_service::prepared_length(), 13);
    reset(true, false);
    let word = production_store::load([program_store::TICKET, 77]);
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
