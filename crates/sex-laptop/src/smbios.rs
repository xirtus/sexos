// SPDX-License-Identifier: GPL-3.0-only
//! SMBIOS (DMTF DSP0134): the firmware's table of what the machine is.
//!
//! Used for the system's vendor and model (quirk tables key on them, e.g. "Alienware" /
//! "Alienware x17 R1") and for vendor structures such as Dell's token table (type 0xDA,
//! see [`crate::dell_smbios`]). The kernel finds the entry point and hands the structure
//! table to the ACPI service; this module only parses bytes.

/// Entry point (`_SM3_` 64-bit or `_SM_` 32-bit) -> (table address, table length, major, minor).
pub fn entry_point(ep: &[u8]) -> Option<(u64, u32, u8, u8)> {
    if ep.len() >= 24 && &ep[..5] == b"_SM3_" {
        let len = ep[6] as usize;
        if len < 24 || len > ep.len() || checksum(&ep[..len]) != 0 { return None; }
        let max = u32::from_le_bytes([ep[12], ep[13], ep[14], ep[15]]);
        let addr = u64::from_le_bytes([ep[16], ep[17], ep[18], ep[19], ep[20], ep[21], ep[22], ep[23]]);
        return Some((addr, max, ep[7], ep[8]));
    }
    if ep.len() >= 31 && &ep[..4] == b"_SM_" {
        let len = ep[5] as usize;
        if len < 31 || len > ep.len() || checksum(&ep[..len]) != 0 { return None; }
        if &ep[16..21] != b"_DMI_" { return None; }
        let tlen = u16::from_le_bytes([ep[22], ep[23]]) as u32;
        let addr = u32::from_le_bytes([ep[24], ep[25], ep[26], ep[27]]) as u64;
        return Some((addr, tlen, ep[6], ep[7]));
    }
    None
}

fn checksum(b: &[u8]) -> u8 { b.iter().fold(0u8, |s, v| s.wrapping_add(*v)) }

/// One structure: its formatted area and the strings after it.
#[derive(Clone, Copy, Debug)]
pub struct Structure<'a> {
    pub kind: u8,
    pub handle: u16,
    /// The formatted area, header included (`data[1]` is its length).
    pub data: &'a [u8],
    strings: &'a [u8],
}

impl<'a> Structure<'a> {
    /// String number `n` (1-based, 0 = none), as SMBIOS numbers them.
    pub fn string(&self, n: u8) -> Option<&'a str> {
        if n == 0 { return None; }
        let s = self.strings.split(|b| *b == 0).nth(n as usize - 1)?;
        core::str::from_utf8(s).ok().map(str::trim).filter(|s| !s.is_empty())
    }
    /// The string whose number sits at `offset` in the formatted area.
    pub fn string_at(&self, offset: usize) -> Option<&'a str> { self.data.get(offset).and_then(|n| self.string(*n)) }
}

/// Every structure, stopping at end-of-table (type 127) or the first malformed one.
pub fn structures(table: &[u8]) -> impl Iterator<Item = Structure<'_>> {
    let mut at = 0usize;
    let mut done = false;
    core::iter::from_fn(move || {
        if done || at + 4 > table.len() { return None; }
        let len = table[at + 1] as usize;
        if len < 4 || at + len > table.len() { done = true; return None; }
        // Strings end with a double NUL (two NULs right away when there are none).
        let strings_at = at + len;
        let mut end = strings_at;
        loop {
            if end + 1 >= table.len() { done = true; return None; }
            if table[end] == 0 && table[end + 1] == 0 { break; }
            end += 1;
        }
        let s = Structure {
            kind: table[at],
            handle: u16::from_le_bytes([table[at + 2], table[at + 3]]),
            data: &table[at..at + len],
            strings: &table[strings_at..end + 1],
        };
        at = end + 2;
        if s.kind == 127 { done = true; }
        Some(s)
    })
}

/// Vendor, product and BIOS version, for logs and quirk tables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct System<'a> {
    pub vendor: Option<&'a str>,
    pub product: Option<&'a str>,
    pub sku: Option<&'a str>,
    pub bios_version: Option<&'a str>,
}

pub fn system(table: &[u8]) -> System<'_> {
    let mut s = System::default();
    for st in structures(table) {
        match st.kind {
            0 => s.bios_version = st.string_at(5),
            1 => {
                s.vendor = st.string_at(4);
                s.product = st.string_at(5);
                if st.data.len() > 0x19 { s.sku = st.string_at(0x19); }
            }
            _ => {}
        }
    }
    s
}

