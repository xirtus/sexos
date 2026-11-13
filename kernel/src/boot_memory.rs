// SPDX-License-Identifier: GPL-3.0-only
//! BOOT_DAG bootstrap reservation and immutable region genesis.
//! Runtime resolution never calls these allocator/mapping mechanisms. Sext's
//! bounded paging pool and page-table infrastructure are constructed here too.

use crate::capability::{CapabilityData, ProtectionDomain};
use crate::ipc::{BootPhase, DOMAIN_REGISTRY};
use sex_pdx::memory_region::{RegionGrant, QUERY_VIEW, READ, WRITE, DMA_ADDRESS};
use x86_64::{VirtAddr, structures::paging::PageTableFlags};

fn require_boot() {
    assert_eq!(crate::ipc::BOOT_PHASE.load(core::sync::atomic::Ordering::Acquire),
        BootPhase::RegistryBuild as u8, "BOOT_MEMORY_AFTER_GENESIS");
}

pub(crate) fn backing(length: u64) -> u64 {
    require_boot();
    assert!(length > 0 && length % 4096 == 0);
    let pages = (length / 4096).next_power_of_two();
    let physical = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(pages.trailing_zeros() as usize)
        .expect("BOOT_REGION_NO_FRAMES");
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("BOOT_REGION_NO_VAS")
        .frame_allocator.reserve_boot_region(physical, pages * 4096)
        .expect("BOOT_REGION_FRAME_OWNERSHIP_COLLISION");
    let hhdm = crate::HHDM_REQUEST.response().expect("BOOT_REGION_NO_HHDM").offset;
    // Zero the entire reservation, including buddy rounding, before publication.
    unsafe { core::ptr::write_bytes((physical + hhdm) as *mut u8, 0, (pages * 4096) as usize); }
    physical
}

/// Retained source authority: private BOOT frames, no user/DMA grant or runtime free.
pub fn retain_image_source(bytes:&[u8])->crate::app_resource_state::ImageSource{
    require_boot();
    assert!(!bytes.is_empty() && bytes.len()<=sex_pdx::program_store::IMAGE_BYTES,"BOOT_IMAGE_SOURCE_SIZE");
    let physical=backing(bytes.len().next_multiple_of(4096) as u64);
    let hhdm=crate::HHDM_REQUEST.response().expect("BOOT_IMAGE_SOURCE_HHDM").offset;
    let address=(physical+hhdm) as usize;
    unsafe{core::ptr::copy_nonoverlapping(bytes.as_ptr(),address as *mut u8,bytes.len());
        crate::app_resource_state::ImageSource::boot_private(address,bytes.len())}
}

