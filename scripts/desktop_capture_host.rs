//! Actual capture protocol, Display server and Spindle client against scripted IPC.
#![allow(
    dead_code,
    unused_variables,
    unused_imports,
    unused_unsafe,
    static_mut_refs
)]
extern crate self as sex_graphics;
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/memory_region.rs"]
pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
#[path = "../apps/spindle/src/capture_command.rs"]
mod client;
#[path = "../crates/sex-pdx/src/desktop_look.rs"]
pub mod desktop_look;
#[path = "../crates/sex-pdx/src/fs.rs"]
pub mod fs;
#[path = "../crates/sex-graphics/src/png_encode.rs"]
pub mod png_encode;
#[path = "../crates/sex-pdx/src/screen_capture.rs"]
pub mod screen_capture;
#[path = "../servers/sexdisplay/src/screen_capture.rs"]
mod server;
pub mod app_catalog {
    pub const SPINDLE_TARGETS: [u32; 1] = [12];
}
#[macro_export]
macro_rules! serial_record {
    ($($x:tt)*) => {{}};
}
#[derive(Clone, Copy)]
pub struct PdxMessage {
    pub type_id: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub caller_pd: u32,
    pub caller_task: u32,
}
pub mod surface_lifecycle {
    pub const CATALOG_CAPACITY: usize = 1;
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Phase {
        Live,
        Prepared,
        Committed,
        Acknowledged,
    }
    #[derive(Clone, Copy)]
    pub struct Catalog {
        pub target_pd: u32,
        pub task_id: u32,
    }
    pub struct Lifecycle {
        pub task: u32,
        pub phase: Phase,
    }
    impl Lifecycle {
        pub fn catalog(&self, _: usize) -> Option<Catalog> {
            Some(Catalog {
                target_pd: 12,
                task_id: self.task,
            })
        }
        pub fn phase(&self, _: usize) -> Option<Phase> {
            Some(self.phase)
        }
    }
}
#[derive(Default)]
pub struct Scrollback {
    pub lines: Vec<Vec<u8>>,
}
impl Scrollback {
    pub fn push(&mut self, b: &[u8]) {
        self.lines.push(b.to_vec());
    }
}
pub struct Dummy;
impl Dummy {
    pub fn busy(&self) -> bool {
        false
    }
    pub fn join(&self, b: &[u8], out: &mut [u8]) -> Option<usize> {
        if b.len() + 1 > out.len() {
            return None;
        }
        out[0] = b'/';
        out[1..b.len() + 1].copy_from_slice(b);
        Some(b.len() + 1)
    }
}
mod fs_commands {
    pub static FS: super::Dummy = super::Dummy;
}
mod fetch_command {
    pub static FETCH: super::Dummy = super::Dummy;
}
mod update_command {
    pub static UPDATE: super::Dummy = super::Dummy;
}
pub static INSTANCE_TASK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(12);
pub const SLOT_DISPLAY: u64 = 5;
pub const SLOT_STORAGE: u64 = 6;
pub const ERR_CAP_INVALID: u64 = 1;
pub const ERR_SERVICE_NOT_READY: u64 = 2;
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    cell::RefCell,
    collections::{HashMap, VecDeque},
};
struct Memory {
    p: usize,
    n: usize,
}
impl Memory {
    fn new(n: usize) -> Self {
        let p = unsafe { alloc_zeroed(Layout::from_size_align(n, 4096).unwrap()) } as usize;
        assert_ne!(p, 0);
        Self { p, n }
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        unsafe {
            dealloc(
                self.p as *mut u8,
                Layout::from_size_align(self.n, 4096).unwrap(),
            )
        }
    }
}
struct World {
    bank: Memory,
    files: Memory,
    ticks: u64,
    pd: u32,
    calls: VecDeque<(u64, u64, [u64; 3])>,
    replies: VecDeque<(u32, u64)>,
    reject_write: bool,
}
impl World {
    fn new() -> Self {
        Self {
            bank: Memory::new(screen_capture::BYTES),
            files: Memory::new(fs::REGION_BYTES),
            ticks: 1,
            pd: 12,
            calls: VecDeque::new(),
            replies: VecDeque::new(),
            reject_write: false,
        }
    }
}
thread_local! {static WORLD:RefCell<World>=RefCell::new(World::new());}
fn reset() {
    WORLD.with(|w| *w.borrow_mut() = World::new());
    screen_capture_bank(true).unwrap().install_epoch(1);
}
pub fn instance_pd() -> u32 {
    WORLD.with(|w| w.borrow().pd)
}
pub fn get_ticks() -> u64 {
    WORLD.with(|w| w.borrow().ticks)
}
pub fn screen_capture_bank(writer: bool) -> Result<screen_capture::Bank, u64> {
    use memory_region::*;
    WORLD.with(|w| {
        let base = w.borrow().bank.p as u64;
        let info = MemoryRegionInfo {
            region_id: screen_capture::REGION_ID,
            length: screen_capture::BYTES as u64,
            rights: QUERY_VIEW | READ | if writer { WRITE } else { 0 },
            peer_pd: Some(if writer { 12 } else { 1 }),
        };
        let view = RegionView {
            region_id: info.region_id,
            virtual_address: base,
            length: info.length,
            dma_address: None,
        };
        unsafe { screen_capture::Bank::from_resolved(info, view, writer) }.ok_or(1)
    })
}
pub fn memory_region_view(_: u64, _: u64, _: u64, _: u8) -> Result<memory_region::RegionView, u64> {
    WORLD.with(|w| {
        Ok(memory_region::RegionView {
            region_id: 0,
            virtual_address: w.borrow().files.p as u64,
            length: fs::REGION_BYTES as u64,
            dma_address: None,
        })
    })
}
pub fn pdx_call_checked(slot: u64, op: u64, a: u64, b: u64, c: u64) -> Result<u64, u64> {
    WORLD.with(|w| {
        let mut w = w.borrow_mut();
        if slot == SLOT_STORAGE && ((a >> 32) & 255) == fs::WRITE as u64 && w.reject_write {
            w.reject_write = false;
            return Err(ERR_SERVICE_NOT_READY);
        }
        w.calls.push_back((slot, op, [a, b, c]));
        Ok(0)
    })
}
pub fn pdx_reply(_: u32, word: u64) -> u64 {
    WORLD.with(|w| w.borrow_mut().replies.push_back((1, word)));
    0
}
fn msg(pd: u32, task: u32, kind: u64, seq: u32) -> PdxMessage {
    PdxMessage {
        type_id: screen_capture::OP,
        arg0: kind,
        arg1: seq as u64,
        arg2: 1,
        caller_pd: pd,
        caller_task: task,
    }
}
fn live() -> surface_lifecycle::Lifecycle {
    surface_lifecycle::Lifecycle {
        task: 12,
        phase: surface_lifecycle::Phase::Live,
    }
}
fn header(seq: u32, w: usize, h: usize) -> screen_capture::Header {
    screen_capture::Header {
        epoch: 1,
        sequence: seq,
        task: 12,
        generation: 1,
        width: w,
        height: h,
        night: 0,
    }
}
fn erased() {
    let b = screen_capture_bank(false).unwrap();
    assert!(b.header().is_none());
    assert!((0..screen_capture::MAX_PIXELS).all(|i| b.pixel(i) == Some(0)));
}
#[test]
fn authority_bounds_and_custody() {
    use screen_capture as c;
    reset();
    let mut s = server::Server::new();
    s.attach(1);
    let mut l = live();
    for (pd, task, seq) in [(99, 12, 1), (12, 99, 1), (12, 12, 0)] {
        assert!(!s.request(msg(pd, task, c::BEGIN, seq), &l, 1, 1280, 800));
    }
    assert!(!s.request(msg(12, 12, c::BEGIN, 1), &l, 1, 8192, 4320));
    erased();
    assert!(s.request(msg(12, 12, c::BEGIN, 1), &l, 1, 2, 1));
    assert!(!s.request(msg(12, 12, c::BEGIN, 2), &l, 1, 2, 1));
    s.published(1, 2, 1, &[0x123456, 0xabcdef], 0);
    let b = screen_capture_bank(false).unwrap();
    assert_eq!(b.header(), Some(header(1, 2, 1)));
    assert_eq!(b.pixel(0), Some(0x123456));
    assert!(!s.request(msg(99, 12, c::RELEASE, 1), &l, 2, 2, 1));
    assert!(b.header().is_some());
    assert!(!s.request(msg(12, 12, c::RELEASE, 2), &l, 2, 2, 1));
    assert!(b.header().is_some());
    assert!(!s.request(msg(12, 12, c::RELEASE, 1), &l, 2, 2, 1));
    erased();
    assert!(!s.request(msg(12, 12, c::BEGIN, 1), &l, 2, 2, 1));
    assert!(s.request(msg(12, 12, c::BEGIN, 2), &l, 2, 2, 1));
    s.published(2, 2, 1, &[1, 2], 0);
    s.reconcile(&l, 2 + c::LEASE_TICKS);
    erased();
    s.poll();
    assert!(s.request(msg(12, 12, c::BEGIN, 3), &l, 3 + c::LEASE_TICKS, 2, 1));
    s.published(3, 2, 1, &[3, 4], 0);
    l.phase = surface_lifecycle::Phase::Prepared;
    s.reconcile(&l, 4 + c::LEASE_TICKS);
    erased();
    l.task = 13;
    l.phase = surface_lifecycle::Phase::Live;
    assert!(s.request(msg(12, 13, c::BEGIN, 1), &l, 5 + c::LEASE_TICKS, 2, 1));
    s.published(4, 2, 1, &[5, 6], 0);
    assert_eq!(b.header().unwrap().task, 13);
    // Display restart scrubs a prior lease before serving any capture request.
    let mut restarted = server::Server::new();
    restarted.attach(1);
    erased();
}
#[test]
fn exact_region_validation() {
    use memory_region::*;
    use screen_capture as c;
    reset();
    let base = WORLD.with(|w| w.borrow().bank.p as u64);
    let good = MemoryRegionInfo {
        region_id: c::REGION_ID,
        length: c::BYTES as u64,
        rights: QUERY_VIEW | READ,
        peer_pd: Some(1),
    };
    let view = RegionView {
        region_id: c::REGION_ID,
        virtual_address: base,
        length: c::BYTES as u64,
        dma_address: None,
    };
    for info in [
        MemoryRegionInfo {
            region_id: 2,
            ..good
        },
        MemoryRegionInfo {
            length: 4096,
            ..good
        },
        MemoryRegionInfo {
            rights: QUERY_VIEW | READ | WRITE,
            ..good
        },
        MemoryRegionInfo {
            peer_pd: Some(99),
            ..good
        },
    ] {
        assert!(unsafe { c::Bank::from_resolved(info, view, false) }.is_none())
    }
    for v in [
        RegionView {
            region_id: 2,
            ..view
        },
        RegionView {
            length: 4096,
            ..view
        },
        RegionView {
            virtual_address: base + 1,
            ..view
        },
        RegionView {
            dma_address: Some(4096),
            ..view
        },
    ] {
        assert!(unsafe { c::Bank::from_resolved(good, v, false) }.is_none())
    }
    assert!(unsafe { c::Bank::from_resolved(good, view, true) }.is_none());
    assert!(c::pixels(2560, 1600).is_some());
    assert!(c::pixels(3840, 2160).is_none());
}
/// Exercise the actual complete Spindle state machine against actual Display
/// capture state; only filesystem transport/storage is a deterministic fake.
fn run_client(
    short_write: bool,
    queue_full: bool,
    restart_before_ack: bool,
) -> (Vec<u8>, Scrollback) {
    use screen_capture as c;
    reset();
    WORLD.with(|w| w.borrow_mut().reject_write = queue_full);
    let pixels: Vec<u32> = (0..129 * 173)
        .map(|i| {
            let x = i % 129;
            let y = i / 129;
            ((x * 17 & 255) << 16 | (y * 29 & 255) << 8 | ((x ^ y) & 255)) as u32
        })
        .collect();
    let mut display = server::Server::new();
    display.attach(1);
    let mut client = client::Capture::new();
    let mut sb = Scrollback::default();
    let l = live();
    let mut files = HashMap::<Vec<u8>, Vec<u8>>::new();
    client.start(b"screen.png", &mut sb);
    for _ in 0..5000 {
        client.step(&mut sb, true);
        while let Some((slot, op, a)) = WORLD.with(|w| w.borrow_mut().calls.pop_front()) {
            if slot == SLOT_DISPLAY {
                if restart_before_ack && a[0] == c::RELEASE {
                    display.attach(2);
                    pdx_reply(12, c::reply(a[1] as u32, c::OK, c::RELEASE));
                } else {
                    if display.request(msg(12, 12, a[0], a[1] as u32), &l, get_ticks(), 129, 173) {
                        display.published(1, 129, 173, &pixels, 0);
                    }
                    display.poll();
                }
            } else {
                assert_eq!(op, fs::OP);
                let r = fs::Request::decode(a[0], a[1], a[2]).unwrap();
                let memory = WORLD.with(|w| w.borrow().files.p);
                let region =
                    unsafe { std::slice::from_raw_parts(memory as *const u8, fs::REGION_BYTES) };
                let path = region[..r.path_len as usize].to_vec();
                let mut status = fs::OK;
                let mut value = 0;
                match r.kind {
                    fs::STAT => {
                        if !files.contains_key(&path) {
                            status = fs::NOT_FOUND
                        }
                    }
                    fs::CREATE => {
                        if files.insert(path, Vec::new()).is_some() {
                            status = fs::EXISTS
                        }
                    }
                    fs::WRITE => {
                        let f = files.get_mut(&path).unwrap();
                        assert_eq!(
                            r.offset,
                            f.len() as u64,
                            "queue-full retry skipped PNG bytes"
                        );
                        value = if short_write { r.length - 1 } else { r.length };
                        f.extend_from_slice(
                            &region[fs::PATH_BYTES..fs::PATH_BYTES + value as usize],
                        );
                    }
                    fs::RENAME => {
                        let i = path.iter().position(|&b| b == 0).unwrap();
                        let body = files.remove(&path[..i]).unwrap();
                        assert!(files.insert(path[i + 1..].to_vec(), body).is_none());
                    }
                    fs::SYNC => {}
                    _ => panic!("unexpected file operation"),
                }
                WORLD.with(|w| {
                    w.borrow_mut()
                        .replies
                        .push_back((11, fs::reply(r.seq, status, value as u32)))
                });
            }
        }
        while let Some((caller, word)) = WORLD.with(|w| w.borrow_mut().replies.pop_front()) {
            client.receive(caller, word, &mut sb);
        }
        if !client.busy() {
            erased();
            return (files.remove(&b"/screen.png"[..]).unwrap_or_default(), sb);
        }
    }
    panic!("capture never finished")
}
#[test]
fn complete_stream_file_and_queue_retry() {
    let (bytes, sb) = run_client(false, true, false);
    assert!(sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
    assert!(bytes.starts_with(b"\x89PNG"));
    if let Ok(path) = std::env::var("SEXOS_CAPTURE_HOST_PNG") {
        std::fs::write(path, &bytes).unwrap()
    }
}
#[test]
fn short_write_never_reports_success() {
    let (bytes, sb) = run_client(true, false, false);
    assert!(bytes.is_empty());
    assert!(!sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
}
fn client_waiting_for_stat() -> (
    client::Capture,
    server::Server,
    Scrollback,
    surface_lifecycle::Lifecycle,
    fs::Request,
) {
    use screen_capture as c;
    reset();
    let mut display = server::Server::new();
    display.attach(1);
    let mut client = client::Capture::new();
    let mut sb = Scrollback::default();
    let l = live();
    client.start(b"cancel.png", &mut sb);
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    assert!(display.request(msg(12, 12, a[0], a[1] as u32), &l, 1, 2, 1));
    display.published(1, 2, 1, &[0x123456, 0xabcdef], 0);
    display.poll();
    let (caller, word) = WORLD.with(|w| w.borrow_mut().replies.pop_front()).unwrap();
    assert!(client.receive(caller, word, &mut sb));
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    let request = fs::Request::decode(a[0], a[1], a[2]).unwrap();
    assert_eq!(request.kind, fs::STAT);
    (client, display, sb, l, request)
}
#[test]
fn cancellation_scrubs_promptly_but_fences_pending_file_buffer() {
    use screen_capture as c;
    let (mut client, mut display, mut sb, l, request) = client_waiting_for_stat();
    client.cancel(&mut sb);
    assert!(client.busy());
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    assert_eq!(a[0], c::RELEASE);
    display.request(msg(12, 12, a[0], a[1] as u32), &l, 2, 2, 1);
    erased();
    let (caller, word) = WORLD.with(|w| w.borrow_mut().replies.pop_front()).unwrap();
    client.receive(caller, word, &mut sb);
    assert!(
        client.busy(),
        "Files buffer reused before its outstanding reply"
    );
    assert!(!client.receive(11, fs::reply(request.seq + 1, fs::NOT_FOUND, 0), &mut sb));
    assert!(client.busy());
    client.receive(11, fs::reply(request.seq, fs::NOT_FOUND, 0), &mut sb);
    assert!(!client.busy());
    assert!(!sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
}
#[test]
fn file_timeout_fences_buffer_and_revokes_frame() {
    let (mut client, mut display, mut sb, l, request) = client_waiting_for_stat();
    WORLD.with(|w| w.borrow_mut().ticks = 20_000);
    client.step(&mut sb, true);
    assert!(client.busy());
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    display.request(msg(12, 12, a[0], a[1] as u32), &l, 20_000, 2, 1);
    erased();
    let (caller, word) = WORLD.with(|w| w.borrow_mut().replies.pop_front()).unwrap();
    client.receive(caller, word, &mut sb);
    assert!(client.busy());
    client.receive(11, fs::reply(request.seq, fs::IO, 0), &mut sb);
    assert!(!client.busy());
    assert!(!sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
}
#[test]
fn old_ready_receipt_cannot_release_cancelled_lease() {
    use screen_capture as c;
    reset();
    let mut client = client::Capture::new();
    let mut sb = Scrollback::default();
    client.start(b"cancel.png", &mut sb);
    client.step(&mut sb, true);
    WORLD.with(|w| w.borrow_mut().calls.clear());
    client.cancel(&mut sb);
    client.step(&mut sb, true);
    client.receive(1, c::reply(1, c::OK, c::BEGIN), &mut sb);
    assert!(
        client.busy(),
        "stale frame receipt accepted as release acknowledgement"
    );
    client.receive(1, c::reply(1, c::OK, c::RELEASE), &mut sb);
    assert!(!client.busy());
}
#[test]
fn abandoned_timeout_receipt_not_delivered_to_new_incarnation() {
    use screen_capture as c;
    reset();
    let mut s = server::Server::new();
    s.attach(1);
    let mut l = live();
    assert!(s.request(msg(12, 12, c::BEGIN, 1), &l, 1, 2, 1));
    s.published(1, 2, 1, &[1, 2], 0);
    s.reconcile(&l, c::LEASE_TICKS + 1);
    l.task = 13;
    s.reconcile(&l, c::LEASE_TICKS + 2);
    s.poll();
    assert!(WORLD.with(|w| w.borrow().replies.is_empty()));
    assert!(s.request(msg(12, 13, c::BEGIN, 1), &l, c::LEASE_TICKS + 3, 2, 1));
}
#[test]
fn file_create_error_invalidates_capture_and_never_saves() {
    let (mut client, mut display, mut sb, l, stat) = client_waiting_for_stat();
    client.receive(11, fs::reply(stat.seq, fs::NOT_FOUND, 0), &mut sb);
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    let create = fs::Request::decode(a[0], a[1], a[2]).unwrap();
    assert_eq!(create.kind, fs::CREATE);
    client.receive(11, fs::reply(create.seq, fs::NO_SPACE, 0), &mut sb);
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    display.request(msg(12, 12, a[0], a[1] as u32), &l, 2, 2, 1);
    erased();
    let (caller, word) = WORLD.with(|w| w.borrow_mut().replies.pop_front()).unwrap();
    client.receive(caller, word, &mut sb);
    assert!(!client.busy());
    assert!(!sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
}
#[test]
fn revoked_frame_at_png_eof_never_reaches_rename() {
    let (mut client, mut display, mut sb, mut l, stat) = client_waiting_for_stat();
    client.receive(11, fs::reply(stat.seq, fs::NOT_FOUND, 0), &mut sb);
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    let create = fs::Request::decode(a[0], a[1], a[2]).unwrap();
    client.receive(11, fs::reply(create.seq, fs::OK, 0), &mut sb);
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    let write = fs::Request::decode(a[0], a[1], a[2]).unwrap();
    client.receive(
        11,
        fs::reply(write.seq, fs::OK, write.length as u32),
        &mut sb,
    );
    client.step(&mut sb, true);
    assert!(
        WORLD.with(|w| w.borrow().calls.is_empty()),
        "EOF should stage rename on the next turn"
    );
    l.phase = surface_lifecycle::Phase::Prepared;
    display.reconcile(&l, 2);
    erased();
    client.step(&mut sb, true);
    assert!(
        WORLD.with(|w| w.borrow().calls.is_empty()),
        "revoked frame reached file rename"
    );
    assert!(!sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
}
#[test]
fn same_task_display_restart_rejects_old_epoch_requests() {
    use screen_capture as c;
    reset();
    let mut old = server::Server::new();
    old.attach(1);
    let l = live();
    assert!(old.request(msg(12, 12, c::BEGIN, 1), &l, 1, 2, 1));
    old.published(1, 2, 1, &[1, 2], 0);
    let mut new = server::Server::new();
    new.attach(2);
    erased();
    let bank = screen_capture_bank(false).unwrap();
    assert_eq!(bank.service_epoch(), 2);
    assert!(
        !new.request(msg(12, 12, c::BEGIN, 2), &l, 2, 2, 1),
        "queued old-epoch request captured after same-task service restart"
    );
    assert!(bank.header().is_none());
    let mut fresh = msg(12, 12, c::BEGIN, 1);
    fresh.arg2 = 2;
    assert!(new.request(fresh, &l, 2, 2, 1));
    new.published(1, 2, 1, &[3, 4], 0);
    assert_eq!(bank.header().unwrap().epoch, 2);
    let mut unavailable = server::Server::new();
    unavailable.attach(0);
    erased();
    assert_eq!(bank.service_epoch(), 0);
    assert!(!unavailable.request(fresh, &l, 3, 2, 1));
}
#[test]
fn stale_release_receipt_after_display_restart_never_reports_saved() {
    let (bytes, sb) = run_client(false, false, true);
    assert!(
        !bytes.is_empty(),
        "file had already been synced before provider restart"
    );
    assert!(!sb.lines.iter().any(|s| s.starts_with(b"Screenshot saved")));
}
#[test]
fn new_person_action_binds_new_display_epoch() {
    reset();
    let mut display = server::Server::new();
    display.attach(2);
    let mut client = client::Capture::new();
    let mut sb = Scrollback::default();
    client.start(b"new.png", &mut sb);
    client.step(&mut sb, true);
    let (_, _, a) = WORLD.with(|w| w.borrow_mut().calls.pop_front()).unwrap();
    assert_eq!(a[2], 2);
}
#[test]
fn invalid_publication_never_creates_a_readable_frame() {
    use screen_capture as c;
    reset();
    let mut s = server::Server::new();
    s.attach(1);
    let l = live();
    assert!(s.request(msg(12, 12, c::BEGIN, 1), &l, 1, 2, 1));
    assert!(screen_capture_bank(false).unwrap().header().is_none());
    s.published(0, 2, 1, &[1, 2], 0);
    erased();
    s.poll();
    let (_, word) = WORLD.with(|w| w.borrow_mut().replies.pop_front()).unwrap();
    assert_eq!(c::decode_reply(word), Some((1, c::CANCELLED, c::BEGIN)));
}
