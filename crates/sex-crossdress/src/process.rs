// SPDX-License-Identifier: GPL-3.0-only
//! One Windows process (a guest program inside a VM, not a SexOS process) in a cell: the image at its preferred base, a PEB, one thread
//! (TEB in GS, its stack), one import thunk per imported function, and the exit loop.
//!
//! An import thunk is 8 bytes of user code, `mov eax, CALL|n; out 0xE0, eax; ret`. The
//! host sees the Windows x64 arguments (RCX, RDX, R8, R9, then the stack) in the exit's
//! registers, runs the function (`win32`), puts the result in RAX and resumes after the
//! `out`, so the `ret` returns to the caller. This is the seam where Wine's own PE DLLs
//! and their `__wine_unix_call` go later (CW3): the calls then come from ntdll.dll
//! instead of from the program's import table.
//!
//! Hardware exceptions arrive from the ring-0 stubs (`cell`). The host turns them into
//! a Windows EXCEPTION_RECORD and CONTEXT on the thread's stack and calls the vectored
//! exception handlers in user mode; a handler that returns EXCEPTION_CONTINUE_EXECUTION
//! resumes the thread from the (possibly edited) CONTEXT.
use crate::cell::{self, Access, Cell, Frame, PAGE};
use crate::pe::{self, Image};
use crate::win32::{self, Api};
use alloc::string::String;
use alloc::vec::Vec;
use drag_vmm::memory::Ram;
use drag_vmm::vmm::Vcpu;
use sex_pdx::vm::{exit, Exit, Regs, State};

/// Where the process's own structures live (fixed, like Windows's per-process layout).
pub const THUNK_VA: u64 = 0x0000_7FF7_FFF0_0000;
pub const PEB_VA: u64 = 0x0000_7FF7_FFFD_0000;
pub const TEB_VA: u64 = 0x0000_7FF7_FFFE_0000;
pub const TEB_PAGES: u64 = 2;
pub const STACK_TOP: u64 = 0x0000_7FF7_FF00_0000;
/// VirtualAlloc without an address starts here (64 KiB granularity, like Windows).
pub const ALLOC_BASE: u64 = 0x0000_0200_0000_0000;
pub const ALLOCATION_GRANULARITY: u64 = 0x1_0000;
pub const PROCESS_ID: u32 = 0x100;
pub const THREAD_ID: u32 = 0x104;

const THUNK_SIZE: u64 = 16;
/// Thunk slot 0 returns from a vectored exception handler, slot 1 from the entry point;
/// imports start at slot 2.
const FIRST_IMPORT: u64 = 2;

/// Why the process stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// ExitProcess, or the entry point returned.
    Exited(u32),
    /// A hardware exception no vectored handler took: (Windows status code, address).
    Unhandled { code: u32, address: u64 },
    /// The program called an import this subset does not have.
    Unimplemented { dll: String, name: String },
    /// A fault in the cell's own ring-0 code (a Crossdress bug, not the program's).
    CellFault { vector: u8, rip: u64 },
    TripleFault { rip: u64 },
    /// An exit Crossdress does not expect from a cell.
    Unexpected { reason: u32, rip: u64 },
    /// SYS_VM refused a call (status).
    Hypervisor(u64),
    /// The host asked to stop (window closed).
    Host,
}