/// The screen BOOT found, so program pixel surfaces can cover all of it.
static SCREEN_PIXELS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static PROGRAM_IMAGE_PHYSICAL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// The one region Files reads program files into. Shell's load call decides when
/// the kernel looks at it; Files gets no say over what the kernel does with it.
pub fn init_program_image(files: u32) {
    use sex_pdx::program_store::{IMAGE_BYTES, IMAGE_REGION_ID, IMAGE_SLOT_FILES};
    require_boot();
    if files == 0 { return; }
    let pd = DOMAIN_REGISTRY.get(files).expect("BOOT_PROGRAM_IMAGE_FILES");
    let length = IMAGE_BYTES as u64;
    let physical = backing(length);
    let virtual_base = alias(pd, physical, length, true, false);
    grant(pd, IMAGE_SLOT_FILES, RegionGrant { region_id: IMAGE_REGION_ID, virtual_base, length,
        rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: None });
    PROGRAM_IMAGE_PHYSICAL.store(physical, core::sync::atomic::Ordering::Release);
}
/// The boot image Limine loaded beside the kernel, if any, mapped read-only into Files
/// so `install` can write the very system that is running. It grants only reading it.
pub fn init_boot_image(files: u32) {
    use sex_pdx::fs as w;
    require_boot();
    if files == 0 { return; }
    let Some(module) = crate::MODULE_REQUEST.response().and_then(|m| m.modules().iter().find(|m| m.path().ends_with(w::BOOT_IMAGE_MODULE))) else { return };
    let hhdm = crate::HHDM_REQUEST.response().expect("BOOT_IMAGE_NO_HHDM").offset;
    let data = module.data();
    let virt = data.as_ptr() as u64;
    let length = (data.len() as u64).next_multiple_of(4096);
    if virt % 4096 != 0 || length == 0 { return; }
    let pd = DOMAIN_REGISTRY.get(files).expect("BOOT_IMAGE_FILES");
    let virtual_base = alias(pd, virt - hhdm, length, false, false);
    grant(pd, w::BOOT_IMAGE_SLOT, RegionGrant { region_id: w::BOOT_IMAGE_REGION_ID, virtual_base, length,
        rights: QUERY_VIEW | READ, dma_base: None, peer_pd: None });
    crate::serial_println!("[boot.image.region] files={} bytes={}", files, data.len());
}
/// Where Limine loaded the kernel from, read-only in Files (see `sex_pdx::fs::BOOT_FACTS_SLOT`).
pub fn init_boot_facts(files: u32) {
    use sex_pdx::fs as w;
    require_boot();
    if files == 0 { return; }
    let physical = backing(4096);
    let hhdm = crate::HHDM_REQUEST.response().expect("BOOT_FACTS_NO_HHDM").offset;
    if let Some(file) = crate::KERNEL_FILE_REQUEST.response().map(|r| r.executable_file()) {
        let u = file.gpt_part_uuid;
        let mut guid = [0u8; 16];
        guid[0..4].copy_from_slice(&u.a.to_le_bytes());
        guid[4..6].copy_from_slice(&u.b.to_le_bytes());
        guid[6..8].copy_from_slice(&u.c.to_le_bytes());
        guid[8..16].copy_from_slice(&u.d);
        unsafe { core::ptr::copy_nonoverlapping(guid.as_ptr(), (physical + hhdm) as *mut u8, 16); }
        // The file record as Limine wrote it, from `media_type` on: Limine 7 has a word the
        // crate's layout lacks before the UUIDs, so Files matches GUIDs in it at any offset.
        unsafe { core::ptr::copy_nonoverlapping((file as *const limine::file::File as *const u8).add(40), (physical + hhdm + 16) as *mut u8, 80); }
        crate::serial_println!("[boot.facts] partition={:08x}-{:04x}-{:04x}-{:02x}{:02x} part_index={}", u.a, u.b, u.c, u.d[0], u.d[1], file.partition_index);
    }
    let pd = DOMAIN_REGISTRY.get(files).expect("BOOT_FACTS_FILES");
    let virtual_base = alias(pd, physical, 4096, false, false);
    grant(pd, w::BOOT_FACTS_SLOT, RegionGrant { region_id: w::BOOT_FACTS_REGION_ID, virtual_base, length: 4096,
        rights: QUERY_VIEW | READ, dma_base: None, peer_pd: None });
}
/// One region per possible Spindle for the open-internet stream, shared with the
/// network service alone.
pub fn init_web_regions(net: u32) {
    use sex_pdx::web as w;
    require_boot();
    if net == 0 { return; }
    for (index, client) in sex_pdx::app_catalog::SPINDLE_TARGETS.into_iter().enumerate() {
        paired_at(net, client, w::PROVIDER_SLOT_BASE + index as u64, w::CLIENT_SLOT,
            w::REGION_ID_BASE + index as u32, w::REGION_BYTES as u64, true);
    }
    let first_browser = sex_pdx::app_catalog::SPINDLE_TARGETS.len();
    for (index, plan) in sex_pdx::browser_sources::PLANS.iter().enumerate() {
        paired_at(net, plan.target(), w::PROVIDER_SLOT_BASE + (first_browser + index) as u64, w::CLIENT_SLOT,
            w::REGION_ID_BASE + (first_browser + index) as u32, w::REGION_BYTES as u64, true);
    }    // Program rows too. A run reaches the network only once Shell binds it there.
    let programs = sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i| i.app == sex_pdx::app_catalog::App::Program);
    for (k, row) in programs.enumerate() {
        let index = w::PROGRAM_INDEX_BASE + k;
        paired_at(net, row.target, w::PROVIDER_SLOT_BASE + index as u64, w::CLIENT_SLOT,
            w::REGION_ID_BASE + index as u32, w::REGION_BYTES as u64, true);
    }
}
/// One region per possible Spindle and program row for the file protocol, shared with Files alone.
pub fn init_fs_regions(files: u32) {
    use sex_pdx::fs as w;
    require_boot();
    if files == 0 { return; }
    for (index, client) in sex_pdx::app_catalog::SPINDLE_TARGETS.into_iter().enumerate() {
        paired_at(files, client, w::PROVIDER_SLOT_BASE + index as u64, w::CLIENT_SLOT,
            w::REGION_ID_BASE + index as u32, w::REGION_BYTES as u64, true);
    }    // Each program row too: what it may reach there is decided by Files, per run.
    let programs = sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i| i.app == sex_pdx::app_catalog::App::Program);
    for (k, row) in programs.enumerate() {
        let index = w::PROGRAM_INDEX_BASE + k;
        paired_at(files, row.target, w::PROVIDER_SLOT_BASE + index as u64, w::CLIENT_SLOT,
            w::REGION_ID_BASE + index as u32, w::REGION_BYTES as u64, true);
    }
}
/// The file Files last read, as a copy only the kernel can reach: Files keeps write
/// access to its region, and a checked file must stay the file that gets used. The copy
/// is borrowed from free memory for one load and returned when it drops.
pub struct ImageCopy { physical: u64, order: usize, len: usize }
impl core::ops::Deref for ImageCopy {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        let hhdm = crate::HHDM_REQUEST.response().expect("IMAGE_COPY_NO_HHDM").offset;
        unsafe { core::slice::from_raw_parts((self.physical + hhdm) as *const u8, self.len) }
    }
}
impl Drop for ImageCopy {
    fn drop(&mut self) { crate::memory::allocator::GLOBAL_ALLOCATOR.free(self.physical, self.order); }
}
pub fn program_image(len: usize) -> Option<ImageCopy> {
    let physical = PROGRAM_IMAGE_PHYSICAL.load(core::sync::atomic::Ordering::Acquire);
    if physical == 0 || len == 0 || len > sex_pdx::program_store::IMAGE_BYTES { return None; }
    let order = (len.div_ceil(4096)).next_power_of_two().trailing_zeros() as usize;
    let private = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(order)?;
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    unsafe { core::ptr::copy_nonoverlapping((physical + hhdm) as *const u8, (private + hhdm) as *mut u8, len); }
    Some(ImageCopy { physical: private, order, len })
}
/// The page table for a 2 MiB span, with all of its entries clear.
pub fn reserve_table(base: u64, pkey: u8, any_frame: u64) -> *mut u64 {
    require_boot();
    let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("BOOT_REGION_NO_VAS")
        .map_physical_range(VirtAddr::new(base), any_frame, 4096, flags, pkey).expect("BOOT_RECORD_TABLE_MAP");
    let leaf = existing_leaf_address(base).expect("BOOT_RECORD_TABLE_LEAF");
    unsafe { core::ptr::write_volatile(leaf as *mut u64, 0); }
    x86_64::instructions::tlb::flush(VirtAddr::new(base));
    leaf as *mut u64
}
/// Address space for one program row's arena; no page tables yet, every page absent.
/// Tables are made on first use (`arena_leaf`) and frames are lent per run
/// (`program_frames`), so the size, set once from what this machine holds, costs
/// nothing until a program uses it: one program may use all of the machine's memory.
pub fn reserve_arena(pkey: u8) -> (u64, usize) {
    require_boot();
    use core::sync::atomic::{AtomicUsize, Ordering};
    static PAGES: AtomicUsize = AtomicUsize::new(0);
    const SPAN: u64 = 2 * 1024 * 1024;
    let mut pages = PAGES.load(Ordering::Acquire);
    if pages == 0 {
        let free = crate::memory::allocator::debug_global_free_frames() * 4096;
        let bytes = free.max(1).next_power_of_two().clamp(8 * 1024 * 1024, sex_pdx::program_store::WINDOW_BYTES as u64);
        pages = (bytes / 4096) as usize;
        PAGES.store(pages, Ordering::Release);
    }
    let bytes = pages as u64 * 4096;
    let va = crate::memory::va_allocator::allocate_va((bytes + SPAN) as usize).expect("BOOT_ARENA_NO_VA");
    let base = (va + SPAN - 1) & !(SPAN - 1);
    crate::serial_println!("[boot.program.arena] base={:#x} pages={} bytes={} key={}", base, pages, bytes, pkey);
    (base, pages)
}
/// The leaf for `va` in a program arena, making its page tables on the span's first use.
/// The leaf comes back clear; the caller maps it. Tables are linked directly, zeroed,
/// so no page is ever present while they are made (a temporary mapping would be
/// visible to a program on another core that shares the key). One lock orders makers,
/// so two cores cannot both create one table.
pub fn arena_leaf(va: u64, pkey: u8) -> Option<u64> {
    use x86_64::registers::control::Cr3;
    static MAKE: spin::Mutex<()> = spin::Mutex::new(());
    if let Some(leaf) = existing_leaf_address(va) { return Some(leaf); }
    let _held = MAKE.lock();
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    // Present, writable, user: the leaf alone decides access and carries the key.
    // A row on the shared key takes turns by its spans' user bit, which only
    // `program_tenancy` sets: a span is made closed.
    let mut table = Cr3::read().0.start_address().as_u64();
    for shift in [39, 30, 21] {
        let link: u64 = if shift == 21 && pkey == sex_pdx::organs::SHARED_PROGRAM_KEY { 0b011 } else { 0b111 };
        let slot = (hhdm + table + ((va >> shift) & 511) * 8) as *mut u64;
        let mut entry = unsafe { core::ptr::read_volatile(slot) };
        if entry & 1 == 0 {
            let frame = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0)?;
            zero_frame(frame);
            entry = frame | link;
            unsafe { core::ptr::write_volatile(slot, entry); }
        }
        if entry & (1 << 7) != 0 { return None; }
        if entry & link != link {
            // A kernel mapping made this table; user and write here only let the leaf decide.
            entry |= link;
            unsafe { core::ptr::write_volatile(slot, entry); }
        }
        table = entry & 0x000f_ffff_ffff_f000;
    }
    existing_leaf_address(va)
}
pub(crate) fn write_frames(physical: u64, bytes: &[u8]) {
    let hhdm = crate::HHDM_REQUEST.response().expect("FRAME_WRITE_NO_HHDM").offset;
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), (physical + hhdm) as *mut u8, bytes.len()); }
}

/// One physical page to another, through the kernel's direct map.
pub(crate) fn copy_frame(from: u64, to: u64) {
    let hhdm = crate::HHDM_REQUEST.response().expect("FRAME_COPY_NO_HHDM").offset;
    unsafe { core::ptr::copy_nonoverlapping((from + hhdm) as *const u8, (to + hhdm) as *mut u8, 4096); }
}
pub(crate) fn zero_frame(frame: u64) {
    let hhdm = crate::HHDM_REQUEST.response().expect("FRAME_ZERO_NO_HHDM").offset;
    unsafe { core::ptr::write_bytes((frame + hhdm) as *mut u8, 0, 4096); }
}

