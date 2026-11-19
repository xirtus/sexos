// SPDX-License-Identifier: GPL-3.0-only
//! CW3: Wine's own PE DLLs in a cell, starting with ntdll.dll.
//!
//! Wine splits each DLL into a PE half (Windows code, run here unmodified in the cell)
//! and a Unix half (ntdll.so and friends). On SexOS the Unix half is this host side: the
//! PE half reaches it through two seams Wine already has.
//!
//! - **NT system calls.** Every `Nt*` stub in Wine's ntdll.dll is
//!   `mov r10, rcx; mov eax, N; test byte [0x7FFE0308], 1; jne +3; syscall; ret; ...;
//!   call [0x7FFE1000]`. The byte and the pointer live in KUSER_SHARED_DATA, which this
//!   module maps: the byte is set, so the stub calls the pointer, which is the cell's
//!   syscall dispatcher thunk (`process::SYSCALL_DISPATCHER_VA`). The call numbers are
//!   read from the stubs themselves (`WinProcess::syscall_names`), so the host matches
//!   whatever Wine build is loaded by name, not by number.
//! - **Unix calls.** `__wine_unix_call(handle, code, args)` jumps through ntdll's
//!   `__wine_unix_call_dispatcher` pointer, which this module points at the cell's Unix
//!   call thunk.
//!
//! The system calls implemented so far are the ones Wine's own heap and time functions
//! need. Any other stops the process cleanly as `ntdll.syscall!NtName`, so the next one
//! to write is always named. Threads, files and sections (CW4) come next.
use crate::cell::{Access, PAGE};
use crate::process::{self, Host, Stop, WinProcess};
use crate::win32::{self, Outcome};
use alloc::string::String;
use sex_pdx::vm::Regs;

/// KUSER_SHARED_DATA, read-only for user code, at its fixed Windows address.
pub const KUSER_VA: u64 = 0x7FFE_0000;
const KUSER_PAGES: u64 = 2;
const KUSER_SYSTEM_CALL: u64 = 0x308;
const KUSER_DISPATCHER: u64 = 0x1000;
const DEBUG_OPTIONS_PAGES: u64 = 2;
/// The FILETIME (100 ns since 1601) the cell's clock starts at: 2026-01-01.
pub const EPOCH_FILETIME: u64 = 134_116_992_000_000_000;

pub const STATUS_SUCCESS: u64 = 0;
pub const STATUS_NOT_IMPLEMENTED: u64 = 0xC000_0002;
pub const STATUS_INVALID_PARAMETER: u64 = 0xC000_000D;
pub const STATUS_NO_MEMORY: u64 = 0xC000_0017;
pub const STATUS_CONFLICTING_ADDRESSES: u64 = 0xC000_0018;
pub const STATUS_ACCESS_VIOLATION: u64 = 0xC000_0005;

/// The handle ntdll's own Unix library gets (`__wine_unixlib_handle`).
pub const NTDLL_UNIXLIB: u64 = 0x5E05_0001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WineError { NotNtdll, Cell }

/// Makes a process whose module is Wine's ntdll.dll ready to be called: maps
/// KUSER_SHARED_DATA with the syscall flag and dispatcher, and fills the data exports
/// the Unix half would (`__wine_unix_call_dispatcher`, `__wine_syscall_dispatcher`,
/// `__wine_unixlib_handle`).
pub fn prepare_ntdll(p: &mut WinProcess) -> Result<(), WineError> {
    let unix = p.export("__wine_unix_call_dispatcher").ok_or(WineError::NotNtdll)?;
    if p.syscall_names.is_empty() { return Err(WineError::NotNtdll); }
    p.cell.map(KUSER_VA, KUSER_PAGES, Access::USER_RO).map_err(|_| WineError::Cell)?;
    // Wine's Unix half keeps the debug channel options right after the PEB, at PEB+0x2000:
    // 16-byte entries {flags, name[15]} ending with an empty name. An empty list leaves
    // every channel off (no output through the Unix call that writes it).
    p.cell.map(process::PEB_VA + PAGE, DEBUG_OPTIONS_PAGES, Access::USER_RW).map_err(|_| WineError::Cell)?;
    let mut k = [0u8; 0x400];
    k[0x26C..0x270].copy_from_slice(&10u32.to_le_bytes()); // NtMajorVersion
    k[0x270..0x274].copy_from_slice(&0u32.to_le_bytes()); // NtMinorVersion
    k[0x260..0x264].copy_from_slice(&19045u32.to_le_bytes()); // NtBuildNumber
    k[0x26A..0x26C].copy_from_slice(&0x8664u16.to_le_bytes()); // NativeProcessorArchitecture... (ImageNumberHigh)
    k[0x268..0x26A].copy_from_slice(&0x8664u16.to_le_bytes()); // ImageNumberLow
    k[KUSER_SYSTEM_CALL as usize] = 1;
    p.cell.write(KUSER_VA, &k).ok_or(WineError::Cell)?;
    p.cell.write_u64(KUSER_VA + KUSER_DISPATCHER, process::SYSCALL_DISPATCHER_VA).ok_or(WineError::Cell)?;
    p.cell.write_u64(unix, process::UNIX_CALL_DISPATCHER_VA).ok_or(WineError::Cell)?;
    if let Some(sys) = p.export("__wine_syscall_dispatcher") { p.cell.write_u64(sys, process::SYSCALL_DISPATCHER_VA).ok_or(WineError::Cell)?; }
    if let Some(h) = p.export("__wine_unixlib_handle") { p.cell.write_u64(h, NTDLL_UNIXLIB).ok_or(WineError::Cell)?; }
    Ok(())
}

