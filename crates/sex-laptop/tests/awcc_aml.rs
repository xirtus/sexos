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

struct DellWmi<'a>(&'a Aml);
impl sex_laptop::dell_smbios::Firmware for DellWmi<'_> {
    fn call(&mut self, class: u16, select: u16, input: [u32; 4]) -> Option<[u32; 4]> {
        let mut buf = vec![0u8; 40];
        sex_laptop::dell_smbios::request(class, select, input, 40, &mut buf);
        let args = vec![int(0), int(1), WrappedObject::new(Object::Buffer(buf))];
        match eval(self.0, "\\_SB.DWMI.WMBC", args).as_deref() { Some(Object::Buffer(b)) => sex_laptop::dell_smbios::answer(b), _ => None }
    }
}

#[test]
fn dell_charge_modes_end_to_end() {
    use sex_laptop::dell_smbios::{self as d, Mode, Token};
    let aml = interpreter();
    let wdg = match eval(&aml, "\\_SB.DWMI._WDG", vec![]).as_deref() { Some(Object::Buffer(b)) => b.clone(), _ => panic!("_WDG") };
    assert_eq!(wmi::find(&wdg, &wmi::guids::DELL_SMBIOS).unwrap().method_name(), Some(*b"WMBC"));
    let q = wmi::find(&wdg, &wmi::guids::DELL_DESCRIPTOR).unwrap().query_name().unwrap();
    assert_eq!(&q, b"WQBD");
    let desc = match eval(&aml, "\\_SB.DWMI.WQBD", vec![int(0)]).as_deref() { Some(Object::Buffer(b)) => b.clone(), _ => panic!("WQBD") };
    assert_eq!(d::descriptor(&desc), Some((1, 48)));

    let tokens: Vec<Token> = [(0x0341, 1), (0x0342, 1), (0x0343, 1), (0x0346, 1), (0x0349, 0), (0x034A, 0)]
        .iter().map(|&(id, value)| Token { id, location: id, value }).collect();
    let mut fw = DellWmi(&aml);
    let c = d::charge(&mut fw, &tokens);
    assert_eq!(c.mode, Some(Mode::Standard));
    assert_eq!((c.start, c.end), (Some(50), Some(90)));
    assert!(c.supports(Mode::Custom) && !c.supports(Mode::Express));
    assert!(d::set_mode(&mut fw, &tokens, Mode::Custom, Some((70, 80))));
    let c = d::charge(&mut fw, &tokens);
    assert_eq!(c.mode, Some(Mode::Custom));
    assert_eq!((c.start, c.end), (Some(70), Some(80)));
    assert!(d::set_mode(&mut fw, &tokens, Mode::Adaptive, None));
    assert_eq!(d::charge(&mut fw, &tokens).mode, Some(Mode::Adaptive));
}

/// AccessAs, Connection and extended AccessAs in field lists (GPIO and I2C fields in
/// laptop DSDTs): the table loads, and what follows the fields still runs.
#[test]
fn field_lists_with_connections_load() {
    let aml = interpreter();
    let fields: &'static [u8] = include_bytes!("fixtures/fields_ssdt.aml");
    aml.load_table(&fields[36..]).unwrap();
    assert_eq!(as_int(eval(&aml, "\\CHK", vec![])), 0x2B);
    for name in ["\\_SB.GPO1", "\\_SB.SMB1", "\\_SB.RPB0"] {
        assert!(aml.namespace.lock().get(AmlName::from_str(name).unwrap()).is_ok(), "{name}");
    }
}

/// Not, FindSetLeftBit and FindSetRightBit with a target (the x17 R1's DSDT panicked the
/// interpreter with them), and a divide by zero, which fails its method only.
#[test]
fn unary_maths_and_divide_by_zero() {
    let aml = interpreter();
    let math: &'static [u8] = include_bytes!("fixtures/math_ssdt.aml");
    aml.load_table(&math[36..]).unwrap();
    // Not(0x0F) & 0xFF = 0xF0; FindSetLeftBit(0x30) = 6; FindSetRightBit(0x30) = 5; 0x30 >> 3 = 6.
    assert_eq!(as_int(eval(&aml, "\\MATH", vec![int(0x0F), int(0x30)])), 0xF0_0656);
    assert!(aml.evaluate_if_present(AmlName::from_str("\\DIV0").unwrap(), vec![int(0)]).is_err());
    assert_eq!(as_int(eval(&aml, "\\DIV0", vec![int(5)])), 2);
}