fn alias(pd: &ProtectionDomain, physical: u64, length: u64, writable: bool, mmio: bool) -> u64 {
    require_boot();
    // A row that takes turns on the shared key gets whole 2 MiB spans to itself, so
    // one page-directory entry can open and close the region (`program_tenancy`).
    let va = if crate::program_tenancy::shared(pd.id) {
        let span = crate::program_tenancy::SPAN;
        let va = crate::memory::va_allocator::allocate_va((length.next_multiple_of(span) + span) as usize).expect("BOOT_REGION_NO_VA");
        va.next_multiple_of(span)
    } else { crate::memory::va_allocator::allocate_va(length as usize).expect("BOOT_REGION_NO_VA") };
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE;
    if writable { flags |= PageTableFlags::WRITABLE; }
    if mmio { flags |= PageTableFlags::NO_CACHE | PageTableFlags::WRITE_THROUGH; }
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("BOOT_REGION_NO_VAS")
        .map_physical_range(VirtAddr::new(va), physical, length, flags, pd.pku_key)
        .expect("BOOT_REGION_MAP_FAILED");
    va
}

/// A supervisor-only, uncached mapping of device registers the kernel itself programs
/// (MSI-X tables). No domain can reach it: the pages are not user-accessible.
pub fn kernel_mmio(physical: u64, length: u64) -> u64 {
    require_boot();
    let va = crate::memory::va_allocator::allocate_va(length as usize).expect("BOOT_KERNEL_MMIO_NO_VA");
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE
        | PageTableFlags::NO_CACHE | PageTableFlags::WRITE_THROUGH;
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("BOOT_KERNEL_MMIO_NO_VAS")
        .map_physical_range(VirtAddr::new(va), physical, length, flags, 0)
        .expect("BOOT_KERNEL_MMIO_MAP_FAILED");
    va
}

fn grant(pd: &ProtectionDomain, slot: u64, region: RegionGrant) {
    assert!(pd.find_capability(slot as u32).is_none(), "BOOT_REGION_SLOT_OCCUPIED");
    sex_pdx::memory_region::resolve_info(Some(&region)).expect("BOOT_REGION_INVALID_GRANT");
    pd.grant_capability(slot, CapabilityData::Region(region));
    crate::serial_println!("[boot.memory.region] pd={} slot={} region={} bytes={} rights={} peer={}",
        pd.id, slot, region.region_id, region.length, region.rights, region.peer_pd.unwrap_or(0));
}

fn dma(pd_id: u32, region_id: u32, length: u64) {
    dma_at(pd_id, sex_pdx::SLOT_DMA_REGION, region_id, length);
}

fn dma_at(pd_id: u32, slot: u64, region_id: u32, length: u64) {
    if pd_id == 0 { return; }
    let pd = DOMAIN_REGISTRY.get(pd_id).expect("BOOT_DMA_PD_MISSING");
    let physical = backing(length);
    let virtual_base = alias(pd, physical, length, true, false);
    grant(pd, slot, RegionGrant { region_id, virtual_base, length,
        rights: QUERY_VIEW | READ | WRITE | DMA_ADDRESS, dma_base: Some(physical), peer_pd: None });
}

fn paired(first_id: u32, second_id: u32, slot: u64, region_id: u32, length: u64, second_writable: bool) {
    paired_at(first_id, second_id, slot, slot, region_id, length, second_writable);
}

fn paired_at(first_id: u32, second_id: u32, first_slot: u64, second_slot: u64,
    region_id: u32, length: u64, second_writable: bool)
{
    require_boot();
    if first_id == 0 || second_id == 0 { return; }
    let first = DOMAIN_REGISTRY.get(first_id).expect("BOOT_SHARED_FIRST_MISSING");
    let second = DOMAIN_REGISTRY.get(second_id).expect("BOOT_SHARED_SECOND_MISSING");
    let physical = backing(length);
    let first_va = alias(first, physical, length, true, false);
    let second_va = alias(second, physical, length, second_writable, false);
    grant(first, first_slot, RegionGrant { region_id, virtual_base: first_va, length,
        rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: Some(second.id) });
    grant(second, second_slot, RegionGrant { region_id, virtual_base: second_va, length,
        rights: QUERY_VIEW | READ | if second_writable { WRITE } else { 0 },
        dma_base: None, peer_pd: Some(first.id) });
}

/// One bounded Display-write/first-Spindle-read screenshot bank; BOOT only.
pub fn init_screen_capture(display:u32){
    use sex_pdx::screen_capture as c;
    if DOMAIN_REGISTRY.get(c::CLIENT).is_some(){paired_at(display,c::CLIENT,c::SLOT,c::SLOT,c::REGION_ID,c::BYTES as u64,false);}
}

