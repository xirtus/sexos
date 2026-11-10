// SPDX-License-Identifier: GPL-3.0-only
//! SexACPI: the firmware's AML, run in ring 3 instead of the kernel.
//!
//! ACPI firmware describes batteries, the lid, thermal zones, CPU idle states and the
//! sleep sequence as AML byte code. Running that interpreter in the kernel puts untrusted
//! firmware (and a large parser) where PKU does not apply. Here it runs in its own domain
//! in the Senses organ. The kernel keeps the fixed hardware (S5 from the table bytes,
//! reset, the power button), so shutting down never depends on this service.
//!
//! The service holds the `Power` capability with the `FIRMWARE` right and nothing else.
//! It reads the AML tables through `FW_TABLE`, and every register access the firmware asks
//! for goes through `FW_IO`, `FW_MEM` or `FW_PCI`, where the kernel refuses RAM, its own
//! ports and PCI BARs. It reports what it found (`FW_INVENTORY`, `FW_IDLE`), publishes the
//! sensors every two seconds (`FW_SENSORS`), and runs `\_PTS` when the kernel tells it the
//! machine is about to go off.
//!
//! A firmware bug that panics the interpreter, or a method that loops on hardware that never
//! answers (each pass has a work budget), stops this service only: it reports AML as set
//! aside and parks.
#![no_std]
#![no_main]
extern crate alloc;

use acpi::aml::namespace::{AmlName, NamespaceLevelKind};
use acpi::aml::object::{Object, WrappedObject};
use acpi::aml::{AmlError, Interpreter};
use acpi::registers::FixedRegisters;
use acpi::sdt::fadt::Fadt;
use acpi::{Handle, PciAddress, PhysicalMapping};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::str::FromStr;
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::power as w;
use sex_pdx::serial_println;

// ── Memory ─────────────────────────────────────────────────────────────────