/// Laptop `_BST`/`_BIF`/`_BIX` methods populate their return package via
/// `Store(val, Index(PBST, i))` and `CopyObject(...)`, and EC/lid registers often use
/// `IndexField`.
#[test]
fn battery_package_index_store_and_ec_index_field() {
    use acpi::aml::op_region::{OpRegion, RegionHandler, RegionSpace};
    use acpi::aml::AmlError;
    use std::sync::Mutex;

    // 1. Store and CopyObject through ReferenceKind::RefOf (Index(PBST, i)).
    let mut aml = Vec::new();
    aml.extend_from_slice(&[0x08, b'P', b'B', b'S', b'T', 0x12, 0x06, 0x04, 0x00, 0x00, 0x00, 0x00]);
    let mut body = Vec::new();
    body.extend_from_slice(&[0x70, 0x0a, 0x02, 0x88, b'P', b'B', b'S', b'T', 0x00, 0x00]);
    body.extend_from_slice(&[0x9d, 0x0b, 0x34, 0x12, 0x88, b'P', b'B', b'S', b'T', 0x0a, 0x02, 0x00]);
    body.extend_from_slice(&[0xa4, b'P', b'B', b'S', b'T']);
    aml.push(0x14);
    aml.push((1 + 4 + 1 + body.len()) as u8);
    aml.extend_from_slice(b"_BST");
    aml.push(0x00);
    aml.extend_from_slice(&body);

    let interp = interpreter();
    interp.load_table(&aml).unwrap();
    let res = eval(&interp, "\\_BST", vec![]).unwrap();
    let Object::Package(items) = &*res else { panic!("expected package") };
    assert_eq!(as_int(Some(items[0].clone())), 0x02);
    assert_eq!(as_int(Some(items[1].clone())), 0x00);
    assert_eq!(as_int(Some(items[2].clone())), 0x1234);
    assert_eq!(as_int(Some(items[3].clone())), 0x00);

    // 2. IndexField over an EmbeddedControl OpRegion (e.g. `_LID`).
    struct FakeEc {
        idx: Mutex<u8>,
        ram: Mutex<[u8; 256]>,
    }
    impl RegionHandler for FakeEc {
        fn read_u8(&self, r: &OpRegion) -> Result<u8, AmlError> {
            match r.base {
                0 => Ok(*self.idx.lock().unwrap()),
                1 => Ok(self.ram.lock().unwrap()[*self.idx.lock().unwrap() as usize]),
                _ => Ok(0),
            }
        }
        fn read_u16(&self, _: &OpRegion) -> Result<u16, AmlError> { Ok(0) }
        fn read_u32(&self, _: &OpRegion) -> Result<u32, AmlError> { Ok(0) }
        fn read_u64(&self, _: &OpRegion) -> Result<u64, AmlError> { Ok(0) }
        fn write_u8(&self, r: &OpRegion, v: u8) -> Result<(), AmlError> {
            match r.base {
                0 => *self.idx.lock().unwrap() = v,
                1 => self.ram.lock().unwrap()[*self.idx.lock().unwrap() as usize] = v,
                _ => {}
            }
            Ok(())
        }
        fn write_u16(&self, _: &OpRegion, _: u16) -> Result<(), AmlError> { Ok(()) }
        fn write_u32(&self, _: &OpRegion, _: u32) -> Result<(), AmlError> { Ok(()) }
        fn write_u64(&self, _: &OpRegion, _: u64) -> Result<(), AmlError> { Ok(()) }
    }

    let mut ram = [0u8; 256];
    ram[0x01] = 0x01;
    let interp2 = interpreter();
    interp2.install_region_handler(
        RegionSpace::EmbeddedControl,
        FakeEc { idx: Mutex::new(0), ram: Mutex::new(ram) },
    );

    let mut aml2 = Vec::new();
    aml2.extend_from_slice(&[0x5b, 0x80, b'E', b'C', b'O', b'R', 0x03, 0x00, 0x0a, 0x02]);
    aml2.extend_from_slice(&[
        0x5b, 0x81, 16, b'E', b'C', b'O', b'R', 0x01,
        b'E', b'C', b'I', b'X', 0x08,
        b'E', b'C', b'D', b'A', 0x08,
    ]);
    aml2.extend_from_slice(&[
        0x5b, 0x86, 17, b'E', b'C', b'I', b'X', b'E', b'C', b'D', b'A', 0x01,
        0x00, 0x08,
        b'L', b'I', b'D', b'S', 0x08,
    ]);
    aml2.extend_from_slice(&[0x14, 11, b'_', b'L', b'I', b'D', 0x00, 0xa4, b'L', b'I', b'D', b'S']);

    interp2.load_table(&aml2).unwrap();
    assert_eq!(as_int(eval(&interp2, "\\_LID", vec![])), 1);
}
