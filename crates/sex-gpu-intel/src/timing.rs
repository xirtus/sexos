// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Panel timings without a heap: EDID detailed timing descriptors (DTDs)
//! decoded into the same value-not-minus-one form the transcoder readout uses,
//! so Display (which has no allocator) can compare what the panel offers with
//! what the pipe runs.

/// One display mode's timings. `h`/`v` = [active, sync start, sync end, total].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Timing {
    pub clock_khz: u32,
    pub h: [u16; 4],
    pub v: [u16; 4],
    /// Bit 0 = positive hsync, bit 1 = positive vsync (as `readout` sets them).
    pub flags: u8,
}

impl Timing {
    /// Refresh in millihertz.
    pub fn refresh_mhz(&self) -> u32 {
        let total = self.h[3] as u64 * self.v[3] as u64;
        if total == 0 { 0 } else { (self.clock_khz as u64 * 1_000_000 / total) as u32 }
    }
    /// Same raster: every horizontal and vertical count and the sync
    /// polarities match, only the pixel clock may differ. Moving between two
    /// such modes needs no modeset on a DisplayPort link, only new M/N values.
    pub fn same_raster(&self, other: &Timing) -> bool {
        self.h == other.h && self.v == other.v && self.flags == other.flags
    }
}

/// Decode one 18-byte EDID detailed timing descriptor. None for the display
/// descriptors (clock 0) that share the slots, and for interlaced timings.
pub fn dtd(d: &[u8]) -> Option<Timing> {
    if d.len() < 18 { return None; }
    let clock = u16::from_le_bytes([d[0], d[1]]) as u32 * 10;
    if clock == 0 { return None; }
    if d[17] & 0x80 != 0 { return None; }
    let ha = d[2] as u16 | ((d[4] >> 4) as u16) << 8;
    let hb = d[3] as u16 | ((d[4] & 0xf) as u16) << 8;
    let va = d[5] as u16 | ((d[7] >> 4) as u16) << 8;
    let vb = d[6] as u16 | ((d[7] & 0xf) as u16) << 8;
    let hso = d[8] as u16 | (((d[11] >> 6) & 3) as u16) << 8;
    let hsw = d[9] as u16 | (((d[11] >> 4) & 3) as u16) << 8;
    let vso = (d[10] >> 4) as u16 | (((d[11] >> 2) & 3) as u16) << 4;
    let vsw = (d[10] & 0xf) as u16 | ((d[11] & 3) as u16) << 4;
    if ha == 0 || va == 0 || hb == 0 || vb == 0 { return None; }
    // Digital separate sync (bits 4:3 = 11) carries the polarities in bits 2 and 1.
    let mut flags = 0;
    if (d[17] >> 3) & 3 == 3 {
        if d[17] & 0x2 != 0 { flags |= 1; }
        if d[17] & 0x4 != 0 { flags |= 2; }
    }
    Some(Timing {
        clock_khz: clock,
        h: [ha, ha + hso, ha + hso + hsw, ha + hb],
        v: [va, va + vso, va + vso + vsw, va + vb],
        flags,
    })
}

/// Every detailed timing an EDID lists: the four base-block slots, then the
/// DTDs of each CTA-861 extension block. High-refresh laptop panels list the
/// fast mode and a 60 Hz mode here, usually on the same raster.
pub fn edid_timings(edid: &[u8], out: &mut [Timing]) -> usize {
    let mut n = 0;
    let mut push = |t: Timing, n: &mut usize| {
        if *n < out.len() && !out[..*n].contains(&t) { out[*n] = t; *n += 1; }
    };
    if edid.len() >= 128 {
        for d in edid[54..126].chunks(18) { if let Some(t) = dtd(d) { push(t, &mut n); } }
    }
    let mut block = 128;
    while block + 128 <= edid.len() {
        let b = &edid[block..block + 128];
        // CTA-861: byte 2 is where the DTDs start (4 = none, 0 = no data at all).
        if b[0] == 0x02 && b[2] >= 4 && (b[2] as usize) < 127 {
            let mut at = b[2] as usize;
            while at + 18 <= 127 {
                match dtd(&b[at..at + 18]) { Some(t) => push(t, &mut n), None => break }
                at += 18;
            }
        }
        block += 128;
    }
    n
}