/// What the process needs from the program around it.
pub trait Host {
    /// Bytes the program wrote to a console handle (1 = stdout, 2 = stderr).
    fn output(&mut self, stream: u32, bytes: &[u8]);
    fn now_ms(&mut self) -> u64;
    fn sleep(&mut self, ms: u32);
    /// Called on host interrupts and time slices; false stops the process.
    fn keep_going(&mut self) -> bool { true }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters { pub exits: u64, pub calls: u64, pub exceptions: u64, pub handled: u64 }

/// A hardware exception as the cell's stub reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault { pub vector: u8, pub error: u64, pub rip: u64, pub cr2: u64 }

/// An exception being handed to the vectored handlers.
struct Dispatch { context: u64, pointers: u64, next_handler: usize, code: u32, address: u64 }

pub struct WinProcess {
    pub cell: Cell,
    pub image: Image,
    pub base: u64,
    pub entry: u64,
    pub imports: Vec<(String, String, Option<Api>)>,
    pub handlers: Vec<u64>,
    pub next_alloc: u64,
    pub counters: Counters,
    /// The last hardware exception a stub reported, for diagnostics.
    pub last_fault: Option<Fault>,
    dispatch: Vec<Dispatch>,
    initial: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError { Pe(pe::Error), Cell(cell::Error), TooManyImports }
impl From<pe::Error> for LoadError { fn from(e: pe::Error) -> Self { LoadError::Pe(e) } }
impl From<cell::Error> for LoadError { fn from(e: cell::Error) -> Self { LoadError::Cell(e) } }

impl WinProcess {
    /// Loads a 64-bit Windows executable into a fresh cell over `ram`. The image goes at
    /// its preferred base: a cell is its own address space, so /FIXED images load too.
    pub fn load(ram: Ram, file: &[u8]) -> Result<WinProcess, LoadError> {
        let image = Image::parse(file)?;
        if image.is_dll() { return Err(LoadError::Pe(pe::Error::NotPe)); }
        let base = image.preferred_base;
        let mut mem = image.lay_out(file, base)?;
        let mut cell = Cell::new(ram)?;

        // Thunks: one per import, bound into the image's import address table.
        let list = image.imports(&mem)?;
        let slots = FIRST_IMPORT + list.len() as u64;
        if slots > 0xFFFF { return Err(LoadError::TooManyImports); }
        let mut thunks = Vec::with_capacity((slots * THUNK_SIZE) as usize);
        for n in 0..slots {
            let code: u32 = match n { 0 => cell::HANDLER_RETURN, 1 => cell::ENTRY_RETURN, _ => cell::CALL | n as u32 };
            let mut t = [0xCCu8; THUNK_SIZE as usize];
            // The two return thunks first keep the returned RAX: mov rcx, rax.
            let at = if n < FIRST_IMPORT { t[..3].copy_from_slice(&[0x48, 0x89, 0xC1]); 3 } else { 0 };
            t[at] = 0xB8;
            t[at + 1..at + 5].copy_from_slice(&code.to_le_bytes());
            t[at + 5..at + 7].copy_from_slice(&[0xE7, cell::HYPERCALL_PORT]);
            t[at + 7] = if n < FIRST_IMPORT { 0xF4 } else { 0xC3 }; // returns never come back; calls ret
            thunks.extend_from_slice(&t);
        }
        let mut imports = Vec::with_capacity(list.len());
        for (i, imp) in list.iter().enumerate() {
            pe::bind(&mut mem, imp.slot_rva, THUNK_VA + (FIRST_IMPORT + i as u64) * THUNK_SIZE)?;
            let name = match (&imp.name, imp.ordinal) { (Some(n), _) => n.clone(), (None, Some(o)) => alloc::format!("#{o}"), _ => String::new() };
            imports.push((imp.dll.clone(), name.clone(), win32::lookup(&imp.dll, &name)));
        }
        cell.map(THUNK_VA, pages(thunks.len() as u64), Access::USER_RX)?;
        cell.write(THUNK_VA, &thunks).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;

        // The image: headers read-only, then each section with its own rights.
        let header_pages = pages(image.size_of_headers as u64).max(1);
        cell.map(base, header_pages, Access::USER_RO)?;
        cell.write(base, &mem[..(header_pages * PAGE).min(mem.len() as u64) as usize]).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
        for s in &image.sections {
            let size = s.virtual_size.max(s.raw_size) as u64;
            if size == 0 { continue; }
            let va = base + s.rva as u64;
            let access = Access { write: s.writable(), execute: s.executable(), user: true };
            cell.map(va, pages(size), access)?;
            let end = ((s.rva as u64 + pages(size) * PAGE) as usize).min(mem.len());
            cell.write(va, &mem[s.rva as usize..end]).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
        }

        // Stack (committed whole for now; guard pages and growth come with threads).
        let stack_bytes = image.stack_reserve.clamp(64 << 10, 8 << 20) & !(PAGE - 1);
        let stack_base = STACK_TOP - stack_bytes;
        cell.map(stack_base, stack_bytes / PAGE, Access::USER_RW)?;

        // PEB and TEB.
        cell.map(PEB_VA, 1, Access::USER_RW)?;
        cell.write_u64(PEB_VA + win32::PEB_IMAGE_BASE, base).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
        cell.map(TEB_VA, TEB_PAGES, Access::USER_RW)?;
        let teb = [
            (win32::TEB_EXCEPTION_LIST, u64::MAX),
            (win32::TEB_STACK_BASE, STACK_TOP),
            (win32::TEB_STACK_LIMIT, stack_base),
            (win32::TEB_SELF, TEB_VA),
            (win32::TEB_PROCESS_ID, PROCESS_ID as u64),
            (win32::TEB_THREAD_ID, THREAD_ID as u64),
            (win32::TEB_PEB, PEB_VA),
        ];
        for (offset, value) in teb { cell.write_u64(TEB_VA + offset, value).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?; }

        // The entry is called like a function: RSP = 8 mod 16, returning to the exit thunk.
        let rsp = STACK_TOP - 0x28;
        cell.write_u64(rsp, THUNK_VA + THUNK_SIZE).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
        let entry = base + image.entry_rva as u64;
        let mut initial = cell.user_state(entry, rsp, TEB_VA);
        initial.regs.rcx = PEB_VA;
        Ok(WinProcess {
            cell, image, base, entry, imports, handlers: Vec::new(), next_alloc: ALLOC_BASE,
            counters: Counters::default(), last_fault: None, dispatch: Vec::new(), initial,
        })
    }

