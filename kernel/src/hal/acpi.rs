// SPDX-License-Identifier: GPL-3.0-only
//! ACPI power: shut down (S5), restart, the power button, power sensors and CPU idle and
//! performance states.
//!
//! Fixed hardware comes straight from the FADT and is driven by small, bounds-checked code
//! here, so shutting down and restarting never depend on AML. AML (the `acpi` crate's
//! interpreter) is used for what only it can tell: `\_S5` when it differs from the table
//! bytes, `\_PTS`, batteries, lids, thermal zones, `_CST` and I2C HID devices. Firmware AML
//! is untrusted input, so it runs on its own stack: if the interpreter panics on a firmware
//! bug, the panic handler returns to the caller here, AML is set aside for the rest of the
//! boot, and everything that does not need it keeps working.
//!
//! The authority to shut down is a `Power` capability (`sex_pdx::power`); see `invoke`.

use crate::serial_println;
use acpi::address::{AddressSpace, GenericAddress};
use acpi::aml::namespace::{AmlName, NamespaceLevelKind};
use acpi::aml::object::{Object, WrappedObject};
use acpi::aml::Interpreter;
use acpi::sdt::fadt::Fadt;
use acpi::{AcpiTables, Handle, PciAddress, PhysicalMapping};
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::NonNull;
use core::str::FromStr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use sex_pdx::power as w;
use x86_64::instructions::port::Port;

/// The IDT vector the SCI is routed to.
pub const SCI_VECTOR: u8 = 0x29;

static HHDM: AtomicU64 = AtomicU64::new(0);

// ── Hardware access for the tables and the interpreter ──────────────────────

#[derive(Clone, Copy)]
pub struct Hw;

fn phys<T>(address: usize) -> *mut T { (HHDM.load(Ordering::Relaxed) + address as u64) as *mut T }

fn pci_ecam(address: PciAddress, offset: u16) -> Option<usize> {
    let regions = crate::apic::PCI_REGIONS.try_lock()?;
    let r = regions.iter().find(|r| r.segment == address.segment() && (r.bus_start..=r.bus_end).contains(&address.bus()))?;
    Some(r.phys_addr + (((address.bus() - r.bus_start) as usize) << 20 | (address.device() as usize) << 15 | (address.function() as usize) << 12) + offset as usize)
}
fn pci_read(address: PciAddress, offset: u16, bytes: u8) -> u32 {
    if let Some(at) = pci_ecam(address, offset) {
        return unsafe { match bytes { 1 => phys::<u8>(at).read_volatile() as u32, 2 => phys::<u16>(at).read_volatile() as u32, _ => phys::<u32>(at).read_volatile() } };
    }
    if address.segment() != 0 || offset >= 256 { return u32::MAX; }
    // Legacy config ports only through the HAL's locked accessor.
    let word = unsafe { crate::hal::pci::pci_config_read(address.bus(), address.device(), address.function(), (offset & 0xFC) as u8) };
    let shift = (offset & 3) * 8;
    match bytes { 1 => (word >> shift) & 0xFF, 2 => (word >> shift) & 0xFFFF, _ => word }
}
fn pci_write(address: PciAddress, offset: u16, bytes: u8, value: u32) {
    if let Some(at) = pci_ecam(address, offset) {
        unsafe { match bytes { 1 => phys::<u8>(at).write_volatile(value as u8), 2 => phys::<u16>(at).write_volatile(value as u16), _ => phys::<u32>(at).write_volatile(value) } }
        return;
    }
    if address.segment() != 0 || offset >= 256 { return; }
    let (b, d, f, at) = (address.bus(), address.device(), address.function(), (offset & 0xFC) as u8);
    let shift = (offset & 3) * 8;
    let mask = match bytes { 1 => 0xFFu32, 2 => 0xFFFF, _ => u32::MAX };
    unsafe {
        let old = if bytes == 4 { 0 } else { crate::hal::pci::pci_config_read(b, d, f, at) };
        crate::hal::pci::pci_config_write(b, d, f, at, (old & !(mask << shift)) | ((value & mask) << shift));
    }
}
/// About a microsecond: a write to the POST port is an ISA bus cycle.
fn stall_us(us: u64) { for _ in 0..us { unsafe { Port::<u8>::new(0x80).write(0) } } }

static MUTEXES: AtomicU64 = AtomicU64::new(1);

