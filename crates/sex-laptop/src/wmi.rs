// SPDX-License-Identifier: GPL-3.0-only
//! ACPI-WMI (Windows Management Instrumentation mapping, `PNP0C14`).
//!
//! PC vendors hang their extras off WMI: Dell and Alienware fans, thermal profiles, hotkeys
//! and battery charge modes; Lenovo, ASUS and HP do the same with their own GUIDs. A WMI
//! device has a `_WDG` buffer listing data blocks, each a 16-byte GUID, a two-character
//! object id, an instance count and flags. A method block with id `AB` is called as
//! `WMAB(instance, method_id, input_buffer)`; an event block's first id byte is the
//! `Notify` value the firmware raises, and `_WED(notify_value)` returns the event's data.
//!
//! This module only reads `_WDG` and names methods: the AML runs in `servers/sexacpi`.

/// One `_WDG` entry is 20 bytes.
pub const ENTRY: usize = 20;
/// The block is costly to collect (`WCxx` must enable it first).
pub const FLAG_EXPENSIVE: u8 = 0x1;
/// A method block (`WMxx`).
pub const FLAG_METHOD: u8 = 0x2;
/// The data is a string.
pub const FLAG_STRING: u8 = 0x4;
/// An event block: the first id byte is the notify value.
pub const FLAG_EVENT: u8 = 0x8;

/// A GUID in `_WDG` byte order: the first three fields little-endian, the last eight as
/// written. `guid("A70591CE-A997-11DA-B012-B622A1EF5492")`.
pub const fn guid(text: &str) -> [u8; 16] {
    const fn hex(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("guid: not a hex digit"),
        }
    }
    let s = text.as_bytes();
    assert!(s.len() == 36, "guid: wrong length");
    // Bytes in text order, dashes skipped.
    let mut raw = [0u8; 16];
    let (mut i, mut n) = (0, 0);
    while i < 36 {
        if s[i] == b'-' { i += 1; continue; }
        raw[n] = hex(s[i]) << 4 | hex(s[i + 1]);
        n += 1;
        i += 2;
    }
    assert!(n == 16, "guid: wrong digit count");
    [raw[3], raw[2], raw[1], raw[0], raw[5], raw[4], raw[7], raw[6],
     raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15]]
}

/// Writes a `_WDG`-order GUID back as text (36 bytes), for logs.
pub fn guid_text(g: &[u8; 16], out: &mut [u8; 36]) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut at = 0;
    for (k, &i) in order.iter().enumerate() {
        if matches!(k, 4 | 6 | 8 | 10) { out[at] = b'-'; at += 1; }
        out[at] = HEX[(g[i] >> 4) as usize];
        out[at + 1] = HEX[(g[i] & 15) as usize];
        at += 2;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub guid: [u8; 16],
    /// Object id (`AB` in `WMAB`), or `[notify_id, reserved]` for an event.
    pub id: [u8; 2],
    pub instances: u8,
    pub flags: u8,
}

impl Block {
    fn parse(e: &[u8]) -> Option<Self> {
        if e.len() < ENTRY { return None; }
        let mut guid = [0u8; 16];
        guid.copy_from_slice(&e[..16]);
        Some(Self { guid, id: [e[16], e[17]], instances: e[18], flags: e[19] })
    }
    pub fn is_method(&self) -> bool { self.flags & FLAG_METHOD != 0 }
    pub fn is_event(&self) -> bool { self.flags & FLAG_EVENT != 0 }
    /// `WMxx` for a method block; `None` when the id is not two name characters.
    pub fn method_name(&self) -> Option<[u8; 4]> {
        let ok = |c: u8| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_';
        (self.is_method() && ok(self.id[0]) && ok(self.id[1])).then_some([b'W', b'M', self.id[0], self.id[1]])
    }
    /// `WQxx` for a data block (neither method nor event): reads instance `n`.
    pub fn query_name(&self) -> Option<[u8; 4]> {
        let ok = |c: u8| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_';
        (!self.is_method() && !self.is_event() && ok(self.id[0]) && ok(self.id[1])).then_some([b'W', b'Q', self.id[0], self.id[1]])
    }
    /// The `Notify` value an event block is raised with (and `_WED`'s argument).
    pub fn notify_id(&self) -> Option<u8> { self.is_event().then_some(self.id[0]) }
}

/// The blocks in a `_WDG` buffer (a trailing partial entry is ignored).
pub fn blocks(wdg: &[u8]) -> impl Iterator<Item = Block> + '_ {
    wdg.chunks_exact(ENTRY).filter_map(Block::parse)
}

/// The first block with this GUID.
pub fn find(wdg: &[u8], guid: &[u8; 16]) -> Option<Block> { blocks(wdg).find(|b| &b.guid == guid) }

