// SPDX-License-Identifier: GPL-3.0-only
//! Starts the other cores. Each gets its own GDT, TSS, kernel stack and CoreLocal,
//! and the boot core's control registers. No task runs on them yet: they idle, and
//! answer TLB shootdowns so a page removed on one core is gone on all of them.
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
    tlb::COVERED.store(ONLINE.load(Ordering::Acquire), Ordering::Release);
    crate::serial_println!("[smp.online] cores={} present={}", ONLINE.load(Ordering::Acquire), smp.cpus().len());
}

extern "C" fn limine_ap_entry(info: &MpInfo) -> ! {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr3Flags, Cr4, Cr4Flags};
    use x86_64::registers::model_specific::{Efer, EferFlags};
    use x86_64::registers::xcontrol::{XCr0, XCr0Flags};
    use x86_64::structures::paging::PhysFrame;
    use x86_64::PhysAddr;
    x86_64::instructions::interrupts::disable();
    unsafe {
        Efer::write(EferFlags::from_bits_truncate(EFER.load(Ordering::Acquire)));
        Cr0::write(Cr0Flags::from_bits_truncate(CR0.load(Ordering::Acquire)));
        Cr4::write(Cr4Flags::from_bits_truncate(CR4.load(Ordering::Acquire)));
        let xcr0 = XCR0.load(Ordering::Acquire);
        if xcr0 != 0 { XCr0::write(XCr0Flags::from_bits_truncate(xcr0)); }
        Cr3::write(PhysFrame::containing_address(PhysAddr::new(CR3.load(Ordering::Acquire))), Cr3Flags::empty());
        let tss = crate::gdt::init_core();
        crate::interrupts::init_idt();
        let core = NEXT_CORE.fetch_add(1, Ordering::Relaxed);
        crate::core_local::CoreLocal::init_with_tss(core, tss);
        crate::serial_println!("[smp.core.online] core={} lapic={}", core, info.lapic_id);
    }
    crate::apic::enable_local();
    ONLINE.fetch_add(1, Ordering::Release);
    loop { x86_64::instructions::interrupts::enable_and_hlt(); }
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

    fn invalidate(va: u64) { x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(va)); }

    /// Answer a request not yet answered here. Touches no per-core GS data, since an
    /// interrupt from user mode arrives with the user's GS still loaded.
    pub fn service() {
        let generation = GENERATION.load(Ordering::Acquire);
        let core = crate::apic::local_id() as usize;
        if SERVED[core].load(Ordering::Relaxed) == generation { return; }
        SERVED[core].store(generation, Ordering::Relaxed);
        invalidate(PAGE.load(Ordering::Acquire));
        OWED.fetch_sub(1, Ordering::AcqRel);
    }

    /// Remove `va`'s translation on every core. Returns after all have done so. Waits
    /// with interrupts off, so while waiting it answers requests from other cores.
    pub fn flush(va: u64) {
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
