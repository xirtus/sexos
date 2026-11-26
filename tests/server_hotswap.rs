// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for Live Userspace Server Hot-Swap:
//! - Wire header parsing, FNV-1a checksum, and server-name -> PD mapping (`sex_pdx::crash_log`)
//! - Kernel `service_restart::replace_image` safety validation (bad magic, oversized, non-restartable PD, WRPKRU/PKU scan)
//! - End-to-end staged hot-swap + relocation + pristine re-capture + rearm via `service_restart::poll()`
#![allow(dead_code)]
extern crate alloc;
extern crate self as spin;
extern crate self as sex_pdx;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicPtr, Ordering};

pub struct Mutex<T>(std::sync::Mutex<T>);
impl<T> Mutex<T> {
    pub const fn new(v: T) -> Self { Self(std::sync::Mutex::new(v)) }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> { self.0.lock().unwrap() }
    pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> { self.0.try_lock().ok() }
}

pub const SEXFS_V0_META_END_LBA: u64 = 47;
pub const SEXDRIVE_AP4_MULTI_BASE_LBA: u64 = 128;
pub mod program_store { pub const DIRECTORY_LBA: u64 = 2048; }

#[path = "../crates/sex-pdx/src/crash_log.rs"]
mod wire;
pub mod crash_log {
    pub use crate::wire::*;
    pub fn poll() {}
    pub fn service(_: u32, _: u32, _: bool) {}
    pub fn fault(_: u32, _: u64, _: u64, _: u64, _: u64, _: &str) {}
}
#[path = "../crates/sex-pdx/src/program_image.rs"]
mod program_image;
#[path = "../crates/sex-pdx/src/program_code.rs"]
pub mod program_code;
#[path = "../kernel/src/capability_slot.rs"]
mod capability_slot;
#[path = "../kernel/src/task_ownership.rs"]
pub mod task_ownership;

#[macro_export]
macro_rules! serial_println { ($($t:tt)*) => {{ let _ = format_args!($($t)*); }}; }

pub mod endpoint {
    pub const E_CLOSED: u64 = 0xFFFF_FFFF_FFFF_FFF3;
    pub const E_RESTARTED: u64 = 0xFFFF_FFFF_FFFF_FFF7;
}
mod ipc_v2 { pub mod syscall { pub fn teardown_pd(_: u32, _: u64) -> usize { 0 } } }
mod scheduler {
    pub use crate::task_ownership;
    pub struct Context { pub pd_id: u64, pub pd_ptr: *const crate::capability::ProtectionDomain, pub rip: u64, pub rsp: u64 }
    pub struct Task { pub id: u32, pub context: Context, pub state: task_ownership::TaskOwnership }
    impl Task {
        pub fn rearm(&mut self, id: u32, entry: u64, stack: u64, pd: &crate::capability::ProtectionDomain) {
            self.id = id;
            self.context = Context { pd_id: pd.id as u64, pd_ptr: pd, rip: entry, rsp: stack };
            self.state = task_ownership::TaskOwnership::new();
        }
    }
    pub struct Scheduler;
    pub static SCHEDULERS: [Scheduler; 1] = [Scheduler];
    impl Scheduler { pub fn activate_task(&self, t: *mut Task) -> bool { unsafe { (*t).state.activate() } } }
    pub fn unpark_thread(_: *mut Task) {}
    pub fn purge_handoff_for_task(_: *mut Task) -> usize { 0 }
    pub fn purge_handoff_for_pd(_: u32) -> usize { 0 }
}

static REGISTRY: [AtomicPtr<capability::ProtectionDomain>; 1024] = [const { AtomicPtr::new(std::ptr::null_mut()) }; 1024];

