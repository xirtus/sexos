// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Display timings: one [`Mode`] is everything a CRTC needs to scan a picture out.

/// Sync polarity and scan flags.
pub const FLAG_HSYNC_POSITIVE: u8 = 1 << 0;
pub const FLAG_VSYNC_POSITIVE: u8 = 1 << 1;
pub const FLAG_INTERLACED: u8 = 1 << 2;
/// The monitor names this its preferred (native) mode.
pub const FLAG_PREFERRED: u8 = 1 << 3;

/// One video timing. Horizontal values in pixels, vertical in lines, clock in kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mode {
    pub clock_khz: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub flags: u8,
}

impl Mode {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(clock_khz: u32, h: [u16; 4], v: [u16; 4], flags: u8) -> Self {
        Self { clock_khz, hdisplay: h[0], hsync_start: h[1], hsync_end: h[2], htotal: h[3],
            vdisplay: v[0], vsync_start: v[1], vsync_end: v[2], vtotal: v[3], flags }
    }
    /// Timings are ordered and non-empty, so a CRTC can be programmed with them.
    pub fn valid(&self) -> bool {
        self.clock_khz > 0 && self.hdisplay > 0 && self.vdisplay > 0
            && self.hdisplay <= self.hsync_start && self.hsync_start < self.hsync_end && self.hsync_end <= self.htotal
            && self.vdisplay <= self.vsync_start && self.vsync_start < self.vsync_end && self.vsync_end <= self.vtotal
    }
    /// Refresh rate in millihertz (60 Hz = 60_000).
    pub fn refresh_mhz(&self) -> u32 {
        let frame = self.htotal as u64 * self.vtotal as u64;
        if frame == 0 { return 0; }
        let mut r = self.clock_khz as u64 * 1_000_000 / frame;
        if self.flags & FLAG_INTERLACED != 0 { r *= 2; }
        r as u32
    }
    /// Refresh rounded to whole hertz.
    pub fn refresh_hz(&self) -> u32 { (self.refresh_mhz() + 500) / 1000 }
    /// One frame in microseconds: the vsync period a flip waits on.
    pub fn frame_us(&self) -> u32 {
        let r = self.refresh_mhz() as u64;
        if r == 0 { 0 } else { (1_000_000_000 / r) as u32 }
    }
    /// The vertical blanking interval in microseconds.
    pub fn vblank_us(&self) -> u32 {
        if self.clock_khz == 0 { return 0; }
        let lines = (self.vtotal - self.vdisplay) as u64;
        (lines * self.htotal as u64 * 1000 / self.clock_khz as u64) as u32
    }
    pub fn preferred(&self) -> bool { self.flags & FLAG_PREFERRED != 0 }
    /// Same picture timing; ignores the preferred flag.
    pub fn same_timing(&self, other: &Mode) -> bool {
        let mask = !FLAG_PREFERRED;
        Mode { flags: self.flags & mask, ..*self } == Mode { flags: other.flags & mask, ..*other }
    }
    /// Build a mode from a measured scanout: totals from the registers, refresh
    /// counted from the frame counter. Used when firmware already lit the panel
    /// and the PLL setup is not decoded.
    pub fn from_measured(h: [u16; 4], v: [u16; 4], flags: u8, refresh_mhz: u32) -> Self {
        let clock = h[3] as u64 * v[3] as u64 * refresh_mhz as u64 / 1_000_000;
        Self::new(clock as u32, h, v, flags)
    }
}

/// VESA CVT reduced-blanking (v1) timing for any size and refresh. Used for EDID
/// standard timings and for modes a monitor did not list in full.
pub fn cvt_rb(width: u16, height: u16, refresh_hz: u16) -> Option<Mode> {
    if width < 64 || height < 64 || refresh_hz == 0 { return None; }
    let w = width - width % 8;
    let vsync: u16 = aspect_vsync(w, height);
    let rate = refresh_hz as u64;
    // Picoseconds avoid floating point.
    let field_ps = 1_000_000_000_000u64 / rate;
    let min_vblank_ps = 460_000_000u64;
    if field_ps <= min_vblank_ps { return None; }
    let hperiod_ps = (field_ps - min_vblank_ps) / height as u64;
    if hperiod_ps == 0 { return None; }
    let vbi = (min_vblank_ps / hperiod_ps + 1).max(3 + vsync as u64 + 6) as u16;
    let vtotal = height.checked_add(vbi)?;
    let htotal = w.checked_add(160)?;
    let hz = rate * vtotal as u64 * htotal as u64;
    let clock_khz = (hz / 1000) / 250 * 250;
    Some(Mode::new(clock_khz as u32, [w, w + 48, w + 80, htotal],
        [height, height + 3, height + 3 + vsync, vtotal], FLAG_HSYNC_POSITIVE))
}

