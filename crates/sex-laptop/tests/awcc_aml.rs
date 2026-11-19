// SPDX-License-Identifier: GPL-3.0-only
//! The Alienware fan controller protocol against real AML, run by the same interpreter as
//! `servers/sexacpi` (vendor/acpi-6.1.1). The fixture (tests/fixtures/awcc_ssdt.asl) is the
//! pretend x17 R1 WMI device from the laptop power gate, plus the `DerefOf` and buffer
//! field patterns the interpreter used to fail on.
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use acpi::aml::{AmlError, Interpreter};
use acpi::registers::FixedRegisters;
use acpi::sdt::fadt::Fadt;
use acpi::{Handle, PciAddress, PhysicalMapping};
use sex_laptop::profile::Profile;
use sex_laptop::{awcc, dell_wmi, wmi};
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

fn interpreter() -> Aml {
    let mut raw = vec![0u8; std::mem::size_of::<Fadt>()];
    raw[8] = 1;
    let mut fadt: Fadt = unsafe { std::ptr::read_unaligned(raw.as_ptr() as *const Fadt) };
    fadt.pm1a_event_block = 0x600;
    fadt.pm1_event_length = 4;
    fadt.pm1a_control_block = 0x604;
    fadt.pm1_control_length = 2;
    let aml = Interpreter::new(Host, 2, Arc::new(FixedRegisters::new(&fadt, Host).unwrap()), None);
    let ssdt: &'static [u8] = include_bytes!("fixtures/awcc_ssdt.aml");
    aml.load_table(&ssdt[36..]).unwrap();
    aml
}

fn eval(aml: &Aml, path: &str, args: Vec<WrappedObject>) -> Option<WrappedObject> {
    aml.evaluate_if_present(AmlName::from_str(path).unwrap(), args).unwrap()
}
fn int(v: u64) -> WrappedObject { WrappedObject::new(Object::Integer(v)) }
fn as_int(o: Option<WrappedObject>) -> u64 { match o.as_deref() { Some(Object::Integer(v)) => *v, _ => panic!("not an integer") } }

struct Wmax<'a>(&'a Aml);
impl awcc::Firmware for Wmax<'_> {
    fn call(&mut self, method: u32, a: u32) -> Option<u32> {
        let args = vec![int(0), int(method as u64), WrappedObject::new(Object::Buffer(a.to_le_bytes().to_vec()))];
        match eval(self.0, "\\_SB.AMW0.WMAX", args).as_deref() { Some(Object::Integer(v)) => Some(*v as u32), _ => None }
    }
}

#[test]
fn interpreter_derefs_and_reads_buffer_fields() {
    let aml = interpreter();
    assert_eq!(as_int(eval(&aml, "\\DREF", vec![int(1)])), 0x133);
    assert_eq!(as_int(eval(&aml, "\\BREF", vec![int(2)])), 0x30);
    let buf = WrappedObject::new(Object::Buffer(vec![0xFF, 0x02, 0x03, 0x00]));
    assert_eq!(as_int(eval(&aml, "\\BFLD", vec![buf])), 0x0303);
}

#[test]
fn alienware_wmax_end_to_end() {
    let aml = interpreter();
    let wdg = match eval(&aml, "\\_SB.AMW0._WDG", vec![]).as_deref() { Some(Object::Buffer(b)) => b.clone(), _ => panic!("_WDG") };
    assert_eq!(wmi::find(&wdg, &wmi::guids::ALIENWARE_WMAX).unwrap().method_name(), Some(*b"WMAX"));
    assert_eq!(wmi::find(&wdg, &wmi::guids::DELL_EVENT).unwrap().notify_id(), Some(0xD0));

    let mut fw = Wmax(&aml);
    let inv = awcc::probe(&mut fw).expect("AWCC answers");
    assert_eq!(inv.description.word(), 0x0400_0302);
    assert_eq!(inv.fans().iter().map(|f| (f.id, f.max_rpm)).collect::<Vec<_>>(), [(0x32, 5600), (0x33, 5900)]);
    assert_eq!(inv.sensors(), &[0x01, 0x06, 0x03]);
    assert_eq!(inv.profiles(), &[awcc::PROFILE_USTT_BALANCED, awcc::PROFILE_USTT_PERFORMANCE, awcc::PROFILE_USTT_QUIET]);
    let r = awcc::read(&mut fw, &inv);
    assert_eq!(&r.rpm[..2], &[2400, 2100]);
    assert_eq!(&r.celsius[..3], &[Some(41), Some(38), None]);
    assert_eq!(r.profile, Some(awcc::PROFILE_USTT_BALANCED));

    let quiet = awcc::for_profile(Profile::Saver, inv.profiles()).unwrap();
    assert!(awcc::activate(&mut fw, &inv, quiet));
    assert_eq!(awcc::read(&mut fw, &inv).profile, Some(awcc::PROFILE_USTT_QUIET));

    // The first description read raised the hotkey event; _WED carries mic mute.
    assert!(NOTIFIED.lock().unwrap().contains(&0xD0));
    let wed = match eval(&aml, "\\_SB.AMW0._WED", vec![int(0xD0)]).as_deref() { Some(Object::Buffer(b)) => b.clone(), _ => panic!("_WED") };
    let mut keys = [dell_wmi::Key::Other(0); 4];
    let n = dell_wmi::decode(&wed, &mut keys);
    assert_eq!(&keys[..n], &[dell_wmi::Key::MicMute]);
}
