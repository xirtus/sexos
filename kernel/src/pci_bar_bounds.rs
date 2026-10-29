// SPDX-License-Identifier: GPL-3.0-only
//! Decode a boot-only BAR probe without granting page padding or wrapped ranges.
pub fn decode_bar0(low: u32, high: u32, mask_low: u32, mask_high: u32) -> Option<(u64, u64)> {
    if low & 1 != 0 { return None; }
    let kind = (low >> 1) & 3;
    let (base, length) = match kind {
        0 => ((low & !15) as u64, (!(mask_low & !15)).checked_add(1)? as u64),
        2 => {
            let base = ((high as u64) << 32) | (low & !15) as u64;
            let mask = ((mask_high as u64) << 32) | (mask_low & !15) as u64;
            (base, (!mask).checked_add(1)?)
        }
        _ => return None,
    };
    if base == 0 || base % 4096 != 0 || length < 4096 || !length.is_power_of_two()
        || base % length != 0 || base.checked_add(length)? > (1u64 << 52)
    { return None; }
    Some((base, length))
}
