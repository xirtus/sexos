// SPDX-License-Identifier: GPL-3.0-only
//! Host checks for sex-iommu: firmware tables as real machines lay them out,
//! page tables, and the lending window.
use sex_iommu::dmar::{Dmar, Structure, SCOPE_ENDPOINT, SCOPE_BRIDGE};
use sex_iommu::ivrs::Ivrs;
use sex_iommu::table::{Format, Frames, MapError, Table, READ, WRITE};
use sex_iommu::vtd::{self, Cap, Ecap, Fault};
use sex_iommu::window::{place, Window};
use sex_iommu::{checksum_ok, Bdf, PAGE};
use std::collections::HashMap;

fn header(sig: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(sig);
    t.extend_from_slice(&((36 + body.len()) as u32).to_le_bytes());
    t.push(1); t.push(0); // revision, checksum (fixed below)
    t.extend_from_slice(b"SEXOS ");
    t.extend_from_slice(b"TESTTABL");
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(b"SEX ");
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(body);
    let sum = t.iter().fold(0u8, |s, b| s.wrapping_add(*b));
    t[9] = 0u8.wrapping_sub(sum);
    t
}

fn scope(kind: u8, bus: u8, path: &[(u8, u8)]) -> Vec<u8> {
    let mut s = vec![kind, (6 + 2 * path.len()) as u8, 0, 0, 0, bus];
    for (d, f) in path { s.push(*d); s.push(*f); }
    s
}

fn drhd(flags: u8, base: u64, scopes: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = scopes.concat();
    let mut s = vec![0, 0];
    s.extend_from_slice(&((16 + body.len()) as u16).to_le_bytes());
    s.push(flags); s.push(0);
    s.extend_from_slice(&0u16.to_le_bytes());
    s.extend_from_slice(&base.to_le_bytes());
    s.extend_from_slice(&body);
    s
}

fn rmrr(base: u64, limit: u64, scopes: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = scopes.concat();
    let mut s = vec![1, 0];
    s.extend_from_slice(&((24 + body.len()) as u16).to_le_bytes());
    s.extend_from_slice(&[0, 0, 0, 0]);
    s.extend_from_slice(&base.to_le_bytes());
    s.extend_from_slice(&limit.to_le_bytes());
    s.extend_from_slice(&body);
    s
}

/// A Tiger Lake laptop's shape: a graphics-only unit for 00:02.0, a catch-all
/// unit, RMRRs for USB (00:14.0) and graphics stolen memory.
fn tiger_lake() -> Vec<u8> {
    let mut body = vec![38, 0x05]; // HAW 39 bits, INTR_REMAP | DMA_CTRL_PLATFORM_OPT_IN
    body.extend_from_slice(&[0; 10]);
    body.extend(drhd(0, 0xfed9_0000, &[scope(SCOPE_ENDPOINT, 0, &[(2, 0)])]));
    body.extend(drhd(1, 0xfed9_1000, &[scope(3, 0, &[(0x1e, 7)]), scope(4, 0, &[(0x1e, 6)])]));
    body.extend(rmrr(0x3e2e_0000, 0x3e2f_ffff, &[scope(SCOPE_ENDPOINT, 0, &[(0x14, 0)])]));
    body.extend(rmrr(0x7b80_0000, 0x7fff_ffff, &[scope(SCOPE_ENDPOINT, 0, &[(2, 0)])]));
    // An ATSR the parser should step over.
    body.extend_from_slice(&[2, 0, 8, 0, 0, 0, 0, 0]);
    header(b"DMAR", &body)
}