fn aspect_vsync(w: u16, h: u16) -> u16 {
    let (w, h) = (w as u32, h as u32);
    if w * 3 == h * 4 { 4 } else if w * 9 == h * 16 { 5 } else if w * 10 == h * 16 { 6 }
    else if w * 4 == h * 5 || w * 9 == h * 15 { 7 } else { 10 }
}

const P: u8 = FLAG_HSYNC_POSITIVE | FLAG_VSYNC_POSITIVE;

/// Established timings from EDID bytes 35..=36 (bit 7 of byte 35 first) that
/// SexOS knows the DMT numbers for. `None` slots are rare legacy modes.
pub const ESTABLISHED: [Option<Mode>; 16] = [
    None, None,
    Some(Mode::new(25_175, [640, 656, 752, 800], [480, 490, 492, 525], 0)),
    None,
    Some(Mode::new(31_500, [640, 664, 704, 832], [480, 489, 492, 520], 0)),
    Some(Mode::new(31_500, [640, 656, 720, 840], [480, 481, 484, 500], 0)),
    None,
    Some(Mode::new(40_000, [800, 840, 968, 1056], [600, 601, 605, 628], P)),
    Some(Mode::new(50_000, [800, 856, 976, 1040], [600, 637, 643, 666], P)),
    Some(Mode::new(49_500, [800, 816, 896, 1056], [600, 601, 604, 625], P)),
    None, None,
    Some(Mode::new(65_000, [1024, 1048, 1184, 1344], [768, 771, 777, 806], 0)),
    Some(Mode::new(75_000, [1024, 1048, 1184, 1328], [768, 771, 777, 806], 0)),
    Some(Mode::new(78_750, [1024, 1040, 1136, 1312], [768, 769, 772, 800], P)),
    Some(Mode::new(135_000, [1280, 1296, 1440, 1688], [1024, 1025, 1028, 1066], P)),
];

/// CTA-861 video identification codes for the formats TVs and HDMI monitors list.
pub fn cta_vic(vic: u8) -> Option<Mode> {
    Some(match vic {
        1 => Mode::new(25_175, [640, 656, 752, 800], [480, 490, 492, 525], 0),
        2 | 3 => Mode::new(27_000, [720, 736, 798, 858], [480, 489, 495, 525], 0),
        4 => Mode::new(74_250, [1280, 1390, 1430, 1650], [720, 725, 730, 750], P),
        16 => Mode::new(148_500, [1920, 2008, 2052, 2200], [1080, 1084, 1089, 1125], P),
        17 | 18 => Mode::new(27_000, [720, 732, 796, 864], [576, 581, 586, 625], 0),
        19 => Mode::new(74_250, [1280, 1720, 1760, 1980], [720, 725, 730, 750], P),
        31 => Mode::new(148_500, [1920, 2448, 2492, 2640], [1080, 1084, 1089, 1125], P),
        34 => Mode::new(74_250, [1920, 2008, 2052, 2200], [1080, 1084, 1089, 1125], P),
        63 => Mode::new(297_000, [1920, 2008, 2052, 2200], [1080, 1084, 1089, 1125], P),
        95 => Mode::new(297_000, [3840, 4016, 4104, 4400], [2160, 2168, 2178, 2250], P),
        96 => Mode::new(594_000, [3840, 4896, 4984, 5280], [2160, 2168, 2178, 2250], P),
        97 => Mode::new(594_000, [3840, 4016, 4104, 4400], [2160, 2168, 2178, 2250], P),
        _ => return None,
    })
}
