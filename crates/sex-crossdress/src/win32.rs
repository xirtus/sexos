// SPDX-License-Identifier: GPL-3.0-only
//! The Windows side the proof needs: TEB/PEB/CONTEXT/EXCEPTION_RECORD layouts, the
//! mapping from CPU exceptions to Windows status codes, and a small KERNEL32 subset run
//! in the host. Layouts are the documented x64 ones (winnt.h). This subset is
//! scaffolding: from CW3 on, KERNEL32 and NTDLL are Wine's own PE DLLs running in the
//! cell, and the host side shrinks to Wine's Unix calls.
use crate::cell::{Access, Frame, PAGE};
use crate::process::{self, Host, WinProcess, Stop};
use sex_pdx::vm::Regs;

// TEB (x64).
pub const TEB_EXCEPTION_LIST: u64 = 0x00;
pub const TEB_STACK_BASE: u64 = 0x08;
pub const TEB_STACK_LIMIT: u64 = 0x10;
pub const TEB_SELF: u64 = 0x30;
pub const TEB_PROCESS_ID: u64 = 0x40;
pub const TEB_THREAD_ID: u64 = 0x48;
pub const TEB_PEB: u64 = 0x60;
pub const TEB_LAST_ERROR: u64 = 0x68;
// PEB (x64).
pub const PEB_IMAGE_BASE: u64 = 0x10;

// CONTEXT (x64): 0x4D0 bytes.
pub const CONTEXT_SIZE: u64 = 0x4D0;
const CONTEXT_AMD64: u32 = 0x10_0000;
const CONTEXT_CONTROL: u32 = 1;
const CONTEXT_INTEGER: u32 = 2;
const CONTEXT_SEGMENTS: u32 = 4;
// EXCEPTION_RECORD (x64): 0x98 bytes.
pub const RECORD_SIZE: u64 = 0x98;

pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_BREAKPOINT: u32 = 0x8000_0003;
pub const STATUS_SINGLE_STEP: u32 = 0x8000_0004;
pub const STATUS_INTEGER_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
pub const STATUS_INTEGER_OVERFLOW: u32 = 0xC000_0095;
pub const STATUS_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const STATUS_PRIVILEGED_INSTRUCTION: u32 = 0xC000_0096;
pub const STATUS_FLOAT_INVALID_OPERATION: u32 = 0xC000_0090;
pub const STATUS_DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;
pub const STATUS_STACK_OVERFLOW: u32 = 0xC000_00FD;

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_ADDRESS: u32 = 487;

/// Console handles GetStdHandle gives out.
const STDIN: u64 = 0x10;
const STDOUT: u64 = 0x14;
const STDERR: u64 = 0x18;

/// The KERNEL32 functions this proof implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    GetStdHandle, WriteFile, ExitProcess, AddVectoredExceptionHandler, RemoveVectoredExceptionHandler,
    VirtualAlloc, GetCurrentThreadId, GetCurrentProcessId, GetTickCount64, GetLastError, SetLastError,
    Sleep, QueryPerformanceCounter, QueryPerformanceFrequency,
}

pub fn lookup(dll: &str, name: &str) -> Option<Api> {
    if !(dll.eq_ignore_ascii_case("kernel32.dll") || dll.eq_ignore_ascii_case("kernelbase.dll")) { return None; }
    Some(match name {
        "GetStdHandle" => Api::GetStdHandle,
        "WriteFile" => Api::WriteFile,
        "ExitProcess" => Api::ExitProcess,
        "AddVectoredExceptionHandler" => Api::AddVectoredExceptionHandler,
        "RemoveVectoredExceptionHandler" => Api::RemoveVectoredExceptionHandler,
        "VirtualAlloc" => Api::VirtualAlloc,
        "GetCurrentThreadId" => Api::GetCurrentThreadId,
        "GetCurrentProcessId" => Api::GetCurrentProcessId,
        "GetTickCount64" => Api::GetTickCount64,
        "GetLastError" => Api::GetLastError,
        "SetLastError" => Api::SetLastError,
        "Sleep" => Api::Sleep,
        "QueryPerformanceCounter" => Api::QueryPerformanceCounter,
        "QueryPerformanceFrequency" => Api::QueryPerformanceFrequency,
        _ => return None,
    })
}