#[test]
fn dmar_units_and_reserved_memory() {
    let t = tiger_lake();
    assert!(checksum_ok(&t));
    let d = Dmar::parse(&t).unwrap();
    assert_eq!(d.host_address_width, 39);
    assert_eq!(d.flags & sex_iommu::dmar::FLAG_DMA_CTRL_PLATFORM_OPT_IN, 4);
    let units: Vec<_> = d.units().collect();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].register_base, 0xfed9_0000);
    assert!(!units[0].include_all && units[1].include_all);
    assert_eq!(units[0].register_bytes, 4096);
    let rmrrs: Vec<_> = d.reserved().collect();
    assert_eq!(rmrrs.len(), 2);
    assert_eq!(rmrrs[0].span(), Some((0x3e2e_0000, 0x3e30_0000)));
    assert_eq!(d.structures().filter(|s| matches!(s, Structure::Other(2))).count(), 1);

    let no_bridges = |_: Bdf| None::<(u8, u8)>;
    let usb = rmrrs[0].scopes().next().unwrap().resolve(&no_bridges);
    assert_eq!(usb, Some(Bdf::new(0, 0x14, 0)));
    // Graphics goes to its own unit, NVMe and everything else to the catch-all.
    assert_eq!(d.unit_for(0, Bdf::new(0, 2, 0), &no_bridges).unwrap().register_base, 0xfed9_0000);
    assert_eq!(d.unit_for(0, Bdf::new(3, 0, 0), &no_bridges).unwrap().register_base, 0xfed9_1000);
    assert!(d.unit_for(1, Bdf::new(0, 2, 0), &no_bridges).is_none());
}

#[test]
fn dmar_bridge_paths() {
    // An endpoint behind root port 00:1d.0 whose secondary bus is 4.
    // 00:1c.0's switch spans buses 6..=8.
    let bridges = |b: Bdf| if b == Bdf::new(0, 0x1d, 0) { Some((4u8, 4u8)) }
        else if b == Bdf::new(0, 0x1c, 0) { Some((6, 8)) } else { None };
    let mut body = vec![38, 0];
    body.extend_from_slice(&[0; 10]);
    body.extend(drhd(0, 0x1000, &[scope(SCOPE_ENDPOINT, 0, &[(0x1d, 0), (0, 0)])]));
    body.extend(drhd(0, 0x2000, &[scope(SCOPE_BRIDGE, 0, &[(0x1c, 0)])]));
    let t = header(b"DMAR", &body);
    let d = Dmar::parse(&t).unwrap();
    assert_eq!(d.unit_for(0, Bdf::new(4, 0, 0), &bridges).unwrap().register_base, 0x1000);
    // Behind 00:1c.0, two switch levels down, still that bridge's unit.
    assert_eq!(d.unit_for(0, Bdf::new(6, 0, 0), &bridges).unwrap().register_base, 0x2000);
    assert_eq!(d.unit_for(0, Bdf::new(8, 0, 0), &bridges).unwrap().register_base, 0x2000);
    // Outside every scope, and no include-all unit: none.
    assert!(d.unit_for(0, Bdf::new(5, 0, 0), &bridges).is_none());
    assert!(d.unit_for(0, Bdf::new(9, 0, 0), &bridges).is_none());
}

#[test]
fn dmar_rejects_malformed() {
    let mut t = tiger_lake();
    assert!(Dmar::parse(&t[..40]).is_err());
    t[0] = b'X';
    assert!(Dmar::parse(&t).is_err());
    let mut t = tiger_lake();
    // A structure claiming to run past the table.
    t[48 + 2] = 0xff;
    assert!(Dmar::parse(&t).is_err());
    let mut t = tiger_lake();
    t[48 + 2] = 2; // length below the 4-byte minimum
    t[48 + 3] = 0;
    assert!(Dmar::parse(&t).is_err());
}

