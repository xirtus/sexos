// SPDX-License-Identifier: GPL-3.0-only
//! ACPI firmware (sexacpi): the FADT and AML tables a laptop's firmware hands the
//! OS, run through the same interpreter and the same bring-up sexacpi does (load
//! the DSDT, `_PIC`, namespace init with `_STA`/`_INI`, then the methods sexacpi
//! calls: battery, lid, thermal, sleep). Every operation region read (I/O ports,
//! memory, PCI config, the EC) answers from the input. Firmware is device input:
//! shipped DSDTs are buggy, and a laptop's must never hang or crash sexacpi.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
#[path = "../../../servers/sexacpi/src/fadt_check.rs"] mod fadt_check;
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use acpi::aml::{AmlError, Interpreter};
use acpi::registers::FixedRegisters;
use acpi::sdt::fadt::Fadt;
use acpi::{Handle, PciAddress, PhysicalMapping};
use core::str::FromStr;
use sexos_driver_fuzz::{tick, Src};
use std::cell::RefCell;
use std::sync::Arc;

thread_local! { static IO: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) }; static AT: RefCell<usize> = const { RefCell::new(0) }; }
fn val(bytes: usize) -> u64 {
    tick();
    IO.with(|io| AT.with(|at| {
        let io = io.borrow();
        let mut at = at.borrow_mut();
        let mut v = 0u64;
        for i in 0..bytes { v |= (*io.get(*at).unwrap_or(&0xff) as u64) << (8 * i); *at += 1; }
        v
    }))
}

#[derive(Clone, Copy)]
struct Fw;
impl acpi::Handler for Fw {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, _size: usize) -> PhysicalMapping<Self, T> {
        // As sexacpi: tables arrive as bytes, never by mapping.
        panic!("map_physical_region({physical_address:#x}) has no ring-3 meaning")
    }
    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
    fn read_u8(&self, _a: usize) -> u8 { val(1) as u8 }
    fn read_u16(&self, _a: usize) -> u16 { val(2) as u16 }
    fn read_u32(&self, _a: usize) -> u32 { val(4) as u32 }
    fn read_u64(&self, _a: usize) -> u64 { val(8) }
    fn write_u8(&self, _a: usize, _v: u8) { tick() }
    fn write_u16(&self, _a: usize, _v: u16) { tick() }
    fn write_u32(&self, _a: usize, _v: u32) { tick() }
    fn write_u64(&self, _a: usize, _v: u64) { tick() }
    fn read_io_u8(&self, _p: u16) -> u8 { val(1) as u8 }
    fn read_io_u16(&self, _p: u16) -> u16 { val(2) as u16 }
    fn read_io_u32(&self, _p: u16) -> u32 { val(4) as u32 }
    fn write_io_u8(&self, _p: u16, _v: u8) { tick() }
    fn write_io_u16(&self, _p: u16, _v: u16) { tick() }
    fn write_io_u32(&self, _p: u16, _v: u32) { tick() }
    fn read_pci_u8(&self, _a: PciAddress, _o: u16) -> u8 { val(1) as u8 }
    fn read_pci_u16(&self, _a: PciAddress, _o: u16) -> u16 { val(2) as u16 }
    fn read_pci_u32(&self, _a: PciAddress, _o: u16) -> u32 { val(4) as u32 }
    fn write_pci_u8(&self, _a: PciAddress, _o: u16, _v: u8) { tick() }
    fn write_pci_u16(&self, _a: PciAddress, _o: u16, _v: u16) { tick() }
    fn write_pci_u32(&self, _a: PciAddress, _o: u16, _v: u32) { tick() }
    fn nanos_since_boot(&self) -> u64 { sex_pdx::get_ticks().saturating_mul(1_000_000) }
    fn stall(&self, us: u64) { for _ in 0..us.min(100_000) / 1000 + 1 { tick() } }
    fn sleep(&self, ms: u64) { for _ in 0..ms.min(2_000) + 1 { tick() } }
    fn create_mutex(&self) -> Handle { Handle(1) }
    fn acquire(&self, _: Handle, _: u16) -> Result<(), AmlError> { Ok(()) }
    fn release(&self, _: Handle) {}
}

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let rev = s.u8();
    let fadt_bytes = s.chunk();
    let aml = s.chunk();
    IO.with(|io| *io.borrow_mut() = s.rest().to_vec());
    AT.with(|a| *a.borrow_mut() = 0);
    let mut raw = [0u8; core::mem::size_of::<Fadt>()];
    let n = raw.len().min(fadt_bytes.len());
    raw[..n].copy_from_slice(&fadt_bytes[..n]);
    let fadt: Fadt = unsafe { core::ptr::read_unaligned(raw.as_ptr() as *const Fadt) };
    if fadt_check::pm1_not_io(&fadt).is_some() { return; }
    let Ok(registers) = FixedRegisters::new(&fadt, Fw) else { return };
    let i = Interpreter::new(Fw, rev, Arc::new(registers), None);
    if i.load_table(aml).is_err() { return; }
    let one = || vec![WrappedObject::new(Object::Integer(1))];
    if let Ok(p) = AmlName::from_str("\\_PIC") { let _ = i.evaluate_if_present(p, one()); }
    i.initialize_namespace();
    for path in ["\\_SB.BAT0._BST", "\\_SB.BAT0._BIX", "\\_SB.LID0._LID", "\\_TZ.TZ00._TMP", "\\_SB.PCI0.LPCB.EC0._REG", "\\_PTS", "\\_WAK", "\\_S5", "\\_SB.AC._PSR"] {
        if let Ok(p) = AmlName::from_str(path) { let _ = i.evaluate_if_present(p, one()); }
    }
}

fn seeds() -> Vec<Vec<u8>> {
    // Method (_PIC, 1) { Store (Arg0, PICM) }, Name (PICM, Zero),
    // OperationRegion (ECR, SystemIO, 0x62, 2) + Field, Method (_STA) { Return (0x0F) }.
    let aml: Vec<u8> = vec![
        0x08, b'P', b'I', b'C', b'M', 0x00,
        0x14, 0x0b, b'_', b'P', b'I', b'C', 0x01, 0x70, 0x68, b'P', b'I', b'C', b'M',
        0x5b, 0x80, b'E', b'C', b'R', b'_', 0x01, 0x0a, 0x62, 0x0a, 0x02,
        0x5b, 0x81, 0x0b, b'E', b'C', b'R', b'_', 0x01, b'E', b'C', b'D', b'_', 0x08,
        0x14, 0x09, b'_', b'P', b'T', b'S', 0x01, 0x70, b'E', b'C', b'D', b'_', b'P', b'I', b'C', b'M',
    ];
    let mut v = vec![2u8];
    let fadt = vec![0u8; 276];
    v.extend_from_slice(&(fadt.len() as u16).to_le_bytes());
    v.extend_from_slice(&fadt);
    v.extend_from_slice(&(aml.len() as u16).to_le_bytes());
    v.extend_from_slice(&aml);
    v.extend_from_slice(&[0x42; 32]);
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
