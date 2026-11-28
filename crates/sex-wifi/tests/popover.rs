// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for SilkBar's live Wi-Fi scanner popover (`sex_pdx::wifi_popover`).
//! Mirrors `status.rs`: runs the `#![no_std]` popover module's own unit tests on the host.
#[path = "../../sex-pdx/src/wifi_popover.rs"]
#[allow(dead_code)]
mod wifi_popover;
