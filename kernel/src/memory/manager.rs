use x86_64::{PhysAddr, VirtAddr, structures::paging::{PageTable, OffsetPageTable, PageTableFlags, FrameAllocator, Size4KiB, PhysFrame}};
use crate::pku;
use super::allocator;
use spin::Mutex;
use lazy_static::lazy_static;

pub struct BootInfoFrameAllocator {
    memory_map: &'static limine::request::MemmapResponse,
    pub(crate) next: usize,
    owned_regions: crate::boot_frame_reservations::BootFrameReservations,
}

impl BootInfoFrameAllocator {
    pub unsafe fn new(memory_map: &'static limine::request::MemmapResponse) -> Self {
        BootInfoFrameAllocator { memory_map, next: 0,
            owned_regions: crate::boot_frame_reservations::BootFrameReservations::new() }
    }

    /// Reject collision with the entire consumed prefix across usable memmap
    /// entries before data is zeroed; then exclude every page from future walks.
    pub fn reserve_boot_region(&mut self, physical: u64, length: u64) -> Result<(), &'static str> {
        let mut remaining = self.next as u64;
        let regions = self.memory_map.entries();
        self.owned_regions.reserve(physical, length, |range| {
            for region in regions.iter().filter(|r| r.type_ == 0) {
                let consumed = remaining.min(region.length / 4096);
                if consumed != 0 && range.overlaps(crate::boot_frame_reservations::Range {
                    start: region.base, end: region.base + consumed * 4096,
                }) { return true; }
                remaining -= consumed;
                if remaining == 0 { break; }
            }
            false
        })
    }

}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    /// O(n²) BUGFIX: usable_frames().nth(self.next) regenerates the iterator every call,
    /// causing ~2.1 billion traversals for 256 MiB heap (65536 pages) under QEMU.
    ///
    /// Fix: cursor-based allocation walks memory regions directly, O(num_regions) per call.
    /// Total: ~30 regions × 65536 calls ≈ 2M operations (vs 2.1B before).
    ///
    /// Removed usable_frames() helper — obsolete after this fix.
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        'next_candidate: loop {
        let regions = self.memory_map.entries();
        let mut frame_index = 0usize;
        for region in regions.iter().filter(|r| r.type_ == 0) {
            let frames_in_region = (region.length / 4096) as usize;
            if frame_index + frames_in_region > self.next {
                let offset = self.next.saturating_sub(frame_index) as u64;
                let phys_addr = region.base + offset * 4096;
                self.next += 1;
                if self.owned_regions.contains(phys_addr) { continue 'next_candidate; }
                // AP1 diagnostic: record boot-frame allocation for overlap detection.
                // PERF_LOG_NOISE_ABLATION_V1: first 4 lines, then power-of-two summaries
                // (67790 per-event lines dominated serial volume; diag_record + reserve
                // still run every call — logging gated only).
                let n = self.next as u64;
                if n <= 4 {
                    crate::serial_println!("[kernel.mem.boot_frame.alloc] phys={:#x} idx={}",
                        phys_addr, self.next);
                } else if n & (n - 1) == 0 {
                    crate::serial_println!("[perf.noise.summary] name=boot_frame.alloc count={} suppressed={}",
                        n, n - 4);
                }
                crate::memory::allocator::diag_record_boot_frame(phys_addr);
                // AP2 fix: reserve this frame in GLOBAL_ALLOCATOR metadata so the
                // buddy allocator cannot reissue page-table frames as data/DMA buffers.
                unsafe { crate::memory::allocator::GLOBAL_ALLOCATOR.reserve_frame(phys_addr); }
                return Some(PhysFrame::containing_address(PhysAddr::new(phys_addr)));
            }
            frame_index += frames_in_region;
        }
        return None;
        }
    }
}

pub struct GlobalVas {
    pub mapper: OffsetPageTable<'static>,
    pub frame_allocator: BootInfoFrameAllocator,
}

