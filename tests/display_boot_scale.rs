// Host test of the SDK's EDID size reader and HiDPI scale choice.
#![allow(dead_code)]
pub const ERR_CAP_INVALID: u64 = 0xFFFF_FFFF_FFFF_FFFC;
pub fn pdx_call_checked(_: u32, _: u64, _: u64, _: u64, _: u64) -> Result<u64, u64> {
    Err(ERR_CAP_INVALID)
}
#[path = "../crates/sex-pdx/src/display_boot.rs"]
mod display_boot;
