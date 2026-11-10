// SPDX-License-Identifier: GPL-3.0-only
//! Starts the other cores. Each gets its own GDT, TSS, kernel stack and CoreLocal,
//! and the boot core's control registers. All answer TLB shootdowns, so a page removed
//! on one core is gone on all of them. Core 1 is the guest core and runs program
//! records, away from the desktop. Cores 2 and up run the services that have moved, apps
//! and programs.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use limine::mp::MpInfo;
use limine::request::MpResponse;

pub static ONLINE: AtomicU32 = AtomicU32::new(1);
static CR3: AtomicU64 = AtomicU64::new(0);
static CR0: AtomicU64 = AtomicU64::new(0);
static CR4: AtomicU64 = AtomicU64::new(0);
static EFER: AtomicU64 = AtomicU64::new(0);
static XCR0: AtomicU64 = AtomicU64::new(0);
static NEXT_CORE: AtomicU32 = AtomicU32::new(1);
static GUEST: AtomicU32 = AtomicU32::new(0);
/// Where records run: core 1, or core 0 on a machine with only one.
pub fn guest_core() -> u32 { GUEST.load(Ordering::Acquire) }
pub const WAKE_VECTOR: u8 = 0xF1;
/// Services proven to run away from core 0, as domain ids. Shell and Display stay:
/// the kernel's lifecycle paths and the desktop's hot path still assume core 0.
const MOVED: [u64; 4] = [13, 11, 8, 2];
/// The core a domain's task belongs to.
pub fn home_core(pd: u64) -> u32 {
    if crate::program_records::is_record(pd) { return guest_core(); }
    let spare = COVERED_SPARE.load(Ordering::Acquire);
    if spare == 0 { return 0; }
    // Apps of every incarnation follow, spread by domain.
    if sex_pdx::app_catalog::app_of(pd as u32).is_some_and(|app| app != sex_pdx::app_catalog::App::Program) { return 2 + pd as u32 % spare; }
    // Programs leave the desktop's core too. Rows sharing a key take turns on one core,
    // the last spare, since one set of present bits serves one core at a time.
    if crate::program_tenancy::shared(pd as u32) { return 2 + spare - 1; }
    if sex_pdx::app_catalog::app_of(pd as u32) == Some(sex_pdx::app_catalog::App::Program) { return 2 + pd as u32 % spare; }
    match MOVED.iter().position(|m| *m == pd) { Some(index) => 2 + index as u32 % spare, None => 0 }
}
static LAPIC_OF: [AtomicU32; 256] = [const { AtomicU32::new(0) }; 256];
pub fn local_apic_of(core: u32) -> Option<u32> {
    LAPIC_OF.get(core as usize)?.load(Ordering::Acquire).checked_sub(1)
}
static COVERED_SPARE: AtomicU32 = AtomicU32::new(0);
/// Cores programs and their workers run on (2 and up); 0 on machines with two or fewer.
pub fn program_cores() -> u32 { COVERED_SPARE.load(Ordering::Acquire) }

pub fn boot_aps(smp: &MpResponse) {
    use x86_64::registers::control::{Cr0, Cr3, Cr4};
    use x86_64::registers::model_specific::Efer;
    CR3.store(Cr3::read().0.start_address().as_u64(), Ordering::Release);
    CR0.store(Cr0::read().bits(), Ordering::Release);
    CR4.store(Cr4::read().bits(), Ordering::Release);
    EFER.store(Efer::read().bits(), Ordering::Release);
    if Cr4::read().contains(x86_64::registers::control::Cr4Flags::OSXSAVE) {
        XCR0.store(x86_64::registers::xcontrol::XCr0::read().bits(), Ordering::Release);
    }
    let bsp_id = smp.bsp_lapic_id;
    LAPIC_OF[0].store(bsp_id + 1, Ordering::Release);
    let mut expected = 1u32;
    for cpu in smp.cpus() {
        if cpu.lapic_id == bsp_id { continue; }
        // One at a time: a core's setup takes the heap and address space locks.
        cpu.bootstrap(limine_ap_entry, 0);
        expected += 1;
        let mut spins = 0u64;
        while ONLINE.load(Ordering::Acquire) < expected {
            core::hint::spin_loop();
            spins += 1;
            assert!(spins < 40_000_000_000, "SMP_CORE_NEVER_ONLINE");
        }
    }
    if ONLINE.load(Ordering::Acquire) > 1 { GUEST.store(1, Ordering::Release); }
    COVERED_SPARE.store(ONLINE.load(Ordering::Acquire).saturating_sub(2), Ordering::Release);
    tlb::COVERED.store(ONLINE.load(Ordering::Acquire), Ordering::Release);
    crate::serial_println!("[smp.online] cores={} present={}", ONLINE.load(Ordering::Acquire), smp.cpus().len());
}