impl acpi::Handler for Hw {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> PhysicalMapping<Self, T> {
        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: NonNull::new(phys::<T>(physical_address)).unwrap(),
            region_length: size,
            mapped_length: size,
            handler: *self,
        }
    }
    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
    fn read_u8(&self, a: usize) -> u8 { unsafe { phys::<u8>(a).read_volatile() } }
    fn read_u16(&self, a: usize) -> u16 { unsafe { phys::<u16>(a).read_volatile() } }
    fn read_u32(&self, a: usize) -> u32 { unsafe { phys::<u32>(a).read_volatile() } }
    fn read_u64(&self, a: usize) -> u64 { unsafe { phys::<u64>(a).read_volatile() } }
    fn write_u8(&self, a: usize, v: u8) { unsafe { phys::<u8>(a).write_volatile(v) } }
    fn write_u16(&self, a: usize, v: u16) { unsafe { phys::<u16>(a).write_volatile(v) } }
    fn write_u32(&self, a: usize, v: u32) { unsafe { phys::<u32>(a).write_volatile(v) } }
    fn write_u64(&self, a: usize, v: u64) { unsafe { phys::<u64>(a).write_volatile(v) } }
    fn read_io_u8(&self, p: u16) -> u8 { unsafe { Port::new(p).read() } }
    fn read_io_u16(&self, p: u16) -> u16 { unsafe { Port::new(p).read() } }
    fn read_io_u32(&self, p: u16) -> u32 { unsafe { Port::new(p).read() } }
    fn write_io_u8(&self, p: u16, v: u8) { unsafe { Port::new(p).write(v) } }
    fn write_io_u16(&self, p: u16, v: u16) { unsafe { Port::new(p).write(v) } }
    fn write_io_u32(&self, p: u16, v: u32) { unsafe { Port::new(p).write(v) } }
    fn read_pci_u8(&self, a: PciAddress, o: u16) -> u8 { pci_read(a, o, 1) as u8 }
    fn read_pci_u16(&self, a: PciAddress, o: u16) -> u16 { pci_read(a, o, 2) as u16 }
    fn read_pci_u32(&self, a: PciAddress, o: u16) -> u32 { pci_read(a, o, 4) }
    fn write_pci_u8(&self, a: PciAddress, o: u16, v: u8) { pci_write(a, o, 1, v as u32) }
    fn write_pci_u16(&self, a: PciAddress, o: u16, v: u16) { pci_write(a, o, 2, v as u32) }
    fn write_pci_u32(&self, a: PciAddress, o: u16, v: u32) { pci_write(a, o, 4, v) }
    fn nanos_since_boot(&self) -> u64 { crate::interrupts::TICKS.load(Ordering::Relaxed).saturating_mul(1_000_000) }
    fn stall(&self, us: u64) { stall_us(us.min(100_000)) }
    fn sleep(&self, ms: u64) { stall_us(ms.min(2_000) * 1000) }
    // AML only ever runs under `AML`'s lock, on one core at a time.
    fn create_mutex(&self) -> Handle { Handle(MUTEXES.fetch_add(1, Ordering::Relaxed) as u32) }
    fn acquire(&self, _: Handle, _: u16) -> Result<(), acpi::aml::AmlError> { Ok(()) }
    fn release(&self, _: Handle) {}
    fn handle_fatal_error(&self, kind: u8, code: u32, arg: u64) {
        serial_println!("[acpi.aml.fatal] type={} code={:#x} arg={:#x}", kind, code, arg);
    }
}

// ── Generic address registers ───────────────────────────────────────────────

fn gas_bits(g: &GenericAddress) -> u8 {
    match g.access_size { 1 => 8, 2 => 16, 3 => 32, 4 => 64, _ => match g.bit_width { 0..=8 => 8, 9..=16 => 16, 17..=32 => 32, _ => 64 } }
}
fn gas_read(g: &GenericAddress) -> Option<u64> {
    if g.address == 0 { return None; }
    let bits = gas_bits(g);
    Some(unsafe {
        match g.address_space {
            AddressSpace::SystemIo => match bits { 8 => Port::<u8>::new(g.address as u16).read() as u64, 16 => Port::<u16>::new(g.address as u16).read() as u64, _ => Port::<u32>::new(g.address as u16).read() as u64 },
            AddressSpace::SystemMemory => match bits { 8 => phys::<u8>(g.address as usize).read_volatile() as u64, 16 => phys::<u16>(g.address as usize).read_volatile() as u64, 32 => phys::<u32>(g.address as usize).read_volatile() as u64, _ => phys::<u64>(g.address as usize).read_volatile() },
            AddressSpace::PciConfigSpace => {
                let a = PciAddress::new(0, 0, (g.address >> 32) as u8, (g.address >> 16) as u8);
                pci_read(a, g.address as u16, (bits / 8).min(4)) as u64
            }
            _ => return None,
        }
    })
}
fn gas_write(g: &GenericAddress, v: u64) -> bool {
    if g.address == 0 { return false; }
    let bits = gas_bits(g);
    unsafe {
        match g.address_space {
            AddressSpace::SystemIo => match bits { 8 => Port::<u8>::new(g.address as u16).write(v as u8), 16 => Port::<u16>::new(g.address as u16).write(v as u16), _ => Port::<u32>::new(g.address as u16).write(v as u32) },
            AddressSpace::SystemMemory => match bits { 8 => phys::<u8>(g.address as usize).write_volatile(v as u8), 16 => phys::<u16>(g.address as usize).write_volatile(v as u16), 32 => phys::<u32>(g.address as usize).write_volatile(v as u32), _ => phys::<u64>(g.address as usize).write_volatile(v) },
            AddressSpace::PciConfigSpace => {
                let a = PciAddress::new(0, 0, (g.address >> 32) as u8, (g.address >> 16) as u8);
                pci_write(a, g.address as u16, (bits / 8).min(4), v as u32);
            }
            _ => return false,
        }
    }
    true
}
/// A sub-register of a block: `bytes` wide at `offset`.
fn gas_part(g: &GenericAddress, offset: u64, bytes: u8) -> GenericAddress {
    GenericAddress { address_space: g.address_space, bit_width: bytes * 8, bit_offset: 0, access_size: match bytes { 1 => 1, 2 => 2, _ => 3 }, address: g.address + offset }
}
fn gas_label(g: &Option<GenericAddress>) -> (&'static str, u64) {
    match g {
        None => ("none", 0),
        Some(g) => (match g.address_space { AddressSpace::SystemIo => "io", AddressSpace::SystemMemory => "mmio", AddressSpace::PciConfigSpace => "pci", _ => "other" }, g.address),
    }
}

