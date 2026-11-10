// SPDX-License-Identifier: GPL-3.0-only
//! Stage W3: from ALIVE to a firmware ready to scan. Hands the platform NVM to
//! AX210-class chips, runs the firmware's own init (NVM access, INIT_COMPLETE),
//! reads the NVM (MAC address, antennas, bands, regulatory), then sends the
//! configuration iwx(4) sends in `iwx_run_init_mvm_ucode` and `iwx_init_hw`:
//! TX antennas, Bluetooth coexistence, SoC latency, DQA, thermal, power, the
//! regulatory domain (MCC), scan configuration and beacon filtering.
//! One bounded step per call; every wait has a deadline.
use crate::bringup::Bringup;
use crate::cmdq::{self, CommandQueue};
use crate::context::{Dma, DmaRead, Layout};
use crate::csr::{self, Registers};
use crate::devices::{Chip, Family};
use crate::hcmd::{self as h, Alive, FirmwareError, Packet};
use crate::ucode::{self as u, Firmware};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Pnvm, WaitPnvm, ExtendedConfig, NvmAccessComplete, WaitInitComplete, NvmInfo, MacAddress,
    TxAntennas, BtCoex, SocConfig, Dqa, Thermal, Power, Mcc, ScanConfig, BeaconFilter,
    Ready, Failed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No answer to a command (group, opcode) within a second.
    Timeout { group: u8, opcode: u8 },
    /// A notification never came (PNVM loaded, INIT_COMPLETE).
    NoNotification(Notification),
    /// The firmware answered with the "failed" flag.
    Refused { group: u8, opcode: u8 },
    /// REPLY_ERROR from the firmware.
    Firmware(FirmwareError),
    /// The chip raised its software or hardware error interrupt.
    Crashed { cause: u32 },
    /// An answer of the wrong size or contents.
    BadAnswer { group: u8, opcode: u8, len: usize },
    NoMacAddress,
    NicLock,
    BadRxBuffer(u16),
    /// The firmware lacks a feature this driver needs (iwx(4) refuses it too).
    Unsupported(&'static str),
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notification { PnvmLoaded, InitComplete }

/// Where the platform NVM came from (AX210 family).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PnvmSource { NotNeeded, Embedded, File, NoMatch, Missing, TooBig }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pnvm { pub source: PnvmSource, pub sections: usize, pub bytes: u32, pub version: u32 }

/// What NVM_GET_INFO said.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nvm {
    pub version: u16,
    pub hw_addrs: u8,
    pub sku_flags: u32,
    pub tx_chains: u8,
    pub rx_chains: u8,
    /// Location-aware regulatory: the firmware picks the channel set (MCC).
    pub lar: bool,
    /// Channels the NVM allows.
    pub channels: u16,
}
pub const SKU_2GHZ: u32 = 1 << 0;
pub const SKU_5GHZ: u32 = 1 << 1;
pub const SKU_11N: u32 = 1 << 2;
pub const SKU_11AC: u32 = 1 << 3;
pub const SKU_11AX: u32 = 1 << 4;
pub const SKU_MIMO_DISABLED: u32 = 1 << 5;

/// The regulatory answer to MCC_UPDATE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regulatory { pub status: u32, pub mcc: [u8; 2], pub source: u8, pub channels: u32 }

