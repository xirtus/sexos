// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A register model of the Tiger Lake audio controller that plays the DSP and its
//! ROM: BAR0 with the extended capability list and stream descriptors, BAR4 with
//! ADSPCS, the IPC doorbells and the SRAM windows, and a host-DMA engine that reads
//! the firmware through the code-loader stream's buffer descriptor list. Faults can
//! be switched on to make any one boot step time out. Shared by the boot tests.
#![allow(dead_code)]

use sex_sof::hw::Hw;
use sex_sof::regs::{dsp, hda};

/// Bus address of the model's DMA scratch.
pub const BUS: u64 = 0x1_2340_0000;
/// Input streams the model reports (as on TGL-H).
pub const INPUTS: u32 = 7;
/// Output streams the model reports.
pub const OUTPUTS: u32 = 6;
/// Capability offsets of the model.
pub const SPIB_AT: u32 = 0x800;
/// Processing pipe capability offset.
pub const PP_AT: u32 = 0xa00;
/// Multi-link capability offset.
pub const ML_AT: u32 = 0xc00;
/// Bytes of FW_READY the model posts.
pub const FW_READY_LEN: usize = 0x40;
/// Microseconds between FW_ENTERED and FW_READY.
pub const FW_READY_AFTER_US: u64 = 5_000;

/// One register write, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum W {
    /// 8-bit BAR0 write.
    H8(u32, u8),
    /// 16-bit BAR0 write.
    H16(u32, u16),
    /// 32-bit BAR0 write.
    H32(u32, u32),
    /// 32-bit BAR4 write.
    D32(u32, u32),
}

/// Switches that make one part of the hardware misbehave.
#[derive(Clone, Copy, Debug, Default)]
pub struct Faults {
    /// GCTL.CRST never reads 1.
    pub controller_in_reset: bool,
    /// No processing pipe capability in the list.
    pub no_pp: bool,
    /// SDxCTL.SRST never reads back.
    pub stream_reset_stuck: bool,
    /// CPA never follows SPA.
    pub power_stuck: bool,
    /// CRST never clears.
    pub reset_stuck: bool,
    /// CSTALL never clears.
    pub stall_stuck: bool,
    /// The ROM never raises HIPCIDA.DONE.
    pub rom_ipc_silent: bool,
    /// The ROM never reports INIT_DONE.
    pub rom_init_stuck: bool,
    /// The DMA runs but the ROM never reports FW_ENTERED.
    pub no_fw_entered: bool,
    /// The firmware never posts FW_READY.
    pub no_fw_ready: bool,
    /// The firmware panics instead of posting FW_READY.
    pub panic: bool,
    /// The firmware posts a trace message before FW_READY.
    pub noise_before_ready: bool,
    /// The firmware never answers host IPC.
    pub ipc_silent: bool,
}

/// The model.
pub struct Model {
    bar0: Vec<u8>,
    bar4: Vec<u32>,
    scratch: Vec<u8>,
    /// Every write, in order.
    pub log: Vec<W>,
    /// The faults in effect.
    pub faults: Faults,
    /// Microseconds of `delay_us` so far.
    pub elapsed_us: u64,
    /// The payload the ROM expects over DMA.
    pub expected_payload: Vec<u8>,
    /// The last purge IPC the ROM saw.
    pub purge: Option<u32>,
    /// Bytes the DMA engine moved to the DSP.
    pub dma_bytes: Vec<u8>,
    entered_at: Option<u64>,
    ready_posted: bool,
    noise_posted: bool,
    /// Host IPC messages the firmware received (mailbox bytes).
    pub ipc_rx: Vec<Vec<u8>>,
    /// Number of `dma_sync` calls.
    pub syncs: u32,
}

impl Model {
    /// A healthy controller with a powered-off DSP.
    pub fn new(expected_payload: &[u8], faults: Faults) -> Self {
        let mut m = Model {
            bar0: vec![0; 0x4000],
            bar4: vec![0; (dsp::BAR4_SIZE / 4) as usize],
            scratch: vec![0; 0x10_0000],
            log: Vec::new(),
            faults,
            elapsed_us: 0,
            expected_payload: expected_payload.to_vec(),
            purge: None,
            dma_bytes: Vec::new(),
            entered_at: None,
            ready_posted: false,
            noise_posted: false,
            ipc_rx: Vec::new(),
            syncs: 0,
        };
        let gcap = (OUTPUTS << 12 | INPUTS << 8 | 1) as u16;
        m.put16(hda::GCAP, gcap);
        m.put32(hda::GCTL, if faults.controller_in_reset { 0 } else { hda::GCTL_CRST });
        m.put32(hda::LLCH, SPIB_AT);
        m.put32(SPIB_AT, hda::CAP_ID_SPIB << hda::CAP_ID_SHIFT | PP_AT);
        let pp_id = if faults.no_pp { hda::CAP_ID_DRSM } else { hda::CAP_ID_PP };
        m.put32(PP_AT, pp_id << hda::CAP_ID_SHIFT | ML_AT);
        m.put32(ML_AT, hda::CAP_ID_ML << hda::CAP_ID_SHIFT);
        // Bell's state: L1SEN on, guardband 0x20.
        m.put32(hda::VS_EM2, hda::VS_EM2_L1SEN | 0x5);
        m.bar0[hda::VS_LTRP as usize] = 0xc0 | 0x20;
        // Core powered off, held in reset and stalled; ROM status empty.
        m.bar4[(dsp::ADSPCS / 4) as usize] = dsp::adspcs_crst(dsp::CORE0) | dsp::adspcs_cstall(dsp::CORE0);
        m
    }