/// Alienware x17 R1 (Dell names it exactly so in type 1).
pub fn is_alienware_x17_r1(s: &System) -> bool {
    s.vendor.map_or(false, |v| v.eq_ignore_ascii_case("Alienware") || v.starts_with("Dell"))
        && s.product.map_or(false, |p| p.eq_ignore_ascii_case("Alienware x17 R1"))
}

#[cfg(test)]
pub mod fixture {
    //! An x17 R1-like table: BIOS (type 0), system (type 1), a Dell token table (0xDA)
    //! with the battery tokens, and the end marker.
    pub fn table() -> Vec<u8> {
        let mut t = Vec::new();
        // Type 0: vendor=1, version=2.
        t.extend_from_slice(&[0, 0x18, 0, 0, 1, 2, 0, 0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        t.extend_from_slice(b"Alienware\0" as &[u8]);
        t.extend_from_slice(b"1.21.0\0\0");
        // Type 1: manufacturer=1, product=2, version=0, serial=0, uuid, wake, sku=3, family=0.
        let mut s1 = vec![1u8, 0x1B, 1, 0, 1, 2, 0, 0];
        s1.extend_from_slice(&[0u8; 16]);
        s1.extend_from_slice(&[6, 3, 0]);
        t.extend_from_slice(&s1);
        t.extend_from_slice(b"Alienware\0Alienware x17 R1\0" as &[u8]);
        t.extend_from_slice(b"0A69\0\0");
        // Type 0xDA: cmd io address 0xB2, code 0xDA, supported classes 0, 1, 17.
        let tokens: [(u16, u16, u16); 7] = [(0x0341, 0x0341, 1), (0x0342, 0x0342, 1), (0x0343, 0x0343, 1),
            (0x0346, 0x0346, 1), (0x0349, 0x0349, 0), (0x034A, 0x034A, 0), (0xFFFF, 0, 0)];
        let len = 11 + 6 * tokens.len();
        let mut da = vec![0xDAu8, len as u8, 2, 0, 0xB2, 0x00, 0xDA];
        da.extend_from_slice(&(1u32 | 2 | 1 << 17).to_le_bytes());
        for (id, loc, val) in tokens { da.extend_from_slice(&id.to_le_bytes()); da.extend_from_slice(&loc.to_le_bytes()); da.extend_from_slice(&val.to_le_bytes()); }
        t.extend_from_slice(&da);
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&[127, 4, 3, 0, 0, 0]);
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_points() {
        let mut ep3 = [0u8; 24];
        ep3[..5].copy_from_slice(b"_SM3_");
        ep3[6] = 24;
        ep3[7] = 3;
        ep3[8] = 3;
        ep3[12..16].copy_from_slice(&0x1234u32.to_le_bytes());
        ep3[16..24].copy_from_slice(&0x7AB0_0000u64.to_le_bytes());
        ep3[5] = 0u8.wrapping_sub(checksum(&ep3));
        assert_eq!(entry_point(&ep3), Some((0x7AB0_0000, 0x1234, 3, 3)));
        ep3[20] ^= 1;
        assert_eq!(entry_point(&ep3), None);

        let mut ep2 = [0u8; 31];
        ep2[..4].copy_from_slice(b"_SM_");
        ep2[5] = 31;
        ep2[6] = 2;
        ep2[7] = 8;
        ep2[16..21].copy_from_slice(b"_DMI_");
        ep2[22..24].copy_from_slice(&900u16.to_le_bytes());
        ep2[24..28].copy_from_slice(&0xF0000u32.to_le_bytes());
        ep2[4] = 0u8.wrapping_sub(checksum(&ep2));
        assert_eq!(entry_point(&ep2), Some((0xF0000, 900, 2, 8)));
        assert_eq!(entry_point(b"nothing here at all, no anchor"), None);
    }

    #[test]
    fn x17_system() {
        let t = fixture::table();
        assert_eq!(structures(&t).map(|s| s.kind).collect::<Vec<_>>(), [0, 1, 0xDA, 127]);
        let s = system(&t);
        assert_eq!(s.vendor, Some("Alienware"));
        assert_eq!(s.product, Some("Alienware x17 R1"));
        assert_eq!(s.sku, Some("0A69"));
        assert_eq!(s.bios_version, Some("1.21.0"));
        assert!(is_alienware_x17_r1(&s));
    }

    #[test]
    fn truncated_tables_end_cleanly() {
        let t = fixture::table();
        for cut in 0..t.len() { let _ = structures(&t[..cut]).count(); let _ = system(&t[..cut]); }
        assert_eq!(structures(&[1, 2, 3]).count(), 0);
    }
}
