// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Intel Bluetooth controllers: which USB ids are which part, and loading their
//! operational firmware through Intel's vendor HCI commands.
//!
//! The flow (behaviour as Linux `btintel` implements it, written fresh from that
//! reference; recorded in docs/BRIDGE_LEDGER.md table 2):
//! 1. Intel Read Version (0xFC05, parameter 0xFF). Older parts (AX201, AX200, 9560)
//!    answer with the fixed 10-byte layout (hardware platform 0x37); AX210 and newer
//!    answer with type-length-value records.
//! 2. If the controller already runs operational firmware (fw_variant 0x23 / image
//!    type 3), nothing to load. Otherwise it waits in its bootloader.
//! 3. Secure Send (0xFC09) of the .sfi file, in fragments of at most 252 bytes: the
//!    CSS header (fragment type 0), public key (3), signature (2), then the command
//!    buffer (1) in pieces that end on 4-byte boundaries. RSA headers are 128/256/256
//!    bytes at offset 0; AX210 and newer also carry ECDSA headers (128/96/96) at
//!    offset 644, used when the chip's secure boot engine type says so.
//! 4. Wait for vendor event 0xFF/0x06 (download result), send Intel Reset (0xFC01)
//!    with the boot address the .sfi named in its Write Boot Params (0xFC0E) command,
//!    and wait for vendor event 0xFF/0x02 (booted).
//! 5. Send each record of the .ddc file with Intel DDC Config Write (0xFC8B).
//!
//! In the bootloader, USB parts take Secure Send on the bulk OUT endpoint and may
//! answer on bulk IN; `Out::bulk` and `bootloader_event` carry that through the ring.

use crate::hci::{self, Event};
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const INTEL_VENDOR: u16 = 0x8087;

pub const OP_READ_VERSION: u16 = 0xFC05;
pub const OP_RESET: u16 = 0xFC01;
pub const OP_SECURE_SEND: u16 = 0xFC09;
pub const OP_WRITE_BOOT_PARAMS: u16 = 0xFC0E;
pub const OP_DDC_WRITE: u16 = 0xFC8B;

/// A USB Bluetooth part this stack knows. sex-hwdb covers PCI only, so the USB table
/// lives here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part { pub vendor: u16, pub product: u16, pub name: &'static str, pub tlv: bool }

pub const PARTS: &[Part] = &[
    Part { vendor: INTEL_VENDOR, product: 0x0025, name: "Intel Wireless-AC 9260/9160 (ThP)", tlv: false },
    Part { vendor: INTEL_VENDOR, product: 0x0026, name: "Intel Wi-Fi 6 AX201 (HrP)", tlv: false },
    Part { vendor: INTEL_VENDOR, product: 0x0029, name: "Intel Wi-Fi 6 AX200 (CcP)", tlv: false },
    Part { vendor: INTEL_VENDOR, product: 0x0032, name: "Intel Wi-Fi 6E AX210 / Killer AX1675x (TyP)", tlv: true },
    Part { vendor: INTEL_VENDOR, product: 0x0033, name: "Intel Wi-Fi 6E AX211 / Killer AX1675i (GfP)", tlv: true },
    Part { vendor: INTEL_VENDOR, product: 0x0aaa, name: "Intel Wireless-AC 9560 (JfP)", tlv: false },
];

pub fn part(vendor: u16, product: u16) -> Option<&'static Part> {
    PARTS.iter().find(|p| p.vendor == vendor && p.product == product)
}