// ── What the tables say ────────────────────────────────────────────────────

struct Fixed {
    pm1a_evt: Option<GenericAddress>,
    pm1b_evt: Option<GenericAddress>,
    /// Bytes in each half (status, enable) of a PM1 event block.
    pm1_half: u8,
    pm1a_cnt: Option<GenericAddress>,
    pm1b_cnt: Option<GenericAddress>,
    sleep_cnt: Option<GenericAddress>,
    reset: Option<(GenericAddress, u8)>,
    gpe: [(Option<GenericAddress>, u8); 2],
    sci: u16,
    hw_reduced: bool,
    power_button_fixed: bool,
}
static FIXED: spin::Once<Fixed> = spin::Once::new();
/// SLP_TYPa / SLP_TYPb for S5; from AML when it runs, else from the DSDT bytes.
static S5: AtomicU32 = AtomicU32::new(u32::MAX);
static INFO: spin::Mutex<w::Info> = spin::Mutex::new(w::Info { flags: 0, idle_state: 1, idle_latency_us: 1, batteries: 0, lids: 0, thermal_zones: 0, i2c_hid: 0 });

const PM1_PWRBTN: u64 = 1 << 8;
const PM1_WAK: u64 = 1 << 15;
const PM1_SCI_EN: u64 = 1;
const PM1_SLP_EN: u64 = 1 << 13;

fn pm1_status_read(f: &Fixed) -> u64 {
    let mut v = 0;
    for g in [f.pm1a_evt, f.pm1b_evt].iter().flatten() { v |= gas_read(&gas_part(g, 0, f.pm1_half)).unwrap_or(0); }
    v
}
fn pm1_status_clear(f: &Fixed, bits: u64) {
    for g in [f.pm1a_evt, f.pm1b_evt].iter().flatten() { gas_write(&gas_part(g, 0, f.pm1_half), bits); }
}
fn pm1_enable(f: &Fixed, bits: u64) {
    for g in [f.pm1a_evt, f.pm1b_evt].iter().flatten() {
        let en = gas_part(g, f.pm1_half as u64, f.pm1_half);
        let old = gas_read(&en).unwrap_or(0);
        gas_write(&en, old | bits);
    }
}

// ── AML on its own stack ───────────────────────────────────────────────────

core::arch::global_asm!(r#"
.global sex_aml_enter
sex_aml_enter:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rcx], rsp
    mov r12, rcx
    mov rsp, rdx
    mov rax, rdi
    mov rdi, rsi
    call rax
    mov rsp, [r12]
    xor eax, eax
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
.global sex_aml_escape
sex_aml_escape:
    mov rsp, rdi
    mov eax, 1
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
"#);
extern "C" {
    fn sex_aml_enter(f: extern "C" fn(*mut u8), arg: *mut u8, stack_top: u64, saved: *mut u64) -> u64;
    fn sex_aml_escape(saved: u64) -> !;
}
const AML_STACK_BYTES: usize = 256 * 1024;
#[repr(C, align(16))]
struct AmlStack([u8; AML_STACK_BYTES]);
static mut AML_STACK: AmlStack = AmlStack([0; AML_STACK_BYTES]);
static mut AML_SAVED: u64 = 0;
/// Core running AML, plus one; 0 when none is.
static AML_CORE: AtomicU32 = AtomicU32::new(0);
static AML_QUARANTINED: AtomicBool = AtomicBool::new(false);
static AML: spin::Mutex<Option<Interpreter<Hw>>> = spin::Mutex::new(None);

fn this_core() -> u32 {
    if crate::core_local::INITIALIZED.load(Ordering::Acquire) { crate::core_local::CoreLocal::get().core_id } else { 0 }
}

