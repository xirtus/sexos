//! Read immutable BOOT display metadata. Dimensions grant no framebuffer access.
pub const OP: u64 = 0xE6;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}
pub fn geometry() -> Result<Geometry, u64> {
    let v = crate::pdx_call_checked(0, OP, 0, 0, 0)?;
    let pitch = crate::pdx_call_checked(0, OP, 2, 0, 0)?;
    let g = Geometry {
        width: v as u32,
        height: (v >> 32) as u32,
        pitch: pitch as u32,
    };
    if g.width == 0 || g.height == 0 || g.width > 8192 || g.height > 8192 || pitch > u32::MAX as u64
    {
        return Err(crate::ERR_CAP_INVALID);
    }
    Ok(g)
}
/// Only the exact BOOT display subject can resolve its existing framebuffer.
pub fn framebuffer() -> Result<u64, u64> {
    crate::pdx_call_checked(0, OP, 1, 0, 0)
}

/// What BOOT learned about the screen itself: the monitor's size from its EDID
/// and the whole-number interface scale that follows from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Physical {
    /// 1 for ordinary screens, 2 for HiDPI, 3 for very dense panels.
    pub scale: u32,
    /// Image size in millimetres; 0 when the monitor did not say.
    pub width_mm: u32,
    pub height_mm: u32,
}
/// Largest whole-number scale BOOT chooses or accepts.
pub const MAX_SCALE: u32 = 3;
impl Physical {
    pub const fn pack(self) -> u64 {
        (self.scale as u64 & 0xff) | ((self.width_mm as u64 & 0xffff) << 8) | ((self.height_mm as u64 & 0xffff) << 24)
    }
    pub const fn unpack(v: u64) -> Self {
        Self { scale: (v & 0xff) as u32, width_mm: ((v >> 8) & 0xffff) as u32, height_mm: ((v >> 24) & 0xffff) as u32 }
    }
    /// The scale and size BOOT reads from a monitor's EDID at a given mode.
    pub fn from_edid(edid: &[u8], width: u32, height: u32) -> Self {
        let (width_mm, height_mm) = edid_size_mm(edid).unwrap_or((0, 0));
        Self { scale: default_scale(width, height, width_mm, height_mm), width_mm, height_mm }
    }
}
/// Read the screen's scale and size. Shell, Display and Input may ask.
pub fn physical() -> Result<Physical, u64> {
    let p = Physical::unpack(crate::pdx_call_checked(0, OP, 3, 0, 0)?);
    if p.scale == 0 || p.scale > MAX_SCALE {
        return Err(crate::ERR_CAP_INVALID);
    }
    Ok(p)
}