#[test]
fn ivrs_units() {
    let mut body = Vec::new();
    body.extend_from_slice(&((48u32 << 15) | (40 << 8)).to_le_bytes());
    body.extend_from_slice(&[0; 8]);
    let mut ivhd = vec![0x11, 0xb0, 40, 0];
    ivhd.extend_from_slice(&0x0002u16.to_le_bytes());
    ivhd.extend_from_slice(&0x40u16.to_le_bytes());
    ivhd.extend_from_slice(&0xfd20_0000u64.to_le_bytes());
    ivhd.extend_from_slice(&0u16.to_le_bytes());
    ivhd.resize(40, 0);
    body.extend(ivhd);
    let t = header(b"IVRS", &body);
    let i = Ivrs::parse(&t).unwrap();
    assert_eq!(i.virtual_address_bits(), 48);
    let u: Vec<_> = i.units().collect();
    assert_eq!(u.len(), 1);
    assert_eq!(u[0].register_base, 0xfd20_0000);
    assert_eq!(u[0].device_id, 2);
}

struct Mem { tables: HashMap<u64, [u64; 512]>, next: u64, budget: usize }
impl Mem { fn new(budget: usize) -> Self { Self { tables: HashMap::new(), next: 0x10_0000, budget } } }
impl Frames for Mem {
    fn alloc(&mut self) -> Option<u64> {
        if self.tables.len() >= self.budget { return None; }
        let f = self.next; self.next += PAGE; self.tables.insert(f, [0; 512]); Some(f)
    }
    fn read(&self, t: u64, i: usize) -> u64 { self.tables[&t][i] }
    fn write(&mut self, t: u64, i: usize, v: u64) { self.tables.get_mut(&t).unwrap()[i] = v; }
}

#[test]
fn table_maps_translates_and_unmaps() {
    for (levels, format) in [(3, Format::Vtd), (4, Format::Vtd), (4, Format::AmdVi)] {
        let mut m = Mem::new(64);
        let t = Table::new(&mut m, levels, format).unwrap();
        // A boot pool mapped at its own physical address.
        t.map_range(&mut m, 0x3c4_0000, 0x3c4_0000, 8 * PAGE, READ | WRITE).unwrap();
        assert_eq!(t.translate(&mut m, 0x3c4_0123), Some((0x3c4_0123, READ | WRITE)));
        // A lent page somewhere else entirely, read-only for the device.
        t.map(&mut m, 0x4000_0000, 0x1234_5000, READ, true).unwrap();
        assert_eq!(t.translate(&mut m, 0x4000_0008), Some((0x1234_5008, READ)));
        assert_eq!(t.map(&mut m, 0x4000_0000, 0x9999_9000, READ, true), Err(MapError::Present));
        assert_eq!(t.translate(&mut m, 0x5000_0000), None);
        assert_eq!(t.unmap(&mut m, 0x4000_0000), Some(0x1234_5000));
        assert_eq!(t.translate(&mut m, 0x4000_0000), None);
        assert_eq!(t.unmap(&mut m, 0x4000_0000), None);
        assert_eq!(t.map(&mut m, t.limit(), 0x1000, READ, true), Err(MapError::Range));
        assert_eq!(t.map(&mut m, 0x1001, 0x1000, READ, true), Err(MapError::Unaligned));
        assert_eq!(t.map(&mut m, 0x1000, 0x1000, 0, true), Err(MapError::Rights));
    }
}

#[test]
fn table_vtd_entry_bits() {
    let mut m = Mem::new(8);
    let t = Table::new(&mut m, 3, Format::Vtd).unwrap();
    t.map(&mut m, 0, 0xabc000, WRITE, true).unwrap();
    // Root -> level 2 -> leaf; intermediate entries carry R|W.
    let l2 = m.read(t.root, 0);
    assert_eq!(l2 & 3, 3);
    let leaf = m.read(l2 & !0xfff, 0);
    assert_eq!(m.read(leaf & !0xfff, 0), 0xabc000 | 2);
}