    /// The state to load into a fresh vCPU before the first run.
    pub fn initial_state(&self) -> State { self.initial }

    /// Runs the process until it stops. The caller has loaded `initial_state`.
    pub fn run(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host) -> Stop {
        let mut e = Exit::default();
        let mut load = false;
        loop {
            if let Err(s) = cpu.run(&mut e, load) { return Stop::Hypervisor(s); }
            self.counters.exits += 1;
            load = false;
            match e.reason {
                exit::EXTERNAL_INTERRUPT | exit::PREEMPTED | exit::EXCEPTION_NMI => {
                    if !host.keep_going() { return Stop::Host; }
                }
                exit::IO if e.port == cell::HYPERCALL_PORT as u16 && e.io_flags == 0 && e.io_size == 4 => {
                    let code = e.regs.rax as u32;
                    let next = e.regs.rip + e.instruction_len as u64;
                    match code & cell::MAGIC_MASK {
                        cell::CALL => {
                            let n = (code & 0xFFFF) as u64;
                            if n < FIRST_IMPORT || n - FIRST_IMPORT >= self.imports.len() as u64 { return Stop::CellFault { vector: 0xFF, rip: e.regs.rip }; }
                            let index = (n - FIRST_IMPORT) as usize;
                            self.counters.calls += 1;
                            match win32::call(self, index, &mut e.regs, host) {
                                win32::Outcome::Return(v) => { e.regs.rax = v; e.regs.rip = next; load = true; }
                                win32::Outcome::Stop(s) => return s,
                            }
                        }
                        cell::EXCEPTION => {
                            match self.exception((code & 0xFF) as u8, &mut e.regs, cpu) {
                                Ok(()) => load = true,
                                Err(s) => return s,
                            }
                        }
                        _ if code == cell::HANDLER_RETURN => match self.handler_returned(&mut e.regs) {
                            Ok(()) => load = true,
                            Err(s) => return s,
                        },
                        _ if code == cell::ENTRY_RETURN => return Stop::Exited(e.regs.rcx as u32),
                        _ => return Stop::Unexpected { reason: e.reason, rip: e.regs.rip },
                    }
                }
                exit::TRIPLE_FAULT => return Stop::TripleFault { rip: e.regs.rip },
                r => return Stop::Unexpected { reason: r, rip: e.regs.rip },
            }
        }
    }