/// Runs `f` on the AML stack. None if it panicked; AML is then set aside for good.
/// The caller holds `AML`'s lock, so only one core is ever here.
fn quarantined<R, F: FnOnce() -> R>(f: F) -> Option<R> {
    if AML_QUARANTINED.load(Ordering::Acquire) { return None; }
    let (escaped, r) = on_aml_stack(f);
    if escaped {
        AML_QUARANTINED.store(true, Ordering::Release);
        INFO.lock().flags |= w::AML_QUARANTINED;
        serial_println!("[acpi.aml.quarantined] reason=interpreter_panic tables_only=1");
        return None;
    }
    r
}
/// Proves at boot that a panic on the AML stack returns here, before any firmware AML runs.
static SELF_TEST: AtomicBool = AtomicBool::new(false);
fn quarantine_self_test() -> bool {
    SELF_TEST.store(true, Ordering::Release);
    let (escaped, r) = on_aml_stack(|| -> u32 { if SELF_TEST.load(Ordering::Acquire) { panic!("quarantine self-test") } 7 });
    SELF_TEST.store(false, Ordering::Release);
    let (_, normal) = on_aml_stack(|| 7u32);
    escaped && r.is_none() && normal == Some(7)
}
fn on_aml_stack<R, F: FnOnce() -> R>(f: F) -> (bool, Option<R>) {
    struct Call<F, R> { f: Option<F>, r: Option<R> }
    extern "C" fn trampoline<F: FnOnce() -> R, R>(p: *mut u8) {
        let call = unsafe { &mut *(p as *mut Call<F, R>) };
        if let Some(f) = call.f.take() { call.r = Some(f()); }
    }
    let mut call = Call { f: Some(f), r: None };
    let top = unsafe { (core::ptr::addr_of_mut!(AML_STACK.0) as *mut u8).add(AML_STACK_BYTES) as u64 } & !15;
    AML_CORE.store(this_core() + 1, Ordering::Release);
    let escaped = unsafe { sex_aml_enter(trampoline::<F, R>, &mut call as *mut _ as *mut u8, top, core::ptr::addr_of_mut!(AML_SAVED)) };
    AML_CORE.store(0, Ordering::Release);
    if escaped != 0 {
        // `call` may hold half-built values from the abandoned frames; never drop them.
        core::mem::forget(call);
        return (true, None);
    }
    (false, call.r)
}

/// From the panic handler: if this core is running AML, return to its caller instead of
/// halting the machine.
pub fn panic_escape(info: &core::panic::PanicInfo) {
    let core = AML_CORE.load(Ordering::Acquire);
    if core != 0 && core == this_core() + 1 {
        if !SELF_TEST.load(Ordering::Acquire) { serial_println!("[acpi.aml.panic] {}", info); }
        AML_CORE.store(0, Ordering::Release);
        unsafe { sex_aml_escape(AML_SAVED) }
    }
}

fn name(path: &str) -> Option<AmlName> { AmlName::from_str(path).ok() }
fn int_of(o: &WrappedObject) -> Option<u64> { if let Object::Integer(v) = **o { Some(v) } else { None } }
fn eval(aml: &Interpreter<Hw>, path: &AmlName, method: &str, args: Vec<WrappedObject>) -> Option<WrappedObject> {
    let full = name(method)?.resolve(path).ok()?;
    aml.evaluate_if_present(full, args).ok().flatten()
}
fn eval_int(aml: &Interpreter<Hw>, path: &AmlName, method: &str) -> Option<u64> { eval(aml, path, method, Vec::new()).and_then(|o| int_of(&o)) }
fn id_string(o: &WrappedObject) -> Option<String> {
    match &**o {
        Object::Integer(v) => Some(String::from_utf8_lossy(&w::eisa_id(*v as u32)).into_owned()),
        Object::String(s) => Some(s.clone()),
        _ => None,
    }
}
fn ids_of(aml: &Interpreter<Hw>, path: &AmlName) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(h) = eval(aml, path, "_HID", Vec::new()).and_then(|o| id_string(&o)) { ids.push(h); }
    if let Some(c) = eval(aml, path, "_CID", Vec::new()) {
        match &*c { Object::Package(items) => ids.extend(items.iter().filter_map(id_string)), _ => ids.extend(id_string(&c)) }
    }
    ids
}
fn present(aml: &Interpreter<Hw>, path: &AmlName) -> bool { eval_int(aml, path, "_STA").map_or(true, |s| s & 1 != 0) }

#[derive(Default)]
struct Devices {
    batteries: Vec<AmlName>,
    ac: Vec<AmlName>,
    lids: Vec<AmlName>,
    thermal: Vec<AmlName>,
    processors: Vec<AmlName>,
    i2c_hid: Vec<AmlName>,
}
static DEVICES: spin::Mutex<Option<Devices>> = spin::Mutex::new(None);

fn inventory(aml: &Interpreter<Hw>) -> Devices {
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
                    serial_println!("[acpi.i2c_hid] path={} ids={:?} driver=later", path, ids);
                    d.i2c_hid.push(path);
                }
            }
        }
    }
    d
}

