// SPDX-License-Identifier: GPL-3.0-only
//! The parts of Mesh (the system overview, `apps/monitor`) that need no kernel: the
//! history of each measurement, line graphs and text drawn into a pixel buffer, the
//! tabs and where a click lands, and number formatting. All of it runs and is tested
//! on the host (`cargo test -p sex-mesh`).
#![cfg_attr(not(test), no_std)]

pub mod canvas;
pub mod chart;
pub mod history;
pub mod layout;
pub mod line;
