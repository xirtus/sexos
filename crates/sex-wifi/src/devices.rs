// SPDX-License-Identifier: GPL-3.0-only
//! Which Intel Wi-Fi chips the driver knows, and which firmware each needs.
//! Facts taken from OpenBSD iwx(4) (ISC licence): PCI ids, the subsystem table
//! for Killer/AX210/AX211 modules, and the hardware-revision rules for AX201.
//! Firmware names are the linux-firmware names (OpenBSD renames them `iwx-*`).

pub const VENDOR_INTEL: u16 = 0x8086;
/// The firmware API this driver speaks (the `-77` / `-89` files).
pub const API: u32 = 77;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// AX200 and AX201 (22000 series): context info v1, firmware self-load.
    Gen22000,
    /// AX210, AX211, AX411 (and Killer 1675/1690): context info gen3 and an image loader.
    Ax210,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chip {
    pub name: &'static str,
    pub family: Family,
    /// Basename in the firmware store, e.g. "iwlwifi-ty-a0-gf-a0-77.ucode".
    pub firmware: &'static str,
    /// Platform NVM file for AX210-class chips (regulatory and calibration data).
    pub pnvm: Option<&'static str>,
    /// Integrated (CNVi, part of the chipset) rather than a discrete M.2 card.
    pub integrated: bool,
    pub ltr_delay: u8,
    pub xtal_latency: u16,
    pub low_latency_xtal: bool,
    /// 6 GHz ("ultra high band") allowed.
    pub six_ghz: bool,
}

const CC: &str = "iwlwifi-cc-a0-77.ucode";
const QUZ_HR: &str = "iwlwifi-QuZ-a0-hr-b0-77.ucode";
const QUZ_JF: &str = "iwlwifi-QuZ-a0-jf-b0-77.ucode";
const QU_B_HR: &str = "iwlwifi-Qu-b0-hr-b0-77.ucode";
const QU_C_HR: &str = "iwlwifi-Qu-c0-hr-b0-77.ucode";
const SO_GF: &str = "iwlwifi-so-a0-gf-a0-77.ucode";
const SO_GF_PNVM: &str = "iwlwifi-so-a0-gf-a0.pnvm";
const SO_GF4: &str = "iwlwifi-so-a0-gf4-a0-77.ucode";
const SO_GF4_PNVM: &str = "iwlwifi-so-a0-gf4-a0.pnvm";
const TY_GF: &str = "iwlwifi-ty-a0-gf-a0-77.ucode";
const TY_GF_PNVM: &str = "iwlwifi-ty-a0-gf-a0.pnvm";
const MA_GF: &str = "iwlwifi-ma-b0-gf-a0-89.ucode";
const MA_GF_PNVM: &str = "iwlwifi-ma-b0-gf-a0.pnvm";

pub const LTR_NONE: u8 = 0;
pub const LTR_200: u8 = 1;
pub const LTR_2500: u8 = 2;
pub const LTR_1820: u8 = 3;

const HW_REV_QU_C0: u32 = 0x338;
const HW_REV_QUZ: u32 = 0x354;
const MAC_TYPE_TY: u32 = 0x42;
const RF_TYPE_JF1: u32 = 0x108;
const RF_TYPE_JF2: u32 = 0x105;
const RF_TYPE_GF: u32 = 0x10d;

/// Every PCI device id the driver claims (vendor 0x8086).
pub const PCI_IDS: &[u16] = &[
    0x2723, 0x02f0, 0x06f0, 0xa0f0, 0x34f0, 0x3df0, 0x4df0, 0x43f0,
    0x2725, 0x2726, 0x51f0, 0x51f1, 0x54f0, 0x7a70, 0x7af0, 0x7e40, 0x7f70,
];

pub fn claims(vendor: u16, device: u16) -> bool { vendor == VENDOR_INTEL && PCI_IDS.contains(&device) }

