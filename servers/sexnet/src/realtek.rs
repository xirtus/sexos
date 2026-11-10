// SPDX-License-Identifier: GPL-3.0-only
//! Realtek r8169 (RTL8111/8168/8411, gigabit) and r8125 (RTL8125/8126, 2.5/5 GbE)
//! registers and descriptors. nic::Device and ethernet::Driver run their usual
//! bounded phases and call these for the Realtek register map, so the custody
//! rules (no app address, one BOOT DMA pool, quarantine on doubt) are shared.
//!
//! Byte and word registers are accessed at their own width: a 32-bit
//! read-modify-write would also write neighbours such as the write-one-to-clear
//! interrupt status.
//!
//! QEMU has no 8169 model; this path is proven only against host doubles. What
//! bare-metal testing must check (not done by this code):
//! - PHY power: firmware may leave the PHY powered down (BMCR.PDOWN). Linux
//!   powers it up through PHYAR (0x60, r8169) or OCP (r8125); without that,
//!   Link waits for PHYstatus.LinkSts until the preparation deadline.
//! - Per-revision hw_start magic (ePHY/ERI/OCP writes, ASPM/CLKREQ, RTL8125
//!   MAC MCU patches) is not written. Most 8111E-H and 8125B parts pass packets
//!   without it, older and newer revisions may not.
//! - RxConfig: r8169 uses the parent spec's no-threshold/unlimited-burst value;
//!   r8125 uses Linux's RX_FETCH_DFLT_8125 | RX_DMA_BURST form. The RTL8126 may
//!   also want RX_PAUSE_SLOT_ON.
//! - TxConfig's hardware-version bits read back nonzero; only IFG and the DMA
//!   burst field are verified.
//! - The chip's 256-byte descriptor-ring alignment is met by the pool's page
//!   offsets 0 and 4096.
use sex_pdx::pci_command as c;
use crate::nic::Hardware;

/// Every register this layer touches lies below this offset.
pub const REGISTERS_END: u64 = 0x100;
const IDR0: u32 = 0x00;
const MAR0: u32 = 0x08;
pub const TX_DESC_LOW: u32 = 0x20;
pub const TX_DESC_HIGH: u32 = 0x24;
pub const CHIP_CMD: u32 = 0x37;
pub const CMD_RESET: u8 = 1 << 4;
pub const CMD_RX: u8 = 1 << 3;
pub const CMD_TX: u8 = 1 << 2;
/// r8169 normal-priority transmit poll (byte); r8125 keeps IntrMask here.
pub const TX_POLL_8168: u32 = 0x38;
const NORMAL_PRIORITY: u8 = 1 << 6;
pub const INTR_MASK_8168: u32 = 0x3c;
pub const INTR_STATUS_8168: u32 = 0x3e;
pub const INTR_MASK_8125: u32 = 0x38;
pub const INTR_STATUS_8125: u32 = 0x3c;
pub const TX_CONFIG: u32 = 0x40;
pub const RX_CONFIG: u32 = 0x44;
pub const CFG9346: u32 = 0x50;
const CFG_UNLOCK: u8 = 0xc0;
const CFG_LOCK: u8 = 0x00;
pub const PHY_STATUS_8168: u32 = 0x6c;
pub const PHY_STATUS_8125: u32 = 0x80;
const LINK_STATUS: u8 = 1 << 1;
/// r8125 transmit poll (word, bit 0).
pub const TX_POLL_8125: u32 = 0x90;
pub const RX_MAX_SIZE: u32 = 0xda;
pub const CPLUS_CMD: u32 = 0xe0;
/// C+ VLAN de-tagging and receive checksum offload stay off.
const CPLUS_OFFLOAD: u16 = (1 << 6) | (1 << 5);
pub const RX_DESC_LOW: u32 = 0xe4;
pub const RX_DESC_HIGH: u32 = 0xe8;
/// Standard inter-frame gap and unlimited transmit DMA burst.
pub const TX_CONFIG_VALUE: u32 = 0x0300_0700;
/// Accept broadcast, multicast (the hash stays empty) and our own address.
const ACCEPT: u32 = 0x0e;
const ACCEPT_MASK: u32 = 0x3f;
const RX_DMA_BURST: u32 = 0x700;

/// Descriptor opts1 bits shared by transmit and receive.
pub const OWN: u32 = 1 << 31;
pub const EOR: u32 = 1 << 30;
pub const FIRST: u32 = 1 << 29;
pub const LAST: u32 = 1 << 28;
pub const LENGTH: u32 = 0x3fff;
/// Receive error summary (CRC, runt, watchdog, FIFO overflow).
pub const RX_ERROR: u32 = 1 << 21;
/// Received lengths include the frame check sequence.
pub const CRC: usize = 4;

fn r8125(h: &impl Hardware) -> bool { h.model() == c::Model::Rtl8125 }

pub fn rx_config(model: c::Model) -> u32 {
    // r8169: no receive FIFO threshold (bits 13-15). r8125: RX_FETCH_DFLT_8125.
    if model == c::Model::Rtl8125 { (8 << 27) | RX_DMA_BURST | ACCEPT } else { 0xe000 | RX_DMA_BURST | ACCEPT }
}