/// Immutable Display-only frame storage, reserved at geometry-known genesis.
/// One buddy reservation contains two page-aligned, disjoint frame spans.
/// This is PD-held memory authority; tasks within the PD share its access.
pub fn init_display_frames(display: u32, layout: sex_pdx::display_frames::Layout) {
    use sex_pdx::display_frames::{SLOT, REGION_ID};
    require_boot();
    SCREEN_PIXELS.store(((layout.width() as u64) << 32) | layout.height() as u64, core::sync::atomic::Ordering::Relaxed);
    let pd = DOMAIN_REGISTRY.get(display).expect("BOOT_DISPLAY_FRAME_DOMAIN");
    let main = pd.main_task.load(core::sync::atomic::Ordering::Acquire);
    assert!(!main.is_null(), "BOOT_DISPLAY_FRAME_MAIN");
    assert_eq!(unsafe { (*main).context.pd_ptr }, pd as *const _, "BOOT_DISPLAY_FRAME_CONTEXT");
    let task = unsafe { (*main).id };
    assert!(task != 0, "BOOT_DISPLAY_FRAME_TASK");
    assert!(pd.find_capability(SLOT as u32).is_none(), "BOOT_DISPLAY_FRAME_SLOT_OCCUPIED");
    let physical = backing(layout.total_bytes());
    let virtual_base = alias(pd, physical, layout.total_bytes(), true, false);
    let region = RegionGrant { region_id: REGION_ID, virtual_base, length: layout.total_bytes(),
        rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: None };
    grant(pd, SLOT, region);
    let info = sex_pdx::memory_region::resolve_info(Some(&region)).expect("BOOT_DISPLAY_FRAME_INFO");
    let view = sex_pdx::memory_region::resolve_view(Some(&region), sex_pdx::memory_region::ViewRequest {
        offset: 0, length: layout.total_bytes(), rights: QUERY_VIEW | READ | WRITE,
    }).expect("BOOT_DISPLAY_FRAME_VIEW");
    let checked = unsafe { sex_pdx::display_frames::FrameBacking::from_resolved(layout, info, view) }
        .expect("BOOT_DISPLAY_FRAME_DESCRIPTOR");
    assert_eq!(checked.address(0), Some(virtual_base));
    assert_eq!(checked.address(1), virtual_base.checked_add(layout.span_bytes()));
    match pd.find_capability(SLOT as u32).map(|c| c.data) {
        Some(CapabilityData::Region(g)) => assert_eq!(g, region, "BOOT_DISPLAY_FRAME_READBACK"),
        _ => panic!("BOOT_DISPLAY_FRAME_READBACK"),
    }
    assert_eq!(crate::syscalls::region_query(pd, 55, SLOT, 0, 0, 0),
        [0, layout.total_bytes(), (QUERY_VIEW | READ | WRITE) as u64, 0, REGION_ID as u64],
        "BOOT_DISPLAY_FRAME_INFO_DISPATCH");
    assert_eq!(crate::syscalls::region_query(pd, 54, SLOT, 0, layout.total_bytes(),
        (QUERY_VIEW | READ | WRITE) as u64),
        [0, virtual_base, layout.total_bytes(), 0, REGION_ID as u64], "BOOT_DISPLAY_FRAME_VIEW_DISPATCH");
    let denied = [u64::MAX, 0, 0, 0, 0];
    for (slot, offset, length, rights) in [
        ((1u64 << 32) | SLOT, 0, layout.total_bytes(), QUERY_VIEW | READ | WRITE),
        (SLOT, layout.total_bytes(), 1, QUERY_VIEW | READ),
        (SLOT, 0, layout.total_bytes(), QUERY_VIEW | READ | WRITE | DMA_ADDRESS),
    ] {
        assert_eq!(crate::syscalls::region_query(pd, 54, slot, offset, length, rights as u64), denied,
            "BOOT_DISPLAY_FRAME_NEGATIVE_DISPATCH");
    }
    for id in 1..crate::ipc::MAX_DOMAINS as u32 {
        if id == display { continue; }
        if let Some(other) = DOMAIN_REGISTRY.get(id) {
            assert_eq!(crate::syscalls::region_query(other, 54, SLOT, 0, layout.total_bytes(),
                (QUERY_VIEW | READ | WRITE) as u64), denied, "BOOT_DISPLAY_FRAME_FOREIGN_DISPATCH");
        }
    }
    // Verify actual backing and GLOBAL_VAS leaf permissions before sealing.
    let mut offset = 0;
    while offset < layout.total_bytes() {
        let address = virtual_base.checked_add(offset).expect("BOOT_DISPLAY_FRAME_ADDRESS");
        let leaf = existing_leaf_address(address).expect("BOOT_DISPLAY_FRAME_LEAF");
        let entry = unsafe { core::ptr::read_volatile(leaf as *const u64) };
        assert!(entry & 7 == 7 && entry & (1 << 63) != 0
            && ((entry >> 59) & 15) == pd.pku_key as u64, "BOOT_DISPLAY_FRAME_PERMISSIONS");
        for word in 0..512 {
            assert_eq!(unsafe { core::ptr::read_volatile((address + word * 8) as *const u64) }, 0,
                "BOOT_DISPLAY_FRAME_NOT_ZERO");
        }
        offset += 4096;
    }
    crate::serial_println!("[boot.display.frames] pd={} task={} slot={} region={} width={} height={} pitch={} frame_bytes={} span_bytes={} total_bytes={} reserved_bytes={} buffers=2 pkey={} private=1 zero=1 readback=1 sealed_later=1 runtime_mint=0",
        display, task, SLOT, REGION_ID, layout.width(), layout.height(), layout.pitch(),
        layout.frame_bytes(), layout.span_bytes(), layout.total_bytes(), layout.buddy_bytes(), pd.pku_key);
    // A virtio-gpu scans out from its own host resource; its backing is one more
    // packed frame the device may read (workstream L, slice L2).
    if matches!(pd.find_capability(sex_pdx::virtio::SLOT_GPU as u32).map(|c| c.data),
        Some(CapabilityData::Pci(d)) if d.vendor_id == sex_pdx::virtio::VENDOR && d.device_id == sex_pdx::virtio::GPU) {
        use sex_pdx::display_frames::{SCANOUT_SLOT, SCANOUT_REGION_ID};
        dma_at(display, SCANOUT_SLOT, SCANOUT_REGION_ID, layout.span_bytes());
        crate::serial_println!("[boot.display.scanout] pd={} slot={} region={:#x} bytes={} device=virtio-gpu runtime_mint=0",
            display, SCANOUT_SLOT, SCANOUT_REGION_ID, layout.span_bytes());
    }
}

/// The first UiIR stream: the producer may write its ring and only read the
/// consumer's cursor; the consumer may only read the ring and write its
/// cursor. Both are zeroed, exact-peer BOOT regions; nothing is minted later.
pub fn init_uiir(producer: u32, consumer: u32) {
    use sex_pdx::uiir::{RING_SLOT, CURSOR_SLOT, RING_REGION_ID, CURSOR_REGION_ID, RING_BYTES, CURSOR_BYTES};
    require_boot();
    if producer == 0 || consumer == 0 { return; }
    paired_at(producer, consumer, RING_SLOT, RING_SLOT, RING_REGION_ID, RING_BYTES as u64, false);
    paired_at(consumer, producer, CURSOR_SLOT, CURSOR_SLOT, CURSOR_REGION_ID, CURSOR_BYTES as u64, false);
    crate::serial_println!("[boot.uiir.stream] producer={} consumer={} ring_region={} cursor_region={} runtime_mint=0", producer, consumer, RING_REGION_ID, CURSOR_REGION_ID);
}

/// One app-writable, Display-readable text region per catalog row. Nothing is
/// minted later; a reused slot would mean the catalog and this graph disagree.
pub fn init_text_regions(display: u32) {
    use sex_pdx::text_region::{BYTES, CLIENT_SLOT, display_slot, region_id};
    require_boot();
    if display == 0 { return; }
    for (index, row) in sex_pdx::app_catalog::INCARNATIONS.iter().enumerate() {
        if DOMAIN_REGISTRY.get(row.target).is_none() { continue; }
        paired_at(row.target, display, CLIENT_SLOT, display_slot(index), region_id(index), BYTES, false);
    }
    crate::serial_println!("[boot.text.regions] display={} rows={} bytes={} runtime_mint=0", display, sex_pdx::app_catalog::COUNT, BYTES);
}

/// One pixel surface per program row: the program writes, Display reads.
pub fn init_pixel_regions(display: u32) {
    use sex_pdx::pixel_region::{bytes_for_screen, CLIENT_SLOT, display_slot, region_id};
    require_boot();
    if display == 0 { return; }
    let second = DOMAIN_REGISTRY.get(display).expect("BOOT_PIXEL_DISPLAY_MISSING");
    let programs = || sex_pdx::app_catalog::INCARNATIONS.iter().filter(|r| r.app == sex_pdx::app_catalog::App::Program);
    let rows = programs().count();
    // One buddy reservation per eight rows: per-row backing would spend a
    // boot frame reservation each and run the table out.
    const GROUP: usize = 8;
    let screen = SCREEN_PIXELS.load(core::sync::atomic::Ordering::Relaxed);
    let length = bytes_for_screen(screen >> 32, screen & 0xFFFF_FFFF);
    let mut base = 0u64;
    for (k, row) in programs().enumerate() {
        if k % GROUP == 0 { base = backing(length * GROUP.min(rows - k) as u64); }
        if row.target == 0 { continue; }
        let physical = base + (k % GROUP) as u64 * length;
        let first = DOMAIN_REGISTRY.get(row.target).expect("BOOT_PIXEL_PROGRAM_MISSING");
        let first_va = alias(first, physical, length, true, false);
        let second_va = alias(second, physical, length, false, false);
        grant(first, CLIENT_SLOT, RegionGrant { region_id: region_id(k), virtual_base: first_va, length,
            rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: Some(second.id) });
        grant(second, display_slot(k), RegionGrant { region_id: region_id(k), virtual_base: second_va, length,
            rights: QUERY_VIEW | READ, dma_base: None, peer_pd: Some(first.id) });
    }
    crate::serial_println!("[boot.pixel.regions] display={} bytes={} runtime_mint=0", display, length);
}

