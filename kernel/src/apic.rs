use acpi::{AcpiTables, PhysicalMapping, Handler};
use acpi::platform::interrupt::InterruptModel;
use core::ptr::NonNull;
use x86_64::VirtAddr;
use crate::serial_println;
use alloc::vec::Vec;
use spin::Mutex;
use lazy_static::lazy_static;

#[derive(Clone, Copy)]
pub struct SexAcpiHandler {
    pub physical_memory_offset: VirtAddr,
}

impl Handler for SexAcpiHandler {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> PhysicalMapping<Self, T> {
        let virt_addr = self.physical_memory_offset + physical_address as u64;
        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: NonNull::new(virt_addr.as_mut_ptr()).unwrap(),
            region_length: size,
            mapped_length: size,
            handler: self.clone(),
        }
    }

    fn unmap_physical_region<T>(_mapping: &PhysicalMapping<Self, T>) {
        // No-op
    }

    fn read_u8(&self, _: usize) -> u8 { 0 }
    fn read_u16(&self, _: usize) -> u16 { 0 }
    fn read_u32(&self, _: usize) -> u32 { 0 }
    fn read_u64(&self, _: usize) -> u64 { 0 }
    fn write_u8(&self, _: usize, _: u8) { /* no-op */ }
    fn write_u16(&self, _: usize, _: u16) { /* no-op */ }
    fn write_u32(&self, _: usize, _: u32) { /* no-op */ }
    fn write_u64(&self, _: usize, _: u64) { /* no-op */ }
    fn read_io_u8(&self, _: u16) -> u8 { 0xFF }
    fn read_io_u16(&self, _: u16) -> u16 { 0xFFFF }
    fn read_io_u32(&self, _: u16) -> u32 { 0xFFFF_FFFF }
    fn write_io_u8(&self, _: u16, _: u8) { /* no-op */ }
    fn write_io_u16(&self, _: u16, _: u16) { /* no-op */ }
    fn write_io_u32(&self, _: u16, _: u32) { /* no-op */ }
    fn read_pci_u8(&self, _: acpi::PciAddress, _: u16) -> u8 { 0xFF }
    fn read_pci_u16(&self, _: acpi::PciAddress, _: u16) -> u16 { 0xFFFF }
    fn read_pci_u32(&self, _: acpi::PciAddress, _: u16) -> u32 { 0xFFFF_FFFF }
    fn write_pci_u8(&self, _: acpi::PciAddress, _: u16, _: u8) { /* no-op */ }
    fn write_pci_u16(&self, _: acpi::PciAddress, _: u16, _: u16) { /* no-op */ }
    fn write_pci_u32(&self, _: acpi::PciAddress, _: u16, _: u32) { /* no-op */ }
    fn nanos_since_boot(&self) -> u64 { 0 }
    fn stall(&self, _: u64) { /* no-op */ }
    fn sleep(&self, _: u64) { /* no-op */ }
    fn create_mutex(&self) -> acpi::Handle { acpi::Handle(0) }
    fn acquire(&self, _: acpi::Handle, _: u16) -> Result<(), acpi::aml::AmlError> { Ok(()) }
    fn release(&self, _: acpi::Handle) { /* no-op */ }
}

pub struct ProcessorInfo {
    pub id: u32,
    pub local_apic_id: u8,
    pub is_bsp: bool,
}

pub struct IoApicInfo {
    pub id: u8,
    pub address: u32,
    pub global_system_interrupt_base: u32,
}

pub struct SexPciRegion {
    pub segment: u16,
    pub bus_start: u8,
    pub bus_end: u8,
    pub phys_addr: usize,
}

/// Stored MADT Interrupt Source Override entry for IOAPIC polarity/trigger correction.
#[derive(Clone, Copy)]
pub struct IsoOverride {
    pub isa_source: u8,
    pub gsi: u32,
    pub active_low: bool,
    pub level_triggered: bool,
}

lazy_static! {
    pub static ref PROCESSORS: Mutex<Vec<ProcessorInfo>> = Mutex::new(Vec::new());
    pub static ref IO_APICS: Mutex<Vec<IoApicInfo>> = Mutex::new(Vec::new());
    pub static ref PCI_REGIONS: Mutex<Vec<SexPciRegion>> = Mutex::new(Vec::new());
    /// MADT Interrupt Source Overrides — indexed by GSI (sparse, up to 256 entries).
    pub static ref ISO_OVERRIDES: Mutex<[Option<IsoOverride>; 256]> = Mutex::new([None; 256]);
}

