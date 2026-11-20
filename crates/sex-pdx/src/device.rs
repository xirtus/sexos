// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Device leases and their interrupts (syscall 86, `SYS_DEVICE`).
//!
//! BOOT leases each PCI device to the service whose driver manifest
//! (`sex_hwdb::manifest`) matches it. The first device of a kind keeps the slot its
//! driver always used (NVMe 16, xHCI 8, NIC 19, sound 20, AHCI 21); further devices of
//! the same service land at `EXTRA_SLOT_BASE..EXTRA_SLOT_BASE + EXTRA_SLOTS`.
//!
//! Interrupts are notification bits, not messages: when a leased device raises one of
//! its MSI or MSI-X vectors, the kernel sets that vector's bit in the service's pending
//! word and wakes its main task. The bits coalesce, so an interrupt storm can never fill
//! the service's message ring. Vectors start masked; a driver unmasks the ones it is
//! ready for, waits with `sys_wait_self_ticks` (which also returns when a bit is set;
//! the untimed inbox wait does not, so an untaken bit cannot make an idle service
//! spin), and takes the bits with `irq_take`. A driver that never unmasks keeps polling.
//!
//! A device whose interrupt is a wire, not a message (a laptop touchpad's GPIO pad
//! routed to the I/O APIC), uses a *line*: BOOT reserves the vector and pending bit
//! (manifest `lines`), and the service, once firmware told it which I/O APIC input the
//! wire is, binds that input to the line once (`OP_LINE_BIND`). A level line is masked
//! by the kernel each time it fires, before the end of interrupt, so a wire held
//! asserted costs one interrupt, not a storm; the driver unmasks it (`OP_LINE_MASK`)
//! once it has quieted the device.
//!
//! All of this is BOOT-created: the call never mints a lease or a vector.

pub const SYS_DEVICE: u64 = 86;

/// rdi=index → rax=slot, rsi=identity, rdx=location, r10=interrupts.
pub const OP_LIST: u64 = 0;
/// rdi=slot, rsi=vector index → 0. Lets that vector interrupt.
pub const OP_UNMASK: u64 = 1;
/// rdi=slot, rsi=vector index → 0. Holds that vector's interrupts at the device.
pub const OP_MASK: u64 = 2;
/// → the pending bits, cleared atomically.
pub const OP_TAKE: u64 = 3;
/// rdi=line index, rsi=gsi | `LINE_EDGE` | `LINE_ACTIVE_LOW` → rax=the line's pending bit.
/// Points a line BOOT reserved for this service at I/O APIC input `gsi`, masked. Once
/// per line; refused for inputs under 16 (ISA, the kernel's own), inputs already bound
/// and inputs no I/O APIC has.
pub const OP_LINE_BIND: u64 = 4;
/// rdi=line index, rsi=1 mask / 0 unmask → 0.
pub const OP_LINE_MASK: u64 = 5;
pub const LINE_EDGE: u64 = 1 << 16;
pub const LINE_ACTIVE_LOW: u64 = 1 << 17;
/// Lines one service may hold.
pub const MAX_LINES: usize = 2;
/// rdi=slot, rsi=vector index | core << 16 → 0. Aims that vector at one core, so a
/// queue's completions land where its worker runs. `OP_UNMASK` already aims the vector
/// at the unmasking task's core; this is for a worker that unmasks on another's behalf.
/// Refused for vectors routed through another table (Intel VMD).
pub const OP_STEER: u64 = 6;
/// `OP_STEER`'s core: the caller's own.
pub const STEER_HERE: u32 = 0xffff;
/// rdi=slot, rsi=vector index | bit << 16 | notification slot << 32 → 0. From now on
/// that vector signals `bit` of an IPC v2 notification the caller may signal, instead
/// of the pending word and the main task: one worker per queue waits on its own
/// notification. Notification slot `u32::MAX` unbinds. If the notification is
/// destroyed, the vector falls back to the pending word.
pub const OP_BIND: u64 = 7;

pub const EXTRA_SLOT_BASE: u64 = 160;
pub const EXTRA_SLOTS: u64 = 32;
/// Leases BOOT may record, over every service.
pub const MAX_LEASES: usize = 64;
/// Bits in one service's pending word.
pub const MAX_VECTORS_PER_SERVICE: u32 = 64;

pub const ERR_NONE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode { None = 0, Msi = 1, MsiX = 2 }
impl Mode {
    pub fn from(v: u8) -> Mode { match v { 1 => Mode::Msi, 2 => Mode::MsiX, _ => Mode::None } }
    pub fn name(self) -> &'static str { match self { Mode::None => "none", Mode::Msi => "msi", Mode::MsiX => "msix" } }
}

/// One device a service holds, as `OP_LIST` describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub slot: u64,
    pub vendor: u16,
    pub device: u16,
    /// class << 16 | subclass << 8 | prog_if
    pub class: u32,
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub mode: Mode,
    /// Vectors programmed for this device; vector `i` sets pending bit `first_bit + i`.
    pub vectors: u8,
    pub first_bit: u8,
}

