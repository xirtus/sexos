use alloc::vec::Vec;
use crate::hal::pci::PciDevice;

pub mod pci;
pub mod x86_64;
pub mod acpi;
pub mod dgpu;

pub struct Framebuffer {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

pub static MONOTONIC_COUNTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn get_monotonic_counter() -> u64 {
    MONOTONIC_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

pub trait HardwareAbstractionLayer {
    fn init(&self);
    fn init_advanced(&self, rsdp_addr: u64, hhdm_offset: u64);
    fn get_framebuffer(&self) -> Option<Framebuffer>;
    fn enumerate_pci(&self) -> Vec<PciDevice>;
    fn setup_timer(&self, hz: u64);
    fn configure_interrupts(&self);
    fn tlb_flush_local(&self);
}

pub fn init() {
    HAL.init();
}

pub fn init_advanced(rsdp_addr: u64, hhdm_offset: u64) {
    HAL.init_advanced(rsdp_addr, hhdm_offset);
}

pub fn tlb_flush_local() {
    HAL.tlb_flush_local();
}

// Global HAL instance for the kernel.
pub static HAL: x86_64::X86Hal = x86_64::X86Hal::new();