pub enum Outcome { Return(u64), Stop(Stop) }

fn set_last_error(p: &mut WinProcess, code: u32) { let _ = p.cell.write_u32(process::TEB_VA + TEB_LAST_ERROR, code); }

/// Runs import `index` with the Windows x64 arguments in `regs`.
pub fn call(p: &mut WinProcess, index: usize, regs: &mut Regs, host: &mut impl Host) -> Outcome {
    let (dll, name, api) = p.imports[index].clone();
    let Some(api) = api else { return Outcome::Stop(Stop::Unimplemented { dll, name }) };
    // The guest's Windows x64 arguments (a guest register file, not SexOS IPC).
    let (a1, a2, a3, a4) = { let g = &*regs; (g.rcx, g.rdx, g.r8, g.r9) };
    Outcome::Return(match api {
        Api::GetStdHandle => match a1 as u32 as i32 { -10 => STDIN, -11 => STDOUT, -12 => STDERR, _ => { set_last_error(p, ERROR_INVALID_HANDLE); u64::MAX } },
        Api::WriteFile => {
            let stream = match a1 { STDOUT => 1, STDERR => 2, _ => { set_last_error(p, ERROR_INVALID_HANDLE); return Outcome::Return(0) } };
            let n = (a3 as u32).min(64 << 10) as usize;
            let mut buf = alloc::vec![0u8; n];
            if p.cell.read_user(a2, &mut buf).is_none() { set_last_error(p, ERROR_INVALID_PARAMETER); return Outcome::Return(0); }
            host.output(stream, &buf);
            if a4 != 0 && p.cell.write_user(a4, &(n as u32).to_le_bytes()).is_none() { set_last_error(p, ERROR_INVALID_PARAMETER); return Outcome::Return(0); }
            1
        }
        Api::ExitProcess => return Outcome::Stop(Stop::Exited(a1 as u32)),
        Api::AddVectoredExceptionHandler => {
            if a2 == 0 || p.handlers.len() >= 64 { return Outcome::Return(0); }
            if a1 != 0 { p.handlers.insert(0, a2) } else { p.handlers.push(a2) }
            a2
        }
        Api::RemoveVectoredExceptionHandler => match p.handlers.iter().position(|&h| h == a1) {
            Some(i) => { p.handlers.remove(i); 1 }
            None => 0,
        },
        Api::VirtualAlloc => virtual_alloc(p, a1, a2, a3 as u32, a4 as u32),
        Api::GetCurrentThreadId => process::THREAD_ID as u64,
        Api::GetCurrentProcessId => process::PROCESS_ID as u64,
        Api::GetTickCount64 => host.now_ms(),
        Api::GetLastError => { let mut b = [0u8; 4]; let _ = p.cell.read(process::TEB_VA + TEB_LAST_ERROR, &mut b); u32::from_le_bytes(b) as u64 }
        Api::SetLastError => { set_last_error(p, a1 as u32); 0 }
        Api::Sleep => { host.sleep(a1 as u32); 0 }
        Api::QueryPerformanceCounter => {
            let t = host.now_ms().saturating_mul(1000);
            if p.cell.write_user(a1, &t.to_le_bytes()).is_some() { 1 } else { 0 }
        }
        Api::QueryPerformanceFrequency => if p.cell.write_user(a1, &1_000_000u64.to_le_bytes()).is_some() { 1 } else { 0 },
    })
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;

/// VirtualAlloc: reserve and/or commit, at an address or anywhere above ALLOC_BASE.
fn virtual_alloc(p: &mut WinProcess, address: u64, size: u64, kind: u32, protect: u32) -> u64 {
    let fail = |p: &mut WinProcess, e| { set_last_error(p, e); 0 };
    if size == 0 || size > 1 << 40 || kind & !(MEM_COMMIT | MEM_RESERVE) != 0 || kind == 0 { return fail(p, ERROR_INVALID_PARAMETER); }
    let access = match protect {
        0x01 => None,                           // PAGE_NOACCESS
        0x02 => Some(Access::USER_RO),          // PAGE_READONLY
        0x04 => Some(Access::USER_RW),          // PAGE_READWRITE
        0x10 | 0x20 => Some(Access::USER_RX),   // PAGE_EXECUTE, PAGE_EXECUTE_READ
        0x40 => Some(Access::USER_RWX),         // PAGE_EXECUTE_READWRITE
        _ => return fail(p, ERROR_INVALID_PARAMETER),
    };
    let reserve = kind & MEM_RESERVE != 0;
    let (start, bytes) = if address == 0 {
        if !reserve { return fail(p, ERROR_INVALID_ADDRESS); }
        let bytes = size.next_multiple_of(process::ALLOCATION_GRANULARITY);
        let mut at = p.next_alloc;
        while p.cell.is_taken(at, bytes) { at += process::ALLOCATION_GRANULARITY; }
        p.next_alloc = at + bytes;
        (at, bytes)
    } else if reserve {
        let start = address & !(process::ALLOCATION_GRANULARITY - 1);
        (start, (address + size).next_multiple_of(PAGE) - start)
    } else {
        // Commit inside an earlier reservation.
        let start = address & !(PAGE - 1);
        let bytes = (address + size).next_multiple_of(PAGE) - start;
        if !p.cell.in_reservation(start, bytes) { return fail(p, ERROR_INVALID_ADDRESS); }
        (start, bytes)
    };
    if reserve && p.cell.reserve(start, bytes).is_err() { return fail(p, ERROR_INVALID_ADDRESS); }
    if kind & MEM_COMMIT != 0 {
        if let Some(access) = access {
            if p.cell.map(start, bytes / PAGE, access).is_err() { return fail(p, ERROR_NOT_ENOUGH_MEMORY); }
        }
    }
    start
}

/// The Windows exception for CPU `vector`: (status, ExceptionInformation).
pub fn exception_code(vector: u8, f: &Frame) -> (u32, alloc::vec::Vec<u64>) {
    match vector {
        0 => (STATUS_INTEGER_DIVIDE_BY_ZERO, alloc::vec![]),
        1 => (STATUS_SINGLE_STEP, alloc::vec![]),
        3 => (STATUS_BREAKPOINT, alloc::vec![0]),
        4 => (STATUS_INTEGER_OVERFLOW, alloc::vec![]),
        6 => (STATUS_ILLEGAL_INSTRUCTION, alloc::vec![]),
        14 => {
            // Error code: bit 1 write, bit 4 instruction fetch.
            let kind = if f.error & 0x10 != 0 { 8 } else if f.error & 2 != 0 { 1 } else { 0 };
            (STATUS_ACCESS_VIOLATION, alloc::vec![kind, f.cr2])
        }
        13 => (STATUS_ACCESS_VIOLATION, alloc::vec![0, u64::MAX]),
        16 | 19 => (STATUS_FLOAT_INVALID_OPERATION, alloc::vec![]),
        17 => (STATUS_DATATYPE_MISALIGNMENT, alloc::vec![]),
        _ => (STATUS_ILLEGAL_INSTRUCTION, alloc::vec![]),
    }
}

pub fn write_record(r: &mut [u8], code: u32, address: u64, params: &[u64]) {
    r[0..4].copy_from_slice(&code.to_le_bytes());
    r[0x10..0x18].copy_from_slice(&address.to_le_bytes());
    let n = params.len().min(15);
    r[0x18..0x1C].copy_from_slice(&(n as u32).to_le_bytes());
    for (i, v) in params.iter().take(n).enumerate() { r[0x20 + 8 * i..0x28 + 8 * i].copy_from_slice(&v.to_le_bytes()); }
}

/// CONTEXT offsets of RAX..R15 in Windows order, mapped to our register file.
const GPRS: [(usize, fn(&mut Regs) -> &mut u64); 16] = [
    (0x78, |r| &mut r.rax), (0x80, |r| &mut r.rcx), (0x88, |r| &mut r.rdx), (0x90, |r| &mut r.rbx),
    (0x98, |r| &mut r.rsp), (0xA0, |r| &mut r.rbp), (0xA8, |r| &mut r.rsi), (0xB0, |r| &mut r.rdi),
    (0xB8, |r| &mut r.r8), (0xC0, |r| &mut r.r9), (0xC8, |r| &mut r.r10), (0xD0, |r| &mut r.r11),
    (0xD8, |r| &mut r.r12), (0xE0, |r| &mut r.r13), (0xE8, |r| &mut r.r14), (0xF0, |r| &mut r.r15),
];

pub fn write_context(c: &mut [u8], regs: &Regs) {
    let mut r = *regs;
    c[0x30..0x34].copy_from_slice(&(CONTEXT_AMD64 | CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_SEGMENTS).to_le_bytes());
    c[0x34..0x38].copy_from_slice(&0x1F80u32.to_le_bytes());
    for (at, sel) in [(0x38, crate::cell::USER_CS), (0x3A, crate::cell::USER_DS), (0x3C, crate::cell::USER_DS),
        (0x3E, crate::cell::USER_DS), (0x40, crate::cell::USER_DS), (0x42, crate::cell::USER_DS)] {
        c[at..at + 2].copy_from_slice(&sel.to_le_bytes());
    }
    c[0x44..0x48].copy_from_slice(&(regs.rflags as u32).to_le_bytes());
    for (at, f) in GPRS { c[at..at + 8].copy_from_slice(&f(&mut r).to_le_bytes()); }
    c[0xF8..0x100].copy_from_slice(&regs.rip.to_le_bytes());
}

/// Loads the thread's registers from a CONTEXT a handler may have changed. Only the
/// arithmetic, direction, trap and interrupt flags are taken from it.
pub fn read_context(c: &[u8], regs: &mut Regs) {
    let q = |at: usize| { let mut b = [0u8; 8]; b.copy_from_slice(&c[at..at + 8]); u64::from_le_bytes(b) };
    for (at, f) in GPRS { *f(regs) = q(at); }
    regs.rip = q(0xF8);
    let flags = u32::from_le_bytes([c[0x44], c[0x45], c[0x46], c[0x47]]) as u64;
    regs.rflags = (flags & 0x0DD5) | crate::cell::USER_RFLAGS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_round_trips_and_flags_are_tamed() {
        let regs = Regs { rax: 1, rbx: 2, rcx: 3, rdx: 4, rsi: 5, rdi: 6, rbp: 7, r8: 8, r9: 9, r10: 10, r11: 11, r12: 12, r13: 13, r14: 14, r15: 15, rsp: 16, rip: 17, rflags: 0x3246 };
        let mut c = [0u8; CONTEXT_SIZE as usize];
        write_context(&mut c, &regs);
        assert_eq!(&c[0xF8..0x100], &17u64.to_le_bytes(), "Rip");
        assert_eq!(&c[0x98..0xA0], &16u64.to_le_bytes(), "Rsp");
        let mut back = Regs::default();
        read_context(&c, &mut back);
        assert_eq!(back, regs);
        c[0x44..0x48].copy_from_slice(&0x0003_3000u32.to_le_bytes()); // IOPL 3, VM, RF
        read_context(&c, &mut back);
        assert_eq!(back.rflags, crate::cell::USER_RFLAGS, "a handler cannot enter v8086 mode or change IOPL");
    }

    #[test]
    fn page_faults_say_read_write_or_execute() {
        let f = |error, cr2| Frame { error, cr2, ..Frame::default() };
        assert_eq!(exception_code(14, &f(4, 0)), (STATUS_ACCESS_VIOLATION, alloc::vec![0, 0]));
        assert_eq!(exception_code(14, &f(7, 0x1234)), (STATUS_ACCESS_VIOLATION, alloc::vec![1, 0x1234]));
        assert_eq!(exception_code(14, &f(0x15, 0x99)), (STATUS_ACCESS_VIOLATION, alloc::vec![8, 0x99]));
        assert_eq!(exception_code(0, &f(0, 0)).0, STATUS_INTEGER_DIVIDE_BY_ZERO);
        assert_eq!(exception_code(3, &f(0, 0)).0, STATUS_BREAKPOINT);
    }

    #[test]
    fn only_kernel32_names_are_known() {
        assert_eq!(lookup("KERNEL32.dll", "WriteFile"), Some(Api::WriteFile));
        assert_eq!(lookup("kernelbase.dll", "Sleep"), Some(Api::Sleep));
        assert_eq!(lookup("USER32.dll", "MessageBoxA"), None);
        assert_eq!(lookup("KERNEL32.dll", "CreateFileW"), None);
    }
}
