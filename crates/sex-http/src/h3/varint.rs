// SPDX-License-Identifier: GPL-3.0-only
//! RFC 9000 Section 16 variable-length integer codec (1, 2, 4, or 8 bytes).

use alloc::vec::Vec;

/// Maximum value representable as a 62-bit QUIC variable-length integer (`2^62 - 1`).
pub const MAX_VARINT: u64 = (1u64 << 62) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarintError {
    /// Not enough input bytes to decode the variable-length integer.
    Incomplete,
    /// Value exceeds `MAX_VARINT` (`2^62 - 1`).
    Overflow,
}

/// Number of wire bytes required to encode `value`, or `None` if `value > MAX_VARINT`.
pub const fn encoded_len(value: u64) -> Option<usize> {
    if value < (1 << 6) {
        Some(1)
    } else if value < (1 << 14) {
        Some(2)
    } else if value < (1 << 30) {
        Some(4)
    } else if value <= MAX_VARINT {
        Some(8)
    } else {
        None
    }
}

/// Encodes `value` into `out` using the minimal RFC 9000 varint width.
pub fn encode(value: u64, out: &mut Vec<u8>) -> Result<usize, VarintError> {
    if value < (1 << 6) {
        out.push(value as u8);
        Ok(1)
    } else if value < (1 << 14) {
        let bytes = ((value as u16) | 0x4000).to_be_bytes();
        out.extend_from_slice(&bytes);
        Ok(2)
    } else if value < (1 << 30) {
        let bytes = ((value as u32) | 0x8000_0000).to_be_bytes();
        out.extend_from_slice(&bytes);
        Ok(4)
    } else if value <= MAX_VARINT {
        let bytes = (value | 0xC000_0000_0000_0000).to_be_bytes();
        out.extend_from_slice(&bytes);
        Ok(8)
    } else {
        Err(VarintError::Overflow)
    }
}

/// Decodes one RFC 9000 variable-length integer from `input`, returning `(value, bytes_consumed)`.
pub fn decode(input: &[u8]) -> Result<(u64, usize), VarintError> {
    let first = *input.first().ok_or(VarintError::Incomplete)?;
    let len = 1usize << (first >> 6);
    if input.len() < len {
        return Err(VarintError::Incomplete);
    }
    let mut value = (first & 0x3F) as u64;
    for &b in &input[1..len] {
        value = (value << 8) | (b as u64);
    }
    Ok((value, len))
}
