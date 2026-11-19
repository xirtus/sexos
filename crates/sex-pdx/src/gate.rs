// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! PKU gates (syscall 111, `SYS_GATE`): calls between trusted services with no kernel
//! entry (docs/handoff/PKU_GATES_STUDY_2026_11_18.md).
//!
//! A service E makes a Gate for one of its own functions ([`OP_CREATE`]) and grants the
//! Gate capability like any IPC v2 object (`endpoint::OP_GRANT`). A service C holding it
//! binds it ([`OP_BIND`]) and gets back a stub address: calling the stub as
//! `extern "C" fn(u64, u64, u64, u64) -> u64` runs E's function under E's keys on a
//! stack of E's, then returns under C's keys. The kernel is not entered on a call.
//! Each task binds for itself (the bind also sets up that task's gate area); binding
//! again returns the same stub. A call the matrix denies returns [`DENIED`].
//! [`OP_DESTROY`] by E ends the Gate: every bound stub denies from then on.
//!
//! Registers: rax = 111, rdi = op | slot << 8, rsi = argument. Reply: rax = status,
//! rsi = value. Errors are the `endpoint` codes, plus [`UNAVAILABLE`].

pub const SYS_GATE: u64 = 111;
/// rsi = E's entry (code of the caller's own domain). rsi out: the Gate's slot.
pub const OP_CREATE: u64 = 1;
/// slot = a Gate capability with CALL. rsi out: the stub to call.
pub const OP_BIND: u64 = 2;
/// slot = the owner's Gate capability. Every stub bound to it denies from then on.
pub const OP_DESTROY: u64 = 3;
/// No gates on this machine: no PKU, no RDPID, or no free frame key yet.
pub const UNAVAILABLE: u64 = 0xFFFF_FFFF_FFFF_FFE0;
/// What a stub returns when its Gate was destroyed.
pub const DENIED: u64 = u64::MAX;
