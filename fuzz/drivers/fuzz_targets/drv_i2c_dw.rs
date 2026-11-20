// SPDX-License-Identifier: GPL-3.0-only
//! DesignWare I2C controllers (Intel LPSS Serial IO: the x17's touchpad bus):
//! status, FIFO levels, abort sources and received bytes all come from the input,
//! through bring-up and a run of reads and writes the touchpad driver makes.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_i2c_dw::{timing, Controller, Counts, Regs, Speed, COMP_TYPE, IC_COMP_TYPE};
use sexos_driver_fuzz::FuzzRegs;

struct Dev<'a>(FuzzRegs<'a>, bool);
impl Regs for Dev<'_> {
    fn read(&mut self, offset: u32) -> u32 {
        // Mostly a real DesignWare block (so bring-up passes); sometimes not.
        if offset == IC_COMP_TYPE && self.1 { return COMP_TYPE; }
        self.0.read32()
    }
    fn write(&mut self, _offset: u32, _value: u32) { self.0.write32() }
}

fn run(d: &[u8]) {
    // Gone, not hung, past the input: every wait here has a budget, and a hung
    // block costs POLL_BUDGET (2M) reads per transfer, past the hang budget.
    let mut regs = Dev(FuzzRegs::with_tail(d, 0xffff_ffff), true);
    let flags = regs.0.src.u8();
    regs.1 = flags & 0x80 == 0;
    let speed = if flags & 1 != 0 { Speed::Fast } else { Speed::Standard };
    let hz = regs.0.src.u32();
    let counts = if flags & 2 != 0 { timing::counts(hz, speed, regs.0.src.u16() as u32) } else { Counts { high: regs.0.src.u16(), low: regs.0.src.u16(), sda_hold: regs.0.src.u16() } };
    let Ok(mut c) = Controller::new(regs, speed, counts, flags & 4 != 0) else { return };
    let _ = c.fifo_depths();
    for i in 0..16u16 {
        let mut buf = [0u8; 64];
        let n = (i as usize * 7) % 64;
        let _ = match i % 3 {
            0 => c.transfer(0x2c, &[0x20, 0x00], &mut buf[..n]),
            1 => c.write(0x15, &buf[..n]),
            _ => c.read(0x2c | (i & 0x300), &mut buf[..n]),
        };
    }
}

fn seeds() -> Vec<Vec<u8>> {
    // Healthy-ish: parameter word with 64-deep FIFOs, then "TX empty, RX full" status words.
    let mut v = vec![0x00];
    v.extend_from_slice(&400_000u32.to_le_bytes());
    v.extend_from_slice(&[0; 6]);
    v.extend_from_slice(&0x0101_0000u32.to_le_bytes());
    v.extend_from_slice(&0x003f_3f00u32.to_le_bytes());
    for _ in 0..256 { v.extend_from_slice(&0x0000_000eu32.to_le_bytes()); v.extend_from_slice(&0x0000_0040u32.to_le_bytes()); }
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