/// Image size in millimetres from an EDID 1.x block: the first detailed timing
/// descriptor when it carries one, else the coarse centimetre fields.
pub fn edid_size_mm(edid: &[u8]) -> Option<(u32, u32)> {
    if edid.len() < 128 || edid[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    let d = &edid[54..72];
    // A detailed timing has a non-zero pixel clock.
    if d[0] != 0 || d[1] != 0 {
        let w = d[12] as u32 | ((d[14] as u32 >> 4) << 8);
        let h = d[13] as u32 | ((d[14] as u32 & 0xf) << 8);
        if w >= 50 && h >= 30 {
            return Some((w, h));
        }
    }
    let (w, h) = (edid[21] as u32 * 10, edid[22] as u32 * 10);
    // 0 means unknown or a projector; tiny values are bogus.
    (w >= 50 && h >= 30).then_some((w, h))
}

/// Whole-number interface scale for a mode on a screen of the given size.
/// About 96 pixels per inch reads well at 1; laptop and 4K panels need 2.
/// Without a trustworthy size, only very tall modes (4K and up) get 2.
pub const fn default_scale(width: u32, height: u32, width_mm: u32, height_mm: u32) -> u32 {
    if width == 0 || height == 0 {
        return 1;
    }
    // Sizes that disagree with the mode's shape by more than a third are wrong
    // (some monitors report 16:9 cm on a 16:10 panel; that is fine).
    let plausible = width_mm >= 100
        && height_mm >= 60
        && (width as u64 * height_mm as u64 * 3) < (height as u64 * width_mm as u64 * 4)
        && (width as u64 * height_mm as u64 * 4) > (height as u64 * width_mm as u64 * 3);
    if !plausible {
        return if height >= 2000 { 2 } else { 1 };
    }
    // Pixels per inch, horizontally: width / (width_mm / 25.4).
    let dpi = (width as u64 * 254) / (width_mm as u64 * 10);
    // Only modes with room for a useful desktop at the larger scale get it.
    if dpi >= 288 && height >= 1800 {
        3
    } else if dpi >= 150 && height >= 1200 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edid(w_mm: u32, h_mm: u32, cm: (u8, u8)) -> [u8; 128] {
        let mut e = [0u8; 128];
        e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        e[21] = cm.0;
        e[22] = cm.1;
        e[54] = 0x01;
        e[55] = 0x1d;
        e[54 + 12] = w_mm as u8;
        e[54 + 13] = h_mm as u8;
        e[54 + 14] = (((w_mm >> 8) as u8) << 4) | ((h_mm >> 8) as u8 & 0xf);
        e
    }
    #[test]
    fn common_screens() {
        // 13.3" 2560x1600 laptop (286x179 mm): 227 dpi -> 2.
        assert_eq!(Physical::from_edid(&edid(286, 179, (29, 18)), 2560, 1600).scale, 2);
        // 27" 4K (597x336 mm): 163 dpi -> 2.
        assert_eq!(Physical::from_edid(&edid(597, 336, (60, 34)), 3840, 2160).scale, 2);
        // 24" 1080p (531x299 mm): 92 dpi -> 1.
        assert_eq!(Physical::from_edid(&edid(531, 299, (53, 30)), 1920, 1080).scale, 1);
        // 27" 1440p (597x336 mm): 109 dpi -> 1.
        assert_eq!(Physical::from_edid(&edid(597, 336, (60, 34)), 2560, 1440).scale, 1);
        // 14" 2880x1800 OLED (302x189 mm): 242 dpi -> 2; 13" 3840x2400 (286x179): 341 dpi -> 3.
        assert_eq!(Physical::from_edid(&edid(302, 189, (30, 19)), 2880, 1800).scale, 2);
        assert_eq!(Physical::from_edid(&edid(286, 179, (29, 18)), 3840, 2400).scale, 3);
        // QEMU reports 100 dpi.
        assert_eq!(Physical::from_edid(&edid(325, 203, (33, 20)), 1280, 800).scale, 1);
    }
    #[test]
    fn missing_or_bogus_sizes_fall_back_to_mode_height() {
        assert_eq!(Physical::from_edid(&[], 1280, 800), Physical { scale: 1, width_mm: 0, height_mm: 0 });
        assert_eq!(Physical::from_edid(&[], 3840, 2160).scale, 2);
        // Projector: size 0. Bogus: 16x9 mm. Wrong shape: 600x100 mm.
        assert_eq!(Physical::from_edid(&edid(0, 0, (0, 0)), 1920, 1080).scale, 1);
        assert_eq!(default_scale(2560, 1600, 16, 9), 1);
        assert_eq!(default_scale(2560, 1600, 600, 100), 1);
        // Only the coarse centimetre fields.
        let mut e = edid(0, 0, (29, 18));
        e[54] = 0;
        e[55] = 0;
        assert_eq!(Physical::from_edid(&e, 2560, 1600), Physical { scale: 2, width_mm: 290, height_mm: 180 });
    }
    #[test]
    fn pack_round_trips() {
        let p = Physical { scale: 2, width_mm: 597, height_mm: 336 };
        assert_eq!(Physical::unpack(p.pack()), p);
    }
}
