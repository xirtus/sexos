#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod browser_bridge;
pub mod cli;
pub mod json;
pub mod obs_ws;
pub mod script_rpc;
pub mod streamdeck;
