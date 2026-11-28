// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Intel's audio DSP (cAVS 2.5, Tiger Lake) running Sound Open Firmware (SOF).
//!
//! On Tiger Lake laptops such as the Alienware x17 R1 the built-in microphones are
//! digital microphones (DMIC) wired to the DSP, not to the HDA codec. The PCI
//! function (`8086:43c8` TGL-H, `8086:a0c8` TGL-LP) is one controller with two
//! faces: the HD Audio registers in BAR0 (which Bell already drives for the codec)
//! and the DSP in BAR4. This crate is the hardware-independent core that Bell uses
//! to reach the microphones:
//!
//! - [`regs`]: the BAR0 extended capabilities (processing pipe, multi-link) and the
//!   BAR4 DSP registers (core power, IPC doorbells, SRAM windows).
//! - [`hw`]: the narrow trait Bell implements over its leased views and DMA pool.
//! - [`fw`]: checking a signed SOF image (`sof-tgl-h.ri`) before it is loaded.
//! - [`boot`]: powering core 0, the ROM handshake and the code-loader DMA stream.
//! - [`ipc3`]: SOF IPC version 3 messages (headers, FW_READY, replies), as bytes.
//! - [`topology`]: the fixed DMIC capture graph (host PCM <- buffer <- DMIC DAI),
//!   sent as IPC3 messages instead of parsing a topology file.
//!
//! The crate is `no_std`, never allocates, and is tested on the host against a
//! register model and the byte layouts of SOF's own ABI headers.
//! Studied: Linux `sound/soc/sof/intel/` and SOF's `include/ipc/` (BSD-3-Clause).
#![no_std]

pub mod boot;
pub mod fw;
pub mod hw;
pub mod ipc3;
pub mod regs;
pub mod topology;
