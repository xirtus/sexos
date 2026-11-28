// SPDX-License-Identifier: GPL-3.0-only
//! The Alienware x17 R1 touchpad's `_CRS`, byte for byte as its firmware has it (read off
//! the machine with `[sexacpi.aml.hex]`), run by the same interpreter as `servers/sexacpi`.
//! It names `TPDF` inside an `If` when the firmware did not define it, then compares
//! against it in a `While`; the interpreter failed with "LogicalOp on Uninitialized".
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use acpi::aml::{AmlError, Interpreter};
use acpi::registers::FixedRegisters;
use acpi::sdt::fadt::Fadt;
use acpi::{Handle, PciAddress, PhysicalMapping};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

static NOTIFIED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Host;
impl acpi::Handler for Host {
    unsafe fn map_physical_region<T>(&self, _: usize, _: usize) -> PhysicalMapping<Self, T> { panic!("no physical memory in this test") }
    fn unmap_physical_region<T>(_: &PhysicalMapping<Self, T>) {}
    fn read_u8(&self, _: usize) -> u8 { 0 }
    fn read_u16(&self, _: usize) -> u16 { 0 }
    fn read_u32(&self, _: usize) -> u32 { 0 }
    fn read_u64(&self, _: usize) -> u64 { 0 }
    fn write_u8(&self, _: usize, _: u8) {}
    fn write_u16(&self, _: usize, _: u16) {}
    fn write_u32(&self, _: usize, _: u32) {}
    fn write_u64(&self, _: usize, _: u64) {}
    fn read_io_u8(&self, _: u16) -> u8 { 0 }
    fn read_io_u16(&self, _: u16) -> u16 { 0 }
    fn read_io_u32(&self, _: u16) -> u32 { 0 }
    fn write_io_u8(&self, _: u16, _: u8) {}
    fn write_io_u16(&self, _: u16, _: u16) {}
    fn write_io_u32(&self, _: u16, _: u32) {}
    fn read_pci_u8(&self, _: PciAddress, _: u16) -> u8 { 0xFF }
    fn read_pci_u16(&self, _: PciAddress, _: u16) -> u16 { 0xFFFF }
    fn read_pci_u32(&self, _: PciAddress, _: u16) -> u32 { !0 }
    fn write_pci_u8(&self, _: PciAddress, _: u16, _: u8) {}
    fn write_pci_u16(&self, _: PciAddress, _: u16, _: u16) {}
    fn write_pci_u32(&self, _: PciAddress, _: u16, _: u32) {}
    fn nanos_since_boot(&self) -> u64 { 0 }
    fn stall(&self, _: u64) {}
    fn sleep(&self, _: u64) {}
    fn create_mutex(&self) -> Handle { Handle(1) }
    fn acquire(&self, _: Handle, _: u16) -> Result<(), AmlError> { Ok(()) }
    fn release(&self, _: Handle) {}
    fn handle_notify(&self, _: &WrappedObject, value: u64) { NOTIFIED.lock().unwrap().push(value) }
}

type Aml = Interpreter<Host>;

fn interpreter() -> Aml { interpreter_with(include_bytes!("fixtures/tpad_crs.aml")) }

fn interpreter_with(ssdt: &[u8]) -> Aml {
    let mut raw = vec![0u8; std::mem::size_of::<Fadt>()];
    raw[8] = 1;
    let mut fadt: Fadt = unsafe { std::ptr::read_unaligned(raw.as_ptr() as *const Fadt) };
    fadt.pm1a_event_block = 0x600;
    fadt.pm1_event_length = 4;
    fadt.pm1a_control_block = 0x604;
    fadt.pm1_control_length = 2;
    let aml = Interpreter::new(Host, 2, Arc::new(FixedRegisters::new(&fadt, Host).unwrap()), None);
    aml.load_table(&ssdt[36..]).unwrap();
    aml
}

#[test]
fn x17_touchpad_crs_evaluates() {
    let aml = interpreter();
    let crs = aml.evaluate(AmlName::from_str("\\_SB.I2C0.TPAD._CRS").unwrap(), Vec::new());
    let crs = crs.unwrap_or_else(|e| panic!("_CRS failed: {e:?}"));
    let Object::Buffer(bytes) = &*crs else { panic!("_CRS is not a buffer") };
    // I2cSerialBus (0x8e) then GpioInt (0x8c), then the end tag.
    assert_eq!(bytes[0], 0x8e);
    assert!(bytes.windows(1).any(|b| b[0] == 0x8c));
    assert_eq!(bytes[bytes.len() - 2] & 0xf8, 0x78, "ends with the end tag: {bytes:02x?}");
    let adr = aml.evaluate(AmlName::from_str("\\_SB.I2C0.TPAD.ADR0").unwrap(), Vec::new()).unwrap();
    assert!(matches!(*adr, Object::Integer(0)), "the table walk stops at the 0xFE row");
    drop(NOTIFIED.lock());
}

/// Variants: `TPDF` defined by firmware as a name or a field (TPAD_AML=path).
#[test]
fn x17_touchpad_crs_variant() {
    let Ok(path) = std::env::var("TPAD_AML") else { return };
    let aml = interpreter_with(&std::fs::read(path).unwrap());
    let r = aml.evaluate(AmlName::from_str("\\_SB.I2C0.TPAD._CRS").unwrap(), Vec::new());
    assert!(r.is_ok(), "{:?}", r.err());
}
