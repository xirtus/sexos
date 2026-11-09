// SPDX-License-Identifier: GPL-3.0-only
//! Thread-local storage: each task gets its own copy of the program's `#[thread_local]`
//! statics (and C/C++ `thread_local`), laid out as x86-64 expects. The thread pointer
//! (FS base) points at a small control block whose first word points at itself, and
//! the program's TLS image sits just below it.
//!
//! `init` sets up the calling task's block; `thread::spawn` does it for every worker
//! before its job runs. A program with no TLS segment needs none of this.
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

/// Set once the main task has its block. Workers always get one before they run, and
/// the first worker can only be started after this is set (see `thread::spawn`).
static READY: AtomicBool = AtomicBool::new(false);

const PT_TLS: u32 = 7;
const PT_LOAD: u32 = 1;
/// The control block: the self pointer, then a word a C library may use for its own
/// per-thread record (`pthread_self`).
pub const TCB_WORDS: usize = 4;

unsafe extern "C" {
    static __ehdr_start: u8;
}

#[derive(Clone, Copy)]
struct Template { image: usize, file: usize, memory: usize, align: usize }

fn template() -> Option<Template> {
    unsafe {
        let ehdr = core::ptr::addr_of!(__ehdr_start) as usize;
        let at = |o: usize| ehdr + o;
        let phoff = core::ptr::read_unaligned(at(32) as *const u64) as usize;
        let phnum = core::ptr::read_unaligned(at(56) as *const u16) as usize;
        let header = |i: usize| ehdr + phoff + i * 56;
        // The ELF header's own link address: the load segment that starts at offset 0.
        let mut base = None;
        let mut tls = None;
        for i in 0..phnum {
            let h = header(i);
            let kind = core::ptr::read_unaligned(h as *const u32);
            let offset = core::ptr::read_unaligned((h + 8) as *const u64) as usize;
            let vaddr = core::ptr::read_unaligned((h + 16) as *const u64) as usize;
            if kind == PT_LOAD && offset == 0 && base.is_none() { base = Some(vaddr); }
            if kind == PT_TLS {
                let file = core::ptr::read_unaligned((h + 32) as *const u64) as usize;
                let memory = core::ptr::read_unaligned((h + 40) as *const u64) as usize;
                let align = core::ptr::read_unaligned((h + 48) as *const u64) as usize;
                tls = Some((vaddr, file, memory, align.max(1)));
            }
        }
        let (vaddr, file, memory, align) = tls?;
        Some(Template { image: ehdr + (vaddr - base?), file, memory, align })
    }
}

/// One task's thread-local block. Keep it alive while the task runs.
pub struct Block { _storage: Vec<u128>, pointer: usize }

impl Block {
    /// A fresh copy of the program's TLS image with its control block, or `None` if
    /// memory ran out.
    pub fn new() -> Option<Block> {
        let t = template().unwrap_or(Template { image: 0, file: 0, memory: 0, align: 16 });
        // The linker places the image at TP - round_up(memory, p_align), with the
        // segment's own alignment, whatever alignment the thread pointer itself has.
        let below = (t.memory + t.align - 1) & !(t.align - 1);
        let align = t.align.max(16);
        let bytes = align + below + TCB_WORDS * 8;
        let mut storage: Vec<u128> = Vec::new();
        storage.try_reserve_exact(bytes.div_ceil(16)).ok()?;
        storage.resize(bytes.div_ceil(16), 0);
        let start = storage.as_mut_ptr() as usize;
        // The thread pointer is aligned like the TLS segment; the image ends at it.
        let pointer = (start + below + align - 1) & !(align - 1);
        debug_assert!(pointer + TCB_WORDS * 8 <= start + storage.len() * 16);
        unsafe {
            let image = pointer - below;
            if t.file != 0 { core::ptr::copy_nonoverlapping(t.image as *const u8, image as *mut u8, t.file); }
            *(pointer as *mut usize) = pointer;
        }
        Some(Block { _storage: storage, pointer })
    }
    pub fn pointer(&self) -> usize { self.pointer }
    /// Makes this the calling task's block.
    pub fn install(&self) -> bool { sex_pdx::program_threads::set_tls(self.pointer as u64) }
}

/// Gives the calling task (usually the main task, at start) its own TLS block, kept
/// for the rest of the run. Returns false if memory ran out or the call was refused.
pub fn init() -> bool {
    if READY.load(Ordering::Acquire) { return true; }
    match Block::new() {
        Some(block) => {
            let ok = block.install();
            core::mem::forget(block);
            if ok { READY.store(true, Ordering::Release); }
            ok
        }
        None => false,
    }
}

pub(crate) fn ready() -> bool { READY.load(Ordering::Acquire) }

/// The calling task's thread pointer, or 0 before `init`.
pub fn pointer() -> usize {
    if !ready() { return 0; }
    let value: usize;
    // fs:0 holds the block's self pointer.
    unsafe { core::arch::asm!("mov {}, fs:0", out(reg) value, options(readonly, nostack, preserves_flags)) };
    value
}

/// The control block's spare words, for a C library's per-thread record.
pub fn slot(index: usize) -> *mut usize {
    assert!(index > 0 && index < TCB_WORDS);
    let p = pointer();
    if p == 0 { core::ptr::null_mut() } else { (p + index * 8) as *mut usize }
}
