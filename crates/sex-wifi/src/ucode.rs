// SPDX-License-Identifier: GPL-3.0-only
//! Intel "TLV" firmware files (iwlwifi-*.ucode): header, sections and the
//! capability bits the driver must honour. Layout as read by OpenBSD iwx(4).
//! The parser borrows the file; nothing is copied until the sections are placed
//! in DMA memory by `context`.

pub const MAGIC: u32 = 0x0a4c_5749;
pub const MAX_SECTIONS: usize = 69;
pub const CPU1_CPU2_SEPARATOR: u32 = 0xffff_cccc;
pub const PAGING_SEPARATOR: u32 = 0xaaaa_bbbb;
const HEADER: usize = 4 + 4 + 64 + 4 + 4 + 8;

const TLV_PROBE_MAX_LEN: u32 = 6;
const TLV_PAN: u32 = 7;
const TLV_FLAGS: u32 = 18;
const TLV_SEC_RT: u32 = 19;
const TLV_PHY_SKU: u32 = 23;
const TLV_NUM_OF_CPU: u32 = 27;
const TLV_API_CHANGES_SET: u32 = 29;
const TLV_ENABLED_CAPABILITIES: u32 = 30;
const TLV_N_SCAN_CHANNELS: u32 = 31;
const TLV_PAGING: u32 = 32;
const TLV_FW_VERSION: u32 = 36;
const TLV_CMD_VERSIONS: u32 = 48;
const TLV_IML: u32 = 52;
const TLV_UMAC_DEBUG_ADDRS: u32 = 54;
const TLV_LMAC_DEBUG_ADDRS: u32 = 55;
const TLV_HW_TYPE: u32 = 58;
const TLV_PNVM_VERSION: u32 = 62;
const TLV_PNVM_SKU: u32 = 64;
const TLV_PNVM_DATA: u32 = 74;
/// A deprecated separator some PNVM files still carry as a section.
const PNVM_SEPARATOR: u32 = 0xdddd_eeee;
pub const MAX_PNVM_SECTIONS: usize = 64;

/// Capability and API bits the init sequence looks at (iwx(4) names).
pub const CAPA_LAR_SUPPORT: usize = 1;
pub const CAPA_DQA_SUPPORT: usize = 12;
pub const CAPA_LAR_MULTI_MCC: usize = 29;
pub const CAPA_FRAGMENTED_PNVM_IMG: usize = 32;
pub const CAPA_CT_KILL_BY_FW: usize = 74;
pub const CAPA_MCC_UPDATE_11AX_SUPPORT: usize = 89;
pub const API_WIFI_MCC_UPDATE: usize = 9;
pub const API_REGULATORY_NVM_INFO: usize = 48;
pub const API_REDUCED_SCAN_CONFIG: usize = 56;
pub const API_SCAN_EXT_CHAN_VER: usize = 58;
pub const CAPA_DS_PARAM_SET_IE_SUPPORT: usize = 9;
const MAX_SCAN_CHANNELS: u32 = 67;
const FW_ADDR_CACHE_CONTROL: u32 = 0xc000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, Magic, BadTlv(u32), TooManySections, NoRuntime }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section<'a> { pub device_offset: u32, pub data: &'a [u8] }

/// One command's version as the firmware declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandVersion { pub command: u8, pub group: u8, pub version: u8, pub notification: u8 }

pub struct Firmware<'a> {
    pub human_readable: &'a [u8],
    pub major: u32,
    pub minor: u32,
    pub api: u32,
    pub flags: u32,
    pub phy_config: u32,
    pub probe_max_len: u32,
    pub scan_channels: u32,
    pub api_bits: [u32; 4],
    pub capa_bits: [u32; 5],
    /// Image loader (AX210 family).
    pub iml: Option<&'a [u8]>,
    /// Platform NVM shipped inside the firmware file (preferred over the .pnvm file).
    pub pnvm: Option<&'a [u8]>,
    pub cmd_versions: &'a [u8],
    pub umac_error_table: Option<u32>,
    pub lmac_error_table: Option<u32>,
    sections: [Section<'a>; MAX_SECTIONS],
    count: usize,
}

fn u32_at(data: &[u8], at: usize) -> u32 { u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) }