#[test]
fn table_prepared_window_never_allocates() {
    let mut m = Mem::new(16);
    let t = Table::new(&mut m, 4, Format::Vtd).unwrap();
    let base = 0x4000_0000;
    let bytes = 4 * 1024 * 1024; // two leaf tables
    t.prepare(&mut m, base, bytes).unwrap();
    let frames = m.tables.len();
    for i in 0..bytes / PAGE { t.map(&mut m, base + i * PAGE, 0x8000_0000 + i * PAGE, READ | WRITE, false).unwrap(); }
    assert_eq!(m.tables.len(), frames);
    // Outside the prepared range a no-grow map fails rather than allocating.
    assert_eq!(t.map(&mut m, base + bytes, 0x1000, READ, false), Err(MapError::NoTable));
    // A failed range map leaves nothing behind.
    let mut small = Mem::new(4);
    let t2 = Table::new(&mut small, 4, Format::Vtd).unwrap();
    assert_eq!(t2.map_range(&mut small, 0x1ff000, 0x1000, 2 * PAGE, READ), Err(MapError::NoFrames));
    assert_eq!(t2.translate(&mut small, 0x1ff000), None);
}

#[test]
fn vtd_fields() {
    // SAGAW offers 39- and 48-bit tables, PSI, ND=2, 8 fault records at 0x220.
    let cap = Cap((6 << 8) | (47 << 16) | (0x22 << 24) | (1 << 39) | (7 << 40) | 2);
    assert_eq!(cap.levels(), Some((4, 2)));
    assert!(cap.page_selective());
    assert_eq!(cap.domains(), 256);
    assert_eq!(cap.mgaw(), 48);
    assert_eq!(cap.fault_offset(), 0x220);
    assert_eq!(cap.fault_records(), 8);
    assert!(Cap(2 << 8).levels() == Some((3, 1)));
    assert!(Cap(0).levels().is_none());
    assert_eq!(Ecap(0xf0_1000 | 1).iotlb_offset(), 0x100);
    assert!(Ecap(1).coherent());
    // GCMD keeps persistent bits and never echoes one-shot ones (SRTP, WBF).
    assert_eq!(vtd::command(vtd::TE | vtd::SRTP, vtd::SRTP, true), vtd::TE | vtd::SRTP);
    assert_eq!(vtd::command(vtd::SRTP, vtd::TE, true) & vtd::SRTP, 0);
    assert_eq!(vtd::command(vtd::TE, vtd::TE, false), 0);
    let (lo, hi) = vtd::context_entry(0x5000, 2, 7);
    assert_eq!((lo, hi), (0x5001, 2 | (7 << 8)));
    assert_eq!(vtd::context_did(hi), 7);
    let f = Fault::decode(0x1234_5000, (1 << 63) | (5 << 32) | 0x0420).unwrap();
    assert_eq!(f.source, Bdf::new(4, 4, 0));
    assert!(f.write);
    assert_eq!(f.reason_text(), "write_not_permitted");
    assert!(Fault::decode(0, 0).is_none());
}

#[test]
fn window_take_give_place() {
    let mut w: Window<2> = Window::new(0x4000_0000);
    assert_eq!(Window::<2>::PAGES, 128);
    let a = w.take(32).unwrap();
    let b = w.take(64).unwrap();
    assert_eq!((a, b), (0x4000_0000, 0x4000_0000 + 32 * PAGE));
    assert!(w.take(64).is_none());
    assert!(!w.give(a + PAGE * 100, 1)); // never taken
    assert!(!w.give(a + PAGE * 90, 10)); // runs past the window
    assert!(w.give(a, 32));
    assert!(!w.give(a, 32)); // twice
    assert_eq!(w.take(16), Some(a));
    assert_eq!(w.pages_used(), 80);
    assert!(!w.give(0x1000, 1));
    // Windows avoid identity-mapped pools.
    let busy = [(0x3c4_0000, 0x3cc_0000), (0x4000_0000, 0x4010_0000)];
    assert_eq!(place(&busy, 128 << 20, 1 << 30, 1 << 30, 1 << 39), Some(0x8000_0000));
    assert_eq!(place(&[], 1 << 30, 1 << 30, 1 << 30, 1 << 31), Some(1 << 30));
    assert_eq!(place(&[], 1 << 30, 1 << 30, 1 << 30, (1 << 31) - 1), None);
}