mod capability {
    use crate::*;
    #[derive(Clone, Copy)]
    pub enum CapabilityData { Domain(u32), Memory }
    pub struct Capability { pub data: CapabilityData }
    pub struct ProtectionDomain {
        pub id: u32, pub pku_key: u8,
        pub main_task: AtomicPtr<scheduler::Task>,
        pub message_ring: *mut Queue, pub signal_ring: *mut Queue,
        pub incoming_replies: Mutex<VecDeque<ipc::messages::IpcReply>>,
        pub reply_credits: Mutex<Credits>,
        pub caps: Mutex<Vec<(u32, CapabilityData)>>,
    }
    pub struct Queue;
    impl Queue { pub fn dequeue(&self) -> Option<u8> { None } pub fn len(&self) -> usize { 0 } }
    #[derive(Default)]
    pub struct Credits(pub Vec<u32>);
    impl Credits {
        pub fn owed(&self) -> impl Iterator<Item = u32> + '_ { self.0.iter().copied() }
        pub fn clear(&mut self) { self.0.clear() }
    }
    impl ProtectionDomain {
        pub fn new(id: u32, task: u32) -> &'static Self {
            let main = Box::into_raw(Box::new(scheduler::Task {
                id: task,
                context: scheduler::Context { pd_id: id as u64, pd_ptr: std::ptr::null(), rip: 0x4000, rsp: 0x9000 },
                state: task_ownership::TaskOwnership::new(),
            }));
            let pd = Box::leak(Box::new(Self {
                id, pku_key: 3, main_task: AtomicPtr::new(main),
                message_ring: Box::into_raw(Box::new(Queue)), signal_ring: Box::into_raw(Box::new(Queue)),
                incoming_replies: Mutex::new(VecDeque::new()), reply_credits: Mutex::new(Credits::default()),
                caps: Mutex::new(Vec::new()),
            }));
            unsafe { (*main).context.pd_ptr = pd; }
            REGISTRY[id as usize].store(pd, Ordering::Release);
            pd
        }
        pub fn find_capability(&self, slot: u32) -> Option<Capability> {
            self.caps.lock().iter().find(|c| c.0 == slot).map(|c| Capability { data: c.1 })
        }
    }
}

mod ipc {
    pub const MAX_DOMAINS: usize = 1024;
    pub struct Registry;
    pub static DOMAIN_REGISTRY: Registry = Registry;
    impl Registry {
        pub fn get(&self, id: u32) -> Option<&'static crate::capability::ProtectionDomain> {
            let p = crate::REGISTRY.get(id as usize)?.load(crate::Ordering::Acquire);
            unsafe { p.as_ref() }
        }
    }
    pub mod messages {
        pub struct IpcReply { pub value: u64, pub caller_pd: u32, pub caller_task: u32 }
        impl IpcReply { pub fn erase_queue(q: &mut std::collections::VecDeque<Self>) { q.clear() } }
    }
}

// Host-backed 4 KiB frame & page-table leaf simulator so replace_image + apply_swap execute for real.
static LEAVES: std::sync::LazyLock<Mutex<HashMap<u64, u64>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

mod program_frames {
    pub fn take() -> Option<u64> {
        let layout = std::alloc::Layout::from_size_align(4096, 4096).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() { None } else { Some(ptr as u64) }
    }
    pub fn give(frame: u64) {
        if frame != 0 {
            let layout = std::alloc::Layout::from_size_align(4096, 4096).unwrap();
            unsafe { std::alloc::dealloc(frame as *mut u8, layout); }
        }
    }
}

mod boot_memory {
    use crate::LEAVES;
    pub fn existing_leaf_address(va: u64) -> Option<u64> { LEAVES.lock().get(&va).copied() }
    pub fn arena_leaf(va: u64, _pkey: u8) -> Option<u64> {
        let mut m = LEAVES.lock();
        if let Some(&addr) = m.get(&va) { return Some(addr); }
        let leaf = Box::into_raw(Box::new(0u64)) as u64;
        m.insert(va, leaf);
        Some(leaf)
    }
    pub fn backing(_: u64) -> u64 { 0 }
    pub fn copy_frame(from: u64, to: u64) {
        unsafe { core::ptr::copy_nonoverlapping(from as *const u8, to as *mut u8, 4096); }
    }
    pub fn write_frames(physical: u64, bytes: &[u8]) {
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), physical as *mut u8, bytes.len()); }
    }
}

pub struct HhdmResp { pub offset: u64 }
pub struct HhdmReq;
impl HhdmReq { pub fn response(&self) -> Option<HhdmResp> { Some(HhdmResp { offset: 0 }) } }
pub static HHDM_REQUEST: HhdmReq = HhdmReq;
mod smp { pub mod tlb { pub fn flush_pages(_: usize, _: impl Iterator<Item = u64>) {} } }
mod pci_command { pub fn quiesce(_: &crate::capability::ProtectionDomain) {} }
mod iommu { pub fn revoke(_: u32) {} }
mod device_irq { pub fn snapshot(_: u32) {} pub fn reset_begin(_: u32) -> bool { false } pub fn reset_end(_: u32) {} }
mod program_workers { pub fn end(_: u32) {} pub fn all_dead(_: u32) -> bool { true } }
mod console { pub fn stop(_: std::fmt::Arguments) -> ! { panic!("unexpected desktop stop") } }
mod interrupts { pub static TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(100); }

