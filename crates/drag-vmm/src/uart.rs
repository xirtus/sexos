// SPDX-License-Identifier: GPL-3.0-only
//! A 16550A UART (COM1, 0x3F8, IRQ 4): the guest's serial console.
//!
//! Transmit is instant: every byte the guest writes to THR goes to the console at once,
//! so LSR always reports THR empty. Bytes the VMM queues with `receive` are read from
//! RBR. Interrupts: THR-empty and received-data, as IER enables them; `irq` says
//! whether the line is raised. The scratch, modem and FIFO registers keep what the
//! guest writes, as Linux's 8250 probe expects (it identifies a 16550A by them).
use alloc::collections::VecDeque;

pub const COM1: u16 = 0x3F8;
pub const IRQ: u8 = 4;
/// Most received bytes held for the guest.
const RX_MAX: usize = 1024;

const IER_RX: u8 = 1;
const IER_THRE: u8 = 2;
const LCR_DLAB: u8 = 0x80;
const MCR_LOOP: u8 = 0x10;
const LSR_DR: u8 = 1;
const LSR_THRE: u8 = 0x20;
const LSR_TEMT: u8 = 0x40;

pub struct Uart {
    ier: u8, lcr: u8, mcr: u8, scr: u8, fcr: u8, dll: u8, dlm: u8,
    rx: VecDeque<u8>,
    /// A THR-empty interrupt is owed (set on a THR write or IER enabling it, cleared
    /// when the guest reads IIR reporting it).
    thre_pending: bool,
}

impl Default for Uart { fn default() -> Self { Self::new() } }

impl Uart {
    pub fn new() -> Uart {
        Uart { ier: 0, lcr: 3, mcr: 0, scr: 0, fcr: 0, dll: 1, dlm: 0, rx: VecDeque::new(), thre_pending: false }
    }

    /// Queues bytes for the guest to read (keyboard input, later). Drops what does not fit.
    pub fn receive(&mut self, bytes: &[u8]) {
        for &b in bytes { if self.rx.len() < RX_MAX { self.rx.push_back(b); } }
    }

    /// The interrupt line (IRQ 4) is raised.
    pub fn irq(&self) -> bool { self.iir() & 1 == 0 && self.mcr & 0x08 != 0 }

    fn iir(&self) -> u8 {
        let fifo = if self.fcr & 1 != 0 { 0xC0 } else { 0 };
        if self.ier & IER_RX != 0 && !self.rx.is_empty() { return fifo | 0x04; }
        if self.ier & IER_THRE != 0 && self.thre_pending { return fifo | 0x02; }
        fifo | 0x01
    }

    /// A guest read of `offset` (0..=7).
    pub fn read(&mut self, offset: u16) -> u8 {
        match offset & 7 {
            0 if self.lcr & LCR_DLAB != 0 => self.dll,
            0 => self.rx.pop_front().unwrap_or(0),
            1 if self.lcr & LCR_DLAB != 0 => self.dlm,
            1 => self.ier,
            2 => {
                let iir = self.iir();
                if iir & 0x0F == 0x02 { self.thre_pending = false; }
                iir
            }
            3 => self.lcr,
            4 => self.mcr,
            5 => LSR_THRE | LSR_TEMT | if self.rx.is_empty() { 0 } else { LSR_DR },
            6 => {
                if self.mcr & MCR_LOOP != 0 {
                    // Loopback: RTS→CTS, DTR→DSR, OUT1→RI, OUT2→DCD (the 8250 probe checks it).
                    let m = self.mcr;
                    ((m & 2) << 3) | ((m & 1) << 5) | ((m & 4) << 4) | ((m & 8) << 4)
                } else {
                    0xB0 // DCD, DSR, CTS: a cable is there
                }
            }
            _ => self.scr,
        }
    }