/// Reads the system call numbers out of a DLL's `Nt*` stubs (Wine's and Windows's
/// ntdll both start each with `mov r10, rcx; mov eax, N`).
pub fn syscall_numbers(exports: &[(String, u32)], mem: &[u8]) -> alloc::collections::BTreeMap<u32, String> {
    let mut out = alloc::collections::BTreeMap::new();
    for (name, rva) in exports {
        if !name.starts_with("Nt") { continue; }
        let at = *rva as usize;
        let Some(code) = mem.get(at..at + 8) else { continue };
        if code[..4] == [0x4C, 0x8B, 0xD1, 0xB8] {
            out.insert(u32::from_le_bytes([code[4], code[5], code[6], code[7]]), name.clone());
        }
    }
    out
}

/// The `n`th system call argument (0-based) at a syscall exit: R10, RDX, R8, R9, then the
/// caller's stack above the stub's return address, its caller's and the home space.
fn arg(p: &WinProcess, regs: &Regs, n: usize) -> Option<u64> {
    // The guest's Windows x64 argument registers (a guest register file, not SexOS IPC).
    let g = regs;
    match n {
        0 => Some(g.r10),
        1 => Some(g.rdx),
        2 => Some(g.r8),
        3 => Some(g.r9),
        _ => { let mut b = [0u8; 8]; p.cell.read_user(g.rsp + 0x30 + 8 * (n as u64 - 4), &mut b)?; Some(u64::from_le_bytes(b)) }
    }
}

fn get(p: &WinProcess, va: u64) -> Option<u64> { let mut b = [0u8; 8]; p.cell.read_user(va, &mut b)?; Some(u64::from_le_bytes(b)) }
fn put(p: &mut WinProcess, va: u64, v: u64) -> Option<()> { p.cell.write_user(va, &v.to_le_bytes()) }

/// Runs system call `number`.
pub fn syscall(p: &mut WinProcess, number: u32, regs: &Regs, host: &mut impl Host) -> Outcome {
    let name = match p.syscall_names.get(&number) { Some(n) => n.clone(), None => alloc::format!("#{number}") };
    let a = |i| arg(p, regs, i).unwrap_or(0);
    let (a0, a1, a3, a4, a5) = (a(0), a(1), a(3), a(4), a(5));
    let status = match name.as_str() {
        "NtAllocateVirtualMemory" => allocate(p, a0, a1, a3, a4 as u32, a5 as u32),
        // Pages are not given back to the cell yet (no unmap): freeing succeeds and the
        // range stays mapped. Wine's heap only frees what it allocated.
        "NtFreeVirtualMemory" => STATUS_SUCCESS,
        "NtQuerySystemTime" => match put(p, a0, EPOCH_FILETIME + host.now_ms() * 10_000) { Some(()) => STATUS_SUCCESS, None => STATUS_ACCESS_VIOLATION },
        "NtQueryPerformanceCounter" => {
            let ok = put(p, a0, host.now_ms() * 1000).is_some() && (a1 == 0 || put(p, a1, 1_000_000).is_some());
            if ok { STATUS_SUCCESS } else { STATUS_ACCESS_VIOLATION }
        }
        "NtClose" => STATUS_SUCCESS,
        "NtYieldExecution" => STATUS_SUCCESS,
        "NtTerminateProcess" if a0 == 0 || a0 == u64::MAX => return Outcome::Stop(Stop::Exited(a1 as u32)),
        _ => return Outcome::Stop(Stop::Unimplemented { dll: String::from("ntdll.syscall"), name }),
    };
    Outcome::Return(status)
}

/// NtAllocateVirtualMemory(process, *base, zero_bits, *size, type, protect).
fn allocate(p: &mut WinProcess, process: u64, base_ptr: u64, size_ptr: u64, kind: u32, protect: u32) -> u64 {
    if process != u64::MAX { return STATUS_INVALID_PARAMETER; }
    let (Some(base), Some(size)) = (get(p, base_ptr), get(p, size_ptr)) else { return STATUS_ACCESS_VIOLATION };
    if size == 0 { return STATUS_INVALID_PARAMETER; }
    // MEM_TOP_DOWN and friends are hints; only commit/reserve matter here.
    let start = win32::virtual_alloc(p, base, size, kind & 0x3000, protect);
    if start == 0 { return if base != 0 { STATUS_CONFLICTING_ADDRESSES } else { STATUS_NO_MEMORY }; }
    let end = if base != 0 { (base + size).next_multiple_of(PAGE) } else { start + size.next_multiple_of(PAGE) };
    if put(p, base_ptr, start).is_none() || put(p, size_ptr, end - start).is_none() { return STATUS_ACCESS_VIOLATION; }
    STATUS_SUCCESS
}

/// `__wine_unix_call(handle, code, args)`: none are served yet; the first one stops the
/// process with its handle and code so it can be written next.
pub fn unix_call(_p: &mut WinProcess, regs: &Regs, _host: &mut impl Host) -> Outcome {
    let g = regs;
    Outcome::Stop(Stop::Unimplemented { dll: alloc::format!("unixlib#{:#x}", g.rcx), name: alloc::format!("code {}", g.rdx as u32) })
}