impl GlobalVas {
    pub fn map_pku_range(&mut self, va: VirtAddr, size: u64, flags: PageTableFlags, pkey: u8) -> Result<(), &'static str> {
        let start_page = x86_64::structures::paging::Page::<Size4KiB>::containing_address(va);
        let end_page = x86_64::structures::paging::Page::<Size4KiB>::containing_address(va + size - 1u64);

        for page in x86_64::structures::paging::Page::range_inclusive(start_page, end_page) {
            let frame = self.frame_allocator.allocate_frame().ok_or("Out of memory")?;
            unsafe {
                use x86_64::structures::paging::Mapper;
                self.mapper.map_to(page, frame, flags, &mut self.frame_allocator)
                    .map_err(|_| "Mapping failed")?.flush();
                pku::tag_virtual_address(page.start_address().as_u64(), pkey);
            }
        }
        Ok(())
    }

    pub fn map_physical_range(&mut self, va: VirtAddr, pa: u64, size: u64, flags: PageTableFlags, pkey: u8) -> Result<(), &'static str> {
        use x86_64::structures::paging::{Page, Mapper};
        let start_page = Page::<Size4KiB>::containing_address(va);
        let end_page   = Page::<Size4KiB>::containing_address(va + size - 1u64);
        let mut phys = pa & !0xFFF;
        for page in Page::range_inclusive(start_page, end_page) {
            let frame = PhysFrame::containing_address(PhysAddr::new(phys));
            unsafe {
                self.mapper.map_to(page, frame, flags, &mut self.frame_allocator)
                    .map_err(|_| "FB map failed")?.flush();
                pku::tag_virtual_address(page.start_address().as_u64(), pkey);
            }
            phys += 4096;
        }
        Ok(())
    }
}

lazy_static! {
    pub static ref GLOBAL_VAS: Mutex<Option<GlobalVas>> = Mutex::new(None);
}

