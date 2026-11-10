// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Driver manifests: the x17 R1 and QEMU parts reach the services and slots their drivers read.
use sex_hwdb::{Id, Service};
use sex_hwdb::manifest::*;
fn id(vendor: u16, device: u16, class: u8, subclass: u8, prog_if: u8) -> Id { Id { vendor, device, class, subclass, prog_if } }

#[test]
fn x17_r1_parts_find_their_services() {
    // Samsung NVMe, AHCI, TGL-H xHCI, Killer E3100G, TGL-H HDA, UHD + RTX 3080 laptop.
    assert_eq!(claim(id(0x144d, 0xa80a, 1, 8, 2)).map(|(m, e)| (m.service, e.lease)), Some((Service::Drive, Lease::Nvme)));
    assert_eq!(claim(id(0x8086, 0x43d3, 1, 6, 1)).map(|(m, e)| (m.service, e.lease)), Some((Service::Drive, Lease::Ahci)));
    assert_eq!(claim(id(0x8086, 0x43ed, 0x0c, 3, 0x30)).map(|(m, e)| (m.service, e.lease)), Some((Service::Usb, Lease::Xhci)));
    assert_eq!(claim(id(0x10ec, 0x3000, 2, 0, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Net, Lease::Nic)));
    assert_eq!(claim(id(0x8086, 0x43c8, 4, 3, 0x80)).map(|(m, e)| (m.service, e.lease)), Some((Service::Bell, Lease::Hda)));
    assert_eq!(claim(id(0x8086, 0x9a60, 3, 0, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Display, Lease::Display)));
    assert_eq!(claim(id(0x10de, 0x249c, 3, 2, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Display, Lease::Display)));
    // Killer AX1675x (AX210): Intel Wi-Fi goes to the network service beside the wired NIC.
    assert_eq!(claim(id(0x8086, 0x2725, 2, 0x80, 0)).map(|(m, e)| (m.service, e.lease, e.slot)), Some((Service::Net, Lease::Wifi, 22)));
}

#[test]
fn unclaimed_devices_stay_unleased() {
    // VMD (RAID class 01:04), other makers' Wi-Fi, IDE-mode SATA, USB 2 EHCI, virtio-net, bridges.
    for d in [id(0x8086, 0x9a0b, 1, 4, 0), id(0x14c3, 0x0616, 2, 0x80, 0), id(0x8086, 0x2922, 1, 6, 0),
              id(0x8086, 0x293a, 0x0c, 3, 0x20), id(0x1af4, 0x1041, 2, 0, 0), id(0x8086, 0x29c0, 6, 0, 0)] {
        assert!(claim(d).is_none(), "{:?}", d);
    }
}

#[test]
fn virtio_sound_is_claimed_by_id_not_class() {
    assert_eq!(claim(id(0x1af4, 0x1059, 4, 1, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Bell, Lease::Virtio)));
}

#[test]
fn first_slots_are_the_ones_drivers_read() {
    let slot = |d| claim(d).map(|(_, e)| e.slot);
    assert_eq!(slot(id(0x1b36, 0x0010, 1, 8, 2)), Some(16));
    assert_eq!(slot(id(0x8086, 0x2922, 1, 6, 1)), Some(21));
    assert_eq!(slot(id(0x1b36, 0x000d, 0x0c, 3, 0x30)), Some(8));
    assert_eq!(slot(id(0x8086, 0x10d3, 2, 0, 0)), Some(19));
    assert_eq!(slot(id(0x8086, 0x293e, 4, 3, 0)), Some(20));
    assert_eq!(slot(id(0x1af4, 0x1059, 4, 1, 0)), Some(20));
    assert_eq!(slot(id(0x8086, 0x51f0, 2, 0x80, 0)), Some(22));
}

#[test]
fn every_service_has_one_manifest() {
    for (i, m) in ALL.iter().enumerate() {
        assert!(ALL[i + 1..].iter().all(|o| o.service != m.service));
        assert_eq!(of(m.service), Some(m));
    }
}
