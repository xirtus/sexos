// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! RPC function numbers and payloads for GSP-RM 535.113.01.

use crate::{wr16, wr32, wr64, Error};

pub const FN_GSP_SET_SYSTEM_INFO: u32 = 72;
pub const FN_SET_REGISTRY: u32 = 73;
pub const FN_CONTINUATION_RECORD: u32 = 71;

pub const EV_GSP_INIT_DONE: u32 = 0x1001;
pub const EV_GSP_RUN_CPU_SEQUENCER: u32 = 0x1002;
pub const EV_POST_EVENT: u32 = 0x1003;
pub const EV_RC_TRIGGERED: u32 = 0x1004;
pub const EV_MMU_FAULT_QUEUED: u32 = 0x1005;
pub const EV_OS_ERROR_LOG: u32 = 0x1006;
pub const EV_UCODE_LIBOS_PRINT: u32 = 0x100c;
pub const EV_PERF_BRIDGELESS_INFO_UPDATE: u32 = 0x100f;
pub const EV_GSP_SEND_USER_SHARED_DATA: u32 = 0x101b;
pub const EV_GSP_LOCKDOWN_NOTICE: u32 = 0x101d;

pub const SYSTEM_INFO_SIZE: usize = 664;

/// Where the GPU sits; the BAR addresses are CPU physical addresses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemInfo {
    pub bar0: u64,
    pub bar1: u64,
    /// BAR3 on Ampere ("instance memory" BAR, nouveau's BAR2_INST).
    pub bar_inst: u64,
    pub bus: u8,
    pub devfn: u8,
    pub max_user_va: u64,
}

/// `GspSystemInfo` (664 bytes; ACPI method data left zero for now).
pub fn system_info(out: &mut [u8; SYSTEM_INFO_SIZE], i: &SystemInfo) {
    out.fill(0);
    wr64(out, 0, i.bar0);
    wr64(out, 8, i.bar1);
    wr64(out, 16, i.bar_inst);
    wr64(out, 24, ((i.bus as u64) << 8) | i.devfn as u64);
    wr64(out, 56, i.max_user_va);
    wr32(out, 64, crate::regs::PCI_CONFIG_MIRROR);
    wr32(out, 68, crate::regs::PCI_CONFIG_MIRROR_SIZE);
}

pub const REG_TYPE_DWORD: u8 = 1;
const REG_ENTRY: usize = 16;

/// The registry keys nouveau sends (dword values).
pub const DEFAULT_REGISTRY: &[(&str, u32)] = &[("RMSecBusResetEnable", 1), ("RMForcePcieConfigSave", 1), ("RMDevidCheckIgnore", 1)];

/// `PACKED_REGISTRY_TABLE` with dword entries; returns the payload length.
pub fn registry(out: &mut [u8], entries: &[(&str, u32)]) -> Result<usize, Error> {
    let mut strings = 8 + entries.len() * REG_ENTRY;
    let total = strings + entries.iter().map(|(k, _)| k.len() + 1).sum::<usize>();
    if out.len() < total { return Err(Error::BufferTooSmall { need: total, have: out.len() }); }
    out[..total].fill(0);
    wr32(out, 0, total as u32);
    wr32(out, 4, entries.len() as u32);
    for (i, (k, v)) in entries.iter().enumerate() {
        if k.len() + 1 > 64 { return Err(Error::Gsp("registry: key too long")); }
        let e = 8 + i * REG_ENTRY;
        wr32(out, e, strings as u32);
        out[e + 4] = REG_TYPE_DWORD;
        wr32(out, e + 8, *v);
        wr32(out, e + 12, 4);
        out[strings..strings + k.len()].copy_from_slice(k.as_bytes());
        strings += k.len() + 1;
    }
    let _ = wr16; // (kept for future payloads with u16 fields)
    Ok(total)
}