/// `hw_rev` is the CSR_HW_REV register in the driver's "old format"
/// (see `normalize_hw_rev`); `rf_id` is CSR_HW_RF_ID.
pub fn identify(device: u16, subdevice: u16, hw_rev: u32, rf_id: u32) -> Option<Chip> {
    let rev_type = hw_rev & 0xfffc; // type and step, as the AX201 rules compare
    let rf_type = (rf_id & 0x0fff000) >> 12;
    let mac_type = (hw_rev & 0xfff0) >> 4;
    let qu = |integrated_delay: u8, xtal: u16, low: bool| -> Chip {
        let firmware = if rev_type == HW_REV_QU_C0 { QU_C_HR }
            else if rev_type == HW_REV_QUZ { if rf_type == RF_TYPE_JF1 || rf_type == RF_TYPE_JF2 { QUZ_JF } else { QUZ_HR } }
            else { QU_B_HR };
        Chip { name: "Intel Wi-Fi 6 AX201", family: Family::Gen22000, firmware, pnvm: None, integrated: true,
            ltr_delay: integrated_delay, xtal_latency: xtal, low_latency_xtal: low, six_ghz: false }
    };
    let ax210 = |name, firmware, pnvm, integrated, ltr, xtal, low, six| Chip {
        name, family: Family::Ax210, firmware, pnvm: Some(pnvm), integrated, ltr_delay: ltr, xtal_latency: xtal, low_latency_xtal: low, six_ghz: six,
    };
    let mut chip = match device {
        0x2723 => Chip { name: "Intel Wi-Fi 6 AX200", family: Family::Gen22000, firmware: CC, pnvm: None, integrated: false,
            ltr_delay: LTR_NONE, xtal_latency: 0, low_latency_xtal: false, six_ghz: false },
        0x02f0 | 0x06f0 => { if rev_type != HW_REV_QUZ { return None; } Chip { firmware: QUZ_HR, ..qu(LTR_200, 500, false) } }
        0xa0f0 => qu(LTR_200, 500, false),
        0x34f0 | 0x3df0 | 0x4df0 => qu(LTR_1820, 1820, false),
        0x43f0 => qu(LTR_2500, 12000, true),
        0x2725 => ax210("Intel Wi-Fi 6E AX210", SO_GF, SO_GF_PNVM, false, LTR_NONE, 0, false, true),
        0x2726 | 0x51f0 | 0x54f0 | 0x7af0 | 0x7f70 => ax210("Intel Wi-Fi 6E AX211", SO_GF, SO_GF_PNVM, false, LTR_NONE, 0, false, true),
        0x7a70 | 0x51f1 => ax210("Intel Wi-Fi 6E AX211", SO_GF, SO_GF_PNVM, true, LTR_2500, 12000, true, false),
        0x7e40 => ax210("Intel Wi-Fi 6E AX210", MA_GF, MA_GF_PNVM, true, LTR_NONE, 0, false, true),
        _ => return None,
    };
    // Subsystem table: the module vendor (Killer, laptop makers) decides the firmware.
    let entry = SUBSYSTEMS.iter().find(|e| e.device == device && e.subdevice == subdevice);
    if let Some(e) = entry {
        chip.firmware = e.firmware;
        chip.pnvm = e.pnvm;
        if let Some(n) = e.name { chip.name = n; }
        if e.long_xtal { chip.xtal_latency = 12000; chip.low_latency_xtal = true; }
    } else if device == 0x2725 && mac_type == MAC_TYPE_TY {
        // A discrete AX210 whose MAC reports "Ty" runs the Ty image.
        chip.firmware = TY_GF;
        chip.pnvm = Some(TY_GF_PNVM);
    }
    if chip.family == Family::Ax210 && rf_id != 0 && rf_type != RF_TYPE_GF && entry.is_none() {
        // Non-GF radios behind AX210-class MACs (Hr, Jf) are not handled yet.
        return None;
    }
    Some(chip)
}

/// CSR_HW_REV to the old format the tables use: the 8000 series moved the step
/// into bits 0-1; shift it back to bits 2-3.
pub fn normalize_hw_rev(raw: u32) -> u32 { (raw & 0xfff0) | ((((raw << 2) & 0xc) >> 2) << 2) }

