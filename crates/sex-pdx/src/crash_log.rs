// SPDX-License-Identifier: GPL-3.0-only
//! Crash records: what stopped, where, and when. The kernel writes one whenever a
//! service or program faults, a service is started again or given up on, or the kernel
//! itself stops. Files keeps them in a reserved ring on disk, so the next start can show
//! what went wrong last time even on a PC with no serial port.
//!
//! One record is one 512-byte sector, the same bytes in the kernel, on the wire
//! (`SYS_CRASH_LOG`) and on disk.

/// Syscall number. `rdi` selects the operation.
pub const SYSCALL: u64 = 85;
/// rsi: index from 0; rdx: buffer of `RECORD_BYTES`. Copies the record kept at that
/// index (oldest first). Files and Spindle only.
pub const OP_READ: u64 = 0;
/// Returns how many records the kernel holds now in rsi, and the boot id in rdx.
pub const OP_STATUS: u64 = 1;
/// rdx: a record from an earlier start, read from disk. Files only: the kernel keeps it
/// beside this start's own so every reader sees one list.
pub const OP_ADOPT: u64 = 2;
/// Test builds only: Spindle asks the kernel to stop (rsi 0) or to end a service (rsi:
/// its domain id) as if it had faulted.
pub const OP_PROBE_FAULT: u64 = 3;
/// How many times the kernel has started the calling service again (rsi). A service
/// that was started again skips the once-only steps of its first start.
pub const OP_RESTARTS: u64 = 4;

/// The kernel's message to Files (caller 0) when it holds a record Files has not seen.
pub const OP_NOTICE: u64 = 0x0C5A_0001;

pub const OK: u64 = 0;
pub const DENIED: u64 = 1;
pub const NOT_FOUND: u64 = 2;
pub const INVALID: u64 = 3;
/// `OP_ADOPT`: the kernel keeps room for this start's records and takes no more history.
pub const FULL: u64 = 4;

pub const RECORD_BYTES: usize = 512;
pub const MAGIC: [u8; 8] = *b"SEXCRSH1";
/// How many records the kernel keeps for this start, adopted ones included.
pub const KERNEL_CAPACITY: usize = 64;

/// The disk ring: one record per sector in the fixed area before the program store,
/// in sectors no other owner uses (see `crate::DISKFS_*`, `SPINDLE_HISTORY_*`).
pub const DISK_BASE_LBA: u64 = 64;
pub const DISK_SECTORS: u64 = 64;
const _: () = {
    // Past the old SexFS v0 metadata (0..=47), before the first DiskFS proof sectors (128).
    assert!(DISK_BASE_LBA > crate::SEXFS_V0_META_END_LBA);
    assert!(DISK_BASE_LBA + DISK_SECTORS <= crate::SEXDRIVE_AP4_MULTI_BASE_LBA);
    assert!(DISK_BASE_LBA + DISK_SECTORS <= crate::program_store::DIRECTORY_LBA);
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    /// The kernel stopped (a panic or a fault in kernel code).
    KernelStop = 1,
    /// A BOOT service faulted.
    ServiceFault = 2,
    /// A program faulted.
    ProgramFault = 3,
    /// A service was started again after it ended.
    ServiceRestart = 4,
    /// A service kept ending and was left down.
    ServiceGaveUp = 5,
}
impl Kind {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v { 1 => Self::KernelStop, 2 => Self::ServiceFault, 3 => Self::ProgramFault,
            4 => Self::ServiceRestart, 5 => Self::ServiceGaveUp, _ => return None })
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::KernelStop => "kernel stopped",
            Self::ServiceFault => "service fault",
            Self::ProgramFault => "program fault",
            Self::ServiceRestart => "service restarted",
            Self::ServiceGaveUp => "service left down",
        }
    }
}

/// The BOOT services by domain id (1-based), as BOOT spawns them.
pub const SERVICE_NAMES: [&str; 15] = ["Display", "Drive", "Shell", "Input", "USB", "Bar", "Linen",
    "Store", "Quil", "Bell", "Files", "Spindle", "Network", "Pager", "Browser"];
