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
    assert_eq!(claim(id(0x8086, 0x9a60, 3, 0, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Display, Lease::IntelDisplay)));
    assert_eq!(claim(id(0x8086, 0x3e9b, 3, 0, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Display, Lease::Display)));
    // The RTX 3080 laptop (3D controller): to sexgpu-nv when the boot entry starts it,
    // else to Display as identity only.
    assert_eq!(claim(id(0x10de, 0x249c, 3, 2, 0)).map(|(m, e)| (m.service, e.lease, e.slot)), Some((Service::Gpu, Lease::NvidiaGpu, 16)));
    assert_eq!(claim_present(id(0x10de, 0x249c, 3, 2, 0), |s| s != Service::Gpu).map(|(m, e)| (m.service, e.lease)), Some((Service::Display, Lease::Display)));
    // Killer AX1675x (AX210): Intel Wi-Fi goes to the network service beside the wired NIC.
    assert_eq!(claim(id(0x8086, 0x2725, 2, 0x80, 0)).map(|(m, e)| (m.service, e.lease, e.slot)), Some((Service::Net, Lease::Wifi, 22)));
}

#[test]
fn unclaimed_devices_stay_unleased() {
    // VMD (RAID class 01:04), other makers' Wi-Fi, IDE-mode SATA, USB 2 EHCI, legacy virtio-net, bridges.
    for d in [id(0x8086, 0x9a0b, 1, 4, 0), id(0x14c3, 0x0616, 2, 0x80, 0), id(0x8086, 0x2922, 1, 6, 0),
              id(0x8086, 0x293a, 0x0c, 3, 0x20), id(0x1af4, 0x1000, 2, 0, 0), id(0x8086, 0x29c0, 6, 0, 0)] {
        assert!(claim(d).is_none(), "{:?}", d);
    }
}

#[test]
fn amd_display_engines_get_their_register_lease() {
    // Rembrandt APU (03:00) and a hybrid laptop's Navi 23 (03:80) both reach slot 28 first.
    for d in [id(0x1002, 0x1681, 3, 0, 0), id(0x1002, 0x73ef, 3, 0x80, 0)] {
        assert_eq!(claim(d).map(|(m, e)| (m.service, e.lease, e.slot)), Some((Service::Display, Lease::AmdDisplay, 29)));
    }
    // AMD HDMI audio stays with sound, not with Display.
    assert_ne!(claim(id(0x1002, 0x1640, 4, 3, 0)).map(|(m, _)| m.service), Some(Service::Display));
}

#[test]
fn virtio_sound_is_claimed_by_id_not_class() {
    assert_eq!(claim(id(0x1af4, 0x1059, 4, 1, 0)).map(|(m, e)| (m.service, e.lease)), Some((Service::Bell, Lease::Virtio)));
}

#[test]
fn virtio_net_is_claimed_by_id_not_class() {
    assert_eq!(claim(id(0x1af4, 0x1041, 2, 0, 0)).map(|(m, e)| (m.service, e.lease, e.slot)),
        Some((Service::Net, Lease::Virtio, 19)));
    // Legacy/transitional virtio-net (0x1000) stays unleased.
    assert!(claim(id(0x1af4, 0x1000, 2, 0, 0)).is_none());
}

#[test]
fn first_slots_are_the_ones_drivers_read() {
    let slot = |d| claim(d).map(|(_, e)| e.slot);
    assert_eq!(slot(id(0x1b36, 0x0010, 1, 8, 2)), Some(16));
    assert_eq!(slot(id(0x8086, 0x2922, 1, 6, 1)), Some(21));
    assert_eq!(slot(id(0x1b36, 0x000d, 0x0c, 3, 0x30)), Some(8));
    assert_eq!(slot(id(0x8086, 0x10d3, 2, 0, 0)), Some(19));
    assert_eq!(slot(id(0x1af4, 0x1041, 2, 0, 0)), Some(19));
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

#[test]
fn saddle_takes_only_what_no_system_service_drives() {
    // QEMU's edu device goes to Saddle for the Harness template driver.
    assert_eq!(claim(id(0x1234, 0x11e8, 0, 0xff, 0)).map(|(m, e)| (m.service, e.lease, e.slot)),
        Some((Service::Saddle, Lease::Generic, sex_hwdb::manifest::SADDLE_SLOT)));
    // Saddle is consulted last: no device a system service drives reaches it.
    assert_eq!(ALL.last().map(|m| m.service), Some(Service::Saddle));
    for m in ALL.iter().filter(|m| m.service != Service::Saddle) {
        assert!(m.matches.iter().all(|e| e.lease != Lease::Generic), "{:?} uses the generic lease", m.service);
    }
    for e in sex_hwdb::manifest::SADDLE.matches {
        assert_eq!(e.lease, Lease::Generic);
        assert!(e.vendor != 0 && e.device != 0, "Saddle claims exact ids only");
    }
}

#[test]
fn the_nvidia_service_takes_only_nvidia_display_controllers() {
    // RTX 3070 laptop as VGA (mux on the dGPU) and as 3D controller (Optimus).
    for d in [id(0x10de, 0x249d, 3, 0, 0), id(0x10de, 0x249d, 3, 2, 0)] {
        assert_eq!(claim(d).map(|(m, e)| (m.service, e.lease)), Some((Service::Gpu, Lease::NvidiaGpu)));
    }
    // Its HDMI audio function and an Intel GPU are not the GPU service's.
    assert_eq!(claim(id(0x10de, 0x228b, 4, 3, 0)).map(|(m, _)| m.service), Some(Service::Bell));
    assert_eq!(claim(id(0x8086, 0x9a60, 3, 0, 0)).map(|(m, _)| m.service), Some(Service::Display));
    assert_eq!(of(Service::Gpu).map(|m| m.max_devices), Some(1));
}