impl<'a> Firmware<'a> {
    pub fn parse(file: &'a [u8]) -> Result<Self, Error> {
        if file.len() < HEADER { return Err(Error::Short); }
        if u32_at(file, 0) != 0 || u32_at(file, 4) != MAGIC { return Err(Error::Magic); }
        let text = &file[8..72];
        let end = text.iter().position(|&b| b == 0).unwrap_or(64);
        let ver = u32_at(file, 72);
        let mut fw = Firmware {
            human_readable: &text[..end],
            // Newer files put just the API number in the header word.
            major: ver >> 24, minor: (ver >> 16) & 0xff, api: if ver >> 8 == 0 { ver } else { (ver >> 8) & 0xff },
            flags: 0, phy_config: 0, probe_max_len: 0, scan_channels: 40,
            api_bits: [0; 4], capa_bits: [0; 5], iml: None, pnvm: None, cmd_versions: &[],
            umac_error_table: None, lmac_error_table: None,
            sections: [Section { device_offset: 0, data: &[] }; MAX_SECTIONS], count: 0,
        };
        let mut rest = &file[HEADER..];
        while rest.len() >= 8 {
            let kind = u32_at(rest, 0);
            let len = u32_at(rest, 4) as usize;
            rest = &rest[8..];
            if rest.len() < len { return Err(Error::Short); }
            let data = &rest[..len];
            let word = || if len >= 4 { Ok(u32_at(data, 0)) } else { Err(Error::BadTlv(kind)) };
            match kind {
                TLV_PROBE_MAX_LEN => fw.probe_max_len = word()?,
                TLV_PAN => if len != 0 { return Err(Error::BadTlv(kind)) },
                TLV_FLAGS => fw.flags = word()?,
                TLV_NUM_OF_CPU => { let n = word()?; if !(1..=2).contains(&n) { return Err(Error::BadTlv(kind)); } }
                TLV_SEC_RT => {
                    if len < 4 { return Err(Error::BadTlv(kind)); }
                    if fw.count >= MAX_SECTIONS { return Err(Error::TooManySections); }
                    fw.sections[fw.count] = Section { device_offset: u32_at(data, 0), data: &data[4..] };
                    fw.count += 1;
                }
                TLV_PHY_SKU => fw.phy_config = word()?,
                TLV_API_CHANGES_SET | TLV_ENABLED_CAPABILITIES => {
                    if len != 8 { return Err(Error::BadTlv(kind)); }
                    let index = u32_at(data, 0) as usize;
                    let bits = u32_at(data, 4);
                    let table: &mut [u32] = if kind == TLV_API_CHANGES_SET { &mut fw.api_bits } else { &mut fw.capa_bits };
                    if index >= table.len() { return Err(Error::BadTlv(kind)); }
                    table[index] |= bits;
                }
                TLV_N_SCAN_CHANNELS => {
                    fw.scan_channels = word()?;
                    if fw.scan_channels > MAX_SCAN_CHANNELS { return Err(Error::BadTlv(kind)); }
                }
                TLV_PAGING => { word()?; }
                TLV_FW_VERSION => {
                    if len != 12 { return Err(Error::BadTlv(kind)); }
                    fw.major = u32_at(data, 0); fw.minor = u32_at(data, 4); // third word: local build, not the API
                }
                TLV_IML => fw.iml = Some(data),
                TLV_PNVM_DATA => if fw.pnvm.is_none() { fw.pnvm = Some(data) },
                TLV_CMD_VERSIONS => fw.cmd_versions = &data[..len - len % 4],
                TLV_UMAC_DEBUG_ADDRS if len >= 4 => fw.umac_error_table = Some(u32_at(data, 0) & !FW_ADDR_CACHE_CONTROL),
                TLV_LMAC_DEBUG_ADDRS if len >= 4 => fw.lmac_error_table = Some(u32_at(data, 0) & !FW_ADDR_CACHE_CONTROL),
                _ => {}
            }
            let padded = (len + 3) & !3;
            rest = &rest[padded.min(rest.len())..];
        }
        if fw.count == 0 { return Err(Error::NoRuntime); }
        Ok(fw)
    }

    /// Runtime image sections in file order, separators included.
    pub fn sections(&self) -> &[Section<'a>] { &self.sections[..self.count] }

    /// The runtime image split as the hardware wants it: LMAC sections, then
    /// UMAC sections after the CPU1/CPU2 separator, then paging sections.
    pub fn image(&self) -> Image<'_, 'a> {
        let s = self.sections();
        let run = |start: usize| s.iter().skip(start)
            .take_while(|x| x.device_offset != CPU1_CPU2_SEPARATOR && x.device_offset != PAGING_SEPARATOR).count();
        let lmac = run(0);
        let umac = run(lmac + 1);
        let paging = run(lmac + umac + 2);
        let at = |from: usize, n: usize| if from <= s.len() { &s[from..(from + n).min(s.len())] } else { &s[0..0] };
        Image { lmac: at(0, lmac), umac: at(lmac + 1, umac), paging: at(lmac + umac + 2, paging) }
    }

    pub fn has_capa(&self, bit: usize) -> bool { self.capa_bits.get(bit / 32).is_some_and(|w| w & (1 << (bit % 32)) != 0) }
    pub fn has_api(&self, bit: usize) -> bool { self.api_bits.get(bit / 32).is_some_and(|w| w & (1 << (bit % 32)) != 0) }

    /// Version of a host command as declared by the firmware (`None` if not listed).
    pub fn command_version(&self, group: u8, command: u8) -> Option<CommandVersion> {
        self.cmd_versions.chunks_exact(4)
            .map(|c| CommandVersion { command: c[0], group: c[1], version: c[2], notification: c[3] })
            .find(|v| v.group == group && v.command == command)
    }
}

pub struct Image<'f, 'a> { pub lmac: &'f [Section<'a>], pub umac: &'f [Section<'a>], pub paging: &'f [Section<'a>] }
impl Image<'_, '_> {
    pub fn total_bytes(&self) -> usize {
        self.lmac.iter().chain(self.umac).chain(self.paging).map(|s| s.data.len()).sum()
    }
}