pub fn domain_name(pd: u32) -> &'static str {
    match pd { 1..=15 => SERVICE_NAMES[pd as usize - 1], _ => "program" }
}

/// Field offsets in a record.
mod at {
    pub const MAGIC: usize = 0;
    pub const BOOT: usize = 8;
    pub const SEQ: usize = 16;
    pub const KIND: usize = 20;
    pub const FLAGS: usize = 21;
    pub const PD: usize = 24;
    pub const VECTOR: usize = 28;
    pub const RIP: usize = 32;
    pub const ADDR: usize = 40;
    pub const ERROR: usize = 48;
    pub const UPTIME_MS: usize = 56;
    pub const STORE_INDEX: usize = 64;
    pub const NAME: usize = 72;
    pub const MESSAGE_LEN: usize = 88;
    pub const MESSAGE: usize = 90;
    pub const CHECKSUM: usize = 504;
}
pub const NAME_BYTES: usize = 16;
pub const MESSAGE_BYTES: usize = at::CHECKSUM - at::MESSAGE;
/// Flag: the record came from an earlier start (adopted from disk or kept in memory
/// across a restart).
pub const FLAG_EARLIER: u8 = 1;

#[derive(Clone, Copy)]
pub struct Record {
    pub boot: u64,
    pub seq: u32,
    pub kind: Kind,
    pub flags: u8,
    pub pd: u32,
    pub vector: u32,
    pub rip: u64,
    pub addr: u64,
    pub error: u64,
    pub uptime_ms: u64,
    /// Files numbers records as it writes them, so the ring's newest is plain to see.
    pub store_index: u64,
    pub name: [u8; NAME_BYTES],
    pub message_len: u16,
    pub message: [u8; MESSAGE_BYTES],
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes { h ^= b as u64; h = h.wrapping_mul(0x0000_0100_0000_01b3); }
    h
}
fn put(out: &mut [u8], at: usize, v: &[u8]) { out[at..at + v.len()].copy_from_slice(v); }
fn u32_at(b: &[u8], at: usize) -> u32 { u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) }
fn u64_at(b: &[u8], at: usize) -> u64 { u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) }