/// Firmware facts the steps need, copied so `Init` borrows nothing.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    pub phy_config: u32,
    pub capa: [u32; 5],
    pub api: [u32; 4],
    pub scan_config_version: Option<u8>,
    pub scan_request_version: Option<u8>,
    pub mcc_answer_version: Option<u8>,
    /// Channels the firmware takes in one scan request.
    pub scan_channels: u32,
}
impl Features {
    pub fn of(fw: &Firmware<'_>) -> Self {
        Self {
            phy_config: fw.phy_config, capa: fw.capa_bits, api: fw.api_bits,
            scan_config_version: fw.command_version(h::LONG_GROUP, h::SCAN_CFG_CMD).map(|v| v.version),
            scan_request_version: fw.command_version(h::LONG_GROUP, h::SCAN_REQ_UMAC).map(|v| v.version),
            mcc_answer_version: fw.command_version(h::LONG_GROUP, h::MCC_UPDATE_CMD).map(|v| v.notification),
            scan_channels: fw.scan_channels,
        }
    }
    pub fn capa(&self, bit: usize) -> bool { self.capa.get(bit / 32).is_some_and(|w| w & (1 << (bit % 32)) != 0) }
    pub fn api(&self, bit: usize) -> bool { self.api.get(bit / 32).is_some_and(|w| w & (1 << (bit % 32)) != 0) }
    pub fn tx_antennas(&self) -> u8 { ((self.phy_config >> 16) & 0xf) as u8 }
    pub fn rx_antennas(&self) -> u8 { ((self.phy_config >> 20) & 0xf) as u8 }
}

const COMMAND_US: u64 = 1_000_000;
const NOTIFICATION_US: u64 = 2_000_000;
const LOCK_US: u64 = 150_000;
const NVM_INFO_V4: usize = 8 + 4 + 8 + 4 + 4 + 110 * 4;
const NVM_INFO_V3: usize = 8 + 4 + 8 + 4 + 51 * 2 + 2;
const INIT_NVM: u32 = 1 << 1;
const BT_COEX_WIFI: u32 = 0x3;
const SOC_DISCRETE: u32 = 1 << 0;
const SOC_LOW_LATENCY: u32 = 1 << 1;
const SOC_LTR_DELAY_MASK: u32 = 0xc;
const MCC_SOURCE_GET_CURRENT: u8 = 0x10;
const MCC_SOURCE_OLD_FW: u8 = 0;

pub struct Init {
    pub step: Step,
    pub chip: Chip,
    pub features: Features,
    pub cmdq: CommandQueue,
    pub pnvm: Pnvm,
    pub nvm: Option<Nvm>,
    pub mac: [u8; 6],
    pub regulatory: Option<Regulatory>,
    /// Flags of the 2.4/5 GHz channels (`scan::NVM_CHANNELS` order): from the NVM,
    /// then as the regulatory answer (MCC) left them.
    pub channel_flags: [u32; 51],
    pub pnvm_loaded: bool,
    pub init_complete: bool,
    /// Commands answered so far.
    pub answered: u32,
    /// Notifications that were not answers (statistics, debug log, ...).
    pub other: u32,
    pub last_error: Option<FirmwareError>,
    since: u64,
    sent: bool,
    /// The NIC lock iwx_init_hw holds from the MAC address read to the end.
    locked: bool,
}

fn word(d: &[u8], at: usize) -> u32 { u32::from_le_bytes(d[at..at + 4].try_into().unwrap()) }

/// iwx_is_valid_mac_addr: not zero, broadcast, multicast or Intel's placeholder.
pub fn valid_mac(a: &[u8; 6]) -> bool {
    *a != [0; 6] && *a != [0xff; 6] && *a != [0x02, 0xcc, 0xaa, 0xff, 0xee, 0x00] && a[0] & 1 == 0
}

/// The two fuse words hold the address byte-swapped (iwx_flip_hw_address).
pub fn mac_from_words(w0: u32, w1: u32) -> [u8; 6] {
    let a = w0.to_le_bytes();
    let b = w1.to_le_bytes();
    [a[3], a[2], a[1], a[0], b[1], b[0]]
}