    /// A stub reported `vector`. Builds the Windows exception and calls the first handler.
    fn exception(&mut self, vector: u8, regs: &mut Regs, cpu: &mut impl Vcpu) -> Result<(), Stop> {
        let frame = self.cell.frame(regs.rsp).ok_or(Stop::CellFault { vector, rip: regs.rip })?;
        if frame.cs & 3 != 3 { return Err(Stop::CellFault { vector, rip: frame.rip }); }
        self.counters.exceptions += 1;
        self.last_fault = Some(Fault { vector, error: frame.error, rip: frame.rip, cr2: frame.cr2 });
        // The thread's registers at the fault: the stub changed only RAX and RSP.
        let mut ctx = *regs;
        ctx.rax = frame.rax;
        ctx.rip = frame.rip;
        ctx.rsp = frame.rsp;
        ctx.rflags = frame.rflags;
        let (code, params) = win32::exception_code(vector, &frame);
        // Windows reports a breakpoint at the int3 itself.
        if vector == 3 { ctx.rip = ctx.rip.wrapping_sub(1); }
        // Back to ring 3 whatever happens next.
        // The whole ring-3 state, descriptor tables included, so nothing depends on how
        // the hypervisor reports the ring-0 state it stopped in.
        let mut st = self.cell.user_state(ctx.rip, ctx.rsp, TEB_VA);
        st.regs = ctx;
        cpu.set_state(&st).map_err(Stop::Hypervisor)?;
        *regs = ctx;
        self.dispatch_to_handlers(regs, code, ctx.rip, &params, &frame)
    }

    fn dispatch_to_handlers(&mut self, regs: &mut Regs, code: u32, address: u64, params: &[u64], _frame: &Frame) -> Result<(), Stop> {
        if self.handlers.is_empty() || self.dispatch.len() >= 8 {
            return Err(Stop::Unhandled { code, address });
        }
        // Lay out, below the thread's stack: CONTEXT, EXCEPTION_RECORD, EXCEPTION_POINTERS.
        let top = (regs.rsp - 128) & !0xF;
        let context = (top - win32::CONTEXT_SIZE) & !0xF;
        let record = (context - win32::RECORD_SIZE) & !0xF;
        let pointers = record - 16;
        let mut c = [0u8; win32::CONTEXT_SIZE as usize];
        win32::write_context(&mut c, regs);
        let mut r = [0u8; win32::RECORD_SIZE as usize];
        win32::write_record(&mut r, code, address, params);
        let mut p = [0u8; 16];
        p[..8].copy_from_slice(&record.to_le_bytes());
        p[8..].copy_from_slice(&context.to_le_bytes());
        let ok = self.cell.write_user(context, &c).and(self.cell.write_user(record, &r)).and(self.cell.write_user(pointers, &p));
        if ok.is_none() { return Err(Stop::Unhandled { code, address }); }
        self.dispatch.push(Dispatch { context, pointers, next_handler: 0, code, address });
        self.call_handler(regs)
    }

    /// Calls the next vectored handler of the innermost dispatch.
    fn call_handler(&mut self, regs: &mut Regs) -> Result<(), Stop> {
        let d = self.dispatch.last_mut().ok_or(Stop::Unexpected { reason: 0, rip: regs.rip })?;
        let Some(&handler) = self.handlers.get(d.next_handler) else {
            let (code, address) = (d.code, d.address);
            return Err(Stop::Unhandled { code, address });
        };
        d.next_handler += 1;
        // A call: 32 bytes of home space above the return address, RSP = 8 mod 16.
        let rsp = d.pointers - 0x30 - 8;
        if self.cell.write_user(rsp, &THUNK_VA.to_le_bytes()).is_none() { return Err(Stop::Unhandled { code: d.code, address: d.address }); }
        regs.rsp = rsp;
        regs.rcx = d.pointers;
        regs.rip = handler;
        Ok(())
    }

