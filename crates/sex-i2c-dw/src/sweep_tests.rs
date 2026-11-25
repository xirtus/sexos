// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Sweep Test 2: Intel Tiger Lake LPSS + DesignWare I2C controller simulator sweep.
use crate::sim::{Device, Sim};
use crate::*;

struct HidTarget {
    hid_reg: u16,
    hid_desc: [u8; 30],
    report_desc: Vec<u8>,
    last_reg: u16,
    offset: usize,
}

impl HidTarget {
    fn new(hid_reg: u16, rdesc_len: usize) -> Self {
        let mut hid_desc = [0u8; 30];
        let words: [u16; 13] = [
            30, 0x0100, rdesc_len as u16, 0x0002, 0x0003, 0x0040,
            0x0004, 0x0010, 0x0005, 0x0006, 0x06CB, 0xCE26, 0x0001,
        ];
        for (i, w) in words.iter().enumerate() {
            hid_desc[i * 2..i * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
        let report_desc = (0..rdesc_len).map(|i| (i & 0xFF) as u8).collect();
        Self { hid_reg, hid_desc, report_desc, last_reg: 0, offset: 0 }
    }
}

impl Device for HidTarget {
    fn write(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() >= 2 {
            self.last_reg = u16::from_le_bytes([bytes[0], bytes[1]]);
            self.offset = 0;
        }
        true
    }
    fn read(&mut self, out: &mut [u8]) {
        let src: &[u8] = if self.last_reg == self.hid_reg {
            &self.hid_desc
        } else if self.last_reg == 0x0002 {
            &self.report_desc
        } else {
            &[]
        };
        for b in out {
            *b = src.get(self.offset).copied().unwrap_or(0);
            self.offset += 1;
        }
    }
}

#[test]
fn lpss_reset_gating_requires_release_before_designware_access() {
    let mut sim = Sim::new(0x2c, HidTarget::new(0x0020, 64));
    sim.require_lpss_reset = true;
    // Without intel_lpss=true, block stays in reset and reads 0xFFFF_FFFF.
    assert_eq!(
        Controller::new(sim, Speed::Fast, timing::counts(timing::INTEL_TGL_HZ, Speed::Fast, 300), false).err(),
        Some(Error::NotDesignWare(u32::MAX))
    );

    // With intel_lpss=true, LPSS_RESETS is released and DesignWare init succeeds.
    let mut sim = Sim::new(0x2c, HidTarget::new(0x0020, 64));
    sim.require_lpss_reset = true;
    let mut c = Controller::new(sim, Speed::Fast, timing::counts(timing::INTEL_TGL_HZ, Speed::Fast, 300), true).unwrap();
    let mut desc = [0u8; 30];
    c.transfer(0x2c, &0x0020u16.to_le_bytes(), &mut desc).unwrap();
    assert_eq!(u16::from_le_bytes([desc[0], desc[1]]), 30);
    assert_eq!(u16::from_le_bytes([desc[20], desc[21]]), 0x06CB);
    assert!(c.into_regs().lpss_released);
}

#[test]
fn zero_firmware_counts_fall_back_to_safe_tgl_timing() {
    let sim = Sim::new(0x2c, HidTarget::new(0x0020, 32));
    let c = Controller::new(sim, Speed::Fast, Counts { high: 0, low: 0, sda_hold: 0 }, true).unwrap();
    let sim = c.into_regs();
    assert!(sim.reg(IC_FS_SCL_HCNT) > 0);
    assert!(sim.reg(IC_FS_SCL_LCNT) > 0);
    assert!(sim.reg(IC_SDA_HOLD) > 0);
}

#[test]
fn stale_rx_fifo_bytes_are_drained_before_hid_descriptor_read() {
    let mut sim = Sim::new(0x2c, HidTarget::new(0x0020, 32));
    sim.seed_stale_rx(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let mut c = Controller::new(sim, Speed::Fast, timing::counts(timing::INTEL_TGL_HZ, Speed::Fast, 300), true).unwrap();
    let mut desc = [0u8; 30];
    c.transfer(0x2c, &0x0020u16.to_le_bytes(), &mut desc).unwrap();
    assert_eq!(u16::from_le_bytes([desc[0], desc[1]]), 30);
    assert_eq!(u16::from_le_bytes([desc[2], desc[3]]), 0x0100);
}

#[test]
fn clock_stretching_and_large_report_descriptor_chunking_across_16_byte_fifo() {
    let mut sim = Sim::new(0x2c, HidTarget::new(0x0020, 680));
    sim.require_lpss_reset = true;
    sim.rx_delay_polls = 3;
    sim.rx_depth = 16;
    let mut c = Controller::new(sim, Speed::Fast, timing::counts(timing::INTEL_TGL_HZ, Speed::Fast, 300), true).unwrap();
    let mut rdesc = [0u8; 680];
    c.transfer(0x2c, &0x0002u16.to_le_bytes(), &mut rdesc).unwrap();
    for (i, &b) in rdesc.iter().enumerate() {
        assert_eq!(b, (i & 0xFF) as u8, "mismatch at byte {i}");
    }
    assert!(c.into_regs().max_rx_level <= 16);
}

#[test]
fn multi_address_bus_scan_and_arbitration_lost_recovery() {
    let mut sim = Sim::new(0x2c, HidTarget::new(0x0020, 64));
    sim.require_lpss_reset = true;
    sim.arb_lost_next = true;
    let mut c = Controller::new(sim, Speed::Fast, timing::counts(timing::INTEL_TGL_HZ, Speed::Fast, 300), true).unwrap();
    let mut desc = [0u8; 30];
    // First attempt hits ArbitrationLost and recovers cleanly.
    assert_eq!(c.transfer(0x15, &0x0020u16.to_le_bytes(), &mut desc), Err(Error::ArbitrationLost));
    // Sweep non-existent addresses: each returns AddressNack without wedging the controller.
    let mut found = None;
    for addr in [0x10u16, 0x15, 0x20, 0x38, 0x2c] {
        match c.transfer(addr, &0x0020u16.to_le_bytes(), &mut desc) {
            Ok(()) if u16::from_le_bytes([desc[0], desc[1]]) == 30 && u16::from_le_bytes([desc[2], desc[3]]) == 0x0100 => {
                found = Some(addr);
                break;
            }
            Ok(()) => {}
            Err(Error::AddressNack) => {}
            Err(other) => panic!("unexpected bus error at {addr:#x}: {other:?}"),
        }
    }
    assert_eq!(found, Some(0x2c));
}
