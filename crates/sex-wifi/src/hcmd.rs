// SPDX-License-Identifier: GPL-3.0-only
//! Host commands to the firmware and packets back from it.
//! A command is a "wide" header plus payload in a DMA buffer, described to the
//! chip by a transfer descriptor (TFD) on queue 0; responses and notifications
//! arrive on the RX ring as packets with the same header.

pub const LEGACY_GROUP: u8 = 0;
pub const LONG_GROUP: u8 = 1;
pub const SYSTEM_GROUP: u8 = 0x2;
pub const PHY_OPS_GROUP: u8 = 0x4;
pub const DATA_PATH_GROUP: u8 = 0x5;
pub const REGULATORY_AND_NVM_GROUP: u8 = 0xc;

pub const ALIVE: u8 = 0x1;
pub const ALIVE_STATUS_OK: u16 = 0xcafe;
pub const REPLY_ERROR: u8 = 0x2;
pub const INIT_COMPLETE_NOTIF: u8 = 0x4;
pub const SCAN_CFG_CMD: u8 = 0xc;
pub const SCAN_REQ_UMAC: u8 = 0xd;
pub const POWER_TABLE_CMD: u8 = 0x77;
pub const TX_ANT_CONFIGURATION_CMD: u8 = 0x98;
pub const BT_CONFIG: u8 = 0x9b;
pub const MCC_UPDATE_CMD: u8 = 0xc8;
pub const REPLY_BEACON_FILTERING_CMD: u8 = 0xd2;
/// SYSTEM_GROUP
pub const SOC_CONFIGURATION_CMD: u8 = 0x01;
pub const INIT_EXTENDED_CFG_CMD: u8 = 0x03;
/// PHY_OPS_GROUP
pub const TEMP_REPORTING_THRESHOLDS_CMD: u8 = 0x04;
/// DATA_PATH_GROUP
pub const DQA_ENABLE_CMD: u8 = 0x00;
/// REGULATORY_AND_NVM_GROUP
pub const NVM_ACCESS_COMPLETE: u8 = 0x00;
pub const NVM_GET_INFO: u8 = 0x02;
pub const PNVM_INIT_COMPLETE: u8 = 0xfe;
/// Set in a response's group byte when the firmware refused the command.
pub const CMD_FAILED: u8 = 0x40;

// Stage W5: joining a network (LONG_GROUP unless noted).
pub const MAC_CONF_GROUP: u8 = 0x3;
pub const PHY_CONTEXT_CMD: u8 = 0x08;
pub const ADD_STA_KEY: u8 = 0x17;
pub const ADD_STA: u8 = 0x18;
pub const TX_CMD: u8 = 0x1c;
pub const MAC_CONTEXT_CMD: u8 = 0x28;
pub const BINDING_CONTEXT_CMD: u8 = 0x2b;
/// MAC_CONF_GROUP.
pub const SESSION_PROTECTION_CMD: u8 = 0x05;
pub const SESSION_PROTECTION_NOTIF: u8 = 0xfb;
/// DATA_PATH_GROUP.
pub const RLC_CONFIG_CMD: u8 = 0x08;
pub const TLC_MNG_CONFIG_CMD: u8 = 0x0f;
pub const SCD_QUEUE_CONFIG_CMD: u8 = 0x17;
pub const FIRST_TB_SIZE: usize = 20;
pub const WIDE_HEADER: usize = 8;
pub const TFD_BYTES: usize = 256;
pub const NUM_TBS: usize = 25;
const FRAME_SIZE_MASK: u32 = 0x3fff;
const FRAME_INVALID: u32 = 0x5555_0000;
const FRAME_ALIGN: usize = 0x40;

/// Writes a command (header + payload) into `out`; returns its length.
/// Group 0 commands are sent as LONG_GROUP: firmware API 50+ rejects narrow ones.
pub fn encode(out: &mut [u8], group: u8, opcode: u8, version: u8, qid: u8, index: u8, payload: &[u8]) -> Option<usize> {
    let total = WIDE_HEADER + payload.len();
    if out.len() < total || payload.len() > u16::MAX as usize { return None; }
    out[..WIDE_HEADER].copy_from_slice(&header(group, opcode, version, qid, index, payload.len() as u16));
    out[8..total].copy_from_slice(payload);
    Some(total)
}

/// The wide command header alone (see `encode`).
pub fn header(group: u8, opcode: u8, version: u8, qid: u8, index: u8, len: u16) -> [u8; WIDE_HEADER] {
    let l = len.to_le_bytes();
    [opcode, if group == LEGACY_GROUP { LONG_GROUP } else { group }, index, qid, l[0], l[1], 0, version]
}