/// The deepest `_CST` state entered with MWAIT whose exit latency stays under 200 us.
fn choose_cstate(aml: &Interpreter<Hw>, cpu: &AmlName) -> Option<(u8, u16, u32)> {
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

// ── CPU performance and idle ───────────────────────────────────────────────

static MWAIT_HINT: AtomicU32 = AtomicU32::new(u32::MAX);
#[repr(align(64))]
struct Line([AtomicU64; 8]);
static MONITOR_LINES: [Line; 64] = [const { Line([const { AtomicU64::new(0) }; 8]) }; 64];

fn cpuid(leaf: u32, sub: u32) -> core::arch::x86_64::CpuidResult { unsafe { core::arch::x86_64::__cpuid_count(leaf, sub) } }
fn max_leaf() -> u32 { cpuid(0, 0).eax }
fn max_ext_leaf() -> u32 { cpuid(0x8000_0000, 0).eax }

/// Hands frequency choice to the CPU: Intel HWP or AMD CPPC, balanced bias. Per core.
pub fn cpu_init_local() -> u64 {
    use x86_64::registers::model_specific::Msr;
    let mut flags = 0;
    if max_leaf() >= 6 {
        let eax = cpuid(6, 0).eax;
        if eax & (1 << 7) != 0 {
            unsafe {
                Msr::new(0x770).write(1); // IA32_PM_ENABLE
                let caps = Msr::new(0x771).read();
                let (highest, lowest) = (caps & 0xFF, (caps >> 24) & 0xFF);
                let epp = if eax & (1 << 10) != 0 { 0x80u64 << 24 } else { 0 };
                Msr::new(0x774).write(lowest | highest << 8 | epp); // IA32_HWP_REQUEST, desired = auto
            }
            flags |= w::HWP;
        }
    }
    if flags == 0 && max_ext_leaf() >= 0x8000_0008 && cpuid(0x8000_0008, 0).ebx & (1 << 27) != 0 {
        unsafe {
            Msr::new(0xC001_02B1).write(1); // CPPC_ENABLE
            let caps = Msr::new(0xC001_02B0).read();
            let (highest, lowest) = ((caps >> 24) & 0xFF, caps & 0xFF);
            Msr::new(0xC001_02B3).write(highest | lowest << 8 | 0x80 << 24); // max, min, desired auto, EPP
        }
        flags |= w::CPPC;
    }
    flags
}

/// The scheduler's idle: MWAIT into the chosen C-state, or HLT (C1). Interrupts wake it.
pub fn idle() {
    let hint = MWAIT_HINT.load(Ordering::Relaxed);
    if hint == u32::MAX {
        x86_64::instructions::interrupts::enable_and_hlt();
        return;
    }
    let line = &MONITOR_LINES[(this_core() as usize) % 64].0[0] as *const AtomicU64;
    unsafe {
        core::arch::asm!("monitor", in("rax") line, in("ecx") 0, in("edx") 0, options(nostack));
        core::arch::asm!("sti", "mwait", in("eax") hint, in("ecx") 1, options(nostack));
    }
}

// ── Bring-up ──────────────────────────────────────────────────────────────

pub fn init(rsdp: u64, hhdm: u64) {
    HHDM.store(hhdm, Ordering::Release);
    let tables = match unsafe { AcpiTables::from_rsdp(Hw, rsdp as usize) } {
        Ok(t) => t,
        Err(e) => { serial_println!("[acpi.power] tables=0 error={:?}", e); return; }
    };
    let Some(fadt) = tables.find_table::<Fadt>() else { serial_println!("[acpi.power] fadt=0"); return };
    let flags = { fadt.flags };
    let reset = if flags.supports_system_reset_via_fadt() || fadt.header.revision >= 2 {
        fadt.reset_register().ok().filter(|g| g.address != 0).map(|g| (g, fadt.reset_value))
    } else { None };
    let fixed = Fixed {
        pm1a_evt: fadt.pm1a_event_block().ok().filter(|g| g.address != 0),
        pm1b_evt: fadt.pm1b_event_block().ok().flatten().filter(|g| g.address != 0),
        pm1_half: (fadt.pm1_event_length / 2).max(1),
        pm1a_cnt: fadt.pm1a_control_block().ok().filter(|g| g.address != 0),
        pm1b_cnt: fadt.pm1b_control_block().ok().flatten().filter(|g| g.address != 0),
        sleep_cnt: fadt.sleep_control_register().ok().flatten().filter(|g| g.address != 0),
        reset,
        gpe: [(fadt.gpe0_block().ok().flatten(), fadt.gpe0_block_length), (fadt.gpe1_block().ok().flatten(), fadt.gpe1_block_length)],
        sci: fadt.sci_interrupt,
        hw_reduced: flags.system_is_hw_reduced_acpi(),
        power_button_fixed: !flags.power_button_is_control_method(),
    };
    let (smi, enable) = (fadt.smi_cmd_port, fadt.acpi_enable);

    // \_S5 from the DSDT bytes: shut down works even if AML cannot run.
    if let Ok(dsdt) = tables.dsdt() {
        let header = core::mem::size_of::<acpi::sdt::SdtHeader>();
        if dsdt.length as usize > header {
            let bytes = unsafe { core::slice::from_raw_parts(phys::<u8>(dsdt.phys_address + header), dsdt.length as usize - header) };
            if let Some((a, b)) = w::s5_from_dsdt(bytes) { S5.store(a as u32 | (b as u32) << 8, Ordering::Release); }
        }
    }
    drop(fadt);

    // ACPI mode: firmware hands the fixed events to us and raises the SCI for them.
    if !fixed.hw_reduced {
        let sci_en = fixed.pm1a_cnt.as_ref().and_then(gas_read).map_or(false, |v| v & PM1_SCI_EN != 0);
        if !sci_en && smi != 0 && enable != 0 {
            unsafe { Port::<u8>::new(smi as u16).write(enable) };
            for _ in 0..3000 {
                if fixed.pm1a_cnt.as_ref().and_then(gas_read).map_or(false, |v| v & PM1_SCI_EN != 0) { break; }
                stall_us(100);
            }
        }
        // No general purpose events until there is code for them: an armed GPE with
        // no handler would hold the level-triggered SCI asserted.
        for (block, len) in fixed.gpe.iter() {
            let (Some(g), half) = (block, *len / 2) else { continue };
            if g.address_space != AddressSpace::SystemIo { continue; }
            for i in 0..half as u64 {
                gas_write(&gas_part(g, half as u64 + i, 1), 0);
                gas_write(&gas_part(g, i, 1), 0xFF);
            }
        }
        pm1_status_clear(&fixed, 0xFFFF);
        if fixed.power_button_fixed { pm1_enable(&fixed, PM1_PWRBTN); }
        // The SCI is level triggered, active low, unless the MADT says otherwise.
        if (fixed.sci as usize) < 256 {
            let mut iso = crate::apic::ISO_OVERRIDES.lock();
            if iso[fixed.sci as usize].is_none() {
                iso[fixed.sci as usize] = Some(crate::apic::IsoOverride { isa_source: fixed.sci as u8, gsi: fixed.sci as u32, active_low: true, level_triggered: true });
            }
        }
        unsafe { crate::apic::map_irq(fixed.sci as u8, SCI_VECTOR, 0, x86_64::VirtAddr::new(hhdm)); }
    }
    let sci_en = fixed.pm1a_cnt.as_ref().and_then(gas_read).map_or(0, |v| v & PM1_SCI_EN);
    let (rk, ra) = gas_label(&fixed.reset.map(|r| r.0));
    let (ck, ca) = gas_label(&fixed.pm1a_cnt);
    FIXED.call_once(|| fixed);

    // AML: the interpreter, then what only AML knows.
    let mut aml_ok = false;
    let guarded = quarantine_self_test();
    serial_println!("[acpi.aml.quarantine] self_test={}", guarded as u8);
    if !guarded { AML_QUARANTINED.store(true, Ordering::Release); }
    let mut cpu_flags = cpu_init_local();
    if let Ok(platform) = acpi::platform::AcpiPlatform::new(tables, Hw) {
        let mut slot = AML.lock();
        if let Some(Ok(interp)) = quarantined(|| Interpreter::new_from_platform(&platform)) {
            *slot = Some(interp);
            let interp = slot.as_ref().unwrap();
            let ready = quarantined(|| {
                // Interrupts go through the APIC: tell the firmware before _INI runs.
                if let Some(pic) = name("\\_PIC") { let _ = interp.evaluate_if_present(pic, alloc::vec![WrappedObject::new(Object::Integer(1))]); }
                interp.initialize_namespace();
                if let Some(s5) = eval(interp, &AmlName::root(), "\\_S5", Vec::new()) {
                    if let Object::Package(items) = &*s5 {
                        if let (Some(a), Some(b)) = (items.first().and_then(int_of), items.get(1).and_then(int_of)) {
                            S5.store((a & 7) as u32 | ((b & 7) as u32) << 8, Ordering::Release);
                        }
                    }
                }
                let devices = inventory(interp);
                let cstate = devices.processors.first().and_then(|cpu| choose_cstate(interp, cpu));
                (devices, cstate)
            });
            if let Some((devices, cstate)) = ready {
                aml_ok = true;
                let mut info = INFO.lock();
                info.batteries = devices.batteries.len().min(255) as u8;
                info.lids = devices.lids.len().min(255) as u8;
                info.thermal_zones = devices.thermal.len().min(255) as u8;
                info.i2c_hid = devices.i2c_hid.len().min(255) as u8;
                // Deeper than C1 only where the local APIC timer keeps running (ARAT).
                let monitor = cpuid(1, 0).ecx & (1 << 3) != 0;
                let arat = max_leaf() >= 6 && cpuid(6, 0).eax & (1 << 2) != 0;
                if let Some((kind, latency, hint)) = cstate {
                    if monitor && arat && kind >= 2 {
                        MWAIT_HINT.store(hint, Ordering::Release);
                        info.idle_state = kind; info.idle_latency_us = latency;
                        cpu_flags |= w::MWAIT_IDLE;
                    }
                }
                *DEVICES.lock() = Some(devices);
            }
        }
    }
    let s5 = S5.load(Ordering::Acquire);
    let f = FIXED.get().unwrap();
    let mut info = INFO.lock();
    info.flags |= cpu_flags
        | if s5 != u32::MAX && (f.pm1a_cnt.is_some() || f.sleep_cnt.is_some()) { w::CAN_OFF } else { 0 }
        | w::CAN_RESTART
        | if f.power_button_fixed && !f.hw_reduced { w::POWER_BUTTON } else { 0 }
        | if f.hw_reduced { w::HW_REDUCED } else { 0 }
        | if aml_ok { w::AML } else { 0 };
    serial_println!("[acpi.power] s5={} slp_typ_a={} slp_typ_b={} pm1_cnt={}:{:#x} reset={}:{:#x} sci={} sci_en={} power_button={} hw_reduced={} aml={} quarantined={} batteries={} lids={} thermal={} i2c_hid={} idle=C{} hwp={} cppc={}",
        (info.flags & w::CAN_OFF != 0) as u8, s5 & 0xFF, (s5 >> 8) & 0xFF, ck, ca, rk, ra, f.sci, sci_en,
        (info.flags & w::POWER_BUTTON != 0) as u8, f.hw_reduced as u8, aml_ok as u8, AML_QUARANTINED.load(Ordering::Relaxed) as u8,
        info.batteries, info.lids, info.thermal_zones, info.i2c_hid, info.idle_state,
        (cpu_flags & w::HWP != 0) as u8, (cpu_flags & w::CPPC != 0) as u8);
}

// ── Shutting down and restarting ───────────────────────────────────────────

/// 0 idle, 1 waiting for Files, 2 going down.
static STATE: AtomicU8 = AtomicU8::new(0);
static KIND: AtomicU64 = AtomicU64::new(0);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
static FLUSHED: AtomicU8 = AtomicU8::new(0);
/// The domain told before power changes (Files), set by BOOT with its grant.
static FLUSHER: AtomicU32 = AtomicU32::new(0);

pub fn set_flusher(pd: u32) { FLUSHER.store(pd, Ordering::Release); }

/// Starts a shut down or restart. Files is told first; `poll` finishes it.
pub fn begin(kind: u64, why: &str) -> bool {
    if STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() { return false; }
    KIND.store(kind, Ordering::Release);
    let now = crate::interrupts::TICKS.load(Ordering::Relaxed);
    DEADLINE.store(now + w::FLUSH_TICKS, Ordering::Release);
    let what = if kind == w::RESTART { "restart" } else { "off" };
    let files = FLUSHER.load(Ordering::Acquire);
    let told = files != 0 && crate::ipc::DOMAIN_REGISTRY.get(files).map_or(false, |pd| {
        let msg = crate::ipc::messages::MessageType::IpcCall { func_id: w::OP_PREPARE, arg0: kind, arg1: 0, arg2: 0, caller_pd: 0, caller_task: 0 };
        let ok = unsafe { (*pd.message_ring).enqueue(msg) }.is_ok();
        if ok { crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire)); }
        ok
    });
    serial_println!("[acpi.power.begin] kind={} by={} files={} told={}", what, why, files, told as u8);
    if !told { FLUSHED.store(2, Ordering::Release); }
    true
}