    /// A vectored handler returned through thunk 0, which moved its verdict to RCX.
    fn handler_returned(&mut self, regs: &mut Regs) -> Result<(), Stop> {
        let verdict = regs.rcx as u32 as i32;
        match verdict {
            // EXCEPTION_CONTINUE_EXECUTION: resume from the CONTEXT the handler may have edited.
            -1 => {
                let d = self.dispatch.pop().ok_or(Stop::Unexpected { reason: 0, rip: regs.rip })?;
                let mut c = [0u8; win32::CONTEXT_SIZE as usize];
                self.cell.read_user(d.context, &mut c).ok_or(Stop::Unhandled { code: d.code, address: d.address })?;
                win32::read_context(&c, regs);
                self.counters.handled += 1;
                Ok(())
            }
            // EXCEPTION_CONTINUE_SEARCH: the next handler.
            _ => self.call_handler(regs),
        }
    }
}

pub fn pages(bytes: u64) -> u64 { bytes.div_ceil(PAGE) }

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;
    use alloc::vec;
    use crate::cell::{EXCEPTION, CALL, HANDLER_RETURN, SYS_STACK_TOP};

    /// A vCPU whose exits are scripted: each entry is called with the registers the
    /// process loaded and returns the next exit (built from them).
    struct Fake { steps: VecDeque<fn(&Regs) -> Exit>, regs: Regs, state: State }
    impl Vcpu for Fake {
        fn run(&mut self, exit: &mut Exit, load: bool) -> Result<(), u64> {
            if load { self.regs = exit.regs; }
            let step = self.steps.pop_front().ok_or(99u64)?;
            *exit = step(&self.regs);
            self.regs = exit.regs;
            Ok(())
        }
        fn get_state(&mut self, st: &mut State) -> Result<(), u64> { *st = self.state; st.regs = self.regs; Ok(()) }
        fn set_state(&mut self, st: &State) -> Result<(), u64> { self.state = *st; self.regs = st.regs; Ok(()) }
        fn inject(&mut self, _: u8, _: u32, _: Option<u32>) -> Result<(), u64> { Ok(()) }
    }

    #[derive(Default)]
    struct Out { text: Vec<u8> }
    impl Host for Out {
        fn output(&mut self, _: u32, b: &[u8]) { self.text.extend_from_slice(b); }
        fn now_ms(&mut self) -> u64 { 1234 }
        fn sleep(&mut self, _: u32) {}
    }

    fn out_exit(regs: &Regs, eax: u32) -> Exit {
        let mut e = Exit { reason: exit::IO, instruction_len: 2, port: cell::HYPERCALL_PORT as u16, io_size: 4, io_flags: 0, regs: *regs, ..Exit::default() };
        e.regs.rax = eax as u64;
        e
    }

    /// Windows x64 arguments 1-4 into a guest register file.
    fn args(g: &mut Regs, a: [u64; 4]) { g.rcx = a[0]; g.rdx = a[1]; g.r8 = a[2]; g.r9 = a[3]; }

    fn import(p: &WinProcess, name: &str) -> u32 {
        CALL | (FIRST_IMPORT as u32 + p.imports.iter().position(|i| i.1 == name).unwrap() as u32)
    }

    fn load() -> WinProcess { WinProcess::load(Ram::new(16 << 20).unwrap(), crate::TEST_EXE).unwrap() }