/// Platform NVM files (*.pnvm) are TLV lists too; the driver passes the whole
/// SKU-matching chunk to the firmware. This finds the chunk for a SKU id.
pub fn pnvm_for_sku<'a>(file: &'a [u8], sku: [u32; 3]) -> Option<&'a [u8]> {
    let mut rest = file;
    while rest.len() >= 8 {
        let kind = u32_at(rest, 0);
        let len = u32_at(rest, 4) as usize;
        if rest.len() < 8 + len { return None; }
        if kind == TLV_PNVM_SKU && len >= 12 {
            let s = &rest[8..];
            let here = [u32_at(s, 0), u32_at(s, 4), u32_at(s, 8)];
            let after = &rest[8 + ((len + 3) & !3).min(rest.len() - 8)..];
            // The chunk runs to the next SKU TLV.
            let mut end = 0;
            let mut scan = after;
            while scan.len() >= 8 {
                let k = u32_at(scan, 0);
                let l = u32_at(scan, 4) as usize;
                if k == TLV_PNVM_SKU || scan.len() < 8 + l { break; }
                let step = 8 + ((l + 3) & !3).min(scan.len() - 8);
                end += step;
                scan = &scan[step..];
            }
            if here == sku { return Some(&after[..end]); }
        }
        let step = 8 + ((len + 3) & !3).min(rest.len() - 8);
        rest = &rest[step..];
    }
    None
}

/// The sections of one SKU's PNVM chunk that the firmware loads, when the chunk
/// is for this chip (`mac_type` = HW_REV bits 4-15, `rf_type` = RF_ID bits 12-23).
#[derive(Clone, Copy, Debug)]
pub struct PnvmImage<'a> { pub sections: [&'a [u8]; MAX_PNVM_SECTIONS], pub count: usize, pub bytes: usize, pub version: u32 }
impl<'a> PnvmImage<'a> {
    pub fn sections(&self) -> &[&'a [u8]] { &self.sections[..self.count] }
}

/// Picks the PNVM image for this SKU and radio: like `iwx_pnvm_parse`, every
/// chunk whose SKU matches is tried until one is for this hardware.
pub fn pnvm_select(file: &[u8], sku: [u32; 3], mac_type: u16, rf_type: u16) -> Option<PnvmImage<'_>> {
    let mut rest = file;
    while rest.len() >= 8 {
        let kind = u32_at(rest, 0);
        let len = u32_at(rest, 4) as usize;
        let step = 8 + ((len + 3) & !3);
        if rest.len() - 8 < len || step > rest.len() { return None; }
        if kind == TLV_PNVM_SKU && len >= 12 && [u32_at(rest, 8), u32_at(rest, 12), u32_at(rest, 16)] == sku {
            if let Some(img) = pnvm_image(&rest[step..], mac_type, rf_type) { return Some(img); }
        }
        rest = &rest[step..];
    }
    None
}

pub fn pnvm_image(chunk: &[u8], mac_type: u16, rf_type: u16) -> Option<PnvmImage<'_>> {
    let mut img = PnvmImage { sections: [&[]; MAX_PNVM_SECTIONS], count: 0, bytes: 0, version: 0 };
    let mut matched = false;
    let mut rest = chunk;
    while rest.len() >= 8 {
        let kind = u32_at(rest, 0);
        let len = u32_at(rest, 4) as usize;
        if rest.len() - 8 < len { return None; }
        let data = &rest[8..8 + len];
        match kind {
            TLV_PNVM_VERSION if len >= 4 => img.version = u32_at(data, 0),
            TLV_HW_TYPE if len >= 4 && !matched => {
                let mac = u16::from_le_bytes([data[0], data[1]]);
                let rf = u16::from_le_bytes([data[2], data[3]]);
                matched = mac == mac_type && rf == rf_type;
            }
            TLV_SEC_RT if len >= 4 => {
                if u32_at(data, 0) != PNVM_SEPARATOR {
                    if img.count >= MAX_PNVM_SECTIONS { return None; }
                    img.sections[img.count] = &data[4..];
                    img.count += 1;
                    img.bytes += len - 4;
                }
            }
            TLV_PNVM_SKU => break,
            _ => {}
        }
        let step = 8 + ((len + 3) & !3);
        if step > rest.len() { break; }
        rest = &rest[step..];
    }
    (matched && img.bytes > 0).then_some(img)
}
