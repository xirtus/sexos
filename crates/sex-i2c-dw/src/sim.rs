// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A simulated DesignWare controller with one device on its bus, for tests.
use crate::*;
use std::collections::VecDeque;

pub trait Device {
    /// One write phase; false refuses (NACKs) the data.
    fn write(&mut self, bytes: &[u8]) -> bool;
    fn read(&mut self, out: &mut [u8]);
}

pub struct Sim<D: Device> {
    pub address: u16,
    pub device: D,
    regs: [u32; 0x400 / 4],
    rx: VecDeque<u8>,
    pending_write: Vec<u8>,
    in_transfer: bool,
    abort_source: u32,
    raw: u32,
    nacked: bool,
    pub restarts: u32,
    pub stops: u32,
    pub max_rx_level: usize,
    pub comp_type: u32,
    pub lpss_released: bool,
    pub hang: bool,
    pub rx_depth: usize,
}

impl<D: Device> Sim<D> {
    pub fn new(address: u16, device: D) -> Self {
        Self { address, device, regs: [0; 0x400 / 4], rx: VecDeque::new(), pending_write: Vec::new(), in_transfer: false,
            abort_source: 0, raw: 0, nacked: false, restarts: 0, stops: 0, max_rx_level: 0, comp_type: COMP_TYPE,
            lpss_released: false, hang: false, rx_depth: 16 }
    }
    pub fn reg(&self, offset: u32) -> u32 { self.regs[offset as usize / 4] }
    fn target(&self) -> u16 { (self.reg(IC_TAR) & 0x3ff) as u16 }
    fn flush_write(&mut self) {
        if !self.pending_write.is_empty() {
            let bytes = core::mem::take(&mut self.pending_write);
            if !self.nacked && !self.device.write(&bytes) { self.abort(ABRT_TXDATA_NOACK); }
        }
    }
    fn abort(&mut self, source: u32) { self.nacked = true; self.abort_source |= source; self.raw |= INTR_TX_ABRT; self.rx.clear(); }
    fn command(&mut self, cmd: u32) {
        if !self.in_transfer {
            self.in_transfer = true;
            if self.target() != self.address { self.abort(ABRT_7B_ADDR_NOACK); }
        }
        if cmd & CMD_RESTART != 0 { self.restarts += 1; self.flush_write(); }
        if cmd & CMD_READ != 0 {
            self.flush_write();
            if !self.nacked {
                let mut b = [0u8; 1];
                self.device.read(&mut b);
                assert!(self.rx.len() < self.rx_depth, "receive FIFO overflow");
                self.rx.push_back(b[0]);
                self.max_rx_level = self.max_rx_level.max(self.rx.len());
            }
        } else { self.pending_write.push(cmd as u8); }
        if cmd & CMD_STOP != 0 {
            self.flush_write();
            self.stops += 1;
            self.raw |= INTR_STOP_DET;
            self.in_transfer = false;
            self.nacked = false;
        }
    }
}

impl<D: Device> Regs for Sim<D> {
    fn read(&mut self, offset: u32) -> u32 {
        match offset {
            IC_COMP_TYPE => self.comp_type,
            IC_COMP_PARAM_1 => ((16 - 1) << 16) | ((self.rx_depth as u32 - 1) << 8),
            IC_ENABLE_STATUS => self.reg(IC_ENABLE) & 1,
            IC_STATUS => {
                let mut s = STATUS_TFNF;
                if !self.rx.is_empty() { s |= STATUS_RFNE; }
                if self.hang { s |= STATUS_MST_ACTIVITY; }
                s
            }
            IC_RXFLR => self.rx.len() as u32,
            IC_DATA_CMD => self.rx.pop_front().unwrap_or(0) as u32,
            IC_RAW_INTR_STAT => self.raw,
            IC_TX_ABRT_SOURCE => self.abort_source,
            IC_CLR_TX_ABRT => { self.raw &= !INTR_TX_ABRT; self.abort_source = 0; 0 }
            IC_CLR_STOP_DET => { self.raw &= !INTR_STOP_DET; 0 }
            IC_CLR_INTR => { self.raw = 0; self.abort_source = 0; 0 }
            o => self.reg(o),
        }
    }
    fn write(&mut self, offset: u32, value: u32) {
        match offset {
            IC_DATA_CMD => {
                assert!(self.reg(IC_ENABLE) & 1 == 1, "command while disabled");
                self.command(value)
            }
            IC_TAR | IC_CON => {
                assert!(self.reg(IC_ENABLE) & 1 == 0, "target or mode changed while enabled");
                self.regs[offset as usize / 4] = value;
            }
            LPSS_RESETS => { if value == LPSS_RESETS_RELEASE { self.lpss_released = true; } self.regs[offset as usize / 4] = value; }
            o => self.regs[o as usize / 4] = value,
        }
    }
}
