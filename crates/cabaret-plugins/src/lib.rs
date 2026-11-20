//! Native, deterministic, 100% safe-Rust (`#![forbid(unsafe_code)]`, `no_std + alloc`)
//! ports of the 12 essential OBS Studio plugins plus the `sex-wasm` video FX bridge for Cabaret.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
extern crate alloc;

mod util;

pub mod adv_switcher;
pub mod downstream_keyer;
pub mod local_captions;
pub mod move_transition;
pub mod multi_canvas;
pub mod overlays;
pub mod shader_filter;
pub mod source_record;
pub mod spectralizer;
pub mod streamfx_3d;
pub mod studio_link;
pub mod time_warp;
pub mod wasm_fx;
