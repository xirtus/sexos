//! One ordered track identity across audio, MIDI, headers and the mixer.
//! Project order supplies the identity; selection brightens it without recoloring.
pub const ACCENTS: [u32; crate::MAX_TRACKS] = [
    0x7da6ff, // blue
    0x55d1e3, // cyan
    0x73d69a, // green
    0xd5d76b, // yellow
    0xf1b46b, // orange
    0xf18a83, // coral
    0xe58ac8, // pink
    0xb49aee, // purple
];
pub const fn accent(track: usize) -> u32 {
    ACCENTS[track % ACCENTS.len()]
}
fn mix(a: u32, b: u32, n: u32, d: u32) -> u32 {
    let channel = |shift: u32| -> u32 {
        (((a >> shift) & 255u32) * (d - n) + ((b >> shift) & 255u32) * n) / d
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
pub fn region(track: usize) -> u32 {
    mix(0, accent(track), 9, 20)
}
pub fn ink(track: usize) -> u32 {
    mix(accent(track), 0xffffff, 1, 5)
}
pub fn selection(rgb: u32) -> u32 {
    mix(rgb, 0xffffff, 1, 12)
}
pub fn label(track: usize, background: weft::Color) -> weft::Color {
    let color = weft::Color::hex(accent(track));
    if color.contrast(background) >= 4.5 {
        color
    } else {
        weft::Color::hex(mix(0, accent(track), 2, 5))
    }
}
