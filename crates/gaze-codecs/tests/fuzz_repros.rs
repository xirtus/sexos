// SPDX-License-Identifier: GPL-3.0-only
//! Inputs found by fuzzing that once panicked.

/// Fuzz #259: a 12-byte input with `ftyp` at 4..8 made the ISO-BMFF sniffer clamp(16, 12).
#[test]
fn short_ftyp_does_not_panic() {
    let data: &[u8] = &[71, 73, 70, 56, 102, 116, 121, 112, 56, 119, 96, 130];
    let _ = gaze_codecs::sniff(data);
    for n in 0..data.len() {
        let _ = gaze_codecs::sniff(&data[..n]);
    }
}
