// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Per-core I/O lanes for SexOS drivers.
//!
//! A *lane* is one core's private path between a client and a driver worker: a
//! submission ring and a completion ring in memory both domains see (one address
//! space, so a descriptor carries a plain address and nothing is copied), a device
//! queue of its own (an NVMe I/O queue pair, a NIC RX/TX pair) and that queue's
//! interrupt aimed at the same core. No lock is shared between cores; a core that is
//! busy polls its lane, an idle one sleeps on its queue's interrupt.
//!
//! - [`ring`]: single-producer single-consumer rings over shared memory, with a
//!   wake-up flag so a busy peer never makes a system call to signal.
//! - [`poll`]: when a worker switches between interrupts and polling.
//! - [`nvme`]: the NVMe commands and doorbells for one queue pair per core.
//! - [`rss`]: Toeplitz receive-side scaling, so each flow lands on one core.
//! - [`plan`]: which core gets which queue and vector when there are fewer of
//!   either than cores.
//!
//! Everything here is plain logic, tested on the host; drivers own the hardware.
#![no_std]

pub mod nvme;
pub mod plan;
pub mod poll;
pub mod ring;
pub mod rss;