const HEAP_BYTES: usize = 3 * 1024 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct HeapBytes([u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

/// The interpreter gets its own stack: deep firmware call chains outgrow the 64 KiB one.
const AML_STACK_BYTES: usize = 512 * 1024;
#[repr(C, align(16))]
struct AmlStack([u8; AML_STACK_BYTES]);
static mut AML_STACK: AmlStack = AmlStack([0; AML_STACK_BYTES]);

core::arch::global_asm!(r#"
.global sexacpi_on_stack
sexacpi_on_stack:
    push rbp
    mov rbp, rsp
    mov rsp, rdx
    mov rax, rdi
    mov rdi, rsi
    call rax
    mov rsp, rbp
    pop rbp
    ret
"#);
extern "C" { fn sexacpi_on_stack(f: extern "C" fn(*mut u8), arg: *mut u8, stack_top: u64); }

/// Runs `f` on the AML stack.
fn on_aml_stack<R, F: FnOnce() -> R>(f: F) -> R {
    struct Call<F, R> { f: Option<F>, r: Option<R> }
    extern "C" fn trampoline<F: FnOnce() -> R, R>(p: *mut u8) {
        let call = unsafe { &mut *(p as *mut Call<F, R>) };
        if let Some(f) = call.f.take() { call.r = Some(f()); }
    }
    let mut call = Call { f: Some(f), r: None };
    let top = unsafe { (core::ptr::addr_of_mut!(AML_STACK.0) as *mut u8).add(AML_STACK_BYTES) as u64 } & !15;
    unsafe { sexacpi_on_stack(trampoline::<F, R>, &mut call as *mut _ as *mut u8, top) };
    call.r.expect("AML_STACK_RETURN")
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[sexacpi.aml.panic] {}", info);
    // AML is set aside for the rest of the boot; the kernel keeps S5, reset and the button.
    let _ = w::call4(w::FW_INVENTORY, 0, u64::MAX, 1);
    loop { let _ = sex_pdx::sys_wait_self_ticks(64); }
}

// ── Time ───────────────────────────────────────────────────────────────────

static TSC_PER_US: AtomicU64 = AtomicU64::new(1000);
fn rdtsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }
/// Measures the TSC against the kernel's millisecond tick.
fn calibrate() {
    let start_tick = sex_pdx::get_ticks();
    let mut t = start_tick;
    while t == start_tick { core::hint::spin_loop(); t = sex_pdx::get_ticks(); }
    let (t0, c0) = (t, rdtsc());
    while sex_pdx::get_ticks() < t0 + 20 { core::hint::spin_loop(); }
    let per_us = (rdtsc() - c0) / 20_000;
    TSC_PER_US.store(per_us.clamp(100, 10_000), Ordering::Relaxed);
}
fn stall_us(us: u64) {
    let end = rdtsc().saturating_add(us.saturating_mul(TSC_PER_US.load(Ordering::Relaxed)));
    while rdtsc() < end { core::hint::spin_loop(); }
}

// ── Work budget ────────────────────────────────────────────────────────────

/// Work one AML pass may do: one unit per register access, one per microsecond stalled.
/// The interpreter has no loop limit, so a method waiting on hardware that never answers
/// would hang the service; running out panics instead, and AML is set aside.
static BUDGET: AtomicU64 = AtomicU64::new(u64::MAX);
/// About ten seconds of register access and stalls, `_INI` included.
const PASS_BUDGET: u64 = 10_000_000;
fn spend(units: u64) {
    let left = BUDGET.load(Ordering::Relaxed);
    if left == u64::MAX { return; }
    if left < units { BUDGET.store(u64::MAX, Ordering::Relaxed); panic!("AML work budget spent"); }
    BUDGET.store(left - units, Ordering::Relaxed);
}
fn bounded<R>(f: impl FnOnce() -> R) -> R {
    BUDGET.store(PASS_BUDGET, Ordering::Relaxed);
    let r = on_aml_stack(f);
    BUDGET.store(u64::MAX, Ordering::Relaxed);
    r
}

// ── Hardware, through the kernel's firmware window ────────────────────────

static DENIED: AtomicU64 = AtomicU64::new(0);
fn window(reply: [u64; 5], what: &str, at: u64) -> u64 {
    if reply[0] != w::OK {
        if DENIED.fetch_add(1, Ordering::Relaxed) < 8 { serial_println!("[sexacpi.window.refused] {}={:#x} status={:#x}", what, at, reply[0]); }
        return u64::MAX;
    }
    reply[1]
}
fn io(port: u16, width: u64, write: Option<u64>) -> u64 {
    spend(1);
    let arg = port as u64 | width << 16 | (write.is_some() as u64) << 24;
    window(w::call4(w::FW_IO, arg, write.unwrap_or(0), 0), "port", port as u64)
}
fn mem(address: usize, width: u64, write: Option<u64>) -> u64 {
    spend(1);
    window(w::call4(w::FW_MEM, address as u64, width | (write.is_some() as u64) << 8, write.unwrap_or(0)), "memory", address as u64)
}
fn pci(a: PciAddress, offset: u16, width: u64, write: Option<u64>) -> u64 {
    spend(1);
    let at = w::pci_address(a.segment(), a.bus(), a.device(), a.function(), offset);
    window(w::call4(w::FW_PCI, at, width | (write.is_some() as u64) << 8, write.unwrap_or(0)), "pci", at)
}

#[derive(Clone, Copy)]
struct Fw;
impl acpi::Handler for Fw {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, _size: usize) -> PhysicalMapping<Self, T> {
        // Nothing here maps physical memory: tables come through FW_TABLE and registers
        // through the read/write calls below.
        panic!("map_physical_region({:#x}) has no ring-3 meaning", physical_address)
    }
    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
    fn read_u8(&self, a: usize) -> u8 { mem(a, 1, None) as u8 }
    fn read_u16(&self, a: usize) -> u16 { mem(a, 2, None) as u16 }
    fn read_u32(&self, a: usize) -> u32 { mem(a, 4, None) as u32 }
    fn read_u64(&self, a: usize) -> u64 { mem(a, 8, None) }
    fn write_u8(&self, a: usize, v: u8) { mem(a, 1, Some(v as u64)); }
    fn write_u16(&self, a: usize, v: u16) { mem(a, 2, Some(v as u64)); }
    fn write_u32(&self, a: usize, v: u32) { mem(a, 4, Some(v as u64)); }
    fn write_u64(&self, a: usize, v: u64) { mem(a, 8, Some(v)); }
    fn read_io_u8(&self, p: u16) -> u8 { io(p, 1, None) as u8 }
    fn read_io_u16(&self, p: u16) -> u16 { io(p, 2, None) as u16 }
    fn read_io_u32(&self, p: u16) -> u32 { io(p, 4, None) as u32 }
    fn write_io_u8(&self, p: u16, v: u8) { io(p, 1, Some(v as u64)); }
    fn write_io_u16(&self, p: u16, v: u16) { io(p, 2, Some(v as u64)); }
    fn write_io_u32(&self, p: u16, v: u32) { io(p, 4, Some(v as u64)); }
    fn read_pci_u8(&self, a: PciAddress, o: u16) -> u8 { pci(a, o, 1, None) as u8 }
    fn read_pci_u16(&self, a: PciAddress, o: u16) -> u16 { pci(a, o, 2, None) as u16 }
    fn read_pci_u32(&self, a: PciAddress, o: u16) -> u32 { pci(a, o, 4, None) as u32 }
    fn write_pci_u8(&self, a: PciAddress, o: u16, v: u8) { pci(a, o, 1, Some(v as u64)); }
    fn write_pci_u16(&self, a: PciAddress, o: u16, v: u16) { pci(a, o, 2, Some(v as u64)); }
    fn write_pci_u32(&self, a: PciAddress, o: u16, v: u32) { pci(a, o, 4, Some(v as u64)); }
    fn nanos_since_boot(&self) -> u64 { sex_pdx::get_ticks().saturating_mul(1_000_000) }
    fn stall(&self, us: u64) { let us = us.min(100_000); spend(us); stall_us(us) }
    fn sleep(&self, ms: u64) {
        let ms = ms.min(2_000);
        spend(ms * 1000);
        let end = sex_pdx::get_ticks() + ms;
        while sex_pdx::get_ticks() < end { let _ = sex_pdx::sys_wait_self_ticks((end - sex_pdx::get_ticks()).clamp(1, 64)); }
    }
    // One task runs AML; its mutexes never contend.
    fn create_mutex(&self) -> Handle { static NEXT: AtomicU64 = AtomicU64::new(1); Handle(NEXT.fetch_add(1, Ordering::Relaxed) as u32) }
    fn acquire(&self, _: Handle, _: u16) -> Result<(), AmlError> { Ok(()) }
    fn release(&self, _: Handle) {}
    fn handle_fatal_error(&self, kind: u8, code: u32, arg: u64) {
        serial_println!("[sexacpi.aml.fatal] type={} code={:#x} arg={:#x}", kind, code, arg);
    }
}

// ── Tables ─────────────────────────────────────────────────────────────────

/// Table `index` (0 FADT, 1 DSDT, 2.. SSDTs) as bytes, header included.
fn table(index: u64) -> Option<(u32, Vec<u8>)> {
    let first = w::call4(w::FW_TABLE, index, 0, 0);
    if first[0] != w::OK { return None; }
    let (length, signature) = (first[1] as u32 as usize, (first[1] >> 32) as u32);
    if length > 4 * 1024 * 1024 { return None; }
    let mut bytes = Vec::with_capacity(length);
    let mut offset = 0;
    while offset < length {
        let r = if offset == 0 { first } else { w::call4(w::FW_TABLE, index, offset as u64, 0) };
        if r[0] != w::OK { return None; }
        for word in &r[2..5] { bytes.extend_from_slice(&word.to_le_bytes()); }
        offset += 24;
    }
    bytes.truncate(length);
    Some((signature, bytes))
}

// ── Helpers ────────────────────────────────────────────────────────────────

type Aml = Interpreter<Fw>;
fn name(path: &str) -> Option<AmlName> { AmlName::from_str(path).ok() }
fn int_of(o: &WrappedObject) -> Option<u64> { if let Object::Integer(v) = **o { Some(v) } else { None } }
fn eval(aml: &Aml, path: &AmlName, method: &str, args: Vec<WrappedObject>) -> Option<WrappedObject> {
    let full = name(method)?.resolve(path).ok()?;
    aml.evaluate_if_present(full, args).ok().flatten()
}
fn eval_int(aml: &Aml, path: &AmlName, method: &str) -> Option<u64> { eval(aml, path, method, Vec::new()).and_then(|o| int_of(&o)) }
fn id_string(o: &WrappedObject) -> Option<String> {
    match &**o {
        Object::Integer(v) => Some(String::from_utf8_lossy(&w::eisa_id(*v as u32)).into_owned()),
        Object::String(s) => Some(s.clone()),
        _ => None,
    }
}
fn ids_of(aml: &Aml, path: &AmlName) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(h) = eval(aml, path, "_HID", Vec::new()).and_then(|o| id_string(&o)) { ids.push(h); }
    if let Some(c) = eval(aml, path, "_CID", Vec::new()) {
        match &*c { Object::Package(items) => ids.extend(items.iter().filter_map(id_string)), _ => ids.extend(id_string(&c)) }
    }
    ids
}
fn present(aml: &Aml, path: &AmlName) -> bool { eval_int(aml, path, "_STA").map_or(true, |s| s & 1 != 0) }

