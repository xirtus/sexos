// SPDX-License-Identifier: GPL-3.0-only
//! Crash records the kernel keeps for this start (see `sex_pdx::crash_log`), and the one
//! memory area that outlives a restart: when the kernel stops, it writes the record and
//! the end of its log to a few pages BOOT keeps out of the allocator, and the next start
//! reads them back. RAM keeps its contents across a warm restart on PCs and in QEMU; a
//! power cut loses them, so the stop screen restarts the machine by itself. Files then
//! copies every record to the reserved ring on disk, which survives anything.
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::crash_log::{self as wire, Kind, Record, RECORD_BYTES};
use spin::Mutex;

/// Physical home of the kept stop: 48 MiB, low enough to be RAM on any PC, above where
/// firmware and the boot allocator's first pages live, and the same on every start.
pub const PSTORE_PHYS: u64 = 0x0300_0000;
pub const PSTORE_BYTES: u64 = 8192;
pub const TAIL_BYTES: usize = 4096;
const PSTORE_MAGIC: [u8; 8] = *b"SEXPSTR1";
/// Page 0: magic, state, the record at 64, the tail's checksum at 576. Page 1: the tail.
const STATE_EMPTY: u64 = 0;
const STATE_STOPPED: u64 = 1;

static PSTORE_VIRT: AtomicU64 = AtomicU64::new(0);
static BOOT_ID: AtomicU64 = AtomicU64::new(0);

struct Ring { records: [[u8; RECORD_BYTES]; wire::KERNEL_CAPACITY], count: usize, next_seq: u32 }
static RING: Mutex<Ring> = Mutex::new(Ring { records: [[0; RECORD_BYTES]; wire::KERNEL_CAPACITY], count: 0, next_seq: 1 });

/// The last start's stop, if this start found one: its record and its log's end.
struct Earlier { record: Option<Record>, tail: [u8; TAIL_BYTES] }
static EARLIER: Mutex<Earlier> = Mutex::new(Earlier { record: None, tail: [0; TAIL_BYTES] });

pub fn boot_id() -> u64 { BOOT_ID.load(Ordering::Relaxed) }

/// True when the kept-stop pages lie wholly in RAM the memory map calls usable.
pub fn pstore_usable(memmap: &limine::request::MemmapResponse) -> bool {
    memmap.entries().iter().any(|e| e.type_ == 0 && e.base <= PSTORE_PHYS && PSTORE_PHYS + PSTORE_BYTES <= e.base + e.length)
}

/// After memory is up: a fresh boot id, and whatever the last start kept.
pub fn init(hhdm: u64, pstore: bool) {
    let mut id = unsafe { core::arch::x86_64::_rdtsc() };
    let mut r = 0u64;
    // RDRAND only where CPUID says it exists (leaf 1, ECX bit 30); otherwise the TSC alone.
    let rdrand = unsafe { core::arch::x86_64::__cpuid(1) }.ecx & (1 << 30) != 0;
    if rdrand && unsafe { core::arch::x86_64::_rdrand64_step(&mut r) } == 1 { id ^= r; }
    BOOT_ID.store(id | 1, Ordering::Relaxed);
    if !pstore { crate::serial_println!("[crash.pstore] available=0"); return; }
    let virt = hhdm + PSTORE_PHYS;
    PSTORE_VIRT.store(virt, Ordering::Release);
    let page = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PSTORE_BYTES as usize) };
    let state = u64::from_le_bytes(page[8..16].try_into().unwrap());
    let found = if page[..8] == PSTORE_MAGIC && state == STATE_STOPPED {
        let tail_sum = u64::from_le_bytes(page[576..584].try_into().unwrap());
        let tail = &page[4096..4096 + TAIL_BYTES];
        Record::decode(&page[64..64 + RECORD_BYTES]).map(|r| (r, fnv(tail) == tail_sum))
    } else { None };
    if let Some((mut record, tail_ok)) = found {
        record.flags |= wire::FLAG_EARLIER;
        let mut e = EARLIER.lock();
        if tail_ok { e.tail.copy_from_slice(&page[4096..4096 + TAIL_BYTES]); }
        e.record = Some(record);
        drop(e);
        push(&record);
        crate::serial_println!("[crash.pstore.found] boot={:#x} seq={} kind={} tail={}", record.boot, record.seq, record.kind as u8, tail_ok as u8);
    } else {
        crate::serial_println!("[crash.pstore] available=1 found=0");
    }
    // This start's stop, if it comes, replaces it; the record lives on in the ring and on disk.
    page.fill(0);
    page[..8].copy_from_slice(&PSTORE_MAGIC);
    page[8..16].copy_from_slice(&STATE_EMPTY.to_le_bytes());
}

