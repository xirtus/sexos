// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Intel's integrated Thunderbolt 4 / USB4 host interface (NHI, class 0c0340, Ice
//! Lake and later) around a D-state change. Its link controller takes commands
//! through vendor words in config space (offsets from Linux `icl_nhi_*`, read as a
//! reference):
//!
//! * going down, GO2SX tells the link controller the host is entering sleep with
//!   wake enabled, so plugging a dock or display can wake the machine;
//! * coming up, force power goes back on and the connection-manager firmware boots
//!   again until it reports FW_READY. The driver then says DRIVER_READY anew.

use crate::{Bdf, Config};

pub const CLASS: u32 = 0x0c_03_40;
/// FW_READY in bit 31.
pub const VS_CAP_9: u16 = 0xc8;
pub const FW_READY: u32 = 1 << 31;
/// Link controller mailbox: DONE (bit 0) in VS_CAP_18, command (7:1) and VALID (bit 0) in VS_CAP_19.
pub const VS_CAP_18: u16 = 0xf4;
pub const DONE: u32 = 1;
pub const VS_CAP_19: u16 = 0xf8;
pub const VALID: u32 = 1;
/// Force power (bit 1) and the DMA delay (31:24).
pub const VS_CAP_22: u16 = 0xfc;
pub const FORCE_POWER: u32 = 1 << 1;
pub const DMA_DELAY: u32 = 0x22 << 24;
/// Enter sleep, wake on plug.
pub const GO2SX: u32 = 0x02;
/// How long the link controller and the firmware get, in milliseconds.
pub const MAILBOX_MS: u32 = 500;
pub const FW_READY_MS: u32 = 500;

/// Sends GO2SX and waits for DONE; Some(milliseconds taken), None on a timeout.
pub fn go2sx<C: Config + ?Sized>(c: &mut C, f: Bdf) -> Option<u32> {
    c.write(f, VS_CAP_19, (GO2SX << 1 & 0xfe) | VALID);
    for ms in 0..MAILBOX_MS {
        let r = c.read(f, VS_CAP_18);
        if r != u32::MAX && r & DONE != 0 { c.write(f, VS_CAP_19, 0); return Some(ms); }
        c.delay_us(1000);
    }
    c.write(f, VS_CAP_19, 0);
    None
}

/// Raises force power and waits for FW_READY; Some(milliseconds taken), None on a timeout.
pub fn power_up<C: Config + ?Sized>(c: &mut C, f: Bdf) -> Option<u32> {
    let word = c.read(f, VS_CAP_22);
    c.write(f, VS_CAP_22, (word & 0x00ff_ffff) | DMA_DELAY | FORCE_POWER);
    for ms in 0..FW_READY_MS {
        let r = c.read(f, VS_CAP_9);
        if r != u32::MAX && r & FW_READY != 0 { return Some(ms); }
        c.delay_us(1000);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The link controller finishes a command `after` reads; FW_READY likewise.
    struct Lc { words: [u32; 64], after: u32, reads: u32, delayed: u64, gone: bool }
    impl Config for Lc {
        fn read(&mut self, _: Bdf, at: u16) -> u32 {
            if self.gone { return u32::MAX; }
            if at == VS_CAP_18 || at == VS_CAP_9 {
                self.reads += 1;
                if self.reads > self.after {
                    if at == VS_CAP_18 && self.words[(VS_CAP_19 / 4) as usize] & VALID != 0 { return DONE; }
                    if at == VS_CAP_9 && self.words[(VS_CAP_22 / 4) as usize] & FORCE_POWER != 0 { return FW_READY; }
                }
                return 0;
            }
            self.words[(at / 4) as usize]
        }
        fn write(&mut self, _: Bdf, at: u16, v: u32) { self.words[(at / 4) as usize] = v; }
        fn delay_us(&mut self, us: u32) { self.delayed += us as u64; }
    }
    fn lc(after: u32) -> Lc { Lc { words: [0; 64], after, reads: 0, delayed: 0, gone: false } }
    const F: Bdf = Bdf { bus: 0, dev: 0x0d, func: 2 };

    #[test]
    fn go2sx_posts_the_command_and_clears_valid() {
        let mut c = lc(3);
        assert_eq!(go2sx(&mut c, F), Some(3));
        assert_eq!(c.words[(VS_CAP_19 / 4) as usize], 0);
        assert_eq!(c.delayed, 3000);
    }

    #[test]
    fn go2sx_gives_up_after_the_mailbox_timeout() {
        let mut c = lc(u32::MAX);
        assert_eq!(go2sx(&mut c, F), None);
        assert_eq!(c.delayed, MAILBOX_MS as u64 * 1000);
        assert_eq!(c.words[(VS_CAP_19 / 4) as usize], 0);
        let mut gone = lc(0);
        gone.gone = true;
        assert_eq!(go2sx(&mut gone, F), None, "all ones is not DONE");
    }

    #[test]
    fn power_up_keeps_the_low_bits_and_waits_for_fw_ready() {
        let mut c = lc(5);
        c.words[(VS_CAP_22 / 4) as usize] = 0xab00_1234;
        assert_eq!(power_up(&mut c, F), Some(5));
        assert_eq!(c.words[(VS_CAP_22 / 4) as usize], 0x2200_1236);
    }
}