pub static LAPIC_ADDR: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn init_apic(rsdp_addr: u64, physical_memory_offset: VirtAddr) {
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let mut pic2_data: Port<u8> = Port::new(0xA1);
        pic1_data.write(0xFF);
        pic2_data.write(0xFF);
    }

    let handler = SexAcpiHandler { physical_memory_offset };
    let tables = unsafe { AcpiTables::from_rsdp(handler, rsdp_addr as usize).expect("ACPI: Failed to parse tables") };

    let platform = acpi::platform::AcpiPlatform::new(tables, handler).expect("ACPI: Failed to get platform info");

    // Discover PCI Configuration Regions (MCFG)
    if let Some(mcfg) = platform.tables.find_table::<acpi::sdt::mcfg::Mcfg>() {
        let mut regions = PCI_REGIONS.lock();
        for region in mcfg.entries() {
            serial_println!("APIC: Found PCI Region (Segment {}, Bus {}-{}, Phys {:#x})",
                {region.pci_segment_group}, {region.bus_number_start}, {region.bus_number_end}, {region.base_address});
            regions.push(SexPciRegion {
                segment: region.pci_segment_group,
                bus_start: region.bus_number_start,
                bus_end: region.bus_number_end,
                phys_addr: region.base_address as usize,
            });
        }
    } else {
        serial_println!("APIC: Warning: MCFG table not found or failed to parse. PCIe may not work.");
    }

    if let InterruptModel::Apic(apic_info) = platform.interrupt_model {
        let lapic_virt = physical_memory_offset + apic_info.local_apic_address;
        LAPIC_ADDR.store(lapic_virt.as_u64(), core::sync::atomic::Ordering::Release);
        serial_println!("APIC: Found LAPIC at {:#x}", apic_info.local_apic_address);
        
        unsafe {
            init_local_apic(lapic_virt);
        }

        let mut io_apics = IO_APICS.lock();
        for io_apic in apic_info.io_apics.iter() {
            serial_println!("APIC: Found I/O APIC {} at {:#x}", io_apic.id, io_apic.address);
            io_apics.push(IoApicInfo {
                id: io_apic.id,
                address: io_apic.address,
                global_system_interrupt_base: io_apic.global_system_interrupt_base,
            });
        }

        // Store MADT Interrupt Source Overrides for correct polarity/trigger.
        {
            let mut iso_map = ISO_OVERRIDES.lock();
            for iso in apic_info.interrupt_source_overrides.iter() {
                let gsi = iso.global_system_interrupt;
                if (gsi as usize) < 256 {
                    let active_low = matches!(iso.polarity,
                        acpi::platform::interrupt::Polarity::ActiveLow);
                    let level = matches!(iso.trigger_mode,
                        acpi::platform::interrupt::TriggerMode::Level);
                    iso_map[gsi as usize] = Some(IsoOverride {
                        isa_source: iso.isa_source,
                        gsi,
                        active_low,
                        level_triggered: level,
                    });
                    serial_println!("APIC: ISO GSI {} <- ISA {} (active_low={}, level={})",
                        gsi, iso.isa_source, active_low, level);
                }
            }
        }

        let mut processors = PROCESSORS.lock();
        if let Some(proc_info) = platform.processor_info {
            for proc in proc_info.application_processors.iter() {
                processors.push(ProcessorInfo {
                    id: proc.processor_uid,
                    local_apic_id: proc.local_apic_id as u8,
                    is_bsp: false,
                });
            }
            processors.push(ProcessorInfo {
                id: proc_info.boot_processor.processor_uid,
                local_apic_id: proc_info.boot_processor.local_apic_id as u8,
                is_bsp: true,
            });
        }
    }
}

unsafe fn init_local_apic(lapic_virt: VirtAddr) {
    let lapic_ptr = lapic_virt.as_u64() as *mut u32;
    let svr_reg = lapic_ptr.offset(0x0F0 / 4);
    svr_reg.write_volatile(svr_reg.read_volatile() | 0x100 | 0xFF);
}

