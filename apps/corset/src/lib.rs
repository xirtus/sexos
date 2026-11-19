// SPDX-License-Identifier: GPL-3.0-only
//! Corset's plan of changes and the order it writes them in, on top of corset-core. The
//! program (`main.rs`) is the keyboard UI around it; this part also builds on a host so
//! its tests run there.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod plan;