#[derive(Default)]
struct Devices {
    batteries: Vec<AmlName>,
    ac: Vec<AmlName>,
    lids: Vec<AmlName>,
    thermal: Vec<AmlName>,
    processors: Vec<AmlName>,
    i2c_hid: Vec<AmlName>,
}

fn inventory(aml: &Aml) -> Devices {
    let mut found: Vec<(AmlName, NamespaceLevelKind)> = Vec::new();
    let mut ns = aml.namespace.lock().clone();
    let _ = ns.traverse(|path, level| {
        match level.kind {
            NamespaceLevelKind::Device | NamespaceLevelKind::Processor | NamespaceLevelKind::ThermalZone => found.push((path.clone(), level.kind)),
            _ => {}
        }
        Ok(true)
    });
    drop(ns);
    let mut d = Devices::default();
    for (path, kind) in found.into_iter().take(1024) {
        match kind {
            NamespaceLevelKind::ThermalZone => d.thermal.push(path),
            NamespaceLevelKind::Processor => d.processors.push(path),
            _ => {
                let ids = ids_of(aml, &path);
                let has = |x: &str| ids.iter().any(|i| i == x);
                if has("ACPI0007") { d.processors.push(path.clone()); }
                if !present(aml, &path) { continue; }
                if has("PNP0C0A") { d.batteries.push(path); }
                else if has("ACPI0003") { d.ac.push(path); }
                else if has("PNP0C0D") { d.lids.push(path); }
                else if has("PNP0C50") || has("ACPI0C50") {
                    serial_println!("[sexacpi.i2c_hid] path={} ids={:?} driver=later", path, ids);
                    d.i2c_hid.push(path);
                }
            }
        }
    }
    d
}

