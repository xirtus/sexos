// SPDX-License-Identifier: GPL-3.0-only
//! Pure-Rust Virtual Camera Loopback Feed for `Cabaret`.
//!
//! Provides a bounded double-buffer (`VirtualCamRing`) and BT.601 integer color
//! converters (`rgba_to_yuy2`, `rgba_to_nv12`) so `Cabaret` can publish
//! `"Cabaret Virtual Camera"` frames directly to `sexuvc`, `Flirt`, and
//! `Kaleidoscope` without requiring Linux `v4l2loopback` kernel modules.

#![forbid(unsafe_code)]

use crate::pixel::{nv12_to_argb, yuy2_to_argb, PixelError};
use alloc::vec::Vec;

/// Advertised virtual camera device name for Cabaret loopback feeds.
pub const VIRTUAL_CAM_NAME: &str = "Cabaret Virtual Camera";

/// Maximum supported frame dimension in pixels for virtual camera loopback.
pub const MAX_VIRTUAL_CAM_DIM: u32 = 7680;

/// Serialized wire header byte length (`VirtualCamHeader::to_bytes`).
pub const VIRTUAL_CAM_HEADER_BYTES: usize = 32;

const HEADER_MAGIC: [u8; 4] = *b"CVC1";

/// Errors returned by virtual camera frame conversion or ring operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualCamError {
    InvalidDimensions,
    BufferTooShort,
    BufferTooSmall,
    FrameTooLarge,
    InvalidHeader,
    UnsupportedFormat,
    Pixel(PixelError),
}

impl From<PixelError> for VirtualCamError {
    fn from(err: PixelError) -> Self {
        Self::Pixel(err)
    }
}

/// Which Cabaret bus or source feeds the virtual camera loopback device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualCamSource {
    Program,
    Preview,
    Studio(u32),
    IsolatedSource(u32),
}

impl VirtualCamSource {
    pub const fn tag(self) -> u8 {
        match self {
            Self::Program => 0,
            Self::Preview => 1,
            Self::Studio(_) => 2,
            Self::IsolatedSource(_) => 3,
        }
    }

    pub const fn id(self) -> u32 {
        match self {
            Self::Program | Self::Preview => 0,
            Self::Studio(id) | Self::IsolatedSource(id) => id,
        }
    }

    pub const fn from_parts(tag: u8, id: u32) -> Option<Self> {
        match tag {
            0 => Some(Self::Program),
            1 => Some(Self::Preview),
            2 => Some(Self::Studio(id)),
            3 => Some(Self::IsolatedSource(id)),
            _ => None,
        }
    }
}

/// Pixel format carried by a virtual camera frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualCamFormat {
    Yuy2,
    Nv12,
    Rgba32,
}

impl VirtualCamFormat {
    pub const fn tag(self) -> u8 {
        match self {
            Self::Yuy2 => 0,
            Self::Nv12 => 1,
            Self::Rgba32 => 2,
        }
    }

    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Yuy2),
            1 => Some(Self::Nv12),
            2 => Some(Self::Rgba32),
            _ => None,
        }
    }

    /// Validates dimensions and returns the exact payload byte count for this format.
    pub fn frame_bytes(self, width: u32, height: u32) -> Result<usize, VirtualCamError> {
        if width == 0
            || height == 0
            || width > MAX_VIRTUAL_CAM_DIM
            || height > MAX_VIRTUAL_CAM_DIM
        {
            return Err(VirtualCamError::InvalidDimensions);
        }
        let w = width as usize;
        let h = height as usize;
        let pixels = w.checked_mul(h).ok_or(VirtualCamError::InvalidDimensions)?;
        match self {
            Self::Yuy2 => {
                if (w & 1) != 0 {
                    return Err(VirtualCamError::InvalidDimensions);
                }
                pixels.checked_mul(2).ok_or(VirtualCamError::InvalidDimensions)
            }
            Self::Nv12 => {
                if (w & 1) != 0 || (h & 1) != 0 {
                    return Err(VirtualCamError::InvalidDimensions);
                }
                let uv = pixels / 2;
                pixels.checked_add(uv).ok_or(VirtualCamError::InvalidDimensions)
            }
            Self::Rgba32 => pixels.checked_mul(4).ok_or(VirtualCamError::InvalidDimensions),
        }
    }
}