/// One sound ring per program row: the program writes sound, Bell mixes and plays it
/// and writes back how far it got.
pub fn init_sound_rings(bell: u32) {
    use sex_pdx::sound_ring::{BYTES, CLIENT_SLOT, bell_slot, region_id};
    require_boot();
    if bell == 0 { return; }
    let second = DOMAIN_REGISTRY.get(bell).expect("BOOT_SOUND_BELL_MISSING");
    let programs = || sex_pdx::app_catalog::INCARNATIONS.iter().filter(|r| r.app == sex_pdx::app_catalog::App::Program);
    let rows = programs().count();
    // One reservation per eight rows, as for pixel surfaces.
    const GROUP: usize = 8;
    let mut base = 0u64;
    for (k, row) in programs().enumerate() {
        if k % GROUP == 0 { base = backing(BYTES * GROUP.min(rows - k) as u64); }
        if row.target == 0 { continue; }
        let physical = base + (k % GROUP) as u64 * BYTES;
        let first = DOMAIN_REGISTRY.get(row.target).expect("BOOT_SOUND_PROGRAM_MISSING");
        let first_va = alias(first, physical, BYTES, true, false);
        let second_va = alias(second, physical, BYTES, true, false);
        grant(first, CLIENT_SLOT, RegionGrant { region_id: region_id(k), virtual_base: first_va, length: BYTES,
            rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: Some(second.id) });
        grant(second, bell_slot(k), RegionGrant { region_id: region_id(k), virtual_base: second_va, length: BYTES,
            rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: Some(first.id) });
    }
    crate::serial_println!("[boot.sound.rings] bell={} rows={} bytes={} runtime_mint=0", bell, rows, BYTES);
}

/// Permanent BOOT budget, separate from reclaimable app-private pages. Each
/// successor gets different backing and an immutable, exact provider peer.
/// Runtime Load still uses scalar transport until snapshot publication exists.
pub fn init_documents(files: u32, clients: [u32; sex_pdx::document_buffer::CLIENT_COUNT]) {
    use sex_pdx::document_buffer::{BYTES, CLIENT_SLOT, region_id, PROVIDER_SLOTS};
    require_boot();
    if files == 0 { return; }
    // Validate all subjects/slots before the first reservation or grant.
    let provider = DOMAIN_REGISTRY.get(files).expect("BOOT_DOCUMENT_PROVIDER_MISSING");
    for (index, client_id) in clients.into_iter().enumerate() {
        if client_id == 0 { continue; }
        assert!(client_id != files && !clients[..index].contains(&client_id), "BOOT_DOCUMENT_PEER_ALIAS");
        let client = DOMAIN_REGISTRY.get(client_id).expect("BOOT_DOCUMENT_CLIENT_MISSING");
        assert!(provider.find_capability(PROVIDER_SLOTS[index] as u32).is_none()
            && client.find_capability(CLIENT_SLOT as u32).is_none(), "BOOT_DOCUMENT_SLOT_OCCUPIED");
    }
    for (index, client_id) in clients.into_iter().enumerate() {
        paired_at(files, client_id, PROVIDER_SLOTS[index], CLIENT_SLOT,
            region_id(index), BYTES, false);
    }
    document_selftest(files, clients);
}

fn document_selftest(files: u32, clients: [u32; sex_pdx::document_buffer::CLIENT_COUNT]) {
    use sex_pdx::document_buffer::{BYTES, CLIENT_SLOT, region_id, PROVIDER_SLOTS};
    let provider = DOMAIN_REGISTRY.get(files).expect("DOCUMENT_TEST_PROVIDER_MISSING");
    let fail = [u64::MAX, 0, 0, 0, 0];
    let mut views = [(0u64, 0u64); sex_pdx::document_buffer::CLIENT_COUNT];
    let mut pairs = 0;
    for (index, client_id) in clients.into_iter().enumerate() {
        if client_id == 0 { continue; }
        let client = DOMAIN_REGISTRY.get(client_id).expect("DOCUMENT_TEST_CLIENT_MISSING");
        let slot = PROVIDER_SLOTS[index];
        let id = region_id(index) as u64;
        assert_eq!(crate::syscalls::region_query(provider, 55, slot, 0, 0, 0),
            [0, BYTES, (QUERY_VIEW | READ | WRITE) as u64, client_id as u64, id]);
        assert_eq!(crate::syscalls::region_query(client, 55, CLIENT_SLOT, 0, 0, 0),
            [0, BYTES, (QUERY_VIEW | READ) as u64, files as u64, id]);
        let writer = crate::syscalls::region_query(provider, 54, slot, 0, BYTES, (QUERY_VIEW | READ | WRITE) as u64);
        let reader = crate::syscalls::region_query(client, 54, CLIENT_SLOT, 0, BYTES, (QUERY_VIEW | READ) as u64);
        assert_eq!([writer[0], writer[2], writer[3], writer[4]], [0, BYTES, 0, id]);
        assert_eq!([reader[0], reader[2], reader[3], reader[4]], [0, BYTES, 0, id]);
        assert_ne!(writer[1], reader[1], "DOCUMENT_TEST_DOMAIN_ALIAS");
        for rights in [QUERY_VIEW | WRITE, QUERY_VIEW | DMA_ADDRESS] {
            assert_eq!(crate::syscalls::region_query(client, 54, CLIENT_SLOT, 0, BYTES, rights as u64), fail);
        }
        assert_eq!(crate::syscalls::region_query(provider, 54, slot, 0, BYTES, (QUERY_VIEW | DMA_ADDRESS) as u64), fail);
        for wrong_slot in PROVIDER_SLOTS.into_iter().filter(|slot| *slot != CLIENT_SLOT) {
            assert_eq!(crate::syscalls::region_query(client, 55, wrong_slot, 0, 0, 0), fail);
        }
        assert_eq!(crate::syscalls::region_query(client, 54, CLIENT_SLOT, BYTES - 1, 2, (QUERY_VIEW | READ) as u64), fail);
        // Check the actual GLOBAL_VAS leaf permissions as well as authority
        // replies. No second address space or client-side writable alias exists.
        for page in (0..BYTES).step_by(4096) {
            let leaf = existing_leaf_address(reader[1] + page).expect("DOCUMENT_TEST_LEAF_MISSING");
            let entry = unsafe { core::ptr::read_volatile(leaf as *const u64) };
            assert_eq!(entry & 7, 5, "DOCUMENT_TEST_CLIENT_LEAF_WRITABLE");
            assert_ne!(entry & (1u64 << 63), 0, "DOCUMENT_TEST_CLIENT_LEAF_EXECUTABLE");
            assert_eq!((entry >> 59) & 15, client.pku_key as u64, "DOCUMENT_TEST_CLIENT_LEAF_KEY");
        }
        views[index] = (writer[1], reader[1]);
        // Different full-buffer patterns survive subsequent peer publication.
        unsafe { core::ptr::write_bytes(writer[1] as *mut u8, index as u8 + 1, BYTES as usize); }
        pairs += 1;
    }
    for (index, &(writer, reader)) in views.iter().enumerate() {
        if writer == 0 { continue; }
        for offset in 0..BYTES {
            assert_eq!(unsafe { core::ptr::read_volatile((reader + offset) as *const u8) }, index as u8 + 1,
                "DOCUMENT_TEST_CROSS_INCARNATION_ALIAS");
        }
        unsafe { core::ptr::write_bytes(writer as *mut u8, 0, BYTES as usize); }
    }
    for &(writer, reader) in &views {
        if writer == 0 { continue; }
        for offset in 0..BYTES {
            assert_eq!(unsafe { core::ptr::read_volatile((reader + offset) as *const u8) }, 0, "DOCUMENT_TEST_SCRUB");
        }
    }
    crate::serial_println!("[boot.document.views] pairs={} bytes_each={} total_bytes={} readonly=1 distinct=1 scrubbed=1 runtime_mint=0", pairs, BYTES, pairs * BYTES);
}