    #[test]
    fn the_test_program_loads_at_its_base_with_bound_imports() {
        let p = load();
        assert_eq!(p.base, 0x1_4000_0000);
        assert!(p.imports.iter().all(|i| i.2.is_some()), "every import of the test program is implemented");
        let st = p.initial_state();
        assert_eq!(st.regs.rip, p.entry);
        assert_eq!(st.regs.rsp % 16, 8, "called like a function");
        assert_eq!(p.cell.read_u64(st.regs.rsp), Some(THUNK_VA + THUNK_SIZE), "returns to the exit thunk");
        assert_eq!(p.cell.read_u64(TEB_VA + win32::TEB_SELF), Some(TEB_VA));
        assert_eq!(p.cell.read_u64(PEB_VA + win32::PEB_IMAGE_BASE), Some(p.base));
        // The import address table points at the thunks, and each thunk calls its own number.
        let iat = p.image.imports(&p.image.lay_out(crate::TEST_EXE, p.base).unwrap()).unwrap();
        for (i, imp) in iat.iter().enumerate() {
            let slot = p.cell.read_u64(p.base + imp.slot_rva as u64).unwrap();
            assert_eq!(slot, THUNK_VA + (FIRST_IMPORT + i as u64) * THUNK_SIZE);
            let mut code = [0u8; 8];
            p.cell.read_user(slot, &mut code).unwrap();
            assert_eq!(code[0], 0xB8);
            assert_eq!(u32::from_le_bytes([code[1], code[2], code[3], code[4]]), CALL | (FIRST_IMPORT as u32 + i as u32));
            assert_eq!(&code[5..8], &[0xE7, 0xE0, 0xC3]);
        }
        // .text is executable and read-only; .rdata read-only; nothing writable is executable.
        for m in p.cell.maps() { assert!(!(m.access.write && m.access.execute), "W^X in the cell at {:#x}", m.va); }
    }

    #[test]
    fn calls_reach_kernel32_and_return() {
        let mut p = load();
        // The program writes "hi\n" from its stack, then exits with 7.
        let msg_va = STACK_TOP - 0x100;
        p.cell.write_user(msg_va, b"hi\n").unwrap();
        let mut cpu = Fake { steps: VecDeque::new(), regs: p.initial_state().regs, state: p.initial_state() };
        static mut CALLS: [u32; 3] = [0; 3];
        unsafe { CALLS = [import(&p, "GetStdHandle"), import(&p, "WriteFile"), import(&p, "ExitProcess")]; }
        cpu.steps.push_back(|r| { let mut e = out_exit(r, unsafe { CALLS[0] }); e.regs.rcx = (-11i64) as u64; e });
        cpu.steps.push_back(|r| {
            assert_eq!(r.rax, 0x14, "GetStdHandle(STD_OUTPUT_HANDLE) answered");
            let mut e = out_exit(r, unsafe { CALLS[1] });
            args(&mut e.regs, [r.rax, STACK_TOP - 0x100, 3, STACK_TOP - 0x110]);
            e
        });
        cpu.steps.push_back(|r| { assert_eq!(r.rax, 1, "WriteFile succeeded"); let mut e = out_exit(r, unsafe { CALLS[2] }); e.regs.rcx = 7; e });
        let mut out = Out::default();
        let rip0 = cpu.regs.rip;
        assert_eq!(p.run(&mut cpu, &mut out), Stop::Exited(7));
        assert_eq!(out.text, b"hi\n");
        let mut n = [0u8; 4];
        p.cell.read_user(STACK_TOP - 0x110, &mut n).unwrap();
        assert_eq!(u32::from_le_bytes(n), 3, "bytes written");
        assert_eq!(cpu.regs.rip, rip0 + 2 * 2, "each call resumed after its out instruction");
        assert_eq!(p.counters.calls, 3);
    }

