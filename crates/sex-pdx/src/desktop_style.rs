//! Shared native text metrics and focus ink; integer pixels only.
pub const INSET: u32 = 8;
pub const TEXT_TOP: u32 = 12;
pub const ASCII_FIRST: u8 = 32;
pub const ASCII_LAST: u8 = 126;
pub const GLYPH_WIDTH: u32 = 5;
pub const GLYPH_HEIGHT: u32 = 7;
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

/// Visual family classification only. Callers must independently attest live
/// ownership before accepting any surface operation or routing input.
pub const fn native_label(surface:u64)->Option<&'static str>{
    match surface {
        156|201|301|307=>Some("Quil"),
        153|154|160|161|162|302|303|304|305|306=>Some("Spindle"),
        200=>Some("Linen"),300=>Some("Browser"),_=>None,
    }
}
pub const fn native_indicator(surface:u64)->Option<u32>{
    match surface {
        156|201|301|307=>Some(0x00CBA6F7),
        153|154|160|161|162|302|303|304|305|306=>Some(0x0089B4FA),
        200=>Some(0x00A6E3A1),300=>Some(0x005ED9C3),_=>None,
    }
}
pub const NATIVE_CANVAS:u32=0x000C1420;
/// A family ID or hardware key does not establish this exact BOOT ownership.
pub const fn native_canvas(owner:u32,surface:u64)->Option<u32>{
    match(owner,surface){
        (9,156)|(16,301)|(18,307)|(12,154|160|161|162)|(17,302|303|304|305)=>Some(NATIVE_CANVAS),
        _=>None,
    }
}