#[path = "../kernel/src/service_restart.rs"]
mod service_restart;

fn put_u16(b: &mut [u8], at: usize, v: u16) { b[at..at + 2].copy_from_slice(&v.to_le_bytes()); }
fn put_u32(b: &mut [u8], at: usize, v: u32) { b[at..at + 4].copy_from_slice(&v.to_le_bytes()); }
fn put_u64(b: &mut [u8], at: usize, v: u64) { b[at..at + 8].copy_from_slice(&v.to_le_bytes()); }

/// Builds a minimal valid x86-64 PIE ELF with 1 RX code page (at vaddr 0) and 1 RW data page (at vaddr 0x1000).
fn make_pie_elf(code: &[u8], data_word: u64, with_rela: bool) -> Vec<u8> {
    let mut elf = vec![0u8; 0x3000];
    // ELF header
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2; elf[5] = 1; elf[6] = 1;
    put_u16(&mut elf, 16, 3);  // ET_DYN
    put_u16(&mut elf, 18, 62); // EM_X86_64
    put_u64(&mut elf, 24, 0x100); // entry = 0x100
    put_u64(&mut elf, 32, 64); // phoff
    put_u16(&mut elf, 54, 56); // phentsize
    put_u16(&mut elf, 56, if with_rela { 3 } else { 2 }); // phnum
    // PH 0: PT_LOAD RX (offset 0x1000, vaddr 0x0, size 0x1000)
    let p0 = 64;
    put_u32(&mut elf, p0, 1);      // PT_LOAD
    put_u32(&mut elf, p0 + 4, 5);  // PF_R | PF_X
    put_u64(&mut elf, p0 + 8, 0x1000);
    put_u64(&mut elf, p0 + 16, 0x0);
    put_u64(&mut elf, p0 + 32, 0x1000);
    put_u64(&mut elf, p0 + 40, 0x1000);
    // PH 1: PT_LOAD RW (offset 0x2000, vaddr 0x1000, size 0x1000)
    let p1 = 64 + 56;
    put_u32(&mut elf, p1, 1);      // PT_LOAD
    put_u32(&mut elf, p1 + 4, 6);  // PF_R | PF_W
    put_u64(&mut elf, p1 + 8, 0x2000);
    put_u64(&mut elf, p1 + 16, 0x1000);
    put_u64(&mut elf, p1 + 32, 0x1000);
    put_u64(&mut elf, p1 + 40, 0x1000);
    elf[0x1100..0x1100 + code.len()].copy_from_slice(code);
    put_u64(&mut elf, 0x2000, data_word);
    if with_rela {
        // PH 2: PT_DYNAMIC at offset 0x2800 (size 64)
        let p2 = 64 + 112;
        put_u32(&mut elf, p2, 2); // PT_DYNAMIC
        put_u64(&mut elf, p2 + 8, 0x2800);
        put_u64(&mut elf, p2 + 32, 64);
        // Dynamic table: DT_RELA=0x1900, DT_RELASZ=24, DT_RELAENT=24, DT_NULL
        put_u64(&mut elf, 0x2800, 7);  put_u64(&mut elf, 0x2808, 0x1900);
        put_u64(&mut elf, 0x2810, 8);  put_u64(&mut elf, 0x2818, 24);
        put_u64(&mut elf, 0x2820, 9);  put_u64(&mut elf, 0x2828, 24);
        put_u64(&mut elf, 0x2830, 0);  put_u64(&mut elf, 0x2838, 0);
        // Rela entry at vaddr 0x1900 (file offset 0x2900): patch vaddr 0x1008 with base + 0x100
        put_u64(&mut elf, 0x2900, 0x1008);
        put_u64(&mut elf, 0x2908, 8); // R_X86_64_RELATIVE
        put_u64(&mut elf, 0x2910, 0x100);
    }
    elf
}

