// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The census on the bare-metal test machine and on the QEMU gate machine.
use sex_hwdb::*;

fn id(vendor: u16, device: u16, class: u8, subclass: u8, prog_if: u8) -> Id { Id { vendor, device, class, subclass, prog_if } }

#[test]
fn alienware_x17_r1_parts_are_named_and_owned() {
    let gpu = describe(id(0x10de, 0x249c, 0x03, 0x00, 0));
    assert_eq!(gpu.model, "GeForce RTX 3080 Laptop (GA104)");
    assert_eq!((gpu.service, gpu.support), (Service::Display, Support::Firmware));
    assert!(advice(id(0x10de, 0x249c, 0x03, 0x00, 0)).unwrap().contains("firmware framebuffer"));

    let wifi = id(0x8086, 0x2725, 0x02, 0x80, 0);
    assert_eq!(describe(wifi).support, Support::Planned);
    assert!(advice(wifi).unwrap().contains("Ethernet"));

    let killer = describe(id(0x10ec, 0x3000, 0x02, 0x00, 0));
    assert_eq!((killer.service, killer.support), (Service::Net, Support::Driven));
    assert_eq!(describe(id(0x8086, 0x43c8, 0x04, 0x03, 0)).service, Service::Bell);
    assert_eq!(describe(id(0x8086, 0x43ed, 0x0c, 0x03, 0x30)).support, Support::Driven);
}

#[test]
fn any_nvme_ssd_is_driven_and_samsung_is_named() {
    let samsung = describe(id(0x144d, 0xa80a, 0x01, 0x08, 0x02));
    assert_eq!(samsung.model, "NVMe SSD (980 PRO/PM9A1)");
    assert_eq!(samsung.support, Support::Driven);
    let unknown = describe(id(0x1e4b, 0x1202, 0x01, 0x08, 0x02));
    assert_eq!((unknown.model, unknown.service, unknown.support), ("", Service::Drive, Support::Driven));
    assert_eq!(vendor_name(0x1e4b), "Maxio");
    assert_eq!(class_name(0x01, 0x08, 0x02), "NVMe SSD");
}

#[test]
fn vmd_raid_mode_is_a_warning_not_a_disk() {
    let vmd = id(0x8086, 0x9a0b, 0x01, 0x04, 0);
    assert!(is_vmd(vmd));
    assert_eq!(describe(vmd).support, Support::Planned);
    assert!(advice(vmd).unwrap().contains("AHCI/NVMe"));
    assert!(!is_vmd(id(0x8086, 0x43ed, 0x0c, 0x03, 0x30)));
    // A RAID controller from anyone else is still named as a problem.
    assert!(advice(id(0x1000, 0x0097, 0x01, 0x04, 0)).is_some());
}

#[test]
fn census_counts_the_qemu_gate_machine() {
    let mut census = Census::default();
    for d in [
        id(0x8086, 0x29c0, 0x06, 0x00, 0), id(0x1234, 0x1111, 0x03, 0x00, 0), id(0x1b36, 0x0010, 0x01, 0x08, 0x02),
        id(0x1033, 0x0194, 0x0c, 0x03, 0x30), id(0x8086, 0x2918, 0x06, 0x01, 0), id(0x8086, 0x2922, 0x01, 0x06, 0x01),
        id(0x8086, 0x2930, 0x0c, 0x05, 0), id(0x8086, 0x100e, 0x02, 0x00, 0),
    ] { census.add(d); }
    assert_eq!(census, Census { devices: 8, working: 8, in_progress: 0, planned: 0, unsupported: 0, warnings: 0 });
}

#[test]
fn unknown_devices_fall_back_to_class_and_are_not_claimed() {
    let d = describe(id(0xabcd, 0x0001, 0xff, 0x00, 0));
    assert_eq!((d.service, d.support), (Service::Nothing, Support::Unsupported));
    assert_eq!(vendor_name(0xabcd), "Unknown vendor");
    assert_eq!(class_name(0xff, 0, 0), "device");
    assert!(!Support::Planned.works() && Support::Platform.works());
}
