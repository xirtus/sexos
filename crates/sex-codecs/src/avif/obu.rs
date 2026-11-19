// SPDX-License-Identifier: GPL-3.0-only
//! AV1 Open Bitstream Unit (OBU) parsing for AVIF images.

use crate::Error;
use crate::image::MatrixCoefficients;

/// Reads an unsigned LEB128 integer from `data` at `offset`.
pub fn read_leb128(data: &[u8], offset: &mut usize) -> Result<usize, Error> {
    let mut value = 0usize;
    let mut shift = 0;
    while *offset < data.len() && shift < 64 {
        let byte = data[*offset];
        *offset += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if (byte & 0x80) == 0 {
            return Ok(value);
        }
        shift += 7;
    }
    Err(Error::CorruptData)
}

/// Bit reader for uncompressed OBU headers.
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
        if n > 32 {
            return Err(Error::Unsupported);
        }
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | (self.read_bit()? as u32);
        }
        Ok(val)
    }

    #[inline]
    pub fn byte_align(&mut self) {
        let rem = self.bit_pos & 7;
        if rem != 0 {
            self.bit_pos += 8 - rem;
        }
    }

    #[inline]
    pub fn byte_offset(&self) -> usize {
        (self.bit_pos + 7) >> 3
    }
}

/// Header information parsed from an AV1 OBU.
#[derive(Clone, Copy, Debug)]
pub struct ObuHeader {
    pub obu_type: u8,
    pub has_size: bool,
    pub payload_size: usize,
    pub payload_offset: usize,
}

/// Parses an OBU header from `data` at `offset`.
pub fn parse_obu_header(data: &[u8], mut offset: usize) -> Result<ObuHeader, Error> {
    if offset >= data.len() {
        return Err(Error::CorruptData);
    }
    let b0 = data[offset];
    offset += 1;
    let forbidden = (b0 >> 7) & 1;
    if forbidden != 0 {
        return Err(Error::CorruptData);
    }
    let obu_type = (b0 >> 3) & 0x0f;
    let extension_flag = (b0 >> 2) & 1;
    let has_size = (b0 >> 1) & 1 != 0;

    if extension_flag != 0 {
        if offset >= data.len() {
            return Err(Error::CorruptData);
        }
        offset += 1; // Skip extension byte
    }

    let payload_size = if has_size {
        read_leb128(data, &mut offset)?
    } else {
        data.len().saturating_sub(offset)
    };

    if offset.checked_add(payload_size).map_or(true, |end| end > data.len()) {
        return Err(Error::CorruptData);
    }

    Ok(ObuHeader {
        obu_type,
        has_size,
        payload_size,
        payload_offset: offset,
    })
}

/// AV1 Sequence Header information.
#[derive(Clone, Copy, Debug)]
pub struct SequenceHeader {
    pub profile: u8,
    pub still_picture: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub bit_depth: u8,
    pub monochrome: bool,
    pub subsampling_x: u8,
    pub subsampling_y: u8,
    pub matrix_coefficients: MatrixCoefficients,
    pub full_range: bool,
}

/// Parses the AV1 Sequence Header OBU.
pub fn parse_sequence_header(payload: &[u8]) -> Result<SequenceHeader, Error> {
    let mut r = BitReader::new(payload);
    let profile = r.read_bits(3)? as u8;
    let still_picture = r.read_bit()? != 0;
    let _reduced_still_picture_header = r.read_bit()?;

    let (max_width, max_height) = if r.read_bit()? != 0 {
        // timing_info_present_flag
        return Err(Error::Unsupported);
    } else {
        let frame_width_bits = r.read_bits(4)? as usize + 1;
        let frame_height_bits = r.read_bits(4)? as usize + 1;
        let w = r.read_bits(frame_width_bits)? + 1;
        let h = r.read_bits(frame_height_bits)? + 1;
        (w, h)
    };

    // Color config
    let high_bitdepth = r.read_bit()? != 0;
    let bit_depth = if profile == 2 && high_bitdepth {
        let twelve_bit = r.read_bit()? != 0;
        if twelve_bit { 12 } else { 10 }
    } else if high_bitdepth {
        10
    } else {
        8
    };

    let monochrome = if profile == 1 {
        false
    } else {
        r.read_bit()? != 0
    };

    let color_description_present = r.read_bit()? != 0;
    let (matrix, full_range) = if color_description_present {
        let _cp = r.read_bits(8)?;
        let _tc = r.read_bits(8)?;
        let mc = r.read_bits(8)? as u16;
        let range = r.read_bit()? != 0;
        (MatrixCoefficients::from(mc), range)
    } else {
        (MatrixCoefficients::Bt709, false)
    };

    let (sub_x, sub_y) = if monochrome {
        (1, 1)
    } else if profile == 0 {
        (1, 1) // 4:2:0
    } else if profile == 1 {
        (0, 0) // 4:4:4
    } else {
        let ss_x = r.read_bit()?;
        let ss_y = if ss_x != 0 { r.read_bit()? } else { 0 };
        (ss_x, ss_y)
    };

    Ok(SequenceHeader {
        profile,
        still_picture,
        max_width,
        max_height,
        bit_depth,
        monochrome,
        subsampling_x: sub_x,
        subsampling_y: sub_y,
        matrix_coefficients: matrix,
        full_range,
    })
}