/// Timer and syscall paths: finish once Files has answered or its time is up.
pub fn poll() {
    if STATE.load(Ordering::Acquire) != 1 { return; }
    let late = crate::interrupts::TICKS.load(Ordering::Relaxed) >= DEADLINE.load(Ordering::Acquire);
    if FLUSHED.load(Ordering::Acquire) == 0 && !late { return; }
    if STATE.compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire).is_err() { return; }
    let flushed = FLUSHED.load(Ordering::Acquire);
    serial_println!("[acpi.power.flush] files={} late={}", match flushed { 1 => "flushed", 2 => "absent", 3 => "failed", _ => "no_answer" }, late as u8);
    if KIND.load(Ordering::Acquire) == w::RESTART { restart() } else { power_off() }
}

/// The power button (SCI). Runs in the interrupt handler, before EOI.
pub fn sci() {
    let Some(f) = FIXED.get() else { return };
    let status = pm1_status_read(f);
    if status & PM1_PWRBTN != 0 {
        pm1_status_clear(f, PM1_PWRBTN);
        serial_println!("[acpi.power_button] pressed=1");
        begin(w::OFF, "power_button");
    }
    if status & PM1_WAK != 0 { pm1_status_clear(f, PM1_WAK); }
}

fn stop_everything() {
    x86_64::instructions::interrupts::disable();
}