    #[test]
    fn a_page_fault_reaches_the_vectored_handler_and_resumes_from_its_context() {
        let mut p = load();
        p.handlers.push(0x1_4000_1000); // somewhere in .text
        let mut cpu = Fake { steps: VecDeque::new(), regs: p.initial_state().regs, state: p.initial_state() };
        // The thread at rip 0x140001234 writes to 0x0: #PF (write, user), the stub reports it.
        let user_rsp = STACK_TOP - 0x200;
        let frame_rsp = SYS_STACK_TOP - 64;
        let frame = [0u64, 0xAAAA, 6, 0x1_4000_1234, cell::USER_CS as u64, 0x246, user_rsp, cell::USER_DS as u64];
        for (i, v) in frame.iter().enumerate() { p.cell.write_u64(frame_rsp + 8 * i as u64, *v).unwrap(); }
        cpu.steps.push_back(|r| { let mut e = out_exit(r, EXCEPTION | 14); e.regs.rsp = SYS_STACK_TOP - 64; e.regs.rbx = 0xB0B; e });
        // Now in the handler: RCX = EXCEPTION_POINTERS. Check what it was given, then skip
        // 3 bytes in the CONTEXT and return EXCEPTION_CONTINUE_EXECUTION.
        cpu.steps.push_back(|r| {
            assert_eq!(r.rip, 0x1_4000_1000, "in the handler");
            assert_eq!(r.rsp % 16, 8);
            assert!(r.rsp < STACK_TOP - 0x200 - 0x4D0, "below the thread's stack and its CONTEXT");
            let mut e = out_exit(r, HANDLER_RETURN);
            e.regs.rip = THUNK_VA + 3; // the return thunk's out
            e.regs.rcx = u32::MAX as u64; // -1, moved from RAX by the thunk
            e
        });
        cpu.steps.push_back(|r| {
            assert_eq!((r.rip, r.rax, r.rbx, r.rsp), (0x1_4000_1234 + 3, 0xAAAA, 0xB0B, STACK_TOP - 0x200), "resumed from the edited CONTEXT");
            let mut e = out_exit(r, cell::ENTRY_RETURN);
            e.regs.rcx = 0;
            e
        });
        // The test edits the CONTEXT where the handler would: hook the HANDLER_RETURN step.
        struct Edit<'a> { inner: Fake, cell: *mut Cell, _l: core::marker::PhantomData<&'a ()> }
        impl Vcpu for Edit<'_> {
            fn run(&mut self, exit: &mut Exit, load: bool) -> Result<(), u64> {
                if load && exit.regs.rip == 0x1_4000_1000 {
                    let cell = unsafe { &mut *self.cell };
                    let mut ptrs = [0u8; 16];
                    cell.read_user(exit.regs.rcx, &mut ptrs).unwrap();
                    let record = u64::from_le_bytes(ptrs[..8].try_into().unwrap());
                    let context = u64::from_le_bytes(ptrs[8..].try_into().unwrap());
                    let mut r = [0u8; 0x30];
                    cell.read_user(record, &mut r).unwrap();
                    assert_eq!(u32::from_le_bytes(r[..4].try_into().unwrap()), win32::STATUS_ACCESS_VIOLATION);
                    assert_eq!(u64::from_le_bytes(r[0x10..0x18].try_into().unwrap()), 0x1_4000_1234, "ExceptionAddress");
                    assert_eq!(u64::from_le_bytes(r[0x20..0x28].try_into().unwrap()), 1, "write");
                    assert_eq!(u64::from_le_bytes(r[0x28..0x30].try_into().unwrap()), 0, "at 0x0");
                    let rip = cell.read_u64(context + 0xF8).unwrap();
                    cell.write_user(context + 0xF8, &(rip + 3).to_le_bytes()).unwrap();
                }
                self.inner.run(exit, load)
            }
            fn get_state(&mut self, st: &mut State) -> Result<(), u64> { self.inner.get_state(st) }
            fn set_state(&mut self, st: &State) -> Result<(), u64> { self.inner.set_state(st) }
            fn inject(&mut self, a: u8, b: u32, c: Option<u32>) -> Result<(), u64> { self.inner.inject(a, b, c) }
        }
        let cell_ptr: *mut Cell = &mut p.cell;
        let mut edit = Edit { inner: cpu, cell: cell_ptr, _l: core::marker::PhantomData };
        let stop = p.run(&mut edit, &mut Out::default());
        assert_eq!(stop, Stop::Exited(0));
        assert_eq!((p.counters.exceptions, p.counters.handled), (1, 1));
        assert_eq!(edit.inner.state.segments[1].selector, cell::USER_CS, "the vCPU went back to ring 3");
    }

    #[test]
    fn faults_without_handlers_and_ring0_faults_stop_cleanly() {
        let mut p = load();
        let mut cpu = Fake { steps: VecDeque::new(), regs: p.initial_state().regs, state: p.initial_state() };
        let frame_rsp = SYS_STACK_TOP - 64;
        for (i, v) in [0u64, 0, 0, 0x1_4000_1000, cell::USER_CS as u64, 0x202, STACK_TOP - 0x80, 0x2B].iter().enumerate() {
            p.cell.write_u64(frame_rsp + 8 * i as u64, *v).unwrap();
        }
        cpu.steps.push_back(|r| { let mut e = out_exit(r, EXCEPTION); e.regs.rsp = SYS_STACK_TOP - 64; e });
        assert_eq!(p.run(&mut cpu, &mut Out::default()), Stop::Unhandled { code: win32::STATUS_INTEGER_DIVIDE_BY_ZERO, address: 0x1_4000_1000 });
        // A frame whose CS is ring 0 is the cell's own bug.
        p.cell.write_u64(frame_rsp + 32, cell::KERNEL_CS as u64).unwrap();
        cpu.steps.push_back(|r| { let mut e = out_exit(r, EXCEPTION | 13); e.regs.rsp = SYS_STACK_TOP - 64; e });
        assert!(matches!(p.run(&mut cpu, &mut Out::default()), Stop::CellFault { vector: 13, .. }));
        // A stub RSP that is not on the exception stack.
        cpu.steps.push_back(|r| { let mut e = out_exit(r, EXCEPTION | 14); e.regs.rsp = 0x1000; e });
        assert!(matches!(p.run(&mut cpu, &mut Out::default()), Stop::CellFault { vector: 14, .. }));
        // A call number past the import table.
        cpu.steps.push_back(|r| out_exit(r, CALL | 0xFFF0));
        assert!(matches!(p.run(&mut cpu, &mut Out::default()), Stop::CellFault { .. }));
    }

    #[test]
    fn virtual_alloc_reserves_commits_and_refuses() {
        let mut p = load();
        let mut regs = p.initial_state().regs;
        let va = |p: &mut WinProcess, regs: &mut Regs, a: u64, size: u64, kind: u64, prot: u64| {
            args(regs, [a, size, kind, prot]);
            let i = p.imports.iter().position(|i| i.1 == "VirtualAlloc").unwrap();
            match win32::call(p, i, regs, &mut Out::default()) { win32::Outcome::Return(v) => v, _ => panic!() }
        };
        let a = va(&mut p, &mut regs, 0, 5000, 0x3000, 4);
        assert_eq!(a % ALLOCATION_GRANULARITY, 0);
        assert!(p.cell.write_user(a + 4999, b"x").is_some());
        let low = va(&mut p, &mut regs, 0x1000_0000, 0x1_0000, 0x3000, 4);
        assert_eq!(low, 0x1000_0000, "a fixed low address: the cell is its own address space");
        assert_eq!(va(&mut p, &mut regs, 0x1000_0000, 0x1000, 0x3000, 4), 0, "already taken");
        let r = va(&mut p, &mut regs, 0, 0x4_0000, 0x2000, 1);
        assert!(p.cell.write_user(r, b"x").is_none(), "reserved, not committed");
        assert_eq!(va(&mut p, &mut regs, r + 0x1_0000, 0x2000, 0x1000, 4), r + 0x1_0000, "commit inside the reservation");
        assert!(p.cell.write_user(r + 0x1_1FFF, b"x").is_some());
        assert_eq!(va(&mut p, &mut regs, 0x7000_0000, 0x1000, 0x1000, 4), 0, "commit outside any reservation");
        assert_eq!(va(&mut p, &mut regs, 0, 0x1000, 0x3000, 0x80), 0, "unknown protection");
        assert_eq!(va(&mut p, &mut regs, 0, 1 << 41, 0x3000, 4), 0, "too big");
        let ro = va(&mut p, &mut regs, 0, 0x1000, 0x3000, 2);
        assert!(p.cell.write_user(ro, b"x").is_none(), "PAGE_READONLY");
        let _ = vec![0u8];
    }
}