extern "C" fn limine_ap_entry(info: &MpInfo) -> ! {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr4, Cr4Flags};
    use x86_64::registers::model_specific::{Efer, EferFlags};
    use x86_64::registers::xcontrol::{XCr0, XCr0Flags};
    x86_64::instructions::interrupts::disable();
    unsafe {
        Efer::write(EferFlags::from_bits_truncate(EFER.load(Ordering::Acquire)));
        Cr0::write(Cr0Flags::from_bits_truncate(CR0.load(Ordering::Acquire)));
        Cr4::write(Cr4Flags::from_bits_truncate(CR4.load(Ordering::Acquire)));
        let xcr0 = XCR0.load(Ordering::Acquire);
        if xcr0 != 0 { XCr0::write(XCr0Flags::from_bits_truncate(xcr0)); }
        // One address space: the bootloader hands every core the same root.
        assert_eq!(Cr3::read().0.start_address().as_u64(), CR3.load(Ordering::Acquire), "SMP_SECOND_TRANSLATION_ROOT");
        let tss = crate::gdt::init_core();
        crate::interrupts::init_idt();
        let core = NEXT_CORE.fetch_add(1, Ordering::Relaxed);
        crate::core_local::CoreLocal::init_with_tss(core, tss);
        LAPIC_OF[core as usize].store(info.lapic_id + 1, Ordering::Release);
        crate::serial_println!("[smp.core.online] core={} lapic={}", core, info.lapic_id);
    }
    crate::apic::enable_local();
    crate::apic::start_local_timer();
    crate::hal::acpi::cpu_init_local();
    ONLINE.fetch_add(1, Ordering::Release);
    unsafe {
        // The timer wakes this core until BOOT is done; its handler defers until then.
        while (crate::ipc::BOOT_CONTROLLER.phase() as u8) < (crate::ipc::BootPhase::SchedulerRunning as u8)
            || !crate::core_local::INITIALIZED.load(Ordering::Acquire) {
            x86_64::instructions::interrupts::enable_and_hlt();
            x86_64::instructions::interrupts::disable();
        }
    }
    crate::scheduler::idle_core()
}

/// True when every core that has ever started answers shootdowns. Code that retires
/// pages checks this and refuses otherwise, rather than leave a stale translation.
pub fn shootdown_covers_all() -> bool {
    crate::core_local::INITIALIZED_CORES.load(Ordering::Acquire) == tlb::COVERED.load(Ordering::Acquire).max(1)
}

pub mod tlb {
    use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    pub const VECTOR: u8 = 0xF0;
    pub static COVERED: AtomicU32 = AtomicU32::new(0);
    static LOCK: AtomicBool = AtomicBool::new(false);
    static PAGE: AtomicU64 = AtomicU64::new(0);
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static OWED: AtomicU32 = AtomicU32::new(0);
    static SERVED: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];
    static SENT: AtomicU64 = AtomicU64::new(0);

    /// A request for every translation, not one page: `PAGE` holds this instead of an address.
    const EVERYTHING: u64 = u64::MAX;
    /// Past this many pages one request for everything is cheaper than one per page.
    pub const BATCH: usize = 8;
    fn invalidate(va: u64) {
        // User pages are never global, so reloading CR3 drops all of them.
        if va == EVERYTHING { x86_64::instructions::tlb::flush_all(); }
        else { x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(va)); }
    }

    /// Answer a request not yet answered here. Touches no per-core GS data, since an
    /// interrupt from user mode arrives with the user's GS still loaded.
    pub fn service() {
        let generation = GENERATION.load(Ordering::Acquire);
        if generation == 0 { return; }
        let core = crate::apic::local_id() as usize;
        if SERVED[core].load(Ordering::Relaxed) == generation { return; }
        SERVED[core].store(generation, Ordering::Relaxed);
        invalidate(PAGE.load(Ordering::Acquire));
        OWED.fetch_sub(1, Ordering::AcqRel);
    }

    /// Remove `va`'s translation on every core. Returns after all have done so. Waits
    /// with interrupts off, so while waiting it answers requests from other cores.
    pub fn flush(va: u64) { request(va & !0xFFF) }

    /// Removes every page's translation on every core with one request when there are
    /// many, so ending a big program costs one round of interrupts, not one per page.
    /// Callers clear all the leaves first; frames are reusable once this returns.
    pub fn flush_pages(count: usize, pages: impl Iterator<Item = u64>) {
        if count > BATCH { request(EVERYTHING); } else { for va in pages { request(va & !0xFFF); } }
    }
    /// Only this core's translations, for pages only this core can have used.
    pub fn flush_pages_local(count: usize, pages: impl Iterator<Item = u64>) {
        if count > BATCH { invalidate(EVERYTHING); } else { for va in pages { invalidate(va & !0xFFF); } }
    }

    fn request(va: u64) {
        invalidate(va);
        let others = COVERED.load(Ordering::Acquire).saturating_sub(1);
        if others == 0 { return; }
        x86_64::instructions::interrupts::without_interrupts(|| {
            while LOCK.swap(true, Ordering::Acquire) { service(); core::hint::spin_loop(); }
            let me = crate::apic::local_id() as usize;
            PAGE.store(va, Ordering::Release);
            OWED.store(others, Ordering::Release);
            let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
            SERVED[me].store(generation, Ordering::Relaxed);
            crate::apic::send_to_others(VECTOR);
            SENT.fetch_add(1, Ordering::Relaxed);
            while OWED.load(Ordering::Acquire) != 0 { core::hint::spin_loop(); }
            LOCK.store(false, Ordering::Release);
        });
    }
    pub fn sent() -> u64 { SENT.load(Ordering::Relaxed) }
}