struct Subsystem { device: u16, subdevice: u16, firmware: &'static str, pnvm: Option<&'static str>, name: Option<&'static str>, long_xtal: bool }
const fn sub(device: u16, subdevice: u16, firmware: &'static str, pnvm: &'static str, name: Option<&'static str>, long_xtal: bool) -> Subsystem {
    Subsystem { device, subdevice, firmware, pnvm: Some(pnvm), name, long_xtal }
}
const K1675X: Option<&str> = Some("Killer Wi-Fi 6E AX1675x (AX210)");
const K1675W: Option<&str> = Some("Killer Wi-Fi 6E AX1675w (AX210)");
const K1675S: Option<&str> = Some("Killer Wi-Fi 6E AX1675s (AX211)");
const K1675I: Option<&str> = Some("Killer Wi-Fi 6E AX1675i (AX211)");
const K1690S: Option<&str> = Some("Killer Wi-Fi 6E AX1690s (AX411)");
const K1690I: Option<&str> = Some("Killer Wi-Fi 6E AX1690i (AX411)");
static SUBSYSTEMS: &[Subsystem] = &[
    sub(0x2725, 0x0090, SO_GF, SO_GF_PNVM, None, false),
    sub(0x2725, 0x0020, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x2020, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x0024, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x0310, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x0510, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x0a10, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0xe020, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0xe024, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x4020, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x6020, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x6024, TY_GF, TY_GF_PNVM, None, false),
    sub(0x2725, 0x1673, TY_GF, TY_GF_PNVM, K1675W, false),
    sub(0x2725, 0x1674, TY_GF, TY_GF_PNVM, K1675X, false),
    sub(0x51f0, 0x1691, SO_GF4, SO_GF4_PNVM, K1690S, true),
    sub(0x51f0, 0x1692, SO_GF4, SO_GF4_PNVM, K1690I, true),
    sub(0x51f1, 0x1691, SO_GF4, SO_GF4_PNVM, K1690S, true),
    sub(0x51f1, 0x1692, SO_GF4, SO_GF4_PNVM, K1690I, true),
    sub(0x54f0, 0x1691, SO_GF4, SO_GF4_PNVM, K1690S, true),
    sub(0x54f0, 0x1692, SO_GF4, SO_GF4_PNVM, K1690I, true),
    sub(0x7a70, 0x0090, SO_GF, SO_GF_PNVM, None, true),
    sub(0x7a70, 0x0098, SO_GF, SO_GF_PNVM, None, true),
    sub(0x7a70, 0x00b0, SO_GF4, SO_GF4_PNVM, None, false),
    sub(0x7a70, 0x0310, SO_GF, SO_GF_PNVM, None, true),
    sub(0x7a70, 0x0510, SO_GF, SO_GF_PNVM, None, true),
    sub(0x7a70, 0x0a10, SO_GF, SO_GF_PNVM, None, true),
    sub(0x7af0, 0x0090, SO_GF, SO_GF_PNVM, None, false),
    sub(0x7af0, 0x0098, SO_GF, SO_GF_PNVM, None, false),
    sub(0x7af0, 0x00b0, SO_GF4, SO_GF4_PNVM, None, true),
    sub(0x7a70, 0x1691, SO_GF4, SO_GF4_PNVM, K1690S, true),
    sub(0x7a70, 0x1692, SO_GF4, SO_GF4_PNVM, K1690I, true),
    sub(0x7af0, 0x0310, SO_GF, SO_GF_PNVM, None, false),
    sub(0x7af0, 0x0510, SO_GF, SO_GF_PNVM, None, false),
    sub(0x7af0, 0x0a10, SO_GF, SO_GF_PNVM, None, false),
    sub(0x7f70, 0x1691, SO_GF4, SO_GF4_PNVM, K1690S, true),
    sub(0x7f70, 0x1692, SO_GF4, SO_GF4_PNVM, K1690I, true),
    sub(0x2726, 0x1671, SO_GF, SO_GF_PNVM, K1675S, false),
    sub(0x2726, 0x1672, SO_GF, SO_GF_PNVM, K1675I, false),
    sub(0x51f0, 0x1671, SO_GF, SO_GF_PNVM, K1675S, false),
    sub(0x51f0, 0x1672, SO_GF, SO_GF_PNVM, K1675I, false),
    sub(0x54f0, 0x1671, SO_GF, SO_GF_PNVM, K1675S, false),
    sub(0x54f0, 0x1672, SO_GF, SO_GF_PNVM, K1675I, false),
    sub(0x7a70, 0x1671, SO_GF, SO_GF_PNVM, K1675S, false),
    sub(0x7a70, 0x1672, SO_GF, SO_GF_PNVM, K1675I, false),
    sub(0x7af0, 0x1671, SO_GF, SO_GF_PNVM, K1675S, false),
    sub(0x7af0, 0x1672, SO_GF, SO_GF_PNVM, K1675I, false),
    sub(0x7f70, 0x1671, SO_GF, SO_GF_PNVM, K1675S, false),
    sub(0x7f70, 0x1672, SO_GF, SO_GF_PNVM, K1675I, false),
    sub(0x7e40, 0x1671, MA_GF, MA_GF_PNVM, K1675S, false),
    sub(0x7e40, 0x1672, MA_GF, MA_GF_PNVM, K1675I, false),
];

/// Every firmware file any known chip may ask for (what the fetch script downloads).
pub const FIRMWARE_FILES: &[&str] = &[CC, QUZ_HR, QUZ_JF, QU_B_HR, QU_C_HR, SO_GF, SO_GF_PNVM, SO_GF4, SO_GF4_PNVM, TY_GF, TY_GF_PNVM, MA_GF, MA_GF_PNVM];