#[test]
fn hotswap_server_name_mapping_and_header_parsing() {
    assert_eq!(wire::server_pd_by_name("sexdisplay"), Some(1));
    assert_eq!(wire::server_pd_by_name("sexdrive"), Some(2));
    assert_eq!(wire::server_pd_by_name("silk-shell"), Some(3));
    assert_eq!(wire::server_pd_by_name("sexinput"), Some(4));
    assert_eq!(wire::server_pd_by_name("sexusb"), Some(5));
    assert_eq!(wire::server_pd_by_name("sexbell"), Some(10));
    assert_eq!(wire::server_pd_by_name("sexfiles"), Some(11));
    assert_eq!(wire::server_pd_by_name("sexnet"), Some(13));
    assert_eq!(wire::server_pd_by_name("sexacpi"), Some(50));
    assert_eq!(wire::server_pd_by_name("denim"), Some(51));
    assert_eq!(wire::server_pd_by_name("cufflinks"), Some(52));
    assert_eq!(wire::server_pd_by_name("sexgpu-nv"), Some(53));
    assert_eq!(wire::server_pd_by_name("saddle"), Some(54));
    assert_eq!(wire::server_pd_by_name("unknown-srv"), None);

    let payload = b"\x7fELFtest-payload";
    let sum = wire::fnv64(payload);
    let line = format!("SEXOS-HOTSWAP sexbell {} {:016x}", payload.len(), sum);
    let hdr = wire::parse_hotswap_header(&line).expect("valid header");
    assert_eq!(hdr.name, "sexbell");
    assert_eq!(hdr.pd, Some(10));
    assert_eq!(hdr.size, payload.len());
    assert_eq!(hdr.fnv, sum);

    // Checksum mismatch detection
    assert_ne!(wire::fnv64(b"\x7fELFtest-payloaX"), sum);
    // Unknown server still parses (pd=None) so tether can drain bytes cleanly
    let unk = wire::parse_hotswap_header("SEXOS-HOTSWAP mystery 128 deadbeef").unwrap();
    assert_eq!(unk.pd, None);
    // Rejects invalid headers: zero size, oversized, bad hex, extra tokens
    assert!(wire::parse_hotswap_header("SEXOS-HOTSWAP sexbell 0 deadbeef").is_none());
    assert!(wire::parse_hotswap_header(&format!("SEXOS-HOTSWAP sexbell {} deadbeef", wire::HOTSWAP_MAX_BYTES + 1)).is_none());
    assert!(wire::parse_hotswap_header("SEXOS-HOTSWAP sexbell 64 nothex").is_none());
    assert!(wire::parse_hotswap_header("SEXOS-HOTSWAP sexbell 64 deadbeef extra").is_none());
}

#[test]
fn hotswap_rejects_bad_magic_non_restartable_and_wrpkru_then_swaps_cleanly() {
    let pd = capability::ProtectionDomain::new(10, 100);
    service_restart::capture(10, 10);

    // Non-restartable PD -> NOT_FOUND
    let good = make_pie_elf(&[0x90, 0x90, 0xc3], 0xdead_beef, true);
    assert_eq!(service_restart::replace_image(99, &good), Err(wire::NOT_FOUND));

    // Bad magic -> INVALID
    assert_eq!(service_restart::replace_image(10, b"NOT_AN_ELF"), Err(wire::INVALID));

    // WRPKRU (0x0f 0x01 0xef) in executable code -> DENIED
    let evil = make_pie_elf(&[0x90, 0x0f, 0x01, 0xef, 0xc3], 0, false);
    assert_eq!(service_restart::replace_image(10, &evil), Err(wire::DENIED));

    // Valid PIE ELF with relocation -> OK, stages swap and requests end
    let base = 0x4000_0000u64 + 9 * 0x0100_0000u64;
    let entry = service_restart::replace_image(10, &good).expect("valid ELF accepted");
    assert_eq!(entry, base + 0x100);
    assert!(service_restart::end_requested(10));

    // Simulate task exiting and settling so poll() applies the swap and rearms
    let main = pd.main_task.load(Ordering::Acquire);
    unsafe {
        assert!((*main).state.exit());
        assert_eq!((*main).state.snapshot().phase, task_ownership::Phase::Dead);
    }
    assert!(service_restart::poll());
    interrupts::TICKS.fetch_add(200, Ordering::Relaxed);
    let _ = service_restart::poll();

    assert_eq!(service_restart::restarts_of(10), 1);
    assert_eq!(unsafe { (*main).context.rip }, base + 0x100);

    // Verify relocated pointer at vaddr base + 0x1008 equals base + 0x100
    let data_leaf = boot_memory::existing_leaf_address(base + 0x1000).expect("data page mapped");
    let data_frame = unsafe { core::ptr::read_volatile(data_leaf as *const u64) } & 0x000f_ffff_ffff_f000;
    let patched = unsafe { core::ptr::read_unaligned((data_frame + 8) as *const u64) };
    assert_eq!(patched, base + 0x100);
}