/// Maps an IRQ to a vector on a specific I/O APIC.
pub unsafe fn map_irq(irq: u8, vector: u8, dest_lapic_id: u8, physical_memory_offset: VirtAddr) {
    let io_apics = IO_APICS.lock();
    
    // Find the correct I/O APIC based on the Global System Interrupt (GSI) base
    // For simplicity, we assume IRQ maps 1:1 to GSI
    let gsi = irq as u32;
    let io_apic = io_apics.iter()
        .find(|io| gsi >= io.global_system_interrupt_base && gsi < io.global_system_interrupt_base + 24)
        .or_else(|| io_apics.first()) // Fallback to first
        .expect("APIC: No suitable I/O APIC found");

    let io_apic_virt = physical_memory_offset + io_apic.address as u64;
    let io_apic_ptr = io_apic_virt.as_u64() as *mut u32;

    let reg_sel = io_apic_ptr;
    let reg_win = io_apic_ptr.offset(0x10 / 4);

    // Redirection table entry for this IRQ (starts at 0x10, 2 registers per IRQ)
    let relative_irq = gsi - io_apic.global_system_interrupt_base;
    let low_index = 0x10 + relative_irq * 2;
    let high_index = low_index + 1;

    // Construct low 32-bit RTE: apply ISO override for polarity (bit 13) and trigger (bit 15).
    let mut low_val = vector as u32;
    {
        let iso_map = ISO_OVERRIDES.lock();
        if (gsi as usize) < 256 {
            if let Some(ref iso) = iso_map[gsi as usize] {
                if iso.active_low {
                    low_val |= 1 << 13; // INT_POL = active-low
                }
                if iso.level_triggered {
                    low_val |= 1 << 15; // TRIGGER_MODE = level
                }
            }
        }
    }

    // Write low part: vector, delivery mode (000 = fixed), dest mode (0 = physical)
    reg_sel.write_volatile(low_index);
    reg_win.write_volatile(low_val);

    // Write high part: destination (LAPIC ID)
    reg_sel.write_volatile(high_index);
    reg_win.write_volatile((dest_lapic_id as u32) << 24);
    
    serial_println!("APIC: Mapped GSI {} (IOAPIC {}) to Vector {} (Dest LAPIC {}, low={:#x})",
        gsi, io_apic.id, vector, dest_lapic_id, low_val);
}

pub unsafe fn send_ipi(lapic_id: u8, vector: u8, delivery_mode: u32) {
    let lapic_vaddr = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire);
    if lapic_vaddr == 0 { return; }
    let lapic_ptr = lapic_vaddr as *mut u32;
    let icr_high = lapic_ptr.offset(0x310 / 4);
    let icr_low = lapic_ptr.offset(0x300 / 4);
    while (icr_low.read_volatile() & (1 << 12)) != 0 {}
    icr_high.write_volatile((lapic_id as u32) << 24);
    let cmd = (delivery_mode << 8) | (vector as u32);
    icr_low.write_volatile(cmd);
}

pub unsafe fn broadcast_sipi(vector: u8) {
    let lapic_vaddr = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire);
    if lapic_vaddr == 0 { return; }
    let lapic_ptr = lapic_vaddr as *mut u32;
    let icr_low = lapic_ptr.offset(0x300 / 4);
    while (icr_low.read_volatile() & (1 << 12)) != 0 {}
    let cmd = (0b11 << 18) | (0b110 << 8) | (vector as u32);
    icr_low.write_volatile(cmd);
}

