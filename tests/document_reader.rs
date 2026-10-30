// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure reader, shared text metrics and BOOT profile data. No Display,
//! pixels, input routing or network authority is established by this wrapper.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/network_config.rs"]pub mod network_config;
#[path="../apps/kaleidoscope/src/document_reader.rs"]mod document_reader;