pub fn init(memmap: &'static limine::request::MemmapResponse, hhdm_offset: u64) {
    crate::serial_println!("allocator.init.begin");
    let level_4_table = unsafe { active_level_4_table(VirtAddr::new(hhdm_offset)) };
    let mut mapper = unsafe { OffsetPageTable::new(level_4_table, VirtAddr::new(hhdm_offset)) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::new(memmap) };

    // Initialize the Rust heap (linked_list_allocator). Must happen before any Box/Vec.
    // frame_allocator.next advances past the heap pages, so GLOBAL_VAS gets a valid cursor.
    allocator::init_heap(&mut mapper, &mut frame_allocator)
        .unwrap_or_else(|_| {
            loop { core::hint::spin_loop(); }
        });

    let entries = memmap.entries();
    crate::serial_println!("allocator.memory_regions.count={}", entries.len());

    // AP1 diagnostic: log all memory map entries
    for entry in entries.iter() {
        let kind = match entry.type_ {
            0 => "usable",
            1 => "reserved",
            2 => "acpi_reclaimable",
            3 => "acpi_nvs",
            4 => "bad_memory",
            5 => "bootloader_reclaimable",
            6 => "kernel_and_modules",
            7 => "framebuffer",
            _ => "unknown",
        };
        crate::serial_println!("[kernel.mem.boot_frame.region] start={:#x} end={:#x} kind={}",
            entry.base, entry.base + entry.length, kind);
    }

    // Size metadata against highest *usable* physical address only.
    // Including non-usable high regions can over-allocate metadata and prevent allocator bootstrap.
    let mut max_usable_phys_addr = 0u64;
    for entry in entries.iter().filter(|e| e.type_ == 0) {
        let end = entry.base + entry.length;
        if end > max_usable_phys_addr {
            max_usable_phys_addr = end;
        }
    }
    let total_pages = (max_usable_phys_addr + 4095) / 4096;
    let metadata_bytes = total_pages * core::mem::size_of::<crate::memory::allocator::PageMetadata>() as u64;
    let metadata_pages = (metadata_bytes + 4095) / 4096;

    // Seed GLOBAL_ALLOCATOR (LockFreeBuddyAllocator) with remaining usable physical regions.
    // Use the actual BootInfoFrameAllocator cursor (frame_allocator.next), not a
    // HEAP_SIZE estimate, because init_heap may have allocated page-table frames via
    // mapper.map_to() that heap_pages didn't account for.  Using the accurate cursor
    // prevents GLOBAL_ALLOCATOR from receiving physical frames already handed out by
    // the boot frame allocator.
    let consumed_by_init = frame_allocator.next as u64;
    crate::serial_println!("[kernel.mem.global.skip.consumed_by_init] frames={}", consumed_by_init);
    let mut plan = crate::boot_frame_reservations::BootstrapPlanner::new(
        consumed_by_init, metadata_pages * 4096);
    let mut pending_regions = alloc::vec::Vec::new();
    for entry in entries.iter().filter(|e| e.type_ == 0) {
        if let Some(free) = plan.region(entry.base, entry.length).expect("BOOT_MEMORY_PLAN_INVALID") {
            pending_regions.push(free);
        }
    }
    let metadata = plan.metadata.expect("BOOT_ALLOCATOR_METADATA_UNAVAILABLE");
    // Physical placement may be in a later region. Protect the actual interval,
    // never an equivalent page count from the current frame-allocator cursor.
    frame_allocator.reserve_boot_region(metadata.start, metadata.end - metadata.start)
        .expect("BOOT_METADATA_OWNERSHIP_COLLISION");
    crate::serial_println!("[kernel.mem.global.metadata.carve] phys={:#x} size={} pages={} totals={}",
        metadata.start, metadata.end - metadata.start, metadata_pages, total_pages);
    unsafe {
        crate::memory::allocator::GLOBAL_ALLOCATOR.init_metadata(hhdm_offset + metadata.start, total_pages);
    }
    // Regions preceding the metadata placement also wait for initialization;
    // otherwise zeroing metadata would erase their published free-list heads.
    for free in pending_regions {
        unsafe { crate::memory::allocator::GLOBAL_ALLOCATOR.add_memory_region(free.start, free.end - free.start); }
    }
    // AP2 safety net: reserve every frame BootInfoFrameAllocator has consumed so far
    // in GLOBAL_ALLOCATOR metadata.  The buddy allocator pop functions lazily skip
    // entries with state!=0, so this prevents reissue of page-table and heap frames.
    let consumed_total = frame_allocator.next as u64;
    unsafe {
        crate::memory::allocator::GLOBAL_ALLOCATOR.reserve_page_range(0, consumed_total);
    }

    let usable_frames_total = crate::memory::allocator::debug_global_free_frames();
    crate::serial_println!("allocator.usable_frames.total={}", usable_frames_total);
    crate::serial_println!("allocator.init.done");

    *GLOBAL_VAS.lock() = Some(GlobalVas { mapper, frame_allocator });
}

pub unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;
    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();
    &mut *page_table_ptr
}

pub fn update_page_pkey(va: VirtAddr, pkey: u8) {
    unsafe { pku::tag_virtual_address(va.as_u64(), pkey); }
}