/// Shown at the top of the boot log, then the start goes on.
pub fn show_earlier_on_boot_screen() -> bool {
    let e = EARLIER.lock();
    let Some(r) = e.record else { return false };
    let mut head: crate::console::Text<160> = crate::console::Text::new();
    // A panic's message starts with where it was raised; the reason is the line after.
    let msg = r.message();
    let mut lines = msg.split(|&c| c == b'\n').filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or(b"?");
    let (reason, place) = match first.strip_prefix(b"panicked at ") {
        Some(place) => (lines.next().unwrap_or(b"?"), Some(place.strip_suffix(b":").unwrap_or(place))),
        None => (first, None),
    };
    let _ = write!(head, "Last time, SexOS stopped: {}", core::str::from_utf8(reason).unwrap_or("?"));
    if let Some(place) = place { let _ = write!(head, " ({})", core::str::from_utf8(place).unwrap_or("?")); }
    let mut sub: crate::console::Text<160> = crate::console::Text::new();
    let _ = write!(sub, "After {} s running, in {}. Files keeps the record; type `crashes` in Spindle to see it.",
        r.uptime_ms / 1000, wire::domain_name(r.pd));
    // The log's last lines before the stop.
    let tail = &e.tail;
    let start = tail.iter().position(|&b| b != 0).unwrap_or(tail.len());
    let text = &tail[start..];
    let total = text.split(|&b| b == b'\n').filter(|l| !l.is_empty()).count();
    let mut lines: [&[u8]; 8] = [b""; 8];
    lines[0] = head.as_bytes();
    lines[1] = sub.as_bytes();
    lines[2] = b"Its last log lines:";
    let mut n = 3;
    for l in text.split(|&b| b == b'\n').filter(|l| !l.is_empty()).skip(total.saturating_sub(5)) {
        if n == lines.len() { break; }
        lines[n] = l;
        n += 1;
    }
    crate::console::pin_note(&lines[..n]);
    crate::serial_println!("[crash.boot.shown] boot={:#x} seq={}", r.boot, r.seq);
    crate::console::showing()
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes { h ^= b as u64; h = h.wrapping_mul(0x0000_0100_0000_01b3); }
    h
}

fn uptime_ms() -> u64 { crate::interrupts::TICKS.load(Ordering::Relaxed) }

fn push(record: &Record) {
    let mut ring = RING.lock();
    let mut bytes = [0u8; RECORD_BYTES];
    record.encode(&mut bytes);
    // Full: the oldest of this start's records make room.
    if ring.count == wire::KERNEL_CAPACITY {
        ring.records.copy_within(1.., 0);
        ring.count -= 1;
    }
    let n = ring.count;
    ring.records[n] = bytes;
    ring.count += 1;
    ADDED.fetch_add(1, Ordering::Release);
}

fn new_record(kind: Kind, pd: u32) -> Record {
    let mut r = Record::empty(kind);
    r.boot = boot_id();
    r.seq = { let mut ring = RING.lock(); let s = ring.next_seq; ring.next_seq += 1; s };
    r.pd = pd;
    r.uptime_ms = uptime_ms();
    r.set_name(wire::domain_name(pd));
    r
}

