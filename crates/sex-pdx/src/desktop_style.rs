//! Shared native text metrics and focus ink; integer pixels only.
pub const INSET: u32 = 8;
pub const TEXT_TOP: u32 = 12;
pub const ADVANCE: u32 = 6;
pub const LINE: u32 = 9;
pub const TEXT: u32 = 0x00E0F0FF;
pub const FOCUS: u32 = 0x00E9D36A;
pub const fn scale(width: u32) -> u32 {
    if width >= 256 {
        2
    } else {
        1
    }
}