/// Metadata header attached to each frame published on a virtual camera ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualCamHeader {
    pub epoch: u64,
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub format: VirtualCamFormat,
    pub active_tally: bool,
}

impl VirtualCamHeader {
    pub fn payload_len(&self) -> Result<usize, VirtualCamError> {
        self.format.frame_bytes(self.width, self.height)
    }

    /// Encodes the header into a fixed 32-byte little-endian wire block.
    pub fn to_bytes(&self) -> [u8; VIRTUAL_CAM_HEADER_BYTES] {
        let mut out = [0u8; VIRTUAL_CAM_HEADER_BYTES];
        out[0..4].copy_from_slice(&HEADER_MAGIC);
        out[4..12].copy_from_slice(&self.epoch.to_le_bytes());
        out[12..20].copy_from_slice(&self.sequence.to_le_bytes());
        out[20..24].copy_from_slice(&self.width.to_le_bytes());
        out[24..28].copy_from_slice(&self.height.to_le_bytes());
        let fps_u16 = self.fps.min(u16::MAX as u32) as u16;
        out[28..30].copy_from_slice(&fps_u16.to_le_bytes());
        out[30] = self.format.tag();
        out[31] = u8::from(self.active_tally);
        out
    }

    /// Decodes a 32-byte little-endian wire block into a validated [`VirtualCamHeader`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VirtualCamError> {
        if bytes.len() < VIRTUAL_CAM_HEADER_BYTES {
            return Err(VirtualCamError::BufferTooShort);
        }
        if bytes[0..4] != HEADER_MAGIC {
            return Err(VirtualCamError::InvalidHeader);
        }
        let epoch = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
        let sequence = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let width = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let fps = u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as u32;
        let format =
            VirtualCamFormat::from_tag(bytes[30]).ok_or(VirtualCamError::InvalidHeader)?;
        let active_tally = bytes[31] != 0;
        let header = Self {
            epoch,
            sequence,
            width,
            height,
            fps,
            format,
            active_tally,
        };
        let _ = header.payload_len()?;
        Ok(header)
    }
}

/// Computes studio-swing BT.601 `(Y, U, V)` from `(R, G, B)`.
#[inline(always)]
pub fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;

    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

    (
        y.clamp(16, 235) as u8,
        u.clamp(16, 240) as u8,
        v.clamp(16, 240) as u8,
    )
}

#[inline(always)]
fn rgb_to_uv(r: i32, g: i32, b: i32) -> (u8, u8) {
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    (u.clamp(16, 240) as u8, v.clamp(16, 240) as u8)
}

#[inline(always)]
fn rgb_to_y(r: u8, g: u8, b: u8) -> u8 {
    let y = ((66 * r as i32 + 129 * g as i32 + 25 * b as i32 + 128) >> 8) + 16;
    y.clamp(16, 235) as u8
}

/// Converts packed RGBA32 (`[R, G, B, A]` per pixel) to YUY2 (YUV 4:2:2, `[Y0, U, Y1, V]`).
pub fn rgba_to_yuy2(
    rgba: &[u8],
    width: usize,
    height: usize,
    out: &mut [u8],
) -> Result<(), PixelError> {
    if width == 0 || height == 0 || (width & 1) != 0 {
        return Err(PixelError::InvalidDimensions);
    }
    let total_pixels = width.checked_mul(height).ok_or(PixelError::InvalidDimensions)?;
    let rgba_needed = total_pixels.checked_mul(4).ok_or(PixelError::InvalidDimensions)?;
    let yuy2_needed = total_pixels.checked_mul(2).ok_or(PixelError::InvalidDimensions)?;

    if rgba.len() < rgba_needed {
        return Err(PixelError::BufferTooShort);
    }
    if out.len() < yuy2_needed {
        return Err(PixelError::BufferTooSmall);
    }

    let pairs = total_pixels / 2;
    for i in 0..pairs {
        let s0 = i * 8;
        let r0 = rgba[s0];
        let g0 = rgba[s0 + 1];
        let b0 = rgba[s0 + 2];

        let r1 = rgba[s0 + 4];
        let g1 = rgba[s0 + 5];
        let b1 = rgba[s0 + 6];

        let y0 = rgb_to_y(r0, g0, b0);
        let y1 = rgb_to_y(r1, g1, b1);
        let (u, v) = rgb_to_uv(
            (r0 as i32 + r1 as i32 + 1) >> 1,
            (g0 as i32 + g1 as i32 + 1) >> 1,
            (b0 as i32 + b1 as i32 + 1) >> 1,
        );

        let dst = i * 4;
        out[dst] = y0;
        out[dst + 1] = u;
        out[dst + 2] = y1;
        out[dst + 3] = v;
    }

    Ok(())
}

