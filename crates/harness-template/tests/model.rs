// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A model of QEMU's edu device, close to `hw/misc/edu.c`: the factorial and DMA
//! finish a few register reads after they start, interrupts go out on an
//! [`IrqLine`], and DMA reads and writes host memory through a [`BusPort`].
//! Knobs turn it into a broken device: dead (all ones), stuck (never finishes),
//! wrong identity, or a liveness register that does not invert.
#![allow(dead_code)]

use edu::*;
use harness::mock::{BusPort, DeviceModel, IrqLine};

#[derive(Default)]
pub struct Faults { pub dead: bool, pub stuck: bool, pub bad_id: bool, pub no_liveness: bool }

pub struct EduModel {
    pub faults: Faults,
    pub irq: IrqLine,
    pub bus: Option<BusPort>,
    liveness: u32,
    fact: u32,
    status: u32,
    irq_status: u32,
    computing: u32,
    dma_src: u32,
    dma_dst: u32,
    dma_count: u32,
    dma_cmd: u32,
    dma_left: u32,
    pub buffer: [u8; BUFFER_LEN as usize],
    /// A DMA the real device would abort on (hw_error in QEMU).
    pub aborted: bool,
}

/// Register reads a factorial or DMA takes to finish.
pub const LATENCY: u32 = 5;

impl EduModel {
    pub fn new() -> Self {
        EduModel { faults: Faults::default(), irq: IrqLine::new(), bus: None, liveness: 0, fact: 0, status: 0, irq_status: 0,
            computing: 0, dma_src: 0, dma_dst: 0, dma_count: 0, dma_cmd: 0, dma_left: 0, buffer: [0; BUFFER_LEN as usize], aborted: false }
    }
    fn raise(&mut self, value: u32) {
        self.irq_status |= value;
        self.irq.raise(0);
    }
    fn in_buffer(addr: u32, len: u32) -> Option<usize> {
        let off = addr.checked_sub(BUFFER_ADDR)?;
        (off.checked_add(len)? <= BUFFER_LEN).then_some(off as usize)
    }
    fn finish_dma(&mut self) {
        let Some(bus) = self.bus.clone() else { self.aborted = true; return };
        let len = self.dma_count;
        if self.dma_cmd & DMA_TO_RAM != 0 {
            match Self::in_buffer(self.dma_src, len) {
                Some(off) => { if !bus.write(self.dma_dst as u64, &self.buffer[off..off + len as usize]) { self.aborted = true; } }
                None => self.aborted = true,
            }
        } else {
            match Self::in_buffer(self.dma_dst, len) {
                Some(off) => { let mut tmp = vec![0u8; len as usize]; if bus.read(self.dma_src as u64, &mut tmp) { self.buffer[off..off + len as usize].copy_from_slice(&tmp); } else { self.aborted = true; } }
                None => self.aborted = true,
            }
        }
        self.dma_cmd &= !DMA_START;
        if self.dma_cmd & DMA_IRQ != 0 { self.raise(IRQ_DMA); }
    }
}

impl DeviceModel for EduModel {
    fn step(&mut self) {
        if self.faults.stuck { return; }
        if self.computing > 0 {
            self.computing -= 1;
            if self.computing == 0 {
                self.fact = (1..=self.fact).fold(1u32, |a, b| a.wrapping_mul(b));
                self.status &= !STATUS_COMPUTING;
                if self.status & STATUS_IRQ_ON_FACTORIAL != 0 { self.raise(IRQ_FACTORIAL); }
            }
        }
        if self.dma_left > 0 {
            self.dma_left -= 1;
            if self.dma_left == 0 { self.finish_dma(); }
        }
    }
    fn read32(&mut self, offset: u32) -> u32 {
        if self.faults.dead { return 0xffff_ffff; }
        match offset {
            ID => if self.faults.bad_id { 0x0100_00ee } else { 0x0100_00ed },
            LIVENESS => if self.faults.no_liveness { self.liveness } else { !self.liveness },
            FACTORIAL => self.fact,
            STATUS => self.status,
            IRQ_STATUS => self.irq_status,
            DMA_SRC => self.dma_src,
            DMA_DST => self.dma_dst,
            DMA_COUNT => self.dma_count,
            DMA_CMD => self.dma_cmd,
            _ => 0xffff_ffff,
        }
    }
    fn write32(&mut self, offset: u32, value: u32) {
        if self.faults.dead { return; }
        match offset {
            LIVENESS => self.liveness = value,
            FACTORIAL if self.status & STATUS_COMPUTING == 0 => {
                self.fact = value;
                self.status |= STATUS_COMPUTING;
                self.computing = LATENCY;
            }
            STATUS => self.status = (self.status & STATUS_COMPUTING) | (value & STATUS_IRQ_ON_FACTORIAL),
            IRQ_RAISE => self.raise(value),
            IRQ_ACK => self.irq_status &= !value,
            DMA_SRC if self.dma_cmd & DMA_START == 0 => self.dma_src = value,
            DMA_DST if self.dma_cmd & DMA_START == 0 => self.dma_dst = value,
            DMA_COUNT if self.dma_cmd & DMA_START == 0 => self.dma_count = value,
            DMA_CMD if self.dma_cmd & DMA_START == 0 => {
                self.dma_cmd = value;
                if value & DMA_START != 0 { self.dma_left = LATENCY; }
            }
            _ => {}
        }
    }
}