    fn put16(&mut self, off: u32, v: u16) { self.bar0[off as usize..off as usize + 2].copy_from_slice(&v.to_le_bytes()); }
    fn put32(&mut self, off: u32, v: u32) { self.bar0[off as usize..off as usize + 4].copy_from_slice(&v.to_le_bytes()); }
    /// Raw BAR0 word.
    pub fn h32(&self, off: u32) -> u32 {
        let o = off as usize;
        u32::from_le_bytes(self.bar0[o..o + 4].try_into().unwrap())
    }
    /// Raw BAR0 byte.
    pub fn h8(&self, off: u32) -> u8 { self.bar0[off as usize] }
    /// Raw BAR4 word.
    pub fn d32(&self, off: u32) -> u32 { self.bar4[(off / 4) as usize] }
    /// Sets a raw BAR4 word (the DSP side writing).
    pub fn set_d32(&mut self, off: u32, v: u32) { self.bar4[(off / 4) as usize] = v; }
    /// The DMA scratch.
    pub fn scratch(&self) -> &[u8] { &self.scratch }

    /// The loader stream the boot should pick: the last output stream.
    pub fn loader_index() -> u32 { INPUTS + OUTPUTS - 1 }
    /// Its tag.
    pub fn loader_tag() -> u32 { OUTPUTS }
    /// The ICCMAX stream: the last input stream.
    pub fn iccmax_index() -> u32 { INPUTS - 1 }

    /// Position of the first write matching `f`.
    pub fn first(&self, f: impl Fn(&W) -> bool) -> Option<usize> { self.log.iter().position(f) }
    /// Position of the last write matching `f`.
    pub fn last(&self, f: impl Fn(&W) -> bool) -> Option<usize> { self.log.iter().rposition(f) }

    fn core_running(&self) -> bool {
        let cs = self.d32(dsp::ADSPCS);
        let c = dsp::CORE0;
        cs & dsp::adspcs_cpa(c) != 0 && cs & (dsp::adspcs_crst(c) | dsp::adspcs_cstall(c)) == 0
    }