/// The deepest `_CST` state entered with MWAIT whose exit latency stays under 200 us.
fn choose_cstate(aml: &Aml, cpu: &AmlName) -> Option<(u8, u16, u32)> {
    let cst = eval(aml, cpu, "_CST", Vec::new())?;
    let Object::Package(items) = &*cst else { return None };
    let mut best: Option<(u8, u16, u32)> = None;
    for item in items.iter().skip(1) {
        let Object::Package(state) = &**item else { continue };
        if state.len() < 4 { continue; }
        let Object::Buffer(reg) = &*state[0] else { continue };
        let (Some(kind), Some(latency)) = (int_of(&state[1]), int_of(&state[2])) else { continue };
        // Generic register descriptor: 0x82, len(2), space, width, offset, access, address(8).
        if reg.len() < 12 || reg[0] != 0x82 || reg[3] != 0x7F || reg[4] != 1 || reg[5] != 2 { continue; }
        let hint = u32::from_le_bytes([reg[7], reg[8], reg[9], reg[10]]);
        if latency <= 200 && best.map_or(true, |(k, _, _)| kind as u8 > k) { best = Some((kind as u8, latency as u16, hint)); }
    }
    best
}

fn read_sensors(aml: &Aml, d: &Devices) -> w::Sensors {
    let mut s = w::Sensors::default();
    let (mut now, mut full, mut charging) = (0u64, 0u64, false);
    for b in d.batteries.iter() {
        if eval_int(aml, b, "_STA").map_or(true, |v| v & 0x10 == 0) { continue; }
        let bst = eval(aml, b, "_BST", Vec::new());
        let (bix, shift) = match eval(aml, b, "_BIX", Vec::new()) { Some(o) => (Some(o), 1), None => (eval(aml, b, "_BIF", Vec::new()), 0) };
        if let (Some(bst), Some(bix)) = (bst, bix) {
            if let (Object::Package(st), Object::Package(info)) = (&*bst, &*bix) {
                let remaining = st.get(2).and_then(int_of).unwrap_or(0);
                let last_full = info.get(2 + shift).and_then(int_of).filter(|v| *v != 0 && *v != 0xFFFF_FFFF)
                    .or_else(|| info.get(1 + shift).and_then(int_of)).unwrap_or(0);
                if remaining != 0xFFFF_FFFF && last_full != 0 && last_full != 0xFFFF_FFFF { now += remaining; full += last_full; }
                charging |= st.first().and_then(int_of).map_or(false, |v| v & 2 != 0);
            }
        }
    }
    if full != 0 { s.battery_percent = Some(((now * 100) / full).min(100) as u8); s.charging = charging; }
    if let Some(ac) = d.ac.first() { s.on_ac = eval_int(aml, ac, "_PSR").map(|v| v != 0); }
    if let Some(lid) = d.lids.first() { s.lid_open = eval_int(aml, lid, "_LID").map(|v| v != 0); }
    s.hottest_decikelvin = d.thermal.iter().filter_map(|t| eval_int(aml, t, "_TMP")).filter(|v| *v > 0 && *v < 6000).max().map(|v| v as u32);
    s
}