impl Lease {
    pub fn pack(&self) -> [u64; 4] {
        [self.slot,
         self.vendor as u64 | (self.device as u64) << 16 | (self.class as u64) << 32,
         (self.bus as u64) << 16 | (self.dev as u64) << 8 | self.func as u64,
         self.mode as u64 | (self.vectors as u64) << 8 | (self.first_bit as u64) << 16]
    }
    pub fn unpack(w: [u64; 4]) -> Lease {
        Lease { slot: w[0], vendor: w[1] as u16, device: (w[1] >> 16) as u16, class: (w[1] >> 32) as u32 & 0xff_ffff,
            bus: (w[2] >> 16) as u8, dev: (w[2] >> 8) as u8, func: w[2] as u8,
            mode: Mode::from(w[3] as u8), vectors: (w[3] >> 8) as u8, first_bit: (w[3] >> 16) as u8 }
    }
    /// The pending bits that belong to this device.
    pub fn bits(&self) -> u64 {
        if self.vectors == 0 { return 0; }
        let mask = if self.vectors >= 64 { u64::MAX } else { (1u64 << self.vectors) - 1 };
        mask << self.first_bit
    }
}

#[cfg(target_arch = "x86_64")]
fn call(op: u64, rdi: u64, rsi: u64) -> [u64; 4] {
    let (rax, out_rsi, out_rdx, out_r10): (u64, u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS_DEVICE => rax,
            inlateout("rdi") op => _,
            inlateout("rsi") rdi => out_rsi,
            inlateout("rdx") rsi => out_rdx,
            inlateout("r10") 0u64 => out_r10,
            inlateout("r8") 0u64 => _,
            out("rcx") _, out("r11") _);
    }
    [rax, out_rsi, out_rdx, out_r10]
}

/// The `index`th device this service holds, in BOOT's lease order.
#[cfg(target_arch = "x86_64")]
pub fn list(index: u64) -> Option<Lease> {
    let w = call(OP_LIST, index, 0);
    if w[0] == ERR_NONE { None } else { Some(Lease::unpack(w)) }
}

/// The lease at `slot`, if this service holds one there.
#[cfg(target_arch = "x86_64")]
pub fn at_slot(slot: u64) -> Option<Lease> {
    (0..MAX_LEASES as u64).map_while(list).find(|l| l.slot == slot)
}

#[cfg(target_arch = "x86_64")]
pub fn irq_unmask(slot: u64, vector: u64) -> Result<(), u64> {
    let r = call(OP_UNMASK, slot, vector)[0];
    if r == 0 { Ok(()) } else { Err(r) }
}

#[cfg(target_arch = "x86_64")]
pub fn irq_mask(slot: u64, vector: u64) -> Result<(), u64> {
    let r = call(OP_MASK, slot, vector)[0];
    if r == 0 { Ok(()) } else { Err(r) }
}

/// Aims `vector` of the device at `slot` at `core` (`STEER_HERE`: this task's core).
#[cfg(target_arch = "x86_64")]
pub fn irq_steer(slot: u64, vector: u16, core: u32) -> Result<(), u64> {
    let r = call(OP_STEER, slot, vector as u64 | ((core & 0xffff) as u64) << 16)[0];
    if r == 0 { Ok(()) } else { Err(r) }
}

/// Makes `vector` signal `bit` of the notification in `notification_slot`.
#[cfg(target_arch = "x86_64")]
pub fn irq_bind(slot: u64, vector: u16, notification_slot: u32, bit: u8) -> Result<(), u64> {
    let r = call(OP_BIND, slot, vector as u64 | ((bit & 63) as u64) << 16 | (notification_slot as u64) << 32)[0];
    if r == 0 { Ok(()) } else { Err(r) }
}

/// Binds reserved line `index` to I/O APIC input `gsi` (masked); its pending bit.
#[cfg(target_arch = "x86_64")]
pub fn line_bind(index: u64, gsi: u16, edge: bool, active_low: bool) -> Result<u8, u64> {
    let flags = if edge { LINE_EDGE } else { 0 } | if active_low { LINE_ACTIVE_LOW } else { 0 };
    let r = call(OP_LINE_BIND, index, gsi as u64 | flags)[0];
    if r < MAX_VECTORS_PER_SERVICE as u64 { Ok(r as u8) } else { Err(r) }
}

#[cfg(target_arch = "x86_64")]
pub fn line_mask(index: u64, masked: bool) -> Result<(), u64> {
    let r = call(OP_LINE_MASK, index, masked as u64)[0];
    if r == 0 { Ok(()) } else { Err(r) }
}

/// Pending interrupt bits since the last take, cleared.
#[cfg(target_arch = "x86_64")]
pub fn irq_take() -> u64 { call(OP_TAKE, 0, 0)[0] }