pub fn init(drive: u32, display: u32, usb: u32, net: u32, files: u32) {
    require_boot();
    // Fixed BOOT_DAG budget, not runtime allocation policy. Drivers may reserve
    // byte subregions within these grants; they cannot create frame authority.
    // SexUSB: rings, contexts and report pages for up to 32 devices, plus the
    // scratchpad pages real xHCI controllers ask for.
    dma(usb, 1, 2 * 1024 * 1024);
    dma(net, sex_pdx::pci_command::DMA_REGION_ID, sex_pdx::pci_command::DMA_BYTES);
    dma(drive, 3, 512 * 1024);
    paired(files, drive, sex_pdx::SLOT_BLOCK_BUFFER, 4, sex_pdx::BLOCK_MAX_RUN, true);
    // USB sticks: sectors move between SexUSB and Files through this region.
    paired(files, usb, sex_pdx::usb_disk::BUFFER_SLOT, sex_pdx::usb_disk::REGION_ID, sex_pdx::usb_disk::BUFFER_BYTES, true);
    // USB network adapters: frames move between SexUSB and SexNet through this region.
    paired(net, usb, sex_pdx::usb_net::SLOT, sex_pdx::usb_net::REGION_ID, sex_pdx::usb_net::BYTES, true);
    // Files' caches and free-space maps are sized by the disk it finds.
    crate::service_memory::reserve(files);
    // The network service makes a flow (window, queue, TLS buffers) for each connection
    // programs open, so its memory grows with them.
    crate::service_memory::reserve(net);
    paired(drive, display, sex_pdx::SLOT_FRAME_REGION, 5, 3 * 1024 * 1024, false);
    if drive != 0 && files != 0 {
        negative_selftest(DOMAIN_REGISTRY.get(drive).unwrap(), DOMAIN_REGISTRY.get(files).unwrap());
    }
    if net != 0 {
        let pd=DOMAIN_REGISTRY.get(net).expect("BOOT_NIC_PD_MISSING");
        let fail=[sex_pdx::pci_command::DENIED,0,0,0,0];let mut denied=0;
        for (slot,set,clear,a,b) in [(0,0,0,0,0),((1u64<<32)|sex_pdx::SLOT_NIC,0,0,0,0),
            (sex_pdx::SLOT_NIC,1,0,0,0),(sex_pdx::SLOT_NIC,1u64<<32,0,0,0),
            (sex_pdx::SLOT_NIC,2,2,0,0),(sex_pdx::SLOT_NIC,0,0,1,0),(sex_pdx::SLOT_NIC,0,0,0,1)] {
            assert_eq!(crate::pci_command::invoke(pd,slot,set,clear,a,b),fail);denied+=1;
        }
        if files != 0 {
            let other=DOMAIN_REGISTRY.get(files).unwrap();
            for slot in [sex_pdx::SLOT_NIC,sex_pdx::SLOT_BLOCK] {
                assert_eq!(crate::pci_command::invoke(other,slot,0,0,0,0),fail);denied+=1;
            }
        }
        crate::serial_println!("[pci.command.negative.selftest] denied={} output_zero=1 runtime_mint=0 ok=1",denied);
    }
}

fn negative_selftest(pd: &ProtectionDomain, route_holder: &ProtectionDomain) {
    let slot = sex_pdx::SLOT_DMA_REGION;
    let before = crate::syscalls::region_query(pd, 55, slot, 0, 0, 0);
    assert_eq!(before[0], 0, "REGION_TEST_MISSING_DMA");
    let fail = [u64::MAX, 0, 0, 0, 0];
    let mut checked = 0;
    for number in [54, 55] {
        for bad_slot in [0, 1025, (1u64 << 32) | slot, u64::MAX] {
            assert_eq!(crate::syscalls::region_query(pd, number, bad_slot, 0, 4096, QUERY_VIEW as u64), fail);
            checked += 1;
        }
    }
    // Mandatory wrong-kind checks use sexfiles' real block route. Drive has no
    // shell route, and its PCI caps are valid views, so neither tests wrong kind.
    assert!(matches!(route_holder.find_capability(sex_pdx::SLOT_BLOCK as u32).map(|cap| cap.data),
        Some(CapabilityData::Domain(target)) if target == pd.id), "REGION_TEST_WRONG_KIND_ROUTE_MISSING");
    for number in [54, 55] {
        assert_eq!(crate::syscalls::region_query(route_holder, number, sex_pdx::SLOT_BLOCK, 0, 4096, QUERY_VIEW as u64), fail);
        checked += 1;
    }
    for (offset, length, rights) in [
        (0, 0, QUERY_VIEW as u64), (before[1], 1, QUERY_VIEW as u64),
        (u64::MAX, 2, QUERY_VIEW as u64), (0, 4096, 0x81),
        (0, 4096, (1u64 << 32) | QUERY_VIEW as u64),
    ] {
        assert_eq!(crate::syscalls::region_query(pd, 54, slot, offset, length, rights), fail);
        checked += 1;
    }
    assert_eq!(crate::syscalls::region_query(pd, 55, slot, 0, 0, 0), before);
    let first = crate::syscalls::region_query(pd, 54, slot, 0, 4096, QUERY_VIEW as u64);
    assert_eq!(first[0], 0);
    assert_eq!(first[3], 0); // Ordinary view discloses no physical address.
    for _ in 0..1024 { assert_eq!(crate::syscalls::region_query(pd, 54, slot, 0, 4096, QUERY_VIEW as u64), first); }
    crate::serial_println!("[memory.authority.negative.selftest] denied={} idempotent=1024 output_zero=1 ok=1", checked);
}

pub fn pci_bar0(pd: &ProtectionDomain, dev: &crate::hal::pci::PciDevice, slot: u64) -> RegionGrant {
    require_boot();
    let (physical, actual_length) = dev.probe_boot_bar0().expect("BOOT_MMIO_INVALID_BAR");
    // The active drivers need at most 64 KiB; a larger BAR grants only this prefix.
    let length = actual_length.min(64 * 1024);
    pci_view(pd,slot,physical,actual_length,length)
}

/// Only a supported BOOT-selected controller with a complete register prefix.
/// Reject before aliasing; probing itself remains guarded by BOOT RegistryBuild.
pub fn nic_bar0(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64)->Option<RegionGrant>{
    nic_lease(pd,dev,slot).map(|(view,_)|view)
}

/// The NIC's register view and where its registers sit inside it.
pub fn nic_lease(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64)->Option<(RegionGrant,(u32,u32))>{
    require_boot();
    if !crate::nic_boot::supported(dev.vendor_id,dev.device_id){return None}
    // The first memory BAR this controller keeps its registers in.
    let(bar,physical,actual_length)=crate::nic_boot::bars(dev.vendor_id,dev.device_id).iter()
        .find_map(|&bar|dev.probe_boot_bar_any(bar).map(|(p,l)|(bar,p,l)))?;
    let end=crate::nic_boot::registers_end(dev.vendor_id,dev.device_id);
    bar_lease(pd,dev,slot,bar,physical,actual_length,end)
}

/// One BAR as a page-granular view. A BAR of a page or more is leased from its
/// start (at most 64 KiB). A smaller BAR is leased as its whole page only when no
/// other BAR anywhere decodes in that page; the window says where it sits.
fn bar_lease(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,bar:u8,physical:u64,actual_length:u64,end:u64)
    ->Option<(RegionGrant,(u32,u32))>
{
    require_boot();
    if actual_length<end{return None}
    if actual_length>=4096 {
        let length=actual_length.min(64*1024);
        return Some((pci_view(pd,slot,physical,actual_length,length),(0,length as u32)));
    }
    let page=physical&!0xfff;
    if !crate::hal::pci::page_exclusive(page,dev.bus,dev.dev,dev.func,bar){
        crate::serial_println!("[boot.memory.mmio.shared_page] pd={} slot={} bar={} bytes={}",pd.id,slot,bar,actual_length);
        return None;
    }
    Some((pci_view(pd,slot,page,actual_length,4096),((physical-page) as u32,actual_length as u32)))
}

