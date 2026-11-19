// SPDX-License-Identifier: GPL-3.0-only
//! HEVC / H.265 NAL unit and SPS/PPS parsing for HEIC images.

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;

/// Unescapes emulation prevention bytes (`0x00 0x00 0x03` -> `0x00 0x00`).
pub fn unescape_rbsp(nal_data: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    out.try_reserve_exact(nal_data.len()).map_err(|_| Error::OutOfMemory)?;
    let mut i = 0;
    while i < nal_data.len() {
        if i + 2 < nal_data.len() && nal_data[i] == 0 && nal_data[i + 1] == 0 && nal_data[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3; // Skip emulation prevention 0x03
        } else {
            out.push(nal_data[i]);
            i += 1;
        }
    }
    Ok(())
}

/// Bit reader for Exponential-Golomb codes and SPS/PPS fields.
pub struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    #[inline]
    pub fn read_bit(&mut self) -> Result<u8, Error> {
        let byte_idx = self.bit_pos >> 3;
        let bit_idx = 7 - (self.bit_pos & 7);
        if byte_idx >= self.data.len() {
            return Err(Error::CorruptData);
        }
        self.bit_pos += 1;
        Ok((self.data[byte_idx] >> bit_idx) & 1)
    }

    #[inline]
    pub fn read_bits(&mut self, n: usize) -> Result<u32, Error> {
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | (self.read_bit()? as u32);
        }
        Ok(val)
    }

    /// Reads an unsigned Exponential-Golomb coded integer (ue(v)).
    pub fn read_ue(&mut self) -> Result<u32, Error> {
        let mut leading_zeros = 0;
        while self.read_bit()? == 0 {
            leading_zeros += 1;
            if leading_zeros > 31 {
                return Err(Error::CorruptData);
            }
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        Ok((1 << leading_zeros) - 1 + suffix)
    }

    /// Reads a signed Exponential-Golomb coded integer (se(v)).
    pub fn read_se(&mut self) -> Result<i32, Error> {
        let code = self.read_ue()?;
        let val = ((code + 1) >> 1) as i32;
        if (code & 1) == 0 {
            Ok(-val)
        } else {
            Ok(val)
        }
    }
}

/// Sequence Parameter Set (SPS) parameters.
#[derive(Clone, Copy, Debug)]
pub struct SpsInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub chroma_format_idc: u8,
}

/// Parses SPS NAL unit payload.
pub fn parse_sps(rbsp: &[u8]) -> Result<SpsInfo, Error> {
    if rbsp.len() < 16 {
        return Err(Error::CorruptData);
    }
    let mut r = BitReader::new(rbsp);
    // Skip NAL unit header (2 bytes)
    r.read_bits(16)?;

    let _vps_id = r.read_bits(4)?;
    let max_sub_layers_minus1 = r.read_bits(3)? as usize;
    let _temporal_id_nesting_flag = r.read_bit()?;

    // profile_tier_level
    r.read_bits(2)?; // general_profile_space
    r.read_bits(1)?; // general_tier_flag
    r.read_bits(5)?; // general_profile_idc
    r.read_bits(32)?; // general_profile_compatibility_flags
    r.read_bits(48)?; // constraint flags
    r.read_bits(8)?; // general_level_idc

    let mut sub_layer_profile_present_flag = [false; 8];
    let mut sub_layer_level_present_flag = [false; 8];
    for i in 0..max_sub_layers_minus1 {
        sub_layer_profile_present_flag[i] = r.read_bit()? != 0;
        sub_layer_level_present_flag[i] = r.read_bit()? != 0;
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            r.read_bits(2)?;
        }
    }
    for i in 0..max_sub_layers_minus1 {
        if sub_layer_profile_present_flag[i] {
            r.read_bits(88)?;
        }
        if sub_layer_level_present_flag[i] {
            r.read_bits(8)?;
        }
    }

    let _sps_seq_parameter_set_id = r.read_ue()?;
    let chroma_format_idc = r.read_ue()? as u8;
    if chroma_format_idc == 3 {
        r.read_bit()?; // separate_colour_plane_flag
    }

    let pic_width = r.read_ue()?;
    let pic_height = r.read_ue()?;

    let conformance_window = r.read_bit()? != 0;
    let (crop_left, crop_right, crop_top, crop_bottom) = if conformance_window {
        let cl = r.read_ue()?;
        let cr = r.read_ue()?;
        let ct = r.read_ue()?;
        let cb = r.read_ue()?;
        (cl, cr, ct, cb)
    } else {
        (0, 0, 0, 0)
    };

    let bit_depth_luma_minus8 = r.read_ue()? as u8;
    let _bit_depth_chroma_minus8 = r.read_ue()?;

    let (sub_width_c, sub_height_c) = match chroma_format_idc {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };

    let crop_unit_x = sub_width_c;
    let crop_unit_y = sub_height_c;

    let width = pic_width.saturating_sub((crop_left + crop_right) * crop_unit_x);
    let height = pic_height.saturating_sub((crop_top + crop_bottom) * crop_unit_y);

    Ok(SpsInfo {
        width,
        height,
        bit_depth: bit_depth_luma_minus8 + 8,
        chroma_format_idc,
    })
}

/// Parses the `hvcC` box payload to extract VPS, SPS, and PPS NALUs.
pub fn parse_hvcc(hvcc_payload: &[u8]) -> Result<Option<SpsInfo>, Error> {
    if hvcc_payload.len() < 23 {
        return Err(Error::CorruptData);
    }
    let num_of_arrays = hvcc_payload[22] as usize;
    let mut offset = 23;
    let mut sps_info = None;

    for _ in 0..num_of_arrays {
        if offset + 3 > hvcc_payload.len() {
            break;
        }
        let nal_type = hvcc_payload[offset] & 0x3f;
        let num_nalus = u16::from_be_bytes([hvcc_payload[offset + 1], hvcc_payload[offset + 2]]) as usize;
        offset += 3;

        for _ in 0..num_nalus {
            if offset + 2 > hvcc_payload.len() {
                break;
            }
            let nal_len = u16::from_be_bytes([hvcc_payload[offset], hvcc_payload[offset + 1]]) as usize;
            offset += 2;
            if offset + nal_len > hvcc_payload.len() {
                break;
            }
            let nal_bytes = &hvcc_payload[offset..offset + nal_len];
            offset += nal_len;

            if nal_type == 33 {
                // SPS NALU
                let mut unescaped = Vec::new();
                if unescape_rbsp(nal_bytes, &mut unescaped).is_ok() {
                    if let Ok(info) = parse_sps(&unescaped) {
                        sps_info = Some(info);
                    }
                }
            }
        }
    }

    Ok(sps_info)
}