/// Converts packed RGBA32 (`[R, G, B, A]` per pixel) to NV12 (YUV 4:2:0 semi-planar).
pub fn rgba_to_nv12(
    rgba: &[u8],
    width: usize,
    height: usize,
    out: &mut [u8],
) -> Result<(), PixelError> {
    if width == 0 || height == 0 || (width & 1) != 0 || (height & 1) != 0 {
        return Err(PixelError::InvalidDimensions);
    }
    let total_pixels = width.checked_mul(height).ok_or(PixelError::InvalidDimensions)?;
    let rgba_needed = total_pixels.checked_mul(4).ok_or(PixelError::InvalidDimensions)?;
    let uv_size = total_pixels / 2;
    let nv12_needed = total_pixels.checked_add(uv_size).ok_or(PixelError::InvalidDimensions)?;

    if rgba.len() < rgba_needed {
        return Err(PixelError::BufferTooShort);
    }
    if out.len() < nv12_needed {
        return Err(PixelError::BufferTooSmall);
    }

    let (y_plane, uv_rest) = out.split_at_mut(total_pixels);
    let uv_plane = &mut uv_rest[..uv_size];

    for idx in 0..total_pixels {
        let s = idx * 4;
        y_plane[idx] = rgb_to_y(rgba[s], rgba[s + 1], rgba[s + 2]);
    }

    let row_stride = width * 4;
    for uv_row in 0..(height / 2) {
        let r0_off = (uv_row * 2) * row_stride;
        let r1_off = r0_off + row_stride;
        let uv_row_off = uv_row * width;

        for uv_col in 0..(width / 2) {
            let p0 = r0_off + uv_col * 8;
            let p1 = r1_off + uv_col * 8;

            let r_avg = (rgba[p0] as i32
                + rgba[p0 + 4] as i32
                + rgba[p1] as i32
                + rgba[p1 + 4] as i32
                + 2)
                >> 2;
            let g_avg = (rgba[p0 + 1] as i32
                + rgba[p0 + 5] as i32
                + rgba[p1 + 1] as i32
                + rgba[p1 + 5] as i32
                + 2)
                >> 2;
            let b_avg = (rgba[p0 + 2] as i32
                + rgba[p0 + 6] as i32
                + rgba[p1 + 2] as i32
                + rgba[p1 + 6] as i32
                + 2)
                >> 2;

            let (u, v) = rgb_to_uv(r_avg, g_avg, b_avg);
            let dst = uv_row_off + uv_col * 2;
            uv_plane[dst] = u;
            uv_plane[dst + 1] = v;
        }
    }

    Ok(())
}