    fn post(&mut self, tdr: u32, body: &[u8]) {
        for (i, ch) in body.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..ch.len()].copy_from_slice(ch);
            self.set_d32(dsp::MBOX_UPLINK + 4 * i as u32, u32::from_le_bytes(w));
        }
        self.set_d32(dsp::HIPCTDA, 0);
        self.set_d32(dsp::HIPCTDR, dsp::HIPCTDR_BUSY | tdr);
    }

    /// Posts a DSP notification of `body` (first word must be its size).
    pub fn notify(&mut self, primary: u32, ext: u32, body: &[u8]) {
        self.set_d32(dsp::HIPCTDD, ext);
        self.post(primary, body);
    }

    /// The DSP and its ROM react to the current register state.
    fn tick(&mut self) {
        let fsr = self.d32(dsp::SRAM_ROM_STATUS) & dsp::FSR_STATE_MASK;
        // ROM: starts when core 0 runs, reads the purge request.
        if self.core_running() && fsr == 0 {
            let req = self.d32(dsp::HIPCIDR);
            if req & dsp::HIPCIDR_BUSY != 0 && self.purge.is_none() {
                self.purge = Some(req);
                if !self.faults.rom_ipc_silent {
                    self.set_d32(dsp::HIPCIDR, req & !dsp::HIPCIDR_BUSY);
                    self.set_d32(dsp::HIPCIDA, self.d32(dsp::HIPCIDA) | dsp::HIPCIDA_DONE);
                }
                if !self.faults.rom_init_stuck {
                    self.set_d32(dsp::SRAM_ROM_STATUS, dsp::FSR_STATE_INIT_DONE);
                }
            }
        }
        // Code loader: the ROM pulls the firmware once the host starts the stream.
        let fsr = self.d32(dsp::SRAM_ROM_STATUS) & dsp::FSR_STATE_MASK;
        if fsr == dsp::FSR_STATE_INIT_DONE && self.dma_bytes.is_empty() {
            let i = Self::loader_index();
            let sd = hda::sd(i);
            let ctl = self.h32(sd);
            let ppctl = self.h32(PP_AT + hda::PP_PPCTL);
            let decoupled = ppctl & hda::PPCTL_GPROCEN != 0 && ppctl & hda::ppctl_procen(i) != 0;
            let tag = (ctl & hda::SD_CTL_TAG_MASK) >> hda::SD_CTL_TAG_SHIFT;
            let purge_tag = self.purge.map(|p| (p >> dsp::ROM_IPC_DMA_SHIFT & 0x1f) + 1);
            if ctl & hda::SD_CTL_RUN != 0 && decoupled && Some(tag) == purge_tag {
                self.run_dma(sd);
                if self.dma_bytes == self.expected_payload && !self.faults.no_fw_entered {
                    self.set_d32(dsp::SRAM_ROM_STATUS, dsp::FSR_STATE_FW_ENTERED);
                    self.entered_at = Some(self.elapsed_us);
                }
            }
        }
        // Firmware: FW_READY a little after it was entered.
        if let Some(at) = self.entered_at {
            if self.faults.noise_before_ready && !self.noise_posted && self.d32(dsp::HIPCTDR) & dsp::HIPCTDR_BUSY == 0 {
                self.noise_posted = true;
                let mut trace = [0u8; 16];
                trace[0..4].copy_from_slice(&16u32.to_le_bytes());
                trace[4..8].copy_from_slice(&0x9000_0000u32.to_le_bytes());
                self.post(0x9000_0000 >> 1, &trace);
            }
            let free = self.d32(dsp::HIPCTDR) & dsp::HIPCTDR_BUSY == 0;
            if !self.ready_posted && free && self.elapsed_us >= at + FW_READY_AFTER_US && !self.faults.no_fw_ready {
                self.ready_posted = true;
                if self.faults.panic {
                    self.set_d32(dsp::HIPCTDD, 0x123);
                    self.set_d32(dsp::HIPCTDR, dsp::HIPCTDR_BUSY | dsp::IPC_PANIC_MAGIC | 0x5);
                } else {
                    self.post(0x7000_0000 >> 1, &fw_ready_bytes());
                }
            }
        }
    }

    fn run_dma(&mut self, sd: u32) {
        let bdl = (self.h32(sd + hda::SD_BDPU) as u64) << 32 | self.h32(sd + hda::SD_BDPL) as u64;
        let lvi = u16::from_le_bytes([self.h8(sd + hda::SD_LVI), self.h8(sd + hda::SD_LVI + 1)]) as usize;
        let cbl = self.h32(sd + hda::SD_CBL) as usize;
        assert_eq!(bdl % hda::DMA_ALIGN, 0, "BDL must be 128-byte aligned");
        let base = (bdl - BUS) as usize;
        let mut out = Vec::new();
        for e in 0..=lvi {
            let at = base + e * hda::BDLE_SIZE;
            let addr = u64::from_le_bytes(self.scratch[at..at + 8].try_into().unwrap());
            let len = u32::from_le_bytes(self.scratch[at + 8..at + 12].try_into().unwrap()) as usize;
            assert_eq!(addr % hda::DMA_ALIGN, 0, "buffer must be 128-byte aligned");
            let off = (addr - BUS) as usize;
            out.extend_from_slice(&self.scratch[off..off + len]);
        }
        assert_eq!(out.len(), cbl, "BDL lengths add up to CBL");
        self.dma_bytes = out;
    }

    fn host_ipc(&mut self, value: u32) {
        let fsr = self.d32(dsp::SRAM_ROM_STATUS) & dsp::FSR_STATE_MASK;
        if value & dsp::HIPCIDR_BUSY == 0 || fsr != dsp::FSR_STATE_FW_ENTERED || self.faults.ipc_silent {
            return;
        }
        let host = dsp::DEFAULT_HOST_BOX.offset;
        let size = self.d32(host) as usize;
        let mut msg = vec![0u8; size.min(0x2000)];
        for (i, ch) in msg.chunks_mut(4).enumerate() {
            let w = self.d32(host + 4 * i as u32).to_le_bytes();
            ch.copy_from_slice(&w[..ch.len()]);
        }
        let cmd = self.d32(host + 4);
        self.ipc_rx.push(msg);
        // Reply: sof_ipc_reply { size 12, cmd, error 0 }.
        self.set_d32(host, 12);
        self.set_d32(host + 4, cmd);
        self.set_d32(host + 8, 0);
        self.set_d32(dsp::HIPCIDR, value & !dsp::HIPCIDR_BUSY);
        self.set_d32(dsp::HIPCIDA, self.d32(dsp::HIPCIDA) | dsp::HIPCIDA_DONE);
    }

    fn write_dsp(&mut self, off: u32, v: u32) {
        let old = self.d32(off);
        let new = match off {
            dsp::ADSPCS => {
                let c = dsp::CORE0;
                let mut n = v & !dsp::adspcs_cpa(0xff);
                if self.faults.reset_stuck {
                    n |= old & dsp::adspcs_crst(c);
                }
                if self.faults.stall_stuck {
                    n |= old & dsp::adspcs_cstall(c);
                }
                let spa = n & dsp::adspcs_spa(c) != 0;
                if spa && !self.faults.power_stuck {
                    n |= dsp::adspcs_cpa(c);
                }
                if !spa {
                    // Power gone: the ROM forgets everything.
                    self.bar4[(dsp::SRAM_ROM_STATUS / 4) as usize] = 0;
                    self.purge = None;
                    self.dma_bytes.clear();
                    self.entered_at = None;
                    self.ready_posted = false;
                }
                n
            }
            // Write-1-to-clear.
            dsp::HIPCIDA => old & !(v & dsp::HIPCIDA_DONE),
            dsp::HIPCTDR => old & !(v & dsp::HIPCTDR_BUSY),
            _ => v,
        };
        self.bar4[(off / 4) as usize] = new;
        if off == dsp::HIPCIDR {
            self.host_ipc(v);
        }
    }

    fn write_hda(&mut self, off: u32, bytes: &[u8]) {
        let o = off as usize;
        // SDxSTS (byte 3 of each descriptor) is write-1-to-clear.
        for (k, &b) in bytes.iter().enumerate() {
            let a = o + k;
            let in_sd = a >= hda::SD_BASE as usize && a < (hda::sd(INPUTS + OUTPUTS)) as usize;
            if in_sd && (a - hda::SD_BASE as usize) % hda::SD_STRIDE as usize == hda::SD_STS as usize {
                self.bar0[a] &= !b;
            } else {
                self.bar0[a] = b;
            }
        }
        if self.faults.stream_reset_stuck && off >= hda::SD_BASE && off < hda::sd(INPUTS + OUTPUTS) {
            let rel = (off - hda::SD_BASE) % hda::SD_STRIDE;
            if rel == hda::SD_CTL {
                self.bar0[o] &= !(hda::SD_CTL_SRST as u8);
            }
        }
    }
}

