// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Builds a small synthetic SOF image with the same structure as `sof-tgl-h.ri`:
//! extended manifest (version + windows), `$CPD` directory, CSS `$MN2` manifest and
//! the `$AM1` descriptor 8 KiB into the payload. Shared by the fw and boot tests.
#![allow(dead_code)]

/// Extended manifest length of [`image`].
pub const EXT_LEN: usize = 16 + 80 + 72;
/// Payload length of [`image`].
pub const PAYLOAD_LEN: usize = 0x3000;
/// Host mailbox the synthetic windows declare (window 1 + 0, 8 KiB).
pub const HOST_BOX: (u32, u32) = (0xa0000, 0x2000);
/// DSP mailbox the synthetic windows declare (window 0 + 4 KiB, 4 KiB).
pub const DSP_BOX: (u32, u32) = (0x81000, 0x1000);

fn put32(v: &mut Vec<u8>, x: u32) { v.extend_from_slice(&x.to_le_bytes()); }
fn put16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_le_bytes()); }
fn set32(v: &mut [u8], at: usize, x: u32) { v[at..at + 4].copy_from_slice(&x.to_le_bytes()); }

/// The extended manifest alone.
pub fn ext_manifest() -> Vec<u8> {
    let mut v = Vec::new();
    put32(&mut v, 0x6e61_4d58);
    put32(&mut v, EXT_LEN as u32);
    put32(&mut v, 16);
    put32(&mut v, 0x0100_0000);
    // FW_VERSION element: 8 + sof_ipc_fw_version (60) + flags (4), padded to 80.
    put32(&mut v, 0);
    put32(&mut v, 80);
    put32(&mut v, 60);
    for x in [2u16, 2, 0, 0xffff] { put16(&mut v, x); }
    v.extend_from_slice(b"Nov 28 2026\0");
    v.extend_from_slice(b"12:00:00\0\0");
    v.extend_from_slice(b"abcdef");
    put32(&mut v, 0x0301_6001);
    v.resize(16 + 80, 0);
    // WINDOW element: 8 + cmd hdr (8) + type (4) + count (4) + 2 x 24.
    put32(&mut v, 1);
    put32(&mut v, 72);
    put32(&mut v, 64);
    put32(&mut v, 0x7000_0000);
    put32(&mut v, 1);
    put32(&mut v, 2);
    for (kind, id, size, off) in [(1u32, 0u32, 0x1000u32, 0x1000u32), (0, 1, 0x2000, 0)] {
        for x in [0, kind, id, 0, size, off] { put32(&mut v, x); }
    }
    assert_eq!(v.len(), EXT_LEN);
    v
}

/// The DMA payload alone (what the ROM receives).
pub fn payload() -> Vec<u8> {
    let mut p: Vec<u8> = (0..PAYLOAD_LEN).map(|i| (i * 7 + 3) as u8).collect();
    // $CPD header, v2.5 layout (0x14 bytes), 2 entries.
    p[0..4].copy_from_slice(b"$CPD");
    set32(&mut p, 4, 2);
    p[8] = 2;
    p[9] = 1;
    p[10] = 0x14;
    p[11] = 0;
    p[12..16].copy_from_slice(b"ADSP");
    set32(&mut p, 16, 0);
    let entries: [(&[u8], u32, u32); 2] = [(b"ADSP.man", 0x100, 0x80), (b"cavs0015", 0x1000, 0x2000)];
    for (i, (name, off, len)) in entries.iter().enumerate() {
        let e = 0x14 + 24 * i;
        p[e..e + 12].fill(0);
        p[e..e + name.len()].copy_from_slice(name);
        set32(&mut p, e + 12, *off);
        set32(&mut p, e + 16, *len);
        set32(&mut p, e + 20, 0);
    }
    // CSS manifest: type 4, header 0x10 dwords, size 0x20 dwords, $MN2.
    let c = 0x100;
    for (k, x) in [4u32, 0x10, 0x21000, 0, 0x8086, 0x2026_1128, 0x20].iter().enumerate() {
        set32(&mut p, c + 4 * k, *x);
    }
    p[c + 28..c + 32].copy_from_slice(b"$MN2");
    // $AM1 at 0x2000.
    let d = 0x2000;
    p[d..d + 4].copy_from_slice(b"$AM1");
    set32(&mut p, d + 4, 0x34);
    p[d + 8..d + 16].copy_from_slice(b"ADSPFW\0\0");
    set32(&mut p, d + 16, 3);
    set32(&mut p, d + 20, 0);
    set32(&mut p, d + 24, 0xffff);
    for (k, x) in [2u16, 2, 0, 1].iter().enumerate() {
        p[d + 28 + 2 * k..d + 30 + 2 * k].copy_from_slice(&x.to_le_bytes());
    }
    set32(&mut p, d + 36, 2);
    p
}

/// A whole synthetic `.ri`.
pub fn image() -> Vec<u8> {
    let mut v = ext_manifest();
    v.extend_from_slice(&payload());
    v
}
