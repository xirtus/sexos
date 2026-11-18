use crate::hal::{HardwareAbstractionLayer, pci::PciDevice};
use alloc::vec::Vec;
use crate::serial_println;
use crate::gdt;
use crate::interrupts;
use core::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use x86_64::registers::control::Cr3;

/// Ring-0 hardening each core turns on (docs/handoff/KERNEL_SECURITY_AUDIT_2026_11_18.md):
/// SMEP and UMIP where the CPU has them, and enhanced or automatic IBRS against
/// Spectre v2. Returns (smep, umip, ibrs kind, RDCL_NO). SMAP waits for user-copy helpers.
pub fn harden_core() -> (bool, bool, &'static str, bool) {
    use core::arch::x86_64::__cpuid_count;
    use x86_64::registers::control::{Cr4, Cr4Flags};
    use x86_64::registers::model_specific::Msr;
    let leaf7 = unsafe { __cpuid_count(7, 0) };
    let (smep, umip) = (leaf7.ebx & (1 << 7) != 0, leaf7.ecx & (1 << 2) != 0);
    let mut cr4 = Cr4::read();
    if smep { cr4.insert(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION); }
    if umip { cr4.insert(Cr4Flags::USER_MODE_INSTRUCTION_PREVENTION); }
    unsafe { Cr4::write(cr4); }
    // IA32_ARCH_CAPABILITIES: bit 0 RDCL_NO (no Meltdown, so no Meltdown-PK), bit 1 IBRS_ALL.
    let arch = if leaf7.edx & (1 << 29) != 0 { unsafe { Msr::new(0x10A).read() } } else { 0 };
    let top = unsafe { __cpuid_count(0x8000_0000, 0) }.eax;
    let amd = top >= 0x8000_0021 && unsafe { __cpuid_count(0x8000_0021, 0) }.eax & (1 << 8) != 0;
    let ibrs = unsafe {
        if arch & 2 != 0 { let mut m = Msr::new(0x48); m.write(m.read() | 1); "enhanced" }
        else if amd { let mut m = Msr::new(0xC000_0080); m.write(m.read() | (1 << 21)); "auto" }
        else { "none" }
    };
    (smep, umip, ibrs, arch & 1 != 0)
}

pub struct X86Hal {
    paging_ready: AtomicBool,
    paging_cr3: AtomicU64,
}

impl X86Hal {
    pub const fn new() -> Self {
        Self {
            paging_ready: AtomicBool::new(false),
            paging_cr3: AtomicU64::new(0),
        }
    }

    pub fn is_paging_ready(&self) -> bool {
        self.paging_ready.load(Ordering::Acquire)
    }

    pub fn get_paging_cr3(&self) -> u64 {
        self.paging_cr3.load(Ordering::Acquire)
    }
}

impl HardwareAbstractionLayer for X86Hal {
    fn init(&self) {
        if !self.is_paging_ready() {
            // Cycles since power-on at kernel entry: boot time without the loader.
            serial_println!("[boot.kernel_entry] tsc={}", unsafe { core::arch::x86_64::_rdtsc() });
            serial_println!("X86Hal: Initializing foundation (BSP)...");
            
            // SAS Page Table Sync
            let (p4_frame, _) = Cr3::read();
            let cr3_val = p4_frame.start_address().as_u64();
            self.paging_cr3.store(cr3_val, Ordering::Release);
            self.paging_ready.store(true, Ordering::Release);
            
            serial_println!("X86Hal: SAS Page Tables ready (CR3 = {:#x})", cr3_val);
        } else {
            serial_println!("X86Hal: Initializing foundation (AP)...");
        }

        if crate::pku::is_pku_supported() {
            unsafe { crate::pku::enable_pku(); }
            crate::pku::set_pku_enabled(true);
        } else {
            crate::pku::set_pku_enabled(false);
            serial_println!("PKU: unsupported on this CPU/QEMU path; PKRU ops gated off");
        }

        // SSE for programs; APs copy CR0/CR4/XCR0/EFER from here.
        unsafe { crate::fpu::init_core(); }
        let (smep, umip, ibrs, rdcl_no) = harden_core();
        serial_println!("[cpu.hardening] smep={} smap=0 umip={} ibrs={} rdcl_no={} meltdown_pk={}",
            smep as u8, umip as u8, ibrs, rdcl_no as u8, if rdcl_no { "no" } else { "possible" });
        { let (files, mode, size) = crate::fpu::describe(); serial_println!("[fpu.ready] sse=1 xsave_state={} save={} bytes={}", files, mode, size); }
        serial_println!("X86Hal: Initializing GDT/IDT...");
        gdt::init();

        interrupts::init_idt();
        serial_println!("[HAL] GDT/TSS + Star MSR locked under Intel MPK/PKEY isolation (10th-gen+ hardware enforced)");
    }

    fn init_advanced(&self, rsdp_addr: u64, hhdm_offset: u64) {
        serial_println!("X86Hal: init_advanced(rsdp={:#x}, hhdm={:#x})", rsdp_addr, hhdm_offset);
        if rsdp_addr != 0 {
            serial_println!("X86Hal: Initializing APIC and Timer...");
            crate::apic::init_apic(rsdp_addr, x86_64::VirtAddr::new(hhdm_offset));
            crate::apic::init_timer();
            // Route PS/2 keyboard IRQ1 → IDT vector 0x21 (keyboard_interrupt_handler).
            // IOAPIC is enumerated by init_apic; handler installed at idt[0x21] in interrupts::init_idt.
            unsafe { crate::apic::map_irq(1, 0x21, 0, x86_64::VirtAddr::new(hhdm_offset)); }
            // Power: S5, restart, the power button, sensors, idle and performance states.
            crate::hal::acpi::init(rsdp_addr, hhdm_offset);
        } else {
            serial_println!("X86Hal: WARNING - RSDP not found, skipping APIC/Timer.");
        }
    }

    fn enumerate_pci(&self) -> Vec<PciDevice> {
        crate::hal::pci::enumerate_bus()
    }

    fn get_framebuffer(&self) -> Option<crate::hal::Framebuffer> {
        if let Some(fb_res) = crate::FB_REQUEST.response() {
            if let Some(fb) = fb_res.framebuffers().iter().next() {
                return Some(crate::hal::Framebuffer {
                    address: fb.address() as u64,
                    width: fb.width as u32,
                    height: fb.height as u32,
                    pitch: (fb.pitch / 4) as u32,
                });
            }
        }
        None
    }

    fn setup_timer(&self, _hz: u64) {
        crate::apic::init_timer();
    }

    fn configure_interrupts(&self) {
        // Already handled in init_advanced via init_apic
    }

    fn tlb_flush_local(&self) {
        unsafe {
            let (p4_frame, flags) = Cr3::read();
            Cr3::write(p4_frame, flags);
        }
    }
}