// ── Bring-up ───────────────────────────────────────────────────────────────

fn load() -> Option<&'static Aml> {
    let (sig, fadt_bytes) = table(0)?;
    if sig != u32::from_le_bytes(*b"FACP") { return None; }
    let mut raw = [0u8; core::mem::size_of::<Fadt>()];
    let n = raw.len().min(fadt_bytes.len());
    raw[..n].copy_from_slice(&fadt_bytes[..n]);
    let fadt: Fadt = unsafe { core::ptr::read_unaligned(raw.as_ptr() as *const Fadt) };
    let registers = FixedRegisters::new(&fadt, Fw).ok()?;
    let (sig, dsdt) = table(1)?;
    if sig != u32::from_le_bytes(*b"DSDT") || dsdt.len() < 36 { return None; }
    let aml: &'static Aml = Box::leak(Box::new(Interpreter::new(Fw, dsdt[8], Arc::new(registers), None)));
    // The interpreter keeps pointers into each table's byte stream: they live for good.
    let mut streams: Vec<&'static [u8]> = Vec::new();
    streams.push(Box::leak(dsdt.into_boxed_slice()));
    let mut index = 2;
    while let Some((sig, ssdt)) = table(index) {
        if sig == u32::from_le_bytes(*b"SSDT") && ssdt.len() >= 36 { streams.push(Box::leak(ssdt.into_boxed_slice())); }
        index += 1;
    }
    let tables = streams.len();
    let mut loaded = 0;
    for stream in streams {
        if bounded(|| aml.load_table(&stream[36..])).is_ok() { loaded += 1; }
    }
    serial_println!("[sexacpi.tables] loaded={} of={}", loaded, tables);
    (loaded > 0).then_some(aml)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe { ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES); }
    serial_println!("[sexacpi.start] ring=3 aml_in_kernel=0");
    calibrate();
    let Some(aml) = load() else {
        serial_println!("[sexacpi.ready] aml=0 reason=no_tables");
        let _ = w::call4(w::FW_INVENTORY, 0, u64::MAX, 1);
        loop { let _ = sex_pdx::sys_wait_self_ticks(64); }
    };
    let (devices, s5, cstate) = bounded(|| {
        // Interrupts go through the APIC: tell the firmware before _INI runs.
        if let Some(pic) = name("\\_PIC") { let _ = aml.evaluate_if_present(pic, alloc::vec![WrappedObject::new(Object::Integer(1))]); }
        aml.initialize_namespace();
        let mut s5 = u64::MAX;
        if let Some(o) = eval(aml, &AmlName::root(), "\\_S5", Vec::new()) {
            if let Object::Package(items) = &*o {
                if let (Some(a), Some(b)) = (items.first().and_then(int_of), items.get(1).and_then(int_of)) { s5 = (a & 7) | (b & 7) << 8; }
            }
        }
        let devices = inventory(aml);
        let cstate = devices.processors.first().and_then(|cpu| choose_cstate(aml, cpu));
        (devices, s5, cstate)
    });
    let counts = devices.batteries.len().min(255) as u64 | (devices.lids.len().min(255) as u64) << 8
        | (devices.thermal.len().min(255) as u64) << 16 | (devices.i2c_hid.len().min(255) as u64) << 24 | 1 << 32;
    let _ = w::call4(w::FW_INVENTORY, counts, s5, 0);
    let idle = cstate.map_or(u64::MAX, |(kind, latency, hint)| w::call4(w::FW_IDLE, kind as u64 | (latency as u64) << 8 | (hint as u64) << 32, 0, 0)[0]);
    serial_println!("[sexacpi.ready] aml=1 batteries={} ac={} lids={} thermal={} processors={} i2c_hid={} s5={:#x} idle={}",
        devices.batteries.len(), devices.ac.len(), devices.lids.len(), devices.thermal.len(), devices.processors.len(), devices.i2c_hid.len(),
        s5, if idle == w::OK { "mwait" } else { "hlt" });

    let mut next_read = 0u64;
    loop {
        while let Some(msg) = sex_pdx::pdx_try_listen_raw(0) {
            if msg.type_id == w::OP_PREPARE && msg.caller_pd == 0 {
                // \_PTS(5) lets the firmware prepare for off (Files flushes meanwhile).
                if msg.arg0 == w::OFF {
                    let ok = bounded(|| name("\\_PTS").map_or(false, |p| aml.evaluate_if_present(p, alloc::vec![WrappedObject::new(Object::Integer(5))]).is_ok()));
                    serial_println!("[sexacpi.pts] state=5 ok={}", ok as u8);
                }
            }
        }
        let now = sex_pdx::get_ticks();
        if now >= next_read {
            let words = bounded(|| read_sensors(aml, &devices)).encode();
            let _ = w::call4(w::FW_SENSORS, words[1], words[2], words[3]);
            next_read = now + 2000;
        }
        let _ = sex_pdx::sys_wait_self_ticks(64);
    }
}