/// A service or program faulted and the kernel ended it.
pub fn fault(pd: u32, vector: u64, rip: u64, addr: u64, error: u64, what: &str) {
    let kind = if (1..=15).contains(&pd) { Kind::ServiceFault } else { Kind::ProgramFault };
    let mut r = new_record(kind, pd);
    r.vector = vector as u32;
    r.rip = rip;
    r.addr = addr;
    r.error = error;
    let mut m: crate::console::Text<160> = crate::console::Text::new();
    let _ = write!(m, "{} faulted: {} at {:#x}", wire::domain_name(pd), what, rip);
    if vector == 14 { let _ = write!(m, " touching {:#x}", addr); }
    r.push_message(m.as_bytes());
    push(&r);
    crate::serial_println!("[crash.record] seq={} kind={} pd={} vector={} rip={:#x}", r.seq, kind as u8, pd, vector, rip);
}

/// A service was started again, or given up on.
pub fn service(pd: u32, restarts: u32, gave_up: bool) {
    let mut r = new_record(if gave_up { Kind::ServiceGaveUp } else { Kind::ServiceRestart }, pd);
    let mut m: crate::console::Text<160> = crate::console::Text::new();
    if gave_up { let _ = write!(m, "{} kept ending ({} restarts) and was left down", wire::domain_name(pd), restarts); }
    else { let _ = write!(m, "{} started again (restart {})", wire::domain_name(pd), restarts); }
    r.push_message(m.as_bytes());
    push(&r);
    crate::serial_println!("[crash.record] seq={} kind={} pd={} restarts={}", r.seq, r.kind as u8, pd, restarts);
}

/// From the stop path, other cores halted: keep the stop where the next start finds it.
/// Takes no lock that a halted core could hold.
pub fn kernel_stop(message: &[u8], core: u32, pd: u32, tail: &[u8; TAIL_BYTES]) {
    let mut r = Record::empty(Kind::KernelStop);
    r.boot = boot_id();
    unsafe { RING.force_unlock(); }
    r.seq = RING.lock().next_seq;
    r.pd = pd;
    r.vector = core;
    r.uptime_ms = uptime_ms();
    r.set_name("kernel");
    r.push_message(message);
    push(&r);
    let virt = PSTORE_VIRT.load(Ordering::Acquire);
    if virt == 0 { return; }
    let page = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PSTORE_BYTES as usize) };
    let mut bytes = [0u8; RECORD_BYTES];
    r.encode(&mut bytes);
    page[64..64 + RECORD_BYTES].copy_from_slice(&bytes);
    page[4096..4096 + TAIL_BYTES].copy_from_slice(tail);
    page[576..584].copy_from_slice(&fnv(tail).to_le_bytes());
    page[..8].copy_from_slice(&PSTORE_MAGIC);
    page[8..16].copy_from_slice(&STATE_STOPPED.to_le_bytes());
    crate::serial::write_stopped(b"[crash.pstore.kept]\n");
}

/// How long the stop screen stays before the machine restarts.
pub fn stop_restart_seconds() -> u32 {
    match option_env!("SEXOS_STOP_RESTART_SECONDS") { Some(v) => v.parse().unwrap_or(30), None => 30 }
}

/// From the timer: tell Files when the ring holds records it has not been told of.
static NOTIFIED_SEQ: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static ADDED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
pub fn poll() {
    let added = ADDED.load(Ordering::Acquire);
    if added == NOTIFIED_SEQ.load(Ordering::Relaxed) { return; }
    let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(FILES_PD) else { return };
    let msg = crate::ipc::messages::MessageType::IpcCall { func_id: wire::OP_NOTICE, arg0: added as u64, arg1: 0, arg2: 0, caller_pd: 0, caller_task: 0 };
    if unsafe { (*pd.message_ring).enqueue(msg) }.is_ok() {
        NOTIFIED_SEQ.store(added, Ordering::Relaxed);
        crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire));
    }
}

// ── SYS_CRASH_LOG ──────────────────────────────────────────────────────────────