pub fn power_off() -> ! {
    let f = FIXED.get();
    let s5 = S5.load(Ordering::Acquire);
    // \_PTS(5) lets the firmware prepare; skipped if AML is unavailable or busy.
    if let Some(mut aml) = AML.try_lock() {
        if let Some(interp) = aml.as_ref() {
            let _ = quarantined(|| { if let Some(p) = name("\\_PTS") { let _ = interp.evaluate_if_present(p, alloc::vec![WrappedObject::new(Object::Integer(5))]); } });
        }
        if AML_QUARANTINED.load(Ordering::Acquire) { core::mem::forget(aml.take()); }
    }
    serial_println!("[acpi.power.off] s5={:#x} entering=1", s5);
    stop_everything();
    if let (Some(f), true) = (f, s5 != u32::MAX) {
        let (a, b) = ((s5 & 7) as u64, ((s5 >> 8) & 7) as u64);
        pm1_status_clear(f, PM1_WAK);
        if let Some(g) = &f.sleep_cnt {
            gas_write(g, (a << 2) | (1 << 5));
        } else {
            for (g, typ) in [(f.pm1a_cnt, a), (f.pm1b_cnt, b)] {
                let Some(g) = g else { continue };
                let v = gas_read(&g).unwrap_or(0) & !(7 << 10 | PM1_SLP_EN);
                gas_write(&g, v | typ << 10);
            }
            for (g, typ) in [(f.pm1a_cnt, a), (f.pm1b_cnt, b)] {
                let Some(g) = g else { continue };
                let v = gas_read(&g).unwrap_or(0) & !(7 << 10 | PM1_SLP_EN);
                gas_write(&g, v | typ << 10 | PM1_SLP_EN);
            }
        }
        stall_us(200_000);
    }
    serial_println!("[acpi.power.off] failed=1 halted=1 reason={}", if s5 == u32::MAX { "no_s5" } else { "still_running" });
    loop { x86_64::instructions::hlt(); }
}

