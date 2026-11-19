// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! PKU gates (syscall 111, `SYS_GATE`): calls between trusted services with no kernel
//! entry (docs/handoff/PKU_GATES_STUDY_2026_11_18.md).
//!
//! A service E makes a Gate for one of its own functions ([`OP_CREATE`]) and grants the
//! Gate capability like any IPC v2 object (`endpoint::OP_GRANT`). A service C holding it
//! binds it ([`OP_BIND`]) and gets back a stub address. E may also export the Gate under
//! a small number; then any service holding a route to E (a Domain capability, the same
//! authority that lets it send E requests) binds it with [`OP_BIND_ROUTE`]. Either way: calling the stub as
//! `extern "C" fn(u64, u64, u64, u64) -> u64` runs E's function under E's keys on a
//! stack of E's, then returns under C's keys. The kernel is not entered on a call.
//! Each task binds for itself (the bind also sets up that task's gate area); binding
//! again returns the same stub. A call the matrix denies returns [`DENIED`].
//! [`OP_DESTROY`] by E ends the Gate: every bound stub denies from then on.
//!
//! Registers: rax = 111, rdi = op | slot << 8, rsi = argument. Reply: rax = status,
//! rsi = value. Errors are the `endpoint` codes, plus [`UNAVAILABLE`].

pub const SYS_GATE: u64 = 111;
/// rsi = E's entry (code of the caller's own domain), rdx = export number (0: none,
/// else 1..=255, unique in E). rsi out: the Gate's slot.
pub const OP_CREATE: u64 = 1;
/// slot = a Gate capability with CALL. rsi out: the stub to call.
pub const OP_BIND: u64 = 2;
/// slot = the owner's Gate capability. Every stub bound to it denies from then on.
pub const OP_DESTROY: u64 = 3;
/// slot = a route to E (Domain capability), rsi = export number. rsi out: the stub.
pub const OP_BIND_ROUTE: u64 = 4;
/// No gates on this machine: no PKU, no RDPID, or no free frame key yet.
pub const UNAVAILABLE: u64 = 0xFFFF_FFFF_FFFF_FFE0;
/// What a stub returns when its Gate was destroyed.
pub const DENIED: u64 = u64::MAX;

/// Export numbers in use. Each service picks from its own space.
pub const EXPORT_PING: u64 = 1;

#[inline]
fn gate(op: u64, slot: u64, a: u64, b: u64) -> Result<u64, u64> {
    let (status, value): (u64, u64);
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") SYS_GATE => status, in("rdi") op | slot << 8,
            inlateout("rsi") a => value, in("rdx") b, lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    if status == 0 { Ok(value) } else { Err(status) }
}

/// Makes a Gate into `entry`, exported as `export` (0: not exported); returns its slot.
pub fn create(entry: extern "C" fn(u64, u64, u64, u64) -> u64, export: u64) -> Result<u64, u64> {
    gate(OP_CREATE, 0, entry as usize as u64, export)
}

/// Binds the Gate in `slot`; returns the stub.
pub fn bind(slot: u64) -> Result<extern "C" fn(u64, u64, u64, u64) -> u64, u64> {
    gate(OP_BIND, slot, 0, 0).map(|s| unsafe { core::mem::transmute::<usize, extern "C" fn(u64, u64, u64, u64) -> u64>(s as usize) })
}

/// Binds the Gate that the service behind route `slot` exported as `export`.
pub fn bind_route(slot: u64, export: u64) -> Result<extern "C" fn(u64, u64, u64, u64) -> u64, u64> {
    gate(OP_BIND_ROUTE, slot, export, 0).map(|s| unsafe { core::mem::transmute::<usize, extern "C" fn(u64, u64, u64, u64) -> u64>(s as usize) })
}

/// Ends the Gate in `slot` (its owner only).
pub fn destroy(slot: u64) -> Result<u64, u64> { gate(OP_DESTROY, slot, 0, 0) }
