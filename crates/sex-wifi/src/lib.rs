// SPDX-License-Identifier: GPL-3.0-only
//! Intel Wi-Fi 6/6E for SexOS: AX200, AX201, AX210, AX211 and the Killer
//! modules built on them (AX1650, AX1675, AX1690).
//!
//! This crate is the hardware-independent core of the Wi-Fi driver service:
//! chip identification, firmware file parsing, the DMA layout and context info
//! the chip boots from, the register power-up sequence, host command framing,
//! 802.11 frames, and the WPA2/WPA3 supplicant with CCMP. It is `no_std`, never
//! allocates, and is tested on the host against IEEE and RFC test vectors.
//! The network service (sexnet) owns the leased device and drives it.
//!
//! Register maps, init order and the device tables follow OpenBSD's iwx(4)
//! (ISC licence, Stefan Sperling and others); this is a Rust re-implementation,
//! not a copy, and contains no Linux (GPL-2.0-only) code.
#![no_std]

pub mod bringup;
pub mod ccmp;
pub mod cmdq;
pub mod context;
pub mod crypto;
pub mod csr;
pub mod devices;
pub mod eapol;
pub mod hcmd;
pub mod ieee80211;
pub mod init;
pub mod join;
pub mod tx;
pub mod assoc;
pub mod rsna;
pub mod link;
pub mod pack;
pub mod rx;
pub mod sae;
pub mod scan;
pub mod ucode;
