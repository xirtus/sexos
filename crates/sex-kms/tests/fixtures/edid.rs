// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! EDID test fixtures shared by the sex-kms and sex-gpu-intel host tests.
#![allow(dead_code)]
use sex_kms::edid;
use sex_kms::mode::{cta_vic, Mode, FLAG_HSYNC_POSITIVE, FLAG_VSYNC_POSITIVE};

pub fn dtd(m: &Mode, wmm: u16, hmm: u16) -> [u8; 18] {
    let c = (m.clock_khz / 10) as u16;
    let (ha, hb) = (m.hdisplay, m.htotal - m.hdisplay);
    let (va, vb) = (m.vdisplay, m.vtotal - m.vdisplay);
    let (hso, hsw) = (m.hsync_start - m.hdisplay, m.hsync_end - m.hsync_start);
    let (vso, vsw) = (m.vsync_start - m.vdisplay, m.vsync_end - m.vsync_start);
    let mut flags = 0x18;
    if m.flags & FLAG_VSYNC_POSITIVE != 0 { flags |= 4; }
    if m.flags & FLAG_HSYNC_POSITIVE != 0 { flags |= 2; }
    [c as u8, (c >> 8) as u8, ha as u8, hb as u8, ((ha >> 8) << 4 | (hb >> 8)) as u8,
     va as u8, vb as u8, ((va >> 8) << 4 | (vb >> 8)) as u8, hso as u8, hsw as u8,
     ((vso & 0xf) << 4 | (vsw & 0xf)) as u8,
     ((hso >> 8) << 6 | (hsw >> 8) << 4 | (vso >> 4) << 2 | (vsw >> 4)) as u8,
     wmm as u8, hmm as u8, ((wmm >> 8) << 4 | (hmm >> 8)) as u8, 0, 0, flags]
}

pub fn fix(block: &mut [u8]) { let s = block[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b)); block[127] = 0u8.wrapping_sub(s); }

/// A 17.3" 1920x1080 165 Hz laptop panel like the x17 R1's, with a name and range block.
pub fn panel_edid() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..8].copy_from_slice(&edid::HEADER);
    let id: u16 = (((b'A' - 64) as u16) << 10) | (((b'U' - 64) as u16) << 5) | (b'O' - 64) as u16;
    b[8..10].copy_from_slice(&id.to_be_bytes());
    b[10] = 0x9d; b[11] = 0x40; b[17] = 30; b[18] = 1; b[19] = 4; b[20] = 0xa5; b[21] = 38; b[22] = 21;
    b[35] = 0x20; // 640x480@60
    for i in 0..8 { b[38 + 2 * i] = 1; b[39 + 2 * i] = 1; }
    b[38] = (1920 / 8 - 31) as u8; b[39] = 0xc0; // 1920x1080@60 (16:9)
    let native = Mode::new(377_000, [1920, 1968, 2000, 2080], [1080, 1083, 1088, 1101], FLAG_HSYNC_POSITIVE);
    b[54..72].copy_from_slice(&dtd(&native, 382, 215));
    let low = Mode::new(138_500, [1920, 1968, 2000, 2080], [1080, 1083, 1088, 1111], FLAG_HSYNC_POSITIVE);
    b[72..90].copy_from_slice(&dtd(&low, 382, 215));
    b[90..108].copy_from_slice(&[0, 0, 0, 0xfc, 0, b'A', b'U', b'O', b' ', b'B', b'1', b'7', b'3', b'H', b'A', b'N', 0x0a, 0x20]);
    b[108..126].copy_from_slice(&[0, 0, 0, 0xfd, 0, 48, 165, 30, 190, 38, 0, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);
    fix(&mut b);
    b
}

/// A 4K HDMI TV: base block plus CTA-861 extension with VICs 16, 97 and an HDMI VSDB.
pub fn tv_edid() -> Vec<u8> {
    let mut b = vec![0u8; 256];
    b[..8].copy_from_slice(&edid::HEADER);
    b[8] = 0x10; b[9] = 0xac; b[18] = 1; b[19] = 3; b[20] = 0x80; b[21] = 60; b[22] = 34; b[126] = 1;
    for i in 0..8 { b[38 + 2 * i] = 1; b[39 + 2 * i] = 1; }
    let m = cta_vic(16).unwrap();
    b[54..72].copy_from_slice(&dtd(&m, 600, 340));
    fix(&mut b[..128]);
    let x = &mut b[128..];
    x[0] = 2; x[1] = 3; x[3] = 0x40;
    let blocks = [0x43u8, 16, 97, 4, 0x65, 0x03, 0x0c, 0x00, 0x10, 0x00];
    x[4..4 + blocks.len()].copy_from_slice(&blocks);
    x[2] = (4 + blocks.len()) as u8;
    let m720 = cta_vic(19).unwrap();
    let o = x[2] as usize;
    x[o..o + 18].copy_from_slice(&dtd(&m720, 0, 0));
    fix(x);
    b
}

