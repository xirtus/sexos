// SPDX-License-Identifier: GPL-3.0-only
//! CRC-32 (IEEE 802.3, reflected), as GPT headers and partition arrays use it.
const fn table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 { c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }; k += 1; }
        t[i] = c; i += 1;
    }
    t
}
static TABLE: [u32; 256] = table();
#[derive(Clone, Copy)]
pub struct Crc32(u32);
impl Crc32 {
    pub const fn new() -> Self { Self(0xFFFF_FFFF) }
    pub fn update(&mut self, data: &[u8]) { for &b in data { self.0 = TABLE[((self.0 ^ b as u32) & 0xFF) as usize] ^ (self.0 >> 8); } }
    pub fn finish(self) -> u32 { !self.0 }
}
impl Default for Crc32 { fn default() -> Self { Self::new() } }
pub fn crc32(data: &[u8]) -> u32 { let mut c = Crc32::new(); c.update(data); c.finish() }
#[cfg(test)]
mod tests { #[test] fn check_value() { assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926); } }