impl Record {
    pub const fn empty(kind: Kind) -> Self {
        Self { boot: 0, seq: 0, kind, flags: 0, pd: 0, vector: 0, rip: 0, addr: 0, error: 0, uptime_ms: 0,
            store_index: 0, name: [0; NAME_BYTES], message_len: 0, message: [0; MESSAGE_BYTES] }
    }
    pub fn set_name(&mut self, name: &str) {
        let n = name.len().min(NAME_BYTES);
        self.name = [0; NAME_BYTES];
        self.name[..n].copy_from_slice(&name.as_bytes()[..n]);
    }
    pub fn name(&self) -> &[u8] {
        let n = self.name.iter().position(|&b| b == 0).unwrap_or(NAME_BYTES);
        &self.name[..n]
    }
    pub fn message(&self) -> &[u8] { &self.message[..(self.message_len as usize).min(MESSAGE_BYTES)] }
    /// Appends what fits; printable ASCII and newlines only, so any reader can show it.
    pub fn push_message(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.message_len as usize >= MESSAGE_BYTES { return; }
            let b = if b == b'\n' || (0x20..0x7f).contains(&b) { b } else { b'?' };
            self.message[self.message_len as usize] = b;
            self.message_len += 1;
        }
    }
    pub fn encode(&self, out: &mut [u8; RECORD_BYTES]) {
        *out = [0; RECORD_BYTES];
        put(out, at::MAGIC, &MAGIC);
        put(out, at::BOOT, &self.boot.to_le_bytes());
        put(out, at::SEQ, &self.seq.to_le_bytes());
        out[at::KIND] = self.kind as u8;
        out[at::FLAGS] = self.flags;
        put(out, at::PD, &self.pd.to_le_bytes());
        put(out, at::VECTOR, &self.vector.to_le_bytes());
        put(out, at::RIP, &self.rip.to_le_bytes());
        put(out, at::ADDR, &self.addr.to_le_bytes());
        put(out, at::ERROR, &self.error.to_le_bytes());
        put(out, at::UPTIME_MS, &self.uptime_ms.to_le_bytes());
        put(out, at::STORE_INDEX, &self.store_index.to_le_bytes());
        put(out, at::NAME, &self.name);
        let len = (self.message_len as usize).min(MESSAGE_BYTES);
        put(out, at::MESSAGE_LEN, &(len as u16).to_le_bytes());
        put(out, at::MESSAGE, &self.message[..len]);
        let sum = fnv(&out[..at::CHECKSUM]);
        put(out, at::CHECKSUM, &sum.to_le_bytes());
    }
    /// A record whose magic, kind, length and checksum all hold; anything else is not one.
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < RECORD_BYTES || b[..8] != MAGIC { return None; }
        if u64_at(b, at::CHECKSUM) != fnv(&b[..at::CHECKSUM]) { return None; }
        let kind = Kind::from_u8(b[at::KIND])?;
        let len = u16::from_le_bytes([b[at::MESSAGE_LEN], b[at::MESSAGE_LEN + 1]]) as usize;
        if len > MESSAGE_BYTES { return None; }
        let mut r = Self::empty(kind);
        r.boot = u64_at(b, at::BOOT);
        r.seq = u32_at(b, at::SEQ);
        r.flags = b[at::FLAGS];
        r.pd = u32_at(b, at::PD);
        r.vector = u32_at(b, at::VECTOR);
        r.rip = u64_at(b, at::RIP);
        r.addr = u64_at(b, at::ADDR);
        r.error = u64_at(b, at::ERROR);
        r.uptime_ms = u64_at(b, at::UPTIME_MS);
        r.store_index = u64_at(b, at::STORE_INDEX);
        r.name.copy_from_slice(&b[at::NAME..at::NAME + NAME_BYTES]);
        r.message_len = len as u16;
        r.message[..len].copy_from_slice(&b[at::MESSAGE..at::MESSAGE + len]);
        Some(r)
    }
    /// The same event, wherever it was copied: one start's own sequence number.
    pub fn same_event(&self, other: &Self) -> bool { self.boot == other.boot && self.seq == other.seq }
}

/// `SYS_CRASH_LOG`: returns (status, rsi, rdx).
#[cfg(target_arch = "x86_64")]
pub fn call(op: u64, arg: u64, buf: u64) -> (u64, u64, u64) {
    let (status, rsi, rdx): (u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYSCALL => status, inlateout("rdi") op => _,
            inlateout("rsi") arg => rsi, inlateout("rdx") buf => rdx,
            out("rcx") _, out("r11") _, out("r10") _, out("r8") _, options(nostack));
    }
    (status, rsi, rdx)
}
/// How many times this service has been started again; 0 on its first run.
#[cfg(target_arch = "x86_64")]
pub fn restarts() -> u64 {
    match call(OP_RESTARTS, 0, 0) { (OK, n, _) => n, _ => 0 }
}
/// How many records the kernel holds, and this start's boot id.
#[cfg(target_arch = "x86_64")]
pub fn status() -> Option<(usize, u64)> {
    match call(OP_STATUS, 0, 0) { (OK, n, boot) => Some((n as usize, boot)), _ => None }
}
/// The kernel's record at `index`, oldest first.
#[cfg(target_arch = "x86_64")]
pub fn read(index: usize) -> Option<Record> {
    let mut buf = [0u8; RECORD_BYTES];
    match call(OP_READ, index as u64, buf.as_mut_ptr() as u64) { (OK, _, _) => Record::decode(&buf), _ => None }
}

/// Where Files writes the next record: after the ring's newest, or the first sector of an
/// empty ring. `slots` holds each sector's `store_index`, `None` for no record.
pub fn next_disk_slot(slots: &[Option<u64>]) -> (usize, u64) {
    let newest = slots.iter().enumerate().filter_map(|(i, s)| s.map(|v| (v, i))).max();
    match newest {
        Some((index, slot)) => ((slot + 1) % slots.len().max(1), index + 1),
        None => (0, 1),
    }
}