/// Mask and acknowledge every interrupt source; the service polls.
pub fn mask(h: &mut impl Hardware) {
    if r8125(h) { h.write(INTR_MASK_8125, 0); h.write(INTR_STATUS_8125, u32::MAX); }
    else { h.write16(INTR_MASK_8168, 0); h.write16(INTR_STATUS_8168, u16::MAX); }
}
fn interrupt_mask(h: &mut impl Hardware) -> u32 {
    if r8125(h) { h.read(INTR_MASK_8125) } else { h.read16(INTR_MASK_8168) as u32 }
}

/// Stop accepting frames, stop both engines and mask interrupts. The ChipCmd
/// readback flushes posted writes; it is not a DMA-drain proof.
pub fn disable(h: &mut impl Hardware) -> u8 {
    mask(h);
    let rx = h.read(RX_CONFIG);
    if rx != u32::MAX { h.write(RX_CONFIG, rx & !ACCEPT_MASK); }
    h.write8(CHIP_CMD, 0);
    h.read8(CHIP_CMD)
}

/// Present, both engines stopped and every interrupt masked.
pub fn idle(h: &mut impl Hardware) -> bool {
    let command = h.read8(CHIP_CMD);
    command != u8::MAX && command & (CMD_RX | CMD_TX) == 0 && interrupt_mask(h) == 0
}

pub fn station(h: &mut impl Hardware) -> [u8; 6] {
    let mut mac = [0; 6];
    for (i, byte) in mac.iter_mut().enumerate() { *byte = h.read8(IDR0 + i as u32); }
    mac
}

pub fn phy_status(h: &mut impl Hardware) -> u8 {
    h.read8(if r8125(h) { PHY_STATUS_8125 } else { PHY_STATUS_8168 })
}

/// None when the device no longer answers.
pub fn link(h: &mut impl Hardware) -> Option<bool> {
    let status = phy_status(h);
    if status == u8::MAX { None } else { Some(status & LINK_STATUS != 0) }
}

/// Program the rings and the receive size limit. No engine is enabled and bus
/// mastering is still off; `configured` checks every value before that.
pub fn configure(h: &mut impl Hardware, rx_ring: u64, tx_ring: u64, buffer: u16) -> bool {
    h.write8(CFG9346, CFG_UNLOCK);
    let cplus = h.read16(CPLUS_CMD);
    if cplus == u16::MAX { h.write8(CFG9346, CFG_LOCK); return false; }
    h.write16(CPLUS_CMD, cplus & !CPLUS_OFFLOAD);
    h.write16(RX_MAX_SIZE, buffer);
    // High halves first: the low write is what the chip latches (as Linux does).
    h.write(TX_DESC_HIGH, (tx_ring >> 32) as u32);
    h.write(TX_DESC_LOW, tx_ring as u32);
    h.write(RX_DESC_HIGH, (rx_ring >> 32) as u32);
    h.write(RX_DESC_LOW, rx_ring as u32);
    h.write8(CFG9346, CFG_LOCK);
    h.write(MAR0, 0);
    h.write(MAR0 + 4, 0);
    mask(h);
    true
}

pub fn configured(h: &mut impl Hardware, rx_ring: u64, tx_ring: u64, buffer: u16) -> bool {
    h.read(TX_DESC_LOW) == tx_ring as u32 && h.read(TX_DESC_HIGH) == (tx_ring >> 32) as u32
        && h.read(RX_DESC_LOW) == rx_ring as u32 && h.read(RX_DESC_HIGH) == (rx_ring >> 32) as u32
        && h.read16(RX_MAX_SIZE) == buffer && h.read16(CPLUS_CMD) & CPLUS_OFFLOAD == 0 && idle(h)
}

/// Enable both engines (bus mastering is already on), then the frame
/// configuration, which the r8169 only latches with the engines running.
pub fn start(h: &mut impl Hardware) -> bool {
    h.write8(CHIP_CMD, CMD_TX | CMD_RX);
    h.write(TX_CONFIG, TX_CONFIG_VALUE);
    let rx = rx_config(h.model());
    h.write(RX_CONFIG, rx);
    let command = h.read8(CHIP_CMD);
    command != u8::MAX && command & (CMD_TX | CMD_RX) == CMD_TX | CMD_RX
        && h.read(TX_CONFIG) & TX_CONFIG_VALUE == TX_CONFIG_VALUE
        && h.read(RX_CONFIG) & (RX_DMA_BURST | ACCEPT_MASK) == RX_DMA_BURST | ACCEPT
}

/// Tell the chip the normal-priority ring has a new descriptor.
pub fn kick(h: &mut impl Hardware) {
    if r8125(h) { h.write16(TX_POLL_8125, 1); } else { h.write8(TX_POLL_8168, NORMAL_PRIORITY); }
}

fn end(index: u32, count: u32) -> u32 { if index + 1 == count { EOR } else { 0 } }
/// opts1|opts2 of a receive descriptor handed to the chip (opts2 stays zero).
pub fn rx_ready(index: u32, count: u32, buffer: u32) -> u64 { (OWN | end(index, count) | (buffer & LENGTH)) as u64 }
/// opts1|opts2 of a transmit descriptor the driver holds.
pub fn tx_idle(index: u32, count: u32) -> u64 { end(index, count) as u64 }
/// One whole frame in one buffer.
pub fn tx_submit(index: u32, count: u32, length: usize) -> u64 {
    (OWN | end(index, count) | FIRST | LAST | (length as u32 & LENGTH)) as u64
}