/// Initializes the LAPIC timer for pre-emptive scheduling (Vector 0x20).
/// Calibrates against PIT channel 2 (1.193182 MHz) for correct cadence on real CPUs.
/// Falls back to 1,000,000-count (~1ms at 100MHz) if PIT is absent or calibration fails.
pub fn init_timer() {
    let lapic_vaddr = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire);
    if lapic_vaddr == 0 {
        serial_println!("timer.init.done lapic=missing");
        return;
    }
    let lapic_ptr = lapic_vaddr as *mut u32;

    unsafe {
        use x86_64::instructions::port::Port;
        use x86_64::instructions::interrupts;

        // Divide by 16 (bus clock / 16)
        lapic_ptr.offset(0x3E0 / 4).write_volatile(0x3);

        // Calibrate against PIT channel 2 (1.193182 MHz)
        const PIT_FREQ: u64 = 1_193_182;
        const CALIB_MS: u64 = 10;
        const PIT_COUNT_MAX: u16 = 65535;
        let pit_count: u16 = ((PIT_FREQ * CALIB_MS) / 1000).min(PIT_COUNT_MAX as u64) as u16;

        let mut pit_cmd: Port<u8> = Port::new(0x43);
        let mut pit_ch2: Port<u8> = Port::new(0x42);
        let mut spk: Port<u8> = Port::new(0x61);

        interrupts::disable();

        // Mask LAPIC timer during calibration to prevent spurious IRQ.
        lapic_ptr.offset(0x320 / 4).write_volatile(0x10000 | 0x20); // masked, one-shot
        // Init count to max for calibration
        lapic_ptr.offset(0x380 / 4).write_volatile(0xFFFF_FFFF);
        // Re-write LVT unmasked one-shot so timer actually counts for calibration
        lapic_ptr.offset(0x320 / 4).write_volatile(0x20);

        // Program PIT ch2 -> mode 0 (one-shot), lo+hi byte
        pit_cmd.write(0xB0u8);
        pit_ch2.write((pit_count & 0xFF) as u8);
        pit_ch2.write(((pit_count >> 8) & 0xFF) as u8);

        // Spin until PIT ch2 output pin goes high (count reached 0)
        let mut timed_out = false;
        for _ in 0..1_000_000 {
            if (spk.read() & 0x20) != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        if (spk.read() & 0x20) == 0 {
            timed_out = true;
        }

        let remaining = lapic_ptr.offset(0x390 / 4).read_volatile();
        let elapsed = if timed_out || remaining >= 0xFFFF_FFFE {
            0
        } else {
            (0xFFFF_FFFFu32).wrapping_sub(remaining).wrapping_add(1)
        };

        let ticks_per_ms: u32 = if elapsed > 0 {
            elapsed / CALIB_MS as u32
        } else {
            0
        };

        interrupts::enable();

        if ticks_per_ms == 0 || ticks_per_ms > 10_000_000 {
            serial_println!("APIC: LAPIC timer calibration failed (elapsed={} pit_count={}); using fallback=1000000",
                elapsed, pit_count);
            lapic_ptr.offset(0x320 / 4).write_volatile(0x20000 | 0x20);
            lapic_ptr.offset(0x380 / 4).write_volatile(1_000_000);
        } else {
            serial_println!("APIC: LAPIC timer calibrated: {} ticks/ms (elapsed={} over {}ms)",
                ticks_per_ms, elapsed, CALIB_MS);
            lapic_ptr.offset(0x320 / 4).write_volatile(0x20000 | 0x20);
            lapic_ptr.offset(0x380 / 4).write_volatile(ticks_per_ms);
            TIMER_COUNT.store(ticks_per_ms, core::sync::atomic::Ordering::Release);
        }

        serial_println!("APIC: LAPIC Timer initialized at Vector 0x20.");
        // Runtime reachability proof v1: dump LAPIC timer registers to verify delivery path.
        let lvt_val = lapic_ptr.offset(0x320 / 4).read_volatile();
        let cur_cnt = lapic_ptr.offset(0x390 / 4).read_volatile();
        serial_println!("timer.init.done lapic={:#x} vector=0x20 ticks={} lvt={:#x} cur_count={}",
            lapic_vaddr, ticks_per_ms, lvt_val, cur_cnt);
    }
}

/// Turn on this core's local APIC, for a core started after the boot core set it up.
static TIMER_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1_000_000);
/// This core's timer, at the rate core 0 measured.
pub fn start_local_timer() {
    let lapic = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire) as *mut u32;
    unsafe {
        lapic.offset(0x3E0 / 4).write_volatile(0x3);
        lapic.offset(0x320 / 4).write_volatile(0x20000 | 0x20);
        lapic.offset(0x380 / 4).write_volatile(TIMER_COUNT.load(core::sync::atomic::Ordering::Acquire));
    }
}
pub fn enable_local() {
    let lapic = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire) as *mut u32;
    assert!(!lapic.is_null(), "SMP_NO_LAPIC");
    unsafe {
        lapic.offset(0x080 / 4).write_volatile(0);
        let svr = lapic.offset(0x0F0 / 4);
        svr.write_volatile(svr.read_volatile() | 0x1FF);
    }
}
pub fn local_id() -> u32 {
    let lapic = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire) as *const u32;
    unsafe { lapic.offset(0x020 / 4).read_volatile() >> 24 }
}
/// Fixed delivery to every core but this one.
pub fn send_to_others(vector: u8) {
    let lapic = LAPIC_ADDR.load(core::sync::atomic::Ordering::Acquire) as *mut u32;
    unsafe {
        while lapic.offset(0x300 / 4).read_volatile() & (1 << 12) != 0 { core::hint::spin_loop(); }
        lapic.offset(0x300 / 4).write_volatile((0b11 << 18) | (1 << 14) | vector as u32);
    }
}