/// A TFD for a buffer at bus address `addr` of `len` bytes: the first 20 bytes
/// in TB0 and the rest in TB1, as the scheduler reads the header from TB0.
pub fn tfd_for_command(addr: u64, len: usize) -> [u8; TFD_BYTES] {
    let mut tfd = [0u8; TFD_BYTES];
    let first = len.min(FIRST_TB_SIZE);
    let mut put = |i: usize, l: usize, a: u64| {
        let at = 2 + i * 10;
        tfd[at..at + 2].copy_from_slice(&(l as u16).to_le_bytes());
        tfd[at + 2..at + 10].copy_from_slice(&a.to_le_bytes());
    };
    put(0, first, addr);
    let mut n = 1u16;
    if len > FIRST_TB_SIZE { put(1, len - FIRST_TB_SIZE, addr + FIRST_TB_SIZE as u64); n = 2; }
    tfd[0..2].copy_from_slice(&n.to_le_bytes());
    tfd
}

/// The value written to HBUS_TARG_WRPTR to tell the chip a queue advanced.
pub fn write_pointer(qid: u8, index: u16) -> u32 { (qid as u32) << 16 | index as u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> { pub code: u8, pub group: u8, pub index: u8, pub qid: u8, pub data: &'a [u8] }
impl Packet<'_> {
    /// Responses to our commands carry the queue id without the "unsolicited" bit.
    pub fn is_response(&self) -> bool { self.qid & 0x80 == 0 }
    /// The group without the "command failed" flag. Commands of group 0 are sent
    /// as LONG_GROUP and answered there; both are the same command space.
    pub fn group_id(&self) -> u8 { self.group & !CMD_FAILED }
    pub fn failed(&self) -> bool { self.group & CMD_FAILED != 0 }
    /// True for (group, code), counting group 0 and LONG_GROUP as one.
    pub fn is(&self, group: u8, code: u8) -> bool {
        let g = self.group_id();
        self.code == code && (g == group || (g <= LONG_GROUP && group <= LONG_GROUP))
    }
}

/// A REPLY_ERROR notification: the firmware could not run a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirmwareError { pub kind: u32, pub command: u8, pub sequence: u16, pub service: u32 }

pub fn parse_error(p: &Packet<'_>) -> Option<FirmwareError> {
    if !p.is(LEGACY_GROUP, REPLY_ERROR) || p.data.len() < 12 { return None; }
    let d = p.data;
    Some(FirmwareError { kind: u32::from_le_bytes(d[0..4].try_into().unwrap()), command: d[4],
        sequence: u16::from_le_bytes([d[6], d[7]]), service: u32::from_le_bytes(d[8..12].try_into().unwrap()) })
}

/// Splits one RX buffer into packets (a 4 KiB buffer may hold several).
pub fn packets(buffer: &[u8]) -> impl Iterator<Item = Packet<'_>> {
    let mut rest = buffer;
    core::iter::from_fn(move || {
        if rest.len() < 8 { return None; }
        let word = u32::from_le_bytes(rest[..4].try_into().unwrap());
        let len = (word & FRAME_SIZE_MASK) as usize;
        if word == FRAME_INVALID || (rest[4] == 0 && rest[5] == 0 && rest[6] == 0 && rest[7] & 0x7f == 0) || len < 4 || 4 + len > rest.len() { return None; }
        let p = Packet { code: rest[4], group: rest[5], index: rest[6], qid: rest[7], data: &rest[8..4 + len] };
        // Packets are padded to 64-byte boundaries in the buffer (FH_RSCSR_FRAME_ALIGN).
        let step = (4 + len + FRAME_ALIGN - 1) & !(FRAME_ALIGN - 1);
        rest = &rest[step.min(rest.len())..];
        Some(p)
    })
}

/// Firmware versions and status from the ALIVE notification. Layouts v4 (116
/// bytes), v5 (128, adds the SKU id) and v6/v7 (144, adds the IMR region) share
/// their first 116 bytes; the SKU id picks the platform NVM section on AX210.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alive { pub ok: bool, pub lmac_major: u32, pub lmac_minor: u32, pub umac_major: u32, pub umac_minor: u32, pub error_table: u32, pub umac_error_table: u32, pub sku: [u32; 3] }

pub fn parse_alive(p: &Packet<'_>) -> Option<Alive> {
    if p.code != ALIVE || (p.group != LEGACY_GROUP && p.group != LONG_GROUP) || p.data.len() < 4 + 2 * 48 + 16 { return None; }
    let d = p.data;
    let u = |at: usize| u32::from_le_bytes(d[at..at + 4].try_into().unwrap());
    let umac = 4 + 2 * 48;
    Some(Alive {
        ok: u16::from_le_bytes([d[0], d[1]]) == ALIVE_STATUS_OK,
        lmac_major: u(4), lmac_minor: u(8), error_table: u(4 + 16),
        umac_major: u(umac), umac_minor: u(umac + 4), umac_error_table: u(umac + 8),
        sku: if d.len() >= 128 { [u(116), u(120), u(124)] } else { [0; 3] },
    })
}