/// Converts packed RGBA32 (`[R, G, B, A]`) to 0xFFRRGGBB ARGB pixels.
pub fn rgba_to_argb(
    rgba: &[u8],
    width: usize,
    height: usize,
    out: &mut [u32],
) -> Result<(), PixelError> {
    if width == 0 || height == 0 {
        return Err(PixelError::InvalidDimensions);
    }
    let total_pixels = width.checked_mul(height).ok_or(PixelError::InvalidDimensions)?;
    let rgba_needed = total_pixels.checked_mul(4).ok_or(PixelError::InvalidDimensions)?;
    if rgba.len() < rgba_needed {
        return Err(PixelError::BufferTooShort);
    }
    if out.len() < total_pixels {
        return Err(PixelError::BufferTooSmall);
    }
    for i in 0..total_pixels {
        let s = i * 4;
        let r = rgba[s] as u32;
        let g = rgba[s + 1] as u32;
        let b = rgba[s + 2] as u32;
        out[i] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct RingSlot {
    header: VirtualCamHeader,
    data: Vec<u8>,
}

impl RingSlot {
    fn empty() -> Self {
        Self {
            header: VirtualCamHeader {
                epoch: 0,
                sequence: 0,
                width: 0,
                height: 0,
                fps: 0,
                format: VirtualCamFormat::Yuy2,
                active_tally: false,
            },
            data: Vec::new(),
        }
    }
}

/// Bounded double-buffered virtual camera ring connecting `Cabaret` to `sexuvc`,
/// `Flirt`, and `Kaleidoscope` without kernel-level `v4l2loopback`.
#[derive(Clone, Debug)]
pub struct VirtualCamRing {
    source: VirtualCamSource,
    max_frame_bytes: usize,
    slots: [RingSlot; 2],
    latest_idx: Option<usize>,
    epoch: u64,
    next_sequence: u64,
}

impl VirtualCamRing {
    /// Creates a new bounded double-buffer ring for `source` with per-slot cap `max_frame_bytes`.
    pub fn new(source: VirtualCamSource, max_frame_bytes: usize) -> Result<Self, VirtualCamError> {
        if max_frame_bytes == 0 {
            return Err(VirtualCamError::FrameTooLarge);
        }
        Ok(Self {
            source,
            max_frame_bytes,
            slots: [RingSlot::empty(), RingSlot::empty()],
            latest_idx: None,
            epoch: 1,
            next_sequence: 1,
        })
    }

    /// Creates a ring pre-sized for a maximum resolution and pixel format.
    pub fn with_max_frame(
        source: VirtualCamSource,
        max_width: u32,
        max_height: u32,
        format: VirtualCamFormat,
    ) -> Result<Self, VirtualCamError> {
        let max_bytes = format.frame_bytes(max_width, max_height)?;
        Self::new(source, max_bytes)
    }

    pub const fn device_name(&self) -> &'static str {
        VIRTUAL_CAM_NAME
    }

    pub const fn source(&self) -> VirtualCamSource {
        self.source
    }

    pub const fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Switches the active Cabaret source and advances the stream epoch so consumers resync.
    pub fn set_source(&mut self, source: VirtualCamSource) {
        if self.source != source {
            self.source = source;
            self.bump_epoch();
        }
    }

    /// Advances the stream epoch (e.g., on resolution/format renegotiation) and resets sequence.
    pub fn bump_epoch(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
        self.next_sequence = 1;
    }

    fn back_slot_index(&self) -> usize {
        match self.latest_idx {
            Some(0) => 1,
            _ => 0,
        }
    }

    /// Publishes an already-encoded frame (`Yuy2`, `Nv12`, or `Rgba32`) into the inactive slot
    /// and atomically flips it to become the latest readable frame.
    pub fn publish(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
        format: VirtualCamFormat,
        active_tally: bool,
        payload: &[u8],
    ) -> Result<VirtualCamHeader, VirtualCamError> {
        let expected = format.frame_bytes(width, height)?;
        if expected > self.max_frame_bytes {
            return Err(VirtualCamError::FrameTooLarge);
        }
        if payload.len() < expected {
            return Err(VirtualCamError::BufferTooShort);
        }
        if payload.len() > expected {
            return Err(VirtualCamError::InvalidDimensions);
        }

        let seq = self.next_sequence;
        let header = VirtualCamHeader {
            epoch: self.epoch,
            sequence: seq,
            width,
            height,
            fps,
            format,
            active_tally,
        };

        let idx = self.back_slot_index();
        let slot = &mut self.slots[idx];
        slot.data.clear();
        slot.data.extend_from_slice(&payload[..expected]);
        slot.header = header;

        self.latest_idx = Some(idx);
        self.next_sequence = seq.saturating_add(1);
        Ok(header)
    }

    /// Converts a packed RGBA32 frame directly into `out_format` inside the inactive buffer slot
    /// and flips it to become the latest readable frame.
    pub fn publish_rgba(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
        out_format: VirtualCamFormat,
        active_tally: bool,
        rgba: &[u8],
    ) -> Result<VirtualCamHeader, VirtualCamError> {
        let rgba_expected = VirtualCamFormat::Rgba32.frame_bytes(width, height)?;
        if rgba.len() < rgba_expected {
            return Err(VirtualCamError::BufferTooShort);
        }
        let out_expected = out_format.frame_bytes(width, height)?;
        if out_expected > self.max_frame_bytes {
            return Err(VirtualCamError::FrameTooLarge);
        }

        let seq = self.next_sequence;
        let header = VirtualCamHeader {
            epoch: self.epoch,
            sequence: seq,
            width,
            height,
            fps,
            format: out_format,
            active_tally,
        };

        let idx = self.back_slot_index();
        let slot = &mut self.slots[idx];
        slot.data.resize(out_expected, 0);

        match out_format {
            VirtualCamFormat::Rgba32 => {
                slot.data.copy_from_slice(&rgba[..out_expected]);
            }
            VirtualCamFormat::Yuy2 => {
                rgba_to_yuy2(&rgba[..rgba_expected], width as usize, height as usize, &mut slot.data)?;
            }
            VirtualCamFormat::Nv12 => {
                rgba_to_nv12(&rgba[..rgba_expected], width as usize, height as usize, &mut slot.data)?;
            }
        }

        slot.header = header;
        self.latest_idx = Some(idx);
        self.next_sequence = seq.saturating_add(1);
        Ok(header)
    }

    /// Returns the most recently published header and raw payload, if any.
    pub fn latest(&self) -> Option<(&VirtualCamHeader, &[u8])> {
        let idx = self.latest_idx?;
        let slot = &self.slots[idx];
        Some((&slot.header, &slot.data))
    }

    /// Returns the latest frame only if `(epoch, sequence)` is newer than `last_seen`.
    pub fn poll_newer(&self, last_seen: Option<(u64, u64)>) -> Option<(&VirtualCamHeader, &[u8])> {
        let (header, data) = self.latest()?;
        if let Some((seen_epoch, seen_seq)) = last_seen {
            if header.epoch == seen_epoch && header.sequence <= seen_seq {
                return None;
            }
        }
        Some((header, data))
    }

    /// Decodes the latest frame (any supported format) directly into 0xFFRRGGBB ARGB pixels.
    pub fn decode_latest_argb(
        &self,
        out: &mut [u32],
    ) -> Result<Option<(VirtualCamHeader, usize, usize)>, VirtualCamError> {
        let Some((header, data)) = self.latest() else {
            return Ok(None);
        };
        let w = header.width as usize;
        let h = header.height as usize;
        match header.format {
            VirtualCamFormat::Yuy2 => yuy2_to_argb(data, w, h, out)?,
            VirtualCamFormat::Nv12 => nv12_to_argb(data, w, h, out)?,
            VirtualCamFormat::Rgba32 => rgba_to_argb(data, w, h, out)?,
        }
        Ok(Some((*header, w, h)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn make_solid_rgba(width: usize, height: usize, r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(width * height * 4);
        for _ in 0..(width * height) {
            out.extend_from_slice(&[r, g, b, a]);
        }
        out
    }

    #[test]
    fn rgba_to_yuy2_roundtrips_colors_via_bt601() {
        let w = 4;
        let h = 2;
        // 2 black pixels, 2 white pixels on row 0; 2 red, 2 green on row 1
        let mut rgba = Vec::new();
        for _ in 0..2 {
            rgba.extend_from_slice(&[0, 0, 0, 255]);
        }
        for _ in 0..2 {
            rgba.extend_from_slice(&[255, 255, 255, 255]);
        }
        for _ in 0..2 {
            rgba.extend_from_slice(&[255, 0, 0, 255]);
        }
        for _ in 0..2 {
            rgba.extend_from_slice(&[0, 255, 0, 255]);
        }

        let mut yuy2 = vec![0u8; w * h * 2];
        rgba_to_yuy2(&rgba, w, h, &mut yuy2).unwrap();

        let mut argb = vec![0u32; w * h];
        yuy2_to_argb(&yuy2, w, h, &mut argb).unwrap();

        assert_eq!(argb[0], 0xFF00_0000);
        assert_eq!(argb[1], 0xFF00_0000);
        assert_eq!(argb[2], 0xFFFF_FFFF);
        assert_eq!(argb[3], 0xFFFF_FFFF);
        // Red pair
        assert!(((argb[4] >> 16) & 0xFF) > 240);
        assert!(((argb[4] >> 8) & 0xFF) < 10);
        assert!((argb[4] & 0xFF) < 10);
        // Green pair
        assert!(((argb[6] >> 16) & 0xFF) < 10);
        assert!(((argb[6] >> 8) & 0xFF) > 240);
        assert!((argb[6] & 0xFF) < 10);
    }

    #[test]
    fn rgba_to_nv12_roundtrips_and_validates_dimensions() {
        let w = 4;
        let h = 4;
        let rgba = make_solid_rgba(w, h, 0, 0, 255, 255);
        let mut nv12 = vec![0u8; w * h * 3 / 2];
        rgba_to_nv12(&rgba, w, h, &mut nv12).unwrap();

        let mut argb = vec![0u32; w * h];
        nv12_to_argb(&nv12, w, h, &mut argb).unwrap();
        for p in argb {
            let r = (p >> 16) & 0xFF;
            let g = (p >> 8) & 0xFF;
            let b = p & 0xFF;
            assert!(r < 12 && g < 12 && b > 240, "expected blue, got {r},{g},{b}");
        }

        assert_eq!(
            rgba_to_nv12(&rgba, 3, 4, &mut nv12),
            Err(PixelError::InvalidDimensions)
        );
        assert_eq!(
            rgba_to_nv12(&rgba, 4, 3, &mut nv12),
            Err(PixelError::InvalidDimensions)
        );
        let mut small = vec![0u8; 4];
        assert_eq!(
            rgba_to_nv12(&rgba, w, h, &mut small),
            Err(PixelError::BufferTooSmall)
        );
    }

    #[test]
    fn header_wire_roundtrip_and_rejection() {
        let hdr = VirtualCamHeader {
            epoch: 7,
            sequence: 42,
            width: 1280,
            height: 720,
            fps: 60,
            format: VirtualCamFormat::Nv12,
            active_tally: true,
        };
        let bytes = hdr.to_bytes();
        let decoded = VirtualCamHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, hdr);

        let mut bad = bytes;
        bad[0] = b'X';
        assert_eq!(
            VirtualCamHeader::from_bytes(&bad),
            Err(VirtualCamError::InvalidHeader)
        );
        assert_eq!(
            VirtualCamHeader::from_bytes(&bytes[..10]),
            Err(VirtualCamError::BufferTooShort)
        );
    }

    #[test]
    fn double_buffered_ring_publishes_and_tracks_epochs_and_tally() {
        let mut ring =
            VirtualCamRing::with_max_frame(VirtualCamSource::Program, 8, 8, VirtualCamFormat::Rgba32)
                .unwrap();
        assert_eq!(ring.device_name(), VIRTUAL_CAM_NAME);
        assert!(ring.latest().is_none());

        let rgba1 = make_solid_rgba(4, 4, 200, 10, 10, 255);
        let h1 = ring
            .publish_rgba(4, 4, 30, VirtualCamFormat::Yuy2, true, &rgba1)
            .unwrap();
        assert_eq!(h1.epoch, 1);
        assert_eq!(h1.sequence, 1);
        assert!(h1.active_tally);

        assert!(ring.poll_newer(None).is_some());
        assert!(ring.poll_newer(Some((1, 1))).is_none());

        let rgba2 = make_solid_rgba(4, 4, 10, 200, 10, 255);
        let h2 = ring
            .publish_rgba(4, 4, 30, VirtualCamFormat::Nv12, false, &rgba2)
            .unwrap();
        assert_eq!(h2.epoch, 1);
        assert_eq!(h2.sequence, 2);
        assert!(!h2.active_tally);
        assert!(ring.poll_newer(Some((1, 1))).is_some());

        let mut argb = vec![0u32; 16];
        let (decoded_hdr, dw, dh) = ring.decode_latest_argb(&mut argb).unwrap().unwrap();
        assert_eq!((decoded_hdr, dw, dh), (h2, 4, 4));
        assert!(((argb[0] >> 8) & 0xFF) > 180);

        // Switching source advances epoch and resets sequence.
        ring.set_source(VirtualCamSource::Studio(3));
        assert_eq!(ring.source(), VirtualCamSource::Studio(3));
        assert_eq!(ring.epoch(), 2);

        let h3 = ring
            .publish_rgba(4, 4, 60, VirtualCamFormat::Rgba32, true, &rgba1)
            .unwrap();
        assert_eq!(h3.epoch, 2);
        assert_eq!(h3.sequence, 1);

        // Oversized frames are rejected by the ring bound.
        let big = make_solid_rgba(16, 16, 0, 0, 0, 255);
        assert_eq!(
            ring.publish_rgba(16, 16, 30, VirtualCamFormat::Rgba32, false, &big),
            Err(VirtualCamError::FrameTooLarge)
        );
    }
}