pub fn nic_command(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,view:RegionGrant)
    -> sex_pdx::pci_command::Grant
{
    require_boot();
    let extra=slot>=sex_pdx::device::EXTRA_SLOT_BASE&&slot<sex_pdx::device::EXTRA_SLOT_BASE+sex_pdx::device::EXTRA_SLOTS;
    assert!((slot == sex_pdx::SLOT_NIC || extra) && crate::nic_boot::supported(dev.vendor_id,dev.device_id), "BOOT_NIC_COMMAND_SUBJECT");
    assert_eq!(view.region_id,0x10000 + pd.id * 1024 + slot as u32,"BOOT_NIC_COMMAND_VIEW_ID");
    assert!(sex_pdx::memory_region::resolve_info(Some(&view)).is_ok()
        && view.length >= crate::nic_boot::registers_end(dev.vendor_id,dev.device_id) && view.length <= 65536
        && view.rights == QUERY_VIEW|READ|WRITE && view.dma_base.is_none() && view.peer_pd.is_none(), "BOOT_NIC_COMMAND_VIEW");
    let g=sex_pdx::pci_command::Grant {subject:pd.id,bus:dev.bus,device:dev.dev,function:dev.func,
        vendor_id:dev.vendor_id,device_id:dev.device_id,allowed:sex_pdx::pci_command::NIC_BITS};
    assert!(sex_pdx::pci_command::authorize(Some(&g),pd.id,0,0).is_ok(),"BOOT_NIC_COMMAND_GRANT");
    crate::serial_println!("[boot.nic.command] pd={} slot={} bus={} device={} function={} vendor={} product={} allowed={} dma_region={} runtime_mint=0",
        pd.id,slot,g.bus,g.device,g.function,g.vendor_id,g.device_id,g.allowed,sex_pdx::pci_command::DMA_REGION_ID);
    g
}

/// The one BAR holding a supported virtio device's register blocks, if the
/// whole layout fits the leased prefix.
pub fn virtio_bar(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,bar:u8,layout:&sex_pdx::virtio::Layout)->Option<RegionGrant>{
    require_boot();
    if !crate::virtio_boot::supported(dev.vendor_id,dev.device_id){return None}
    let(physical,actual_length)=dev.probe_boot_bar(bar)?;
    let length=actual_length.min(sex_pdx::virtio::VIEW_MAX);
    if !layout.valid(length){return None}
    Some(pci_view(pd,slot,physical,actual_length,length))
}

pub fn virtio_command(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,view:RegionGrant,layout:&sex_pdx::virtio::Layout)
    -> sex_pdx::pci_command::Grant
{
    require_boot();
    assert!(crate::virtio_boot::supported(dev.vendor_id,dev.device_id),"BOOT_VIRTIO_COMMAND_SUBJECT");
    assert_eq!(view.region_id,0x10000 + pd.id * 1024 + slot as u32,"BOOT_VIRTIO_COMMAND_VIEW_ID");
    assert!(sex_pdx::memory_region::resolve_info(Some(&view)).is_ok() && layout.valid(view.length)
        && view.rights == QUERY_VIEW|READ|WRITE && view.dma_base.is_none() && view.peer_pd.is_none(), "BOOT_VIRTIO_COMMAND_VIEW");
    let g=sex_pdx::pci_command::Grant {subject:pd.id,bus:dev.bus,device:dev.dev,function:dev.func,
        vendor_id:dev.vendor_id,device_id:dev.device_id,allowed:sex_pdx::pci_command::NIC_BITS};
    assert!(sex_pdx::pci_command::authorize(Some(&g),pd.id,0,0).is_ok(),"BOOT_VIRTIO_COMMAND_GRANT");
    crate::serial_println!("[boot.virtio.command] pd={} slot={} bus={} device={} function={} product={:#x} allowed={} dma_region={:#x} runtime_mint=0",
        pd.id,slot,g.bus,g.device,g.function,g.device_id,g.allowed,sex_pdx::virtio::DMA_REGION_ID);
    g
}

/// The DMA pool a virtio driver's queues and bounce buffers live in.
pub fn virtio_dma(pd_id:u32){
    require_boot();
    dma(pd_id,sex_pdx::virtio::DMA_REGION_ID,sex_pdx::virtio::DMA_BYTES);
}

/// The display-register prefix of an Intel GPU's BAR0 (GTTMMADR), if present.
pub fn intel_display_bar(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64)->Option<RegionGrant>{
    require_boot();
    let(physical,actual_length)=dev.probe_boot_bar_any(0)?;
    let length=sex_pdx::display_frames::INTEL_REGISTERS;
    if actual_length<length{return None}
    Some(pci_view(pd,slot,physical,actual_length,length))
}

/// The register BAR of a controller leased by class (HD Audio, AHCI), if its
/// whole register prefix fits.
pub fn class_bar(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,kind:crate::device_lease::Kind)->Option<(RegionGrant,(u32,u32))>{
    require_boot();
    let(physical,actual_length)=dev.probe_boot_bar_any(kind.bar())?;
    bar_lease(pd,dev,slot,kind.bar(),physical,actual_length,kind.registers_end())
}

/// Memory-space and bus-master bits for a class-leased controller, like the NIC's.
pub fn class_command(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,view:RegionGrant,kind:crate::device_lease::Kind)
    -> sex_pdx::pci_command::Grant
{
    require_boot();
    assert_eq!(view.region_id,0x10000 + pd.id * 1024 + slot as u32,"BOOT_CLASS_COMMAND_VIEW_ID");
    assert!(sex_pdx::memory_region::resolve_info(Some(&view)).is_ok()
        && view.length >= kind.registers_end() && view.length <= 65536
        && view.rights == QUERY_VIEW|READ|WRITE && view.dma_base.is_none() && view.peer_pd.is_none(), "BOOT_CLASS_COMMAND_VIEW");
    let g=sex_pdx::pci_command::Grant {subject:pd.id,bus:dev.bus,device:dev.dev,function:dev.func,
        vendor_id:dev.vendor_id,device_id:dev.device_id,allowed:sex_pdx::pci_command::NIC_BITS};
    assert!(sex_pdx::pci_command::authorize(Some(&g),pd.id,0,0).is_ok(),"BOOT_CLASS_COMMAND_GRANT");
    let (region,bytes)=kind.dma();
    crate::serial_println!("[boot.class.command] pd={} slot={} kind={:?} bus={} device={} function={} vendor={:#x} product={:#x} allowed={} dma_region={:#x} dma_bytes={} runtime_mint=0",
        pd.id,slot,kind,g.bus,g.device,g.function,g.vendor_id,g.device_id,g.allowed,region,bytes);
    g
}

/// The DMA pool a class-leased driver's rings and buffers live in. The drive
/// service already holds its pool from `init`.
pub fn class_dma(pd_id:u32,kind:crate::device_lease::Kind){
    require_boot();
    if kind==crate::device_lease::Kind::Ahci { return; }
    let (region,bytes)=kind.dma();
    dma_at(pd_id,kind.dma_slot(),region,bytes);
    if kind==crate::device_lease::Kind::Wifi { wifi_firmware(pd_id); }
}