/// What Intel Read Version said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Legacy { hw_platform: u8, hw_variant: u8, hw_revision: u8, fw_variant: u8, fw_revision: u8, fw_build_num: u8, fw_build_ww: u8, fw_build_yy: u8, fw_patch_num: u8 },
    Tlv { cnvi_top: u32, cnvr_top: u32, cnvi_bt: u32, image_type: u8, sbe_type: u8, build_num: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Status { opcode: u16, status: u8 },
    BadVersion,
    UnsupportedVariant(u8),
    BadFirmware(&'static str),
    DownloadFailed(u8),
}

pub mod tlv {
    pub const CNVI_TOP: u8 = 0x10;
    pub const CNVR_TOP: u8 = 0x11;
    pub const CNVI_BT: u8 = 0x12;
    pub const IMAGE_TYPE: u8 = 0x1c;
    pub const BUILD_NUM: u8 = 0x1f;
    pub const SBE_TYPE: u8 = 0x2f;
}

fn le32(b: &[u8]) -> u32 {
    let mut a = [0u8; 4];
    let n = b.len().min(4);
    a[..n].copy_from_slice(&b[..n]);
    u32::from_le_bytes(a)
}

impl Version {
    /// Parse the return parameters of Intel Read Version (status first).
    pub fn parse(ret: &[u8]) -> Result<Version, Error> {
        if ret.is_empty() { return Err(Error::BadVersion); }
        if ret[0] != 0 { return Err(Error::Status { opcode: OP_READ_VERSION, status: ret[0] }); }
        if ret.len() == 10 && ret[1] == 0x37 {
            return Ok(Version::Legacy { hw_platform: ret[1], hw_variant: ret[2], hw_revision: ret[3], fw_variant: ret[4],
                fw_revision: ret[5], fw_build_num: ret[6], fw_build_ww: ret[7], fw_build_yy: ret[8], fw_patch_num: ret[9] });
        }
        let (mut cnvi_top, mut cnvr_top, mut cnvi_bt, mut image_type, mut sbe_type, mut build_num) = (None, None, None, None, 0, 0);
        let mut at = 1;
        while at + 2 <= ret.len() {
            let (t, len) = (ret[at], ret[at + 1] as usize);
            let v = ret.get(at + 2..at + 2 + len).ok_or(Error::BadVersion)?;
            match t {
                tlv::CNVI_TOP => cnvi_top = Some(le32(v)),
                tlv::CNVR_TOP => cnvr_top = Some(le32(v)),
                tlv::CNVI_BT => cnvi_bt = Some(le32(v)),
                tlv::IMAGE_TYPE => image_type = v.first().copied(),
                tlv::SBE_TYPE => sbe_type = v.first().copied().unwrap_or(0),
                tlv::BUILD_NUM => build_num = le32(v),
                _ => {}
            }
            at += 2 + len;
        }
        Ok(Version::Tlv { cnvi_top: cnvi_top.ok_or(Error::BadVersion)?, cnvr_top: cnvr_top.ok_or(Error::BadVersion)?,
            cnvi_bt: cnvi_bt.ok_or(Error::BadVersion)?, image_type: image_type.ok_or(Error::BadVersion)?, sbe_type, build_num })
    }

    pub fn bootloader(&self) -> bool {
        match *self { Version::Legacy { fw_variant, .. } => fw_variant == 0x06, Version::Tlv { image_type, .. } => image_type == 0x01 }
    }

    pub fn hw_variant(&self) -> u8 {
        match *self { Version::Legacy { hw_variant, .. } => hw_variant, Version::Tlv { cnvi_bt, .. } => ((cnvi_bt >> 16) & 0x3f) as u8 }
    }

    /// The firmware file base name in linux-firmware's `intel/` directory, without the
    /// suffix: "ibt-19-0-4" (legacy: hw_variant-hw_revision-fw_revision) or
    /// "ibt-0041-0041" (TLV: CNVi and CNVR top type and step, packed and byte-swapped).
    pub fn firmware_base(&self) -> Result<String, Error> {
        match *self {
            Version::Legacy { hw_variant, hw_revision, fw_revision, .. } => match hw_variant {
                0x11..=0x14 => Ok(format!("ibt-{}-{}-{}", hw_variant, hw_revision, fw_revision)),
                v => Err(Error::UnsupportedVariant(v)),
            },
            Version::Tlv { cnvi_top, cnvr_top, .. } => {
                let pack = |top: u32| (((top & 0xfff) << 4) as u16 | ((top >> 24) & 0xf) as u16).swap_bytes();
                Ok(format!("ibt-{:04x}-{:04x}", pack(cnvi_top), pack(cnvr_top)))
            }
        }
    }
}

/// A command for the controller; `bulk`: send it on the bulk OUT endpoint (Secure Send
/// in the bootloader) instead of the control endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out { pub packet: Vec<u8>, pub bulk: bool }

const RSA_HEADER: usize = 644;
const ECDSA_HEADER: usize = 320;
const CSS_VERSION_AT: usize = 8;

fn secure_send(out: &mut Vec<Vec<u8>>, kind: u8, mut data: &[u8]) {
    while !data.is_empty() {
        let n = data.len().min(252);
        let mut p = alloc::vec![kind];
        p.extend_from_slice(&data[..n]);
        out.push(hci::command(OP_SECURE_SEND, &p));
        data = &data[n..];
    }
}

/// The Secure Send commands for one .sfi file, and the boot address its Write Boot
/// Params command names (0 if it has none).
pub fn secure_send_plan(sfi: &[u8], v: &Version) -> Result<(Vec<Vec<u8>>, u32), Error> {
    let bad = Error::BadFirmware;
    if sfi.len() < RSA_HEADER { return Err(bad("short")); }
    if le32(&sfi[CSS_VERSION_AT..]) != 0x0001_0000 { return Err(bad("rsa css version")); }
    let variant = v.hw_variant();
    let sbe = match *v { Version::Tlv { sbe_type, .. } => sbe_type, _ => 0 };
    let mut out = Vec::new();
    let payload = if variant <= 0x14 {
        if sbe != 0 { return Err(bad("sbe type")); }
        RSA_HEADER
    } else if variant >= 0x17 {
        if sfi.len() < RSA_HEADER + ECDSA_HEADER || sfi[RSA_HEADER] != 0x06 { return Err(bad("ecdsa header")); }
        if le32(&sfi[RSA_HEADER + CSS_VERSION_AT..]) != 0x0002_0000 { return Err(bad("ecdsa css version")); }
        RSA_HEADER + ECDSA_HEADER
    } else {
        return Err(Error::UnsupportedVariant(variant));
    };
    match sbe {
        0 => {
            secure_send(&mut out, 0x00, &sfi[0..128]);
            secure_send(&mut out, 0x03, &sfi[128..384]);
            secure_send(&mut out, 0x02, &sfi[388..644]);
        }
        1 => {
            secure_send(&mut out, 0x00, &sfi[644..772]);
            secure_send(&mut out, 0x03, &sfi[772..868]);
            secure_send(&mut out, 0x02, &sfi[868..964]);
        }
        _ => return Err(bad("sbe type")),
    }
    let mut boot_addr = 0;
    let (mut at, mut frag) = (payload, 0usize);
    while at < sfi.len() {
        let cmd = at + frag;
        if cmd + 3 > sfi.len() { return Err(bad("truncated command")); }
        let opcode = u16::from_le_bytes([sfi[cmd], sfi[cmd + 1]]);
        let plen = sfi[cmd + 2] as usize;
        if cmd + 3 + plen > sfi.len() { return Err(bad("truncated command")); }
        if opcode == OP_WRITE_BOOT_PARAMS && plen >= 4 { boot_addr = le32(&sfi[cmd + 3..]); }
        frag += 3 + plen;
        if frag % 4 == 0 {
            secure_send(&mut out, 0x01, &sfi[at..at + frag]);
            at += frag;
            frag = 0;
        } else if at + frag >= sfi.len() {
            return Err(bad("unaligned end"));
        }
    }
    Ok((out, boot_addr))
}

/// Intel Reset into the loaded firmware at `boot_addr`.
pub fn reset_command(boot_addr: u32) -> Vec<u8> {
    let mut p = alloc::vec![0x00, 0x01, 0x00, 0x01];
    p.extend_from_slice(&boot_addr.to_le_bytes());
    hci::command(OP_RESET, &p)
}

/// The DDC Config Write commands for a .ddc file: records of {length, id (le16), data}.
pub fn ddc_commands(ddc: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < ddc.len() {
        let n = ddc[at] as usize + 1;
        if n < 3 || at + n > ddc.len() { return Err(Error::BadFirmware("ddc record")); }
        out.push(hci::command(OP_DDC_WRITE, &ddc[at..at + n]));
        at += n;
    }
    Ok(out)
}

/// In the bootloader the controller may send events on bulk IN, which the transport
/// hands over as ACL packets: the event they hold, if they hold one.
pub fn bootloader_event(packet: &[u8]) -> Option<Vec<u8>> {
    if packet.len() < 3 || packet[0] != hci::ACL { return None; }
    let raw = &packet[1..];
    if raw.len() == 2 + raw[1] as usize && (raw[0] == hci::ev::COMMAND_COMPLETE || raw[0] == hci::ev::VENDOR || raw[0] == hci::ev::COMMAND_STATUS) {
        let mut e = alloc::vec![hci::EVENT];
        e.extend_from_slice(raw);
        return Some(e);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome { AlreadyRunning, Loaded }

#[derive(Clone, Debug, PartialEq, Eq)]
enum State { Start, Version, NeedFirmware, Download, DownloadResult, Booting, Ddc, Done(Outcome), Failed }

/// The firmware load as a state machine with no I/O.
pub struct Loader {
    state: State,
    pub version: Option<Version>,
    queue: VecDeque<Vec<u8>>,
    waiting: Option<u16>,
    boot_addr: u32,
    ddc: Vec<Vec<u8>>,
    pub sent_fragments: u32,
}

impl Default for Loader { fn default() -> Self { Self::new() } }

impl Loader {
    pub fn new() -> Loader {
        Loader { state: State::Start, version: None, queue: VecDeque::new(), waiting: None, boot_addr: 0, ddc: Vec::new(), sent_fragments: 0 }
    }
    /// Firmware is needed now: the .sfi and .ddc file names.
    pub fn wants_firmware(&self) -> Option<(String, String)> {
        if self.state != State::NeedFirmware { return None; }
        let base = self.version?.firmware_base().ok()?;
        Some((format!("{base}.sfi"), format!("{base}.ddc")))
    }
    pub fn outcome(&self) -> Option<Outcome> { if let State::Done(o) = self.state { Some(o) } else { None } }
    pub fn failed(&self) -> bool { self.state == State::Failed }
    pub fn boot_addr(&self) -> u32 { self.boot_addr }

    /// Hand over the firmware files (a missing .ddc is allowed: no DDC records).
    pub fn provide(&mut self, sfi: &[u8], ddc: Option<&[u8]>) -> Result<(), Error> {
        let v = self.version.ok_or(Error::BadVersion)?;
        let r = (|| {
            let (cmds, boot) = secure_send_plan(sfi, &v)?;
            let ddc = match ddc { Some(d) => ddc_commands(d)?, None => Vec::new() };
            Ok((cmds, boot, ddc))
        })();
        match r {
            Ok((cmds, boot, ddc)) => {
                self.queue = cmds.into();
                self.boot_addr = boot;
                self.ddc = ddc;
                self.state = State::Download;
                Ok(())
            }
            Err(e) => { self.state = State::Failed; Err(e) }
        }
    }

    /// The next command to send, if one is due.
    pub fn next(&mut self) -> Option<Out> {
        if self.waiting.is_some() { return None; }
        match self.state {
            State::Start => {
                self.state = State::Version;
                self.waiting = Some(OP_READ_VERSION);
                Some(Out { packet: hci::command(OP_READ_VERSION, &[0xFF]), bulk: false })
            }
            State::Download | State::Ddc => {
                let p = self.queue.pop_front()?;
                self.waiting = hci::parse_command(&p).map(|c| c.0);
                if self.state == State::Download { self.sent_fragments += 1; }
                Some(Out { packet: p, bulk: self.state == State::Download })
            }
            // Intel Reset: no Command Complete follows, only the boot event.
            State::Booting => self.queue.pop_front().map(|packet| Out { packet, bulk: false }),
            _ => None,
        }
    }

    /// Feed an event. Errors end the load.
    pub fn on_event(&mut self, e: &Event) -> Result<(), Error> {
        let r = self.step(e);
        if r.is_err() { self.state = State::Failed; }
        r
    }

    fn step(&mut self, e: &Event) -> Result<(), Error> {
        match *e {
            Event::CommandComplete { opcode, ret, .. } if Some(opcode) == self.waiting => {
                self.waiting = None;
                if opcode == OP_READ_VERSION {
                    let v = Version::parse(ret)?;
                    self.version = Some(v);
                    if !v.bootloader() { self.state = State::Done(Outcome::AlreadyRunning); return Ok(()); }
                    v.firmware_base()?;
                    self.state = State::NeedFirmware;
                    return Ok(());
                }
                let status = ret.first().copied().unwrap_or(0xff);
                if status != 0 { return Err(Error::Status { opcode, status }); }
                if self.queue.is_empty() {
                    match self.state {
                        State::Download => self.state = State::DownloadResult,
                        State::Ddc => self.state = State::Done(Outcome::Loaded),
                        _ => {}
                    }
                }
                Ok(())
            }
            Event::CommandStatus { opcode, status, .. } if Some(opcode) == self.waiting && status != 0 => {
                Err(Error::Status { opcode, status })
            }
            Event::Vendor { params } if params.first() == Some(&0x06) && matches!(self.state, State::DownloadResult | State::Download) => {
                let result = params.get(1).copied().unwrap_or(0xff);
                if result != 0 { return Err(Error::DownloadFailed(result)); }
                if self.state == State::DownloadResult {
                    // Reset into the new firmware; the controller answers with a boot event, not a completion.
                    self.queue.push_back(reset_command(self.boot_addr));
                    self.state = State::Booting;
                }
                Ok(())
            }
            Event::Vendor { params } if params.first() == Some(&0x02) && self.state == State::Booting => {
                self.queue = core::mem::take(&mut self.ddc).into();
                self.state = if self.queue.is_empty() { State::Done(Outcome::Loaded) } else { State::Ddc };
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
