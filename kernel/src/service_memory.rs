// SPDX-License-Identifier: GPL-3.0-only
//! Memory that grows for BOOT services, as program rows already have. BOOT reserves an
//! arena for each service it names; the service then asks for pages (SYS_PROGRAM_MEMORY,
//! self-only) as it needs them, up to all the machine can lend. A service's caches and
//! maps can then fit the disk and the RAM it finds, not sizes fixed at build time.
//! Pages stay with the service for the boot: a restarted service adopts what it held.
use sex_pdx::program_memory as m;
use x86_64::VirtAddr;

const MAX: usize = 8;
#[derive(Clone, Copy)]
struct Arena { pd: u32, base: u64, pages: usize, grown: usize, key: u8 }
static ARENAS: spin::Mutex<[Option<Arena>; MAX]> = spin::Mutex::new([None; MAX]);

/// BOOT only: gives service `pd_id` an arena. No page or table exists until it grows.
pub fn reserve(pd_id: u32) {
    if pd_id == 0 { return; }
    let pd = crate::ipc::DOMAIN_REGISTRY.get(pd_id).expect("BOOT_SERVICE_ARENA_PD_MISSING");
    let (base, pages) = crate::boot_memory::reserve_arena(pd.pku_key);
    let mut arenas = ARENAS.lock();
    assert!(arenas.iter().flatten().all(|a| a.pd != pd_id), "BOOT_SERVICE_ARENA_TWICE");
    let free = arenas.iter_mut().find(|a| a.is_none()).expect("BOOT_SERVICE_ARENA_FULL");
    *free = Some(Arena { pd: pd_id, base, pages, grown: 0, key: pd.pku_key });
    crate::serial_println!("[boot.service.arena] pd={} base={:#x} pages={} key={}", pd_id, base, pages, pd.pku_key);
}

/// None when `pd_id` has no service arena; otherwise the reply SYS_PROGRAM_MEMORY gives.
pub fn grow(pd_id: u32, want: u64) -> Option<[u64; 3]> {
    let mut arenas = ARENAS.lock();
    let arena = arenas.iter_mut().flatten().find(|a| a.pd == pd_id)?;
    Some(grow_in(arena, want).unwrap_or_else(|e| [e, 0, 0]))
}

fn grow_in(arena: &mut Arena, want: u64) -> Result<[u64; 3], u64> {
    if want > m::MAX_PAGES_PER_CALL { return Err(m::INVALID); }
    let want = want as usize;
    if want == 0 { return Ok([m::OK, arena.base, arena.grown as u64]); }
    if arena.grown + want > arena.pages { return Err(m::FULL); }
    let mut leaves = [0u64; m::MAX_PAGES_PER_CALL as usize];
    for (i, index) in (arena.grown..arena.grown + want).enumerate() {
        let leaf = crate::boot_memory::arena_leaf(arena.base + index as u64 * 4096, arena.key).ok_or(m::FULL)?;
        if unsafe { core::ptr::read_volatile(leaf as *const u64) } != 0 { return Err(m::DENIED); }
        leaves[i] = leaf;
    }
    // All or none: every frame is taken before any page is mapped.
    let mut frames = [0u64; m::MAX_PAGES_PER_CALL as usize];
    for i in 0..want {
        match crate::program_frames::take() {
            Some(f) => frames[i] = f,
            None => { for &f in &frames[..i] { crate::program_frames::give(f); } return Err(m::FULL); }
        }
    }
    for i in 0..want {
        // Present, writable, user, no-execute, the service's own key.
        let entry = frames[i] | 7 | ((arena.key as u64) << 59) | (1 << 63);
        unsafe { core::ptr::write_volatile(leaves[i] as *mut u64, entry); }
        // The leaf was clear, so no core holds a translation for it.
        x86_64::instructions::tlb::flush(VirtAddr::new(arena.base + (arena.grown + i) as u64 * 4096));
    }
    arena.grown += want;
    crate::serial_println!("[service.memory] pd={} granted={} held={} free_pages={}", arena.pd, want, arena.grown, crate::program_frames::available());
    Ok([m::OK, arena.base, arena.grown as u64])
}

/// A page of `pd`'s arena below its mark: its frame goes back (`fill` false), or a hole
/// gets a fresh zeroed frame (`fill` true). The pages stay the service's to refill.
pub fn hole(pd_id: u32, va: u64, fill: bool) -> u64 {
    // A copy: no core may wait on this lock while a shootdown waits on it. Only the
    // service itself calls, so its arena cannot change meanwhile.
    let Some(a) = ARENAS.lock().iter().flatten().find(|a| a.pd == pd_id).copied() else { return m::DENIED };
    if va & 0xFFF != 0 || va < a.base || va >= a.base + a.grown as u64 * 4096 { return m::INVALID; }
    let Some(leaf) = crate::boot_memory::existing_leaf_address(va) else { return m::INVALID };
    let old = unsafe { core::ptr::read_volatile(leaf as *const u64) };
    if fill != (old == 0) { return m::DENIED; }
    if fill {
        let Some(frame) = crate::program_frames::take() else { return m::FULL };
        unsafe { core::ptr::write_volatile(leaf as *mut u64, frame | 7 | ((a.key as u64) << 59) | (1 << 63)); }
        x86_64::instructions::tlb::flush(VirtAddr::new(va));
    } else {
        unsafe { core::ptr::write_volatile(leaf as *mut u64, 0); }
        crate::smp::tlb::flush(va);
        crate::program_frames::give(old & crate::app_resource_state::PHYSICAL_MASK);
    }
    m::OK
}

/// Every page of `pd`'s arena goes back, with one shootdown (a restarted Denim's old store).
pub fn empty(pd_id: u32) -> u64 {
    let Some(a) = ARENAS.lock().iter().flatten().find(|a| a.pd == pd_id).copied() else { return 0 };
    let mut frames = alloc::vec::Vec::new();
    for va in (0..a.grown as u64).map(|i| a.base + i * 4096) {
        let Some(leaf) = crate::boot_memory::existing_leaf_address(va) else { continue };
        let old = unsafe { core::ptr::read_volatile(leaf as *const u64) };
        if old & 1 == 0 { continue; }
        unsafe { core::ptr::write_volatile(leaf as *mut u64, 0); }
        frames.push(old & crate::app_resource_state::PHYSICAL_MASK);
    }
    crate::smp::tlb::flush_pages(frames.len(), (0..a.grown as u64).map(|i| a.base + i * 4096));
    for &f in &frames { crate::program_frames::give(f); }
    frames.len() as u64
}