/// The Wi-Fi firmware pack (a boot module) as a read-only region of the service
/// that holds the chip. The kernel never looks inside; the driver checks each file.
fn wifi_firmware(pd_id:u32){
    use sex_pdx::pci_command as w;
    require_boot();
    let Some(module) = crate::MODULE_REQUEST.response().and_then(|m| m.modules().iter().find(|m| m.path().ends_with(w::WIFI_FIRMWARE_MODULE))) else {
        crate::serial_println!("[boot.wifi.firmware.absent] pd={}", pd_id); return
    };
    let hhdm = crate::HHDM_REQUEST.response().expect("BOOT_WIFI_FIRMWARE_NO_HHDM").offset;
    let data = module.data();
    let virt = data.as_ptr() as u64;
    let length = (data.len() as u64).next_multiple_of(4096);
    if virt % 4096 != 0 || length == 0 { return; }
    let pd = DOMAIN_REGISTRY.get(pd_id).expect("BOOT_WIFI_FIRMWARE_PD");
    let virtual_base = alias(pd, virt - hhdm, length, false, false);
    grant(pd, w::SLOT_WIFI_FIRMWARE, RegionGrant { region_id: w::WIFI_FIRMWARE_REGION_ID, virtual_base, length,
        rights: QUERY_VIEW | READ, dma_base: None, peer_pd: None });
    crate::serial_println!("[boot.wifi.firmware.region] pd={} bytes={}", pd_id, data.len());
}

fn pci_view(pd:&ProtectionDomain,slot:u64,physical:u64,actual_length:u64,length:u64)->RegionGrant{
    require_boot();
    let virtual_base = alias(pd, physical, length, true, true);
    let region = RegionGrant { region_id: 0x10000 + pd.id * 1024 + slot as u32,
        virtual_base, length, rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: None };
    sex_pdx::memory_region::resolve_info(Some(&region)).expect("BOOT_MMIO_INVALID_VIEW");
    crate::serial_println!("[boot.memory.mmio] pd={} slot={} bytes={} actual_bytes={}", pd.id, slot, length, actual_length);
    region
}

pub fn seal_all() {
    require_boot();
    for id in 1..crate::ipc::MAX_DOMAINS as u32 {
        if let Some(pd) = DOMAIN_REGISTRY.get(id) {
            unsafe { (*pd.cap_table).seal(); }
        }
    }
    crate::serial_println!("[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0");
}

/// Prepare one bounded paging arena. The runtime commit mechanism receives
/// exact existing leaf addresses and never invokes a frame/VA/table allocator.
pub fn init_paging(pager_pd: u32, client_pds: [u32; 2]) {
    use crate::paging_state::{FramePoolGrant,PagerMappingGrant,DemandRegionGrant,PAGE,PAGES,CLIENTS};
    use x86_64::structures::paging::{Mapper,Page,Size4KiB};
    require_boot();
    assert!(pager_pd != 0 && client_pds[0] != client_pds[1],"PAGING_BOOT_DOMAINS");
    let pager = DOMAIN_REGISTRY.get(pager_pd).expect("PAGING_BOOT_PAGER_MISSING");
    let length = PAGE * PAGES as u64;
    let physical = backing(length);
    let pool_va = alias(pager,physical,length,true,false);
    let pool = FramePoolGrant {pool_id:1,base_va:pool_va,length};
    let mut mappings = [PagerMappingGrant {range_id:0,pool_id:1,target_pd:0,base_va:0,length,rights:READ|WRITE,quota:4};CLIENTS];
    let mut pkeys = [0u8;CLIENTS];
    let mut leaves = [[0u64;PAGES];CLIENTS];
    for (index,client_pd) in client_pds.into_iter().enumerate() {
        assert!(client_pd != 0 && client_pd != pager_pd,"PAGING_BOOT_DOMAINS");
        let client = DOMAIN_REGISTRY.get(client_pd).expect("PAGING_BOOT_CLIENT_MISSING");
        assert!(client.pku_key != pager.pku_key && !pkeys[..index].contains(&client.pku_key),"PAGING_BOOT_PKEY_ALIAS");
        pkeys[index] = client.pku_key;
        // These temporary aliases precreate intermediate infrastructure only.
        // Every target leaf is absent before any domain begins execution.
        let target_va = alias(client,physical,length,true,false);
        {
            let mut vas = crate::memory::manager::GLOBAL_VAS.lock();
            let vas = vas.as_mut().expect("PAGING_BOOT_VAS_MISSING");
            for (page_index,leaf) in leaves[index].iter_mut().enumerate() {
                let address = target_va + page_index as u64 * PAGE;
                vas.mapper.unmap(Page::<Size4KiB>::containing_address(VirtAddr::new(address)))
                    .expect("PAGING_BOOT_UNMAP_FAILED").1.flush();
                *leaf = existing_leaf_address(address).expect("PAGING_BOOT_LEAF_MISSING");
                assert_eq!(unsafe { core::ptr::read_volatile(*leaf as *const u64) },0,"PAGING_BOOT_LEAF_PRESENT");
            }
        }
        let mapping = PagerMappingGrant {range_id:index as u32+1,pool_id:1,target_pd:client_pd,
            base_va:target_va,length,rights:READ|WRITE,quota:4};
        mappings[index] = mapping;
        crate::scheduler::register_paging_exit_watch(client_pd,pager_pd);
        let region = DemandRegionGrant {range_id:mapping.range_id,base_va:target_va,length,rights:READ|WRITE};
        let map_slot = if index == 0 { sex_pdx::paging::SLOT_PAGER_MAPPING } else { sex_pdx::paging::SLOT_PAGER_MAPPING_SECOND };
        for (pd,slot,data) in [
            (pager,map_slot,CapabilityData::PagerMapping(mapping)),
            (client,sex_pdx::paging::SLOT_DEMAND_REGION,CapabilityData::DemandRegion(region)),
        ] {
            assert!(pd.find_capability(slot as u32).is_none(),"PAGING_BOOT_SLOT_OCCUPIED");
            pd.grant_capability(slot,data);
        }
        crate::serial_println!("[boot.paging.genesis] pager={} target={} pool_pages={} arena_pages={} quota={} target_pkey={} leaves_absent=8 tables_preallocated=1",
            pager_pd,client_pd,PAGES,PAGES,mapping.quota,client.pku_key);
    }
    assert!(pager.find_capability(sex_pdx::paging::SLOT_FRAME_POOL as u32).is_none(),"PAGING_BOOT_SLOT_OCCUPIED");
    pager.grant_capability(sex_pdx::paging::SLOT_FRAME_POOL,CapabilityData::FramePool(pool));
    crate::paging::initialize(pool,mappings,pager_pd,pkeys,physical,leaves);
}

/// The page-directory entry over `address`'s 2 MiB span, if its page table exists.
pub(crate) fn span_entry(address: u64) -> Option<u64> {
    existing_leaf_address(address)?;
    use x86_64::registers::control::Cr3;
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    let mut table = Cr3::read().0.start_address().as_u64();
    for shift in [39,30] {
        table = unsafe { core::ptr::read_volatile((hhdm + table + ((address >> shift)&511)*8) as *const u64) } & 0x000f_ffff_ffff_f000;
    }
    Some(hhdm + table + ((address >> 21)&511)*8)
}

pub(crate) fn existing_leaf_address(address: u64) -> Option<u64> {
    use x86_64::registers::control::Cr3;
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    let mut table = Cr3::read().0.start_address().as_u64();
    for shift in [39,30,21] {
        let entry = unsafe { core::ptr::read_volatile((hhdm + table + ((address >> shift)&511)*8) as *const u64) };
        if entry & 1 == 0 || entry & (1<<7) != 0 { return None; }
        table = entry & 0x000f_ffff_ffff_f000;
    }
    Some(hhdm + table + ((address >> 12)&511)*8)
}
