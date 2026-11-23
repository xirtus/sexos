// SPDX-License-Identifier: GPL-3.0-only
//! Boot services' stacks grow on touch. Each service's slot keeps 1 MiB of address
//! space under its stack top: the loader maps the top 64 KiB, and a touch anywhere
//! lower (a deep call, a big frame) gets a zeroed page of the service's own key, as a
//! program's stack does (#306). The lowest page stays unmapped: a touch there is an
//! overflow and ends the service as any bad fault does, never reaching a neighbour.
//!
//! Memory is spent only where a service really goes, so a 16 GiB machine and a 1 GiB
//! one boot the same services; only the depth they may reach is bigger than before.
//! Pages a run grew stay with the service across its restarts: a restart starts the
//! run again at the same top, and what lies below is scratch it writes before reading.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Address space under the stack top, guard page included: the loader's slot layout
/// (`loader::elf`) leaves exactly this much between a slot's base and its stack top.
pub const RESERVE: u64 = 1024 * 1024;
/// Mapped by the loader before the service first runs.
pub const EAGER: u64 = 64 * 1024;
const DOMAINS: usize = 64;
/// By domain id: its stack top (0: not a growing service stack) and its pages grown.
static TOPS: [AtomicU64; DOMAINS] = [const { AtomicU64::new(0) }; DOMAINS];
static GROWN: [AtomicU32; DOMAINS] = [const { AtomicU32::new(0) }; DOMAINS];
const PTE_FRAME: u64 = 0x000f_ffff_ffff_f000;

/// From the loader, after the eager top is mapped: the service in `pd` may grow its
/// stack. Program slots are never registered (their stacks live in their arenas).
pub fn register(pd: u32, stack_top: u64) {
    if pd == 0 || pd as usize >= DOMAINS || stack_top & 0xfff != 0 || stack_top < RESERVE { return; }
    TOPS[pd as usize].store(stack_top, Ordering::Release);
}

/// The range a service may grow into: [guard page top, eager bottom).
fn growth(pd: u32) -> Option<(u64, u64, u64)> {
    if pd == 0 || pd as usize >= DOMAINS { return None; }
    let top = TOPS[pd as usize].load(Ordering::Acquire);
    if top == 0 { return None; }
    let guard = top - RESERVE;
    Some((guard, top - EAGER, top))
}

/// A touch of `va` by domain `pd` below its stack's mapped top: Some(true) once a
/// zeroed page is there (or already was), Some(false) for the guard page, None when
/// `va` is not in its stack reserve or no memory is left.
pub fn commit(pd: &crate::capability::ProtectionDomain, va: u64) -> Option<bool> {
    let (guard, low_eager, top) = growth(pd.id)?;
    let va = va & !0xfff;
    if va == guard { return Some(false); }
    if va <= guard || va >= low_eager { return None; }
    // The slot's page table holds the mapped top, and the whole reserve shares its
    // 2 MiB span, so the leaf is always there; tables are shared by every view.
    let leaf = crate::boot_memory::existing_leaf_address(va)?;
    let leaf = unsafe { &*(leaf as *const AtomicU64) };
    match leaf.load(Ordering::Acquire) {
        0 => {}
        v if v & 1 != 0 => return Some(true),
        // Not ours to fill: some other use of this leaf.
        _ => return None,
    }
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    let frame = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0)?;
    unsafe { core::ptr::write_bytes((hhdm + frame) as *mut u8, 0, 4096); }
    // Present, writable, user, no execute, the service's own key.
    let pte = (frame & PTE_FRAME) | 7 | ((pd.pku_key as u64 & 15) << 59) | (1 << 63);
    // Another of its tasks may have grown the same page first: theirs stands.
    if let Err(now) = leaf.compare_exchange(0, pte, Ordering::AcqRel, Ordering::Acquire) {
        crate::memory::allocator::GLOBAL_ALLOCATOR.free(frame, 0);
        return (now & 1 != 0).then_some(true);
    }
    // The leaf was clear, so no core holds a translation for it.
    x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(va));
    let grown = GROWN[pd.id as usize].fetch_add(1, Ordering::Relaxed) + 1;
    if grown % 16 == 1 {
        crate::serial_println!("[service.stack.grow] pd={} grown_kib={} depth_kib={}", pd.id, grown * 4,
            (top - va) / 1024);
    }
    Some(true)
}

/// For the fault report: whether `va` is `pd`'s stack guard page.
pub fn is_guard(pd: u32, va: u64) -> bool { growth(pd).is_some_and(|(guard, _, _)| va & !0xfff == guard) }
