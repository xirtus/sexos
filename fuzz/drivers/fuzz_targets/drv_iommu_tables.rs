// SPDX-License-Identifier: GPL-3.0-only
//! IOMMU (Intel VT-d, AMD-Vi): the firmware's DMAR and IVRS tables, the units'
//! capability registers, and the fault/event records a misbehaving device makes
//! the IOMMU write. Firmware tables are device input too: laptops ship broken ones.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_iommu::{amdvi, dmar::Dmar, ivrs::Ivrs, vtd, Bdf};
use sexos_driver_fuzz::Src;

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let cap = vtd::Cap(s.u64());
    let _ = (cap.domains(), cap.sagaw(), cap.mgaw(), cap.fault_offset(), cap.fault_records(), cap.levels(), cap.max_flush_order());
    let e = vtd::Ecap(s.u64());
    let _ = (e.iotlb_offset(), e.coherent(), e.queued_invalidation(), e.scalable_mode());
    for _ in 0..4 { let _ = vtd::decode_fault(s.u64(), s.u64()); let _ = amdvi::decode_event(s.u64(), s.u64()); }
    let table = s.rest();
    let _ = sex_iommu::checksum_ok(table);
    let secondary = |b: Bdf| if b.bus < 4 { Some((b.bus + 1, b.bus + 3)) } else { None };
    if let Ok(t) = Dmar::parse(table) {
        for _ in t.structures() {}
        for u in t.units() { for sc in u.scopes() { let _ = sc.path().count(); let _ = sc.resolve(&secondary); let _ = sc.covers(Bdf { bus: 0, dev: 2, func: 0 }, &secondary); } }
        for r in t.reserved() { let _ = r.span(); for sc in r.scopes() { let _ = sc.resolve(&secondary); } }
        for bus in [0u8, 1, 3, 0xff] { let _ = t.unit_for(0, Bdf { bus, dev: 0x1f, func: 7 }, &secondary); }
    }
    if let Ok(t) = Ivrs::parse(table) {
        let _ = t.virtual_address_bits();
        for u in t.units() { for dev in [0u16, 0x10, 0xa0, 0xffff] { let _ = (u.covers(dev), u.alias(dev)); } }
        for _ in t.reserved() {}
        let _ = t.unit_for(0x0010);
    }
}

fn table(sig: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut t = sig.to_vec();
    t.extend_from_slice(&((36 + body.len()) as u32).to_le_bytes());
    t.extend_from_slice(&[1, 0]);
    t.extend_from_slice(b"SEXOS ");
    t.extend_from_slice(b"FUZZTBL ");
    t.extend_from_slice(&[0; 12]);
    t.extend_from_slice(body);
    let sum = t.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    t[9] = 0u8.wrapping_sub(sum);
    t
}

fn seeds() -> Vec<Vec<u8>> {
    // DMAR: width 39, flags; DRHD for the iGPU (00:02.0) and an include-all DRHD; an RMRR.
    let mut dm = vec![38, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    dm.extend_from_slice(&[0, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0xd1, 0xfe, 0, 0, 0, 0, 1, 8, 0, 0, 0, 0, 0, 2]);
    dm.truncate(12 + 16); dm[14] = 24;
    dm.extend_from_slice(&[1, 8, 0, 0, 0, 0, 0, 2, 0]);
    dm.truncate(12 + 24);
    dm.extend_from_slice(&[0, 0, 16, 0, 1, 0, 0, 0, 0, 0x30, 0xd1, 0xfe, 0, 0, 0, 0]);
    dm.extend_from_slice(&[1, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0x7c, 0, 0, 0, 0, 0xff, 0xff, 0x7f, 0x7c, 0, 0, 0, 0, 1, 8, 0, 0, 0, 0, 0x14, 0]);
    let mut iv = vec![0x00, 0x30, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    iv.extend_from_slice(&[0x10, 0xb0, 32, 0, 0x02, 0, 0x40, 0, 0, 0, 0xf0, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 4, 0xff, 0xff, 0]);
    let mut prefix = vec![0u8; 8 + 8 + 4 * 32];
    let mut a = prefix.clone(); a.extend(table(b"DMAR", &dm));
    prefix.extend(table(b"IVRS", &iv));
    vec![a, prefix]
}

sexos_driver_fuzz::driver_target!(run, seeds);