pub fn restart() -> ! {
    serial_println!("[acpi.power.restart] entering=1");
    stop_everything();
    if let Some((g, value)) = FIXED.get().and_then(|f| f.reset) {
        serial_println!("[acpi.power.restart] method=fadt_reset_register");
        gas_write(&g, value as u64);
        stall_us(50_000);
    }
    unsafe {
        serial_println!("[acpi.power.restart] method=port_cf9");
        Port::<u8>::new(0xCF9).write(0x02);
        stall_us(10);
        Port::<u8>::new(0xCF9).write(0x06);
        stall_us(50_000);
        serial_println!("[acpi.power.restart] method=keyboard_controller");
        for _ in 0..10_000 { if Port::<u8>::new(0x64).read() & 2 == 0 { break; } stall_us(10); }
        Port::<u8>::new(0x64).write(0xFE);
        stall_us(50_000);
        serial_println!("[acpi.power.restart] method=triple_fault");
        let empty = x86_64::structures::DescriptorTablePointer { limit: 0, base: x86_64::VirtAddr::new(0) };
        x86_64::instructions::tables::lidt(&empty);
        core::arch::asm!("int3");
    }
    loop { x86_64::instructions::hlt(); }
}

// ── Sensors ────────────────────────────────────────────────────────────────

fn read_sensors() -> Result<w::Sensors, u64> {
    let Some(mut aml) = AML.try_lock() else { return Err(w::BUSY) };
    let devices = DEVICES.lock();
    let (Some(interp), Some(d)) = (aml.as_ref(), devices.as_ref()) else { return Ok(w::Sensors::default()) };
    let out = quarantined(|| {
        let mut s = w::Sensors::default();
        let (mut now, mut full, mut charging) = (0u64, 0u64, false);
        for b in d.batteries.iter() {
            if eval_int(interp, b, "_STA").map_or(true, |v| v & 0x10 == 0) { continue; }
            let bst = eval(interp, b, "_BST", Vec::new());
            let (bix, shift) = match eval(interp, b, "_BIX", Vec::new()) { Some(o) => (Some(o), 1), None => (eval(interp, b, "_BIF", Vec::new()), 0) };
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
        if let Some(ac) = d.ac.first() { s.on_ac = eval_int(interp, ac, "_PSR").map(|v| v != 0); }
        if let Some(lid) = d.lids.first() { s.lid_open = eval_int(interp, lid, "_LID").map(|v| v != 0); }
        s.hottest_decikelvin = d.thermal.iter().filter_map(|t| eval_int(interp, t, "_TMP")).filter(|v| *v > 0 && *v < 6000).max().map(|v| v as u32);
        s
    });
    if AML_QUARANTINED.load(Ordering::Acquire) { drop(devices); core::mem::forget(aml.take()); }
    out.ok_or(w::UNAVAILABLE)
}

// ── The Power capability (syscall 84) ──────────────────────────────────────

pub fn invoke(pd: &crate::capability::ProtectionDomain, slot: u64, op: u64, arg: u64) -> [u64; 5] {
    let denied = [w::DENIED, 0, 0, 0, 0];
    let Some(slot) = crate::capability_slot::decode_capability_slot(slot) else { return denied };
    let Some(crate::capability::CapabilityData::Power(grant)) = pd.find_capability(slot).map(|c| c.data) else { return denied };
    if grant.holder_pd != pd.id { return denied; }
    let has = |right: u8| grant.rights & right != 0;
    match op {
        w::INFO => INFO.lock().encode(),
        w::OFF | w::RESTART if has(w::REQUEST) => {
            let info = INFO.lock().flags;
            if op == w::OFF && info & w::CAN_OFF == 0 { return [w::UNAVAILABLE, 0, 0, 0, 0]; }
            if begin(op, "shell") { [w::OK, 0, 0, 0, 0] } else { [w::BUSY, 0, 0, 0, 0] }
        }
        w::READY if has(w::FLUSH) => {
            if pd.id != FLUSHER.load(Ordering::Acquire) || STATE.load(Ordering::Acquire) != 1 { return [w::INVALID, 0, 0, 0, 0]; }
            FLUSHED.store(if arg != 0 { 1 } else { 3 }, Ordering::Release);
            poll();
            [w::OK, 0, 0, 0, 0]
        }
        w::SENSORS if has(w::SENSE) || has(w::REQUEST) => match read_sensors() { Ok(s) => s.encode(), Err(e) => [e, 0, 0, 0, 0] },
        w::OFF | w::RESTART | w::READY | w::SENSORS => denied,
        _ => [w::INVALID, 0, 0, 0, 0],
    }
}
