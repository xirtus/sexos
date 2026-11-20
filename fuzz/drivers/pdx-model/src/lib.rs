// SPDX-License-Identifier: GPL-3.0-only
//! `sex_pdx` under device-model fuzzing: the real crate's types and pure code,
//! with every kernel call a driver makes while it runs replaced. Time only moves
//! when the driver looks at it or waits, so every tick-bounded wait ends; each
//! look also counts toward the harness's hang budget (through `set_hook`).
//! Services a driver talks to (Input, Files, the network stack) are absent.
use std::cell::Cell;

pub use real::*;

thread_local! {
    static TICKS: Cell<u64> = const { Cell::new(0) };
    static HOOK: Cell<Option<fn()>> = const { Cell::new(None) };
}

/// The harness's per-read hook (it counts toward the hang budget).
pub fn set_hook(f: fn()) { HOOK.with(|h| h.set(Some(f))); }
pub fn reset_clock() { TICKS.with(|t| t.set(0)); }
pub fn now() -> u64 { TICKS.with(|t| t.get()) }
fn step() { if let Some(f) = HOOK.with(|h| h.get()) { f() } }

/// Milliseconds since boot: one per look, so a wait of N ticks takes N looks.
pub fn get_ticks() -> u64 { step(); TICKS.with(|t| { let v = t.get() + 1; t.set(v); v }) }
pub fn sys_yield() { step(); TICKS.with(|t| t.set(t.get() + 1)); }
pub fn sys_wait_self_ticks(ticks: u64) -> Result<(), u64> { step(); TICKS.with(|t| t.set(t.get() + ticks.max(1))); Ok(()) }

pub fn pdx_call(_slot: u64, _opcode: u64, _a0: u64, _a1: u64, _a2: u64) -> (u64, u64) { (ERR_SERVICE_NOT_READY, 0) }
pub fn pdx_reply(_target_pd: u32, _value: u64) -> u64 { 0 }
pub fn pdx_try_listen_raw(_slot: u64) -> Option<PdxMessage> { None }
pub fn memory_region_info(_slot: u64) -> Result<memory_region::MemoryRegionInfo, u64> { Err(ERR_CAP_INVALID) }
pub fn memory_region_full_view(_slot: u64, _rights: u8) -> Result<memory_region::RegionView, u64> { Err(ERR_CAP_INVALID) }

pub mod device {
    pub use real::device::*;
    pub fn irq_take() -> u64 { 0 }
    pub fn irq_unmask(_slot: u64, _vector: u64) -> Result<(), u64> { Err(real::ERR_CAP_INVALID) }
    pub fn at_slot(_slot: u64) -> Option<Lease> { None }
}

#[doc(hidden)]
pub fn write_serial_record(args: core::fmt::Arguments<'_>) {
    // Format it anyway: a log line must not panic on what a device reported.
    use core::fmt::Write;
    struct Sink;
    impl Write for Sink { fn write_str(&mut self, _: &str) -> core::fmt::Result { Ok(()) } }
    let _ = Sink.write_fmt(args);
    if std::env::var_os("DRIVER_FUZZ_LOG").is_some() { eprintln!("{args}"); }
}

#[macro_export]
macro_rules! serial_record {
    ($($arg:tt)*) => { $crate::write_serial_record(core::format_args!($($arg)*)) };
}
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => { $crate::write_serial_record(core::format_args!($($arg)*)) };
}