/// The event block whose notify value is `value`.
pub fn event_for(wdg: &[u8], value: u8) -> Option<Block> { blocks(wdg).find(|b| b.notify_id() == Some(value)) }

/// GUIDs this tree knows about.
pub mod guids {
    use super::guid;
    /// Alienware Command Center (`WMAX`): fans, temperatures, thermal profiles, lighting.
    pub const ALIENWARE_WMAX: [u8; 16] = guid("A70591CE-A997-11DA-B012-B622A1EF5492");
    /// Dell hotkey events (`dell-wmi`), notify 0xD0.
    pub const DELL_EVENT: [u8; 16] = guid("9DBB5994-A997-11DA-B012-B622A1EF5492");
    /// Dell SMBIOS calling interface (`dell-smbios-wmi`).
    pub const DELL_SMBIOS: [u8; 16] = guid("A80593CE-A997-11DA-B012-B622A1EF5492");
    /// Dell WMI descriptor (interface version and buffer size).
    pub const DELL_DESCRIPTOR: [u8; 16] = guid("8D9DDCBC-A997-11DA-B012-B622A1EF5492");
    /// Dell DDV: battery and thermal sensor details (`dell-wmi-ddv`).
    pub const DELL_DDV: [u8; 16] = guid("8A42EA14-4F2A-FD45-6422-0087F7A7E608");
    /// The MOF data block every WMI device carries.
    pub const MOF: [u8; 16] = guid("05901221-D566-11D1-B2F0-00A0C9062910");
}

/// A known GUID's short name, for logs.
pub fn known(g: &[u8; 16]) -> Option<&'static str> {
    use guids::*;
    [(ALIENWARE_WMAX, "alienware_wmax"), (DELL_EVENT, "dell_event"), (DELL_SMBIOS, "dell_smbios"),
     (DELL_DESCRIPTOR, "dell_descriptor"), (DELL_DDV, "dell_ddv"), (MOF, "mof")]
        .iter().find(|(k, _)| k == g).map(|(_, n)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(g: [u8; 16], id: [u8; 2], instances: u8, flags: u8) -> [u8; ENTRY] {
        let mut e = [0u8; ENTRY];
        e[..16].copy_from_slice(&g);
        e[16] = id[0];
        e[17] = id[1];
        e[18] = instances;
        e[19] = flags;
        e
    }

    #[test]
    fn guid_byte_order() {
        // As it appears in a real Alienware DSDT `_WDG` buffer.
        assert_eq!(guids::ALIENWARE_WMAX, [0xCE, 0x91, 0x05, 0xA7, 0x97, 0xA9, 0xDA, 0x11, 0xB0, 0x12, 0xB6, 0x22, 0xA1, 0xEF, 0x54, 0x92]);
        let mut t = [0u8; 36];
        guid_text(&guids::DELL_EVENT, &mut t);
        assert_eq!(&t, b"9DBB5994-A997-11DA-B012-B622A1EF5492");
        assert_eq!(guid("05901221-d566-11d1-b2f0-00a0c9062910"), guids::MOF);
    }

    #[test]
    fn wdg_blocks() {
        let mut wdg = Vec::new();
        wdg.extend_from_slice(&entry(guids::ALIENWARE_WMAX, *b"AX", 1, FLAG_METHOD));
        wdg.extend_from_slice(&entry(guids::DELL_EVENT, [0xD0, 0], 1, FLAG_EVENT));
        wdg.extend_from_slice(&entry(guids::MOF, *b"MO", 1, 0));
        wdg.extend_from_slice(&[1, 2, 3]); // trailing garbage
        assert_eq!(blocks(&wdg).count(), 3);
        let wmax = find(&wdg, &guids::ALIENWARE_WMAX).unwrap();
        assert_eq!(wmax.method_name(), Some(*b"WMAX"));
        assert_eq!(wmax.notify_id(), None);
        let ev = event_for(&wdg, 0xD0).unwrap();
        assert_eq!(ev.guid, guids::DELL_EVENT);
        assert_eq!(ev.method_name(), None);
        assert_eq!(find(&wdg, &guids::DELL_SMBIOS), None);
        assert_eq!(known(&guids::MOF), Some("mof"));
        assert_eq!(find(&wdg, &guids::MOF).unwrap().query_name(), Some(*b"WQMO"));
        assert_eq!(wmax.query_name(), None);
        // A method id that is not a name segment is refused.
        let odd = Block { guid: [0; 16], id: [0x01, b'X'], instances: 1, flags: FLAG_METHOD };
        assert_eq!(odd.method_name(), None);
    }
}