const FILES_PD: u32 = 11;
/// Ring places adopted history may never take: room for this start's own records.
const ADOPT_RESERVE: usize = 16;
const SHELL_PD: u32 = 3;
fn is_spindle(pd: u32) -> bool { sex_pdx::app_catalog::SPINDLE_TARGETS.contains(&pd) }
fn may_read(pd: u32) -> bool { pd == FILES_PD || pd == SHELL_PD || is_spindle(pd) }

/// A test build's pending "end this service at its next syscall".
static PROBE_KILL: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Test builds: the domain marked for ending is the caller; end it as a fault would.
#[inline(always)]
pub fn probe_pending(pd: u32) -> bool {
    option_env!("SEXOS_CRASH_PROBE").is_some()
        && PROBE_KILL.load(Ordering::Relaxed) == pd
        && PROBE_KILL.compare_exchange(pd, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
}

pub fn syscall(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let caller = crate::core_local::CoreLocal::get().current_pd();
    let (op, arg, buf) = (regs.rdi, regs.rsi, regs.rdx);
    regs.rsi = 0;
    regs.rdx = 0;
    match op {
        wire::OP_STATUS => {
            if !may_read(caller) { return wire::DENIED; }
            regs.rsi = RING.lock().count as u64;
            regs.rdx = boot_id();
            wire::OK
        }
        wire::OP_READ => {
            if !may_read(caller) { return wire::DENIED; }
            if !crate::syscalls::validate_user_bytes(buf, RECORD_BYTES, true) { return wire::INVALID; }
            let ring = RING.lock();
            if arg as usize >= ring.count { return wire::NOT_FOUND; }
            let record = ring.records[arg as usize];
            drop(ring);
            unsafe { core::ptr::copy_nonoverlapping(record.as_ptr(), buf as *mut u8, RECORD_BYTES) };
            wire::OK
        }
        wire::OP_ADOPT => {
            if caller != FILES_PD { return wire::DENIED; }
            if !crate::syscalls::validate_user_bytes(buf, RECORD_BYTES, false) { return wire::INVALID; }
            let mut bytes = [0u8; RECORD_BYTES];
            unsafe { core::ptr::copy_nonoverlapping(buf as *const u8, bytes.as_mut_ptr(), RECORD_BYTES) };
            let Some(mut record) = Record::decode(&bytes) else { return wire::INVALID };
            // This start's records come from the kernel alone.
            if record.boot == boot_id() { return wire::INVALID; }
            record.flags |= wire::FLAG_EARLIER;
            // Earlier history goes in front of this start's records, and only while room is
            // left for new ones: adopting never pushes out a record not yet on disk.
            let mut ring = RING.lock();
            if ring.records[..ring.count].iter().any(|b| Record::decode(b).is_some_and(|r| r.same_event(&record))) { return wire::OK; }
            if ring.count >= wire::KERNEL_CAPACITY - ADOPT_RESERVE { return wire::FULL; }
            let mut bytes = [0u8; RECORD_BYTES];
            record.encode(&mut bytes);
            let n = ring.count;
            ring.records.copy_within(0..n, 1);
            ring.records[0] = bytes;
            ring.count += 1;
            wire::OK
        }
        wire::OP_RESTARTS => {
            regs.rsi = crate::service_restart::restarts_of(caller) as u64;
            wire::OK
        }
        wire::OP_PROBE_FAULT => {
            if option_env!("SEXOS_CRASH_PROBE").is_none() || !is_spindle(caller) { return wire::DENIED; }
            if arg == 0 {
                crate::serial_println!("[crash.probe.kernel]");
                panic!("crash probe: the kernel was asked to stop");
            }
            if !(1..=15).contains(&arg) { return wire::INVALID; }
            crate::serial_println!("[crash.probe.service] pd={}", arg);
            PROBE_KILL.store(arg as u32, Ordering::Release);
            // A service waiting for messages makes no syscall until something wakes it.
            if let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(arg as u32) {
                crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire));
            }
            wire::OK
        }
        _ => wire::INVALID,
    }
}