impl Init {
    /// Starts W3 right after ALIVE. On AX210-class chips this places the platform
    /// NVM section for the SKU the firmware reported (from the firmware file
    /// itself, or else from the chip's .pnvm file) where the chip will look.
    pub fn new(chip: Chip, fw: &Firmware<'_>, alive: &Alive, pnvm_file: Option<&[u8]>, layout: &Layout, base: u64,
        dma: &mut impl Dma, hw_rev: u32, rf_id: u32, now_us: u64) -> Self
    {
        let features = Features::of(fw);
        let mut pnvm = Pnvm { source: PnvmSource::NotNeeded, sections: 0, bytes: 0, version: 0 };
        let mut step = Step::ExtendedConfig;
        if chip.family == Family::Ax210 && alive.sku != [0; 3] {
            step = Step::Pnvm;
            let (from, source) = match (fw.pnvm, pnvm_file) {
                (Some(p), _) => (Some(p), PnvmSource::Embedded),
                (None, Some(p)) => (Some(p), PnvmSource::File),
                (None, None) => (None, PnvmSource::Missing),
            };
            pnvm.source = source;
            if let Some(file) = from {
                let mac_type = ((hw_rev & 0xfff0) >> 4) as u16;
                let rf_type = ((rf_id & 0x0fff_000) >> 12) as u16;
                match u::pnvm_select(file, alive.sku, mac_type, rf_type) {
                    Some(img) => match layout.write_pnvm(dma, base, img.sections(), features.capa(u::CAPA_FRAGMENTED_PNVM_IMG)) {
                        Some((_, bytes)) => pnvm = Pnvm { source, sections: img.count, bytes, version: img.version },
                        None => pnvm.source = PnvmSource::TooBig,
                    },
                    None => pnvm.source = PnvmSource::NoMatch,
                }
            }
        }
        Self { step, chip, features, cmdq: CommandQueue::new(chip.family), pnvm, nvm: None, mac: [0; 6], regulatory: None, channel_flags: [0; 51],
            pnvm_loaded: false, init_complete: false, answered: 0, other: 0, last_error: None, since: now_us, sent: false, locked: false }
    }

    pub fn done(&self) -> bool { matches!(self.step, Step::Ready | Step::Failed(_)) }

    /// A scan of every channel the NVM and regulatory domain allow (stage W4).
    pub fn scan(&self, now_us: u64) -> crate::scan::Scan {
        use crate::scan;
        let five = self.nvm.is_some_and(|n| n.sku_flags & SKU_5GHZ != 0);
        let mut list = [scan::Channel::default(); scan::NVM_CHANNELS.len()];
        let n = scan::channels(&self.channel_flags, five, &mut list);
        let options = scan::Options {
            mac: self.mac, ext_channel: self.features.api(u::API_SCAN_EXT_CHAN_VER),
            ds_param: self.features.capa(u::CAPA_DS_PARAM_SET_IE_SUPPORT), has_5ghz: five,
            max_channels: self.features.scan_channels as usize,
        };
        scan::Scan::new(self.chip.family, &list[..n], &options, now_us)
    }
    fn go(&mut self, step: Step, now: u64) { self.step = step; self.since = now; self.sent = false; }
    fn fail(&mut self, f: Failure) { self.step = Step::Failed(f); }

    /// Reads everything the firmware sent: answers, notifications, errors.
    pub fn on_packet(&mut self, p: &Packet<'_>) {
        if self.cmdq.on_packet(p) { self.answered += 1; return; }
        if p.is(h::REGULATORY_AND_NVM_GROUP, h::PNVM_INIT_COMPLETE) { self.pnvm_loaded = true; }
        else if p.is(h::LEGACY_GROUP, h::INIT_COMPLETE_NOTIF) { self.init_complete = true; }
        else if let Some(e) = h::parse_error(p) { self.last_error = Some(e); }
        else { self.other = self.other.saturating_add(1); }
    }