pub fn read_pte_flags(va: VirtAddr) -> Result<u64, &'static str> {
    use x86_64::registers::control::Cr3;
    let hhdm = crate::HHDM_REQUEST
        .response()
        .ok_or("HHDM missing")?
        .offset;

    let (cr3_frame, _) = Cr3::read();
    let pml4 = (cr3_frame.start_address().as_u64() + hhdm) as *const u64;

    let pml4_i = ((va.as_u64() >> 39) & 0x1ff) as usize;
    let pdpt_i = ((va.as_u64() >> 30) & 0x1ff) as usize;
    let pd_i = ((va.as_u64() >> 21) & 0x1ff) as usize;
    let pt_i = ((va.as_u64() >> 12) & 0x1ff) as usize;

    unsafe {
        let pml4e = *pml4.add(pml4_i);
        if (pml4e & 1) == 0 {
            return Err("pml4e not present");
        }
        let pdpt = ((pml4e & 0x000f_ffff_ffff_f000) + hhdm) as *const u64;
        let pdpte = *pdpt.add(pdpt_i);
        if (pdpte & 1) == 0 {
            return Err("pdpte not present");
        }
        if (pdpte & (1 << 7)) != 0 {
            return Ok(pdpte);
        }
        let pd = ((pdpte & 0x000f_ffff_ffff_f000) + hhdm) as *const u64;
        let pde = *pd.add(pd_i);
        if (pde & 1) == 0 {
            return Err("pde not present");
        }
        if (pde & (1 << 7)) != 0 {
            return Ok(pde);
        }
        let pt = ((pde & 0x000f_ffff_ffff_f000) + hhdm) as *const u64;
        let pte = *pt.add(pt_i);
        if (pte & 1) == 0 {
            return Err("pte not present");
        }
        Ok(pte)
    }
}

pub fn log_page_walk(va: VirtAddr, tag: &str) {
    let hhdm = match crate::HHDM_REQUEST.response() {
        Some(v) => v.offset,
        None => {
            crate::serial_println!("walk:{} va={:#x} hhdm=missing", tag, va.as_u64());
            return;
        }
    };

    use x86_64::registers::control::Cr3;
    let (cr3_frame, _) = Cr3::read();
    let pml4 = (cr3_frame.start_address().as_u64() + hhdm) as *const u64;

    let pml4_i = ((va.as_u64() >> 39) & 0x1ff) as usize;
    let pdpt_i = ((va.as_u64() >> 30) & 0x1ff) as usize;
    let pd_i = ((va.as_u64() >> 21) & 0x1ff) as usize;
    let pt_i = ((va.as_u64() >> 12) & 0x1ff) as usize;

    #[inline]
    fn bits(raw: u64) -> (bool, bool, bool, bool, u64) {
        let p = (raw & 1) != 0;
        let w = (raw & (1 << 1)) != 0;
        let u = (raw & (1 << 2)) != 0;
        let nx = (raw & (1u64 << 63)) != 0;
        let addr = raw & 0x000f_ffff_ffff_f000;
        (p, w, u, nx, addr)
    }

    unsafe {
        let pml4e = *pml4.add(pml4_i);
        let (p, w, u, nx, addr) = bits(pml4e);
        crate::serial_println!(
            "walk:{} PML4E raw={:#x} addr={:#x} p={} w={} u={} nx={}",
            tag, pml4e, addr, p, w, u, nx
        );
        if !p {
            return;
        }
        let pdpt = (addr + hhdm) as *const u64;
        let pdpe = *pdpt.add(pdpt_i);
        let (p, w, u, nx, addr) = bits(pdpe);
        crate::serial_println!(
            "walk:{} PDPE  raw={:#x} addr={:#x} p={} w={} u={} nx={}",
            tag, pdpe, addr, p, w, u, nx
        );
        if !p {
            return;
        }
        if (pdpe & (1 << 7)) != 0 {
            return;
        }
        let pd = (addr + hhdm) as *const u64;
        let pde = *pd.add(pd_i);
        let (p, w, u, nx, addr) = bits(pde);
        crate::serial_println!(
            "walk:{} PDE   raw={:#x} addr={:#x} p={} w={} u={} nx={}",
            tag, pde, addr, p, w, u, nx
        );
        if !p {
            return;
        }
        if (pde & (1 << 7)) != 0 {
            return;
        }
        let pt = (addr + hhdm) as *const u64;
        let pte = *pt.add(pt_i);
        let (p, w, u, nx, addr) = bits(pte);
        crate::serial_println!(
            "walk:{} PTE   raw={:#x} addr={:#x} p={} w={} u={} nx={}",
            tag, pte, addr, p, w, u, nx
        );
    }
}