/// The FW_READY the model posts: `sof_ipc_fw_ready` header and a pattern.
pub fn fw_ready_bytes() -> Vec<u8> {
    let mut v: Vec<u8> = (0..FW_READY_LEN).map(|i| i as u8 ^ 0x5a).collect();
    v[0..4].copy_from_slice(&(FW_READY_LEN as u32).to_le_bytes());
    v[4..8].copy_from_slice(&0x7000_0000u32.to_le_bytes());
    v
}

impl Hw for Model {
    fn hda_read8(&mut self, offset: u32) -> u8 { self.bar0[offset as usize] }
    fn hda_read16(&mut self, offset: u32) -> u16 {
        let o = offset as usize;
        u16::from_le_bytes([self.bar0[o], self.bar0[o + 1]])
    }
    fn hda_read32(&mut self, offset: u32) -> u32 { self.h32(offset) }
    fn hda_write8(&mut self, offset: u32, value: u8) {
        self.log.push(W::H8(offset, value));
        self.write_hda(offset, &[value]);
        self.tick();
    }
    fn hda_write16(&mut self, offset: u32, value: u16) {
        self.log.push(W::H16(offset, value));
        self.write_hda(offset, &value.to_le_bytes());
        self.tick();
    }
    fn hda_write32(&mut self, offset: u32, value: u32) {
        self.log.push(W::H32(offset, value));
        self.write_hda(offset, &value.to_le_bytes());
        self.tick();
    }
    fn dsp_read32(&mut self, offset: u32) -> u32 { self.d32(offset) }
    fn dsp_write32(&mut self, offset: u32, value: u32) {
        self.log.push(W::D32(offset, value));
        self.write_dsp(offset, value);
        self.tick();
    }
    fn delay_us(&mut self, us: u32) {
        self.elapsed_us += us as u64;
        self.tick();
    }
    fn dma(&mut self) -> (&mut [u8], u64) { (&mut self.scratch, BUS) }
    fn dma_sync(&mut self) { self.syncs += 1; }
}