    /// One step. Returns true while more steps are needed.
    pub fn advance<D: Dma + DmaRead>(&mut self, b: &mut Bringup, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        if self.done() { return false; }
        let cause = r.read(csr::INT);
        if cause == u32::MAX { self.fail(Failure::Gone); return false; }
        if cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.fail(Failure::Crashed { cause }); }
        else {
            if cause != 0 { r.write(csr::INT, cause); }
            if let Err(id) = b.receive(r, dma, |p| self.on_packet(p)) { self.fail(Failure::BadRxBuffer(id)); }
            else if let Some(e) = self.last_error { self.fail(Failure::Firmware(e)); }
        }
        if self.done() {
            if self.locked { csr::nic_unlock(r); self.locked = false; }
            return false;
        }
        let waited = now.saturating_sub(self.since);
        match self.step {
            Step::Pnvm => {
                // Tell the firmware the PNVM is in place, or (no file, or no section
                // for this SKU) to go on without it, as iwx_load_pnvm does.
                if csr::nic_lock(r) {
                    csr::write_prph(r, self.chip.family, csr::umac_prph(self.chip.family, csr::UREG_DOORBELL_TO_ISR6), csr::DOORBELL_PNVM);
                    csr::nic_unlock(r);
                    self.go(Step::WaitPnvm, now);
                } else if waited >= LOCK_US { self.fail(Failure::NicLock); }
            }
            Step::WaitPnvm => {
                if self.pnvm_loaded { self.go(Step::ExtendedConfig, now); }
                else if waited >= NOTIFICATION_US { self.fail(Failure::NoNotification(Notification::PnvmLoaded)); }
            }
            Step::WaitInitComplete => {
                if self.init_complete { self.go(Step::NvmInfo, now); }
                else if waited >= NOTIFICATION_US { self.fail(Failure::NoNotification(Notification::InitComplete)); }
            }
            Step::MacAddress => {
                if csr::nic_lock(r) {
                    let mut mac = mac_from_words(r.read(csr::MAC_ADDR_STRAP), r.read(csr::MAC_ADDR_STRAP + 4));
                    if !valid_mac(&mac) { mac = mac_from_words(r.read(csr::MAC_ADDR_OTP), r.read(csr::MAC_ADDR_OTP + 4)); }
                    self.locked = true;
                    if !valid_mac(&mac) { self.fail(Failure::NoMacAddress); csr::nic_unlock(r); self.locked = false; return false; }
                    self.mac = mac;
                    self.go(Step::TxAntennas, now);
                } else if waited >= LOCK_US { self.fail(Failure::NicLock); }
            }
            Step::Ready | Step::Failed(_) => {}
            step => self.command_step(step, b, r, dma, now, waited),
        }
        if self.done() && self.locked { csr::nic_unlock(r); self.locked = false; }
        !self.done()
    }

    fn command_step<D: Dma + DmaRead>(&mut self, step: Step, b: &Bringup, r: &mut impl Registers, dma: &mut D, now: u64, waited: u64) {
        if !self.sent {
            let mut payload = [0u8; 64];
            let Some((group, opcode, len)) = (match self.command(step, &mut payload) {
                Ok(c) => c,
                Err(f) => { self.fail(f); return; }
            }) else {
                let next = self.next(step);
                self.go(next, now);
                return;
            };
            match self.cmdq.send(dma, r, &b.layout, b.base, group, opcode, 0, &payload[..len]) {
                Ok(_) => { self.sent = true; self.since = now; }
                Err(_) => self.fail(Failure::Timeout { group, opcode }),
            }
            return;
        }
        if let Some(a) = self.cmdq.answer {
            if a.failed { self.fail(Failure::Refused { group: a.sent.group, opcode: a.sent.opcode }); return; }
            if let Err(f) = self.answer(step) { self.fail(f); return; }
            self.cmdq.answer = None;
            let next = self.next(step);
            self.go(next, now);
        } else if waited >= COMMAND_US {
            let s = self.cmdq.pending.unwrap_or(cmdq::Sent { group: 0, opcode: 0, index: 0 });
            self.cmdq.abandon();
            self.fail(Failure::Timeout { group: s.group, opcode: s.opcode });
        }
    }

    fn next(&self, step: Step) -> Step {
        match step {
            Step::Pnvm => Step::WaitPnvm,
            Step::WaitPnvm => Step::ExtendedConfig,
            Step::ExtendedConfig => Step::NvmAccessComplete,
            Step::NvmAccessComplete => Step::WaitInitComplete,
            Step::WaitInitComplete => Step::NvmInfo,
            Step::NvmInfo => Step::MacAddress,
            Step::MacAddress => Step::TxAntennas,
            Step::TxAntennas => Step::BtCoex,
            Step::BtCoex => Step::SocConfig,
            Step::SocConfig => Step::Dqa,
            Step::Dqa => Step::Thermal,
            Step::Thermal => Step::Power,
            Step::Power => Step::Mcc,
            Step::Mcc => Step::ScanConfig,
            Step::ScanConfig => Step::BeaconFilter,
            Step::BeaconFilter | Step::Ready => Step::Ready,
            Step::Failed(f) => Step::Failed(f),
        }
    }

    /// The command for a step: (group, opcode, payload length), or None to skip it.
    fn command(&self, step: Step, p: &mut [u8; 64]) -> Result<Option<(u8, u8, usize)>, Failure> {
        let f = &self.features;
        let nvm = self.nvm.unwrap_or_default();
        let put = |p: &mut [u8; 64], at: usize, v: u32| p[at..at + 4].copy_from_slice(&v.to_le_bytes());
        Ok(Some(match step {
            Step::ExtendedConfig => { put(p, 0, INIT_NVM); (h::SYSTEM_GROUP, h::INIT_EXTENDED_CFG_CMD, 4) }
            Step::NvmAccessComplete => (h::REGULATORY_AND_NVM_GROUP, h::NVM_ACCESS_COMPLETE, 4),
            Step::NvmInfo => (h::REGULATORY_AND_NVM_GROUP, h::NVM_GET_INFO, 4),
            Step::TxAntennas => {
                let mut ant = f.tx_antennas();
                if nvm.tx_chains != 0 { ant &= nvm.tx_chains; }
                put(p, 0, ant as u32);
                (h::LEGACY_GROUP, h::TX_ANT_CONFIGURATION_CMD, 4)
            }
            Step::BtCoex => { put(p, 0, BT_COEX_WIFI); (h::LEGACY_GROUP, h::BT_CONFIG, 8) }
            Step::SocConfig => {
                let c = &self.chip;
                let flags = if !c.integrated { SOC_DISCRETE } else {
                    // The delay is a 2-bit value in bits 2-3 of the flags.
                    let mut fl = ((c.ltr_delay as u32) << 2) & SOC_LTR_DELAY_MASK;
                    if f.scan_request_version.is_some_and(|v| v >= 2) && c.low_latency_xtal { fl |= SOC_LOW_LATENCY; }
                    fl
                };
                put(p, 0, flags);
                put(p, 4, c.xtal_latency as u32);
                (h::SYSTEM_GROUP, h::SOC_CONFIGURATION_CMD, 8)
            }
            Step::Dqa => {
                if !f.capa(u::CAPA_DQA_SUPPORT) { return Ok(None); }
                put(p, 0, cmdq::QUEUE as u32);
                (h::DATA_PATH_GROUP, h::DQA_ENABLE_CMD, 4)
            }
            // An empty threshold list hands thermal shutdown to the firmware.
            Step::Thermal => {
                if !f.capa(u::CAPA_CT_KILL_BY_FW) { return Ok(None); }
                (h::PHY_OPS_GROUP, h::TEMP_REPORTING_THRESHOLDS_CMD, 4 + 8 * 2)
            }
            // Device power: no power saving while the driver is young (iwx level 0).
            Step::Power => (h::LEGACY_GROUP, h::POWER_TABLE_CMD, 4),
            Step::Mcc => {
                if !nvm.lar { return Ok(None); }
                if f.mcc_answer_version.is_some_and(|v| v >= 8) { return Err(Failure::Unsupported("mcc_answer_v8")); }
                // "ZZ": the world domain until the firmware learns the country.
                p[0..2].copy_from_slice(&((b'Z' as u16) << 8 | b'Z' as u16).to_le_bytes());
                p[2] = if f.api(u::API_WIFI_MCC_UPDATE) || f.capa(u::CAPA_LAR_MULTI_MCC) { MCC_SOURCE_GET_CURRENT } else { MCC_SOURCE_OLD_FW };
                (h::LONG_GROUP, h::MCC_UPDATE_CMD, 28)
            }
            Step::ScanConfig => {
                if !f.api(u::API_REDUCED_SCAN_CONFIG) { return Err(Failure::Unsupported("reduced_scan_config")); }
                if f.scan_config_version.is_none_or(|v| v < 5) { p[2] = 0xff; }
                let mut rx = f.rx_antennas();
                if nvm.rx_chains != 0 { rx &= nvm.rx_chains; }
                let mut tx = f.tx_antennas();
                if nvm.tx_chains != 0 { tx &= nvm.tx_chains; }
                put(p, 4, tx as u32);
                put(p, 8, rx as u32);
                (h::LONG_GROUP, h::SCAN_CFG_CMD, 12)
            }
            // All zero: beacon filtering off (iwx_disable_beacon_filter).
            Step::BeaconFilter => (h::LEGACY_GROUP, h::REPLY_BEACON_FILTERING_CMD, 15 * 4),
            _ => return Ok(None),
        }))
    }

    /// Reads the answer to a step's command.
    fn answer(&mut self, step: Step) -> Result<(), Failure> {
        let d = self.cmdq.response();
        let len = self.cmdq.answer.map_or(0, |a| a.len);
        match step {
            Step::NvmInfo => {
                let v4 = self.features.api(u::API_REGULATORY_NVM_INFO);
                let want = if v4 { NVM_INFO_V4 } else { NVM_INFO_V3 };
                if len != want { return Err(Failure::BadAnswer { group: h::REGULATORY_AND_NVM_GROUP, opcode: h::NVM_GET_INFO, len }); }
                for (i, f) in self.channel_flags.iter_mut().enumerate() {
                    *f = if v4 { word(d, 28 + i * 4) } else { u16::from_le_bytes([d[24 + i * 2], d[25 + i * 2]]) as u32 };
                }
                let channels = if v4 {
                    (0..110).filter(|i| word(d, 28 + i * 4) & 1 != 0).count()
                } else {
                    (0..51).filter(|i| d[24 + i * 2] & 1 != 0).count()
                } as u16;
                self.nvm = Some(Nvm {
                    version: u16::from_le_bytes([d[4], d[5]]), hw_addrs: d[7], sku_flags: word(d, 8),
                    tx_chains: word(d, 12) as u8, rx_chains: word(d, 16) as u8,
                    lar: word(d, 20) != 0 && self.features.capa(u::CAPA_LAR_SUPPORT), channels,
                });
            }
            Step::Mcc => {
                if len < 20 || len != 20 + 4 * word(d, 16) as usize {
                    return Err(Failure::BadAnswer { group: h::LONG_GROUP, opcode: h::MCC_UPDATE_CMD, len });
                }
                let n = word(d, 16) as usize;
                if n > 0 {
                    // The answer lists the channel flags in NVM order (iwx_init_channel_map).
                    for (i, f) in self.channel_flags.iter_mut().enumerate() {
                        *f = if i < n && 20 + i * 4 + 4 <= d.len() { word(d, 20 + i * 4) } else { 0 };
                    }
                }
                let mcc = u16::from_le_bytes([d[4], d[5]]);
                self.regulatory = Some(Regulatory { status: word(d, 0), mcc: [(mcc >> 8) as u8, mcc as u8], source: d[12], channels: word(d, 16) });
            }
            _ => {}
        }
        Ok(())
    }
}