    /// A guest write of `value` to `offset`. Returns the byte transmitted, if any.
    pub fn write(&mut self, offset: u16, value: u8) -> Option<u8> {
        match offset & 7 {
            0 if self.lcr & LCR_DLAB != 0 => self.dll = value,
            0 => {
                self.thre_pending = true;
                if self.mcr & MCR_LOOP != 0 {
                    self.receive(&[value]);
                    return None;
                }
                return Some(value);
            }
            1 if self.lcr & LCR_DLAB != 0 => self.dlm = value,
            1 => {
                if value & IER_THRE != 0 && self.ier & IER_THRE == 0 { self.thre_pending = true; }
                self.ier = value & 0x0F;
            }
            2 => {
                if value & 2 != 0 { self.rx.clear(); }
                self.fcr = value & 0xC9;
            }
            3 => self.lcr = value,
            4 => self.mcr = value & 0x1F,
            5 | 6 => {}
            _ => self.scr = value,
        }
        None
    }
}

/// Collects transmitted bytes into lines for the VMM's log. Long lines are cut.
pub struct LineBuffer { line: [u8; LINE_MAX], len: usize }
pub const LINE_MAX: usize = 200;

impl Default for LineBuffer { fn default() -> Self { LineBuffer { line: [0; LINE_MAX], len: 0 } } }

impl LineBuffer {
    /// Adds a byte; returns a finished line (without the newline). Control bytes other
    /// than newline are dropped, and non-ASCII shows as '?', so a guest can not forge
    /// the VMM's own log lines across a line break or with escape sequences.
    pub fn push(&mut self, b: u8) -> Option<&[u8]> {
        match b {
            b'\n' => { let n = core::mem::take(&mut self.len); Some(&self.line[..n]) }
            b'\r' | 0..=0x1F | 0x7F => None,
            _ => {
                if self.len == LINE_MAX { return None; } // cut until the newline
                self.line[self.len] = if b < 0x80 { b } else { b'?' };
                self.len += 1;
                None
            }
        }
    }
    /// What is waiting without a newline.
    pub fn pending(&self) -> &[u8] { &self.line[..self.len] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn transmits_and_reports_thr_empty() {
        let mut u = Uart::new();
        assert_eq!(u.write(0, b'A'), Some(b'A'));
        assert_eq!(u.read(5) & 0x60, 0x60);
        // Divisor latch does not transmit.
        u.write(3, 0x80);
        assert_eq!(u.write(0, 1), None);
        assert_eq!(u.read(0), 1);
        u.write(3, 0x03);
        assert_eq!(u.read(3), 3);
    }

    #[test]
    fn receive_and_interrupts() {
        let mut u = Uart::new();
        u.write(4, 0x08); // OUT2: interrupts reach the PIC
        assert!(!u.irq());
        u.write(1, IER_THRE);
        assert!(u.irq(), "enabling THRE interrupts raises one");
        assert_eq!(u.read(2) & 0x0F, 0x02);
        assert!(!u.irq(), "reading IIR clears it");
        u.write(1, IER_RX);
        u.receive(b"hi");
        assert!(u.irq());
        assert_eq!(u.read(5) & 1, 1);
        assert_eq!((u.read(0), u.read(0), u.read(0)), (b'h', b'i', 0));
        assert!(!u.irq());
        u.receive(&[0; 5000]);
        assert_eq!(u.rx.len(), RX_MAX, "bounded");
    }

    #[test]
    fn loopback_and_scratch_for_the_8250_probe() {
        let mut u = Uart::new();
        u.write(7, 0x5A);
        assert_eq!(u.read(7), 0x5A);
        u.write(4, MCR_LOOP | 0x0A); // loop, RTS, OUT2
        assert_eq!(u.read(6) & 0xF0, 0x90, "CTS and DCD");
        assert_eq!(u.write(0, b'x'), None);
        assert_eq!(u.read(0), b'x');
    }

    #[test]
    fn lines_are_clean() {
        let mut l = LineBuffer::default();
        let mut out = Vec::new();
        for &b in b"ok\r\n\x1b[2Jesc\xffaped\n" {
            if let Some(line) = l.push(b) { out.push(line.to_vec()); }
        }
        assert_eq!(out, [b"ok".to_vec(), b"[2Jesc?aped".to_vec()]);
        for _ in 0..LINE_MAX + 5 { assert!(l.push(b'z').is_none()); }
        assert_eq!(l.pending().len(), LINE_MAX);
        assert_eq!(l.push(b'\n').map(|s| s.len()), Some(LINE_MAX));
    }
}
