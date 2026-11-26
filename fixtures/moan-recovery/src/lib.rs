// SPDX-License-Identifier: GPL-3.0-only
#![no_std]
//! Benign offline effect/recovery fixture; no cross-domain memory probes.
pub const FRAMES: usize = 1025;
pub fn input(index: usize) -> [f32; 2] {
    [0.125, if index % 2 == 0 { 0.5 } else { -0.25 }]
}
pub fn expected(index: usize, vst: bool) -> [f32; 2] {
    if vst {
        [0.03125, if index % 2 == 0 { 0.125 } else { -0.0625 }]
    } else {
        [input(index)[1], input(index)[0]]
    }
}
pub fn verifies(pcm: &[[f32; 2]], vst: bool) -> bool {
    pcm.len() == FRAMES && pcm.iter().enumerate().all(|(i, v)| *v == expected(i, vst))
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    #[test]
    fn rejects_wrong_duration_identity_bypass_and_changed_sample() {
        let mut swapped: std::vec::Vec<_> = (0..FRAMES).map(|i| expected(i, false)).collect();
        assert!(verifies(&swapped, false));
        assert!(!verifies(&swapped[..FRAMES - 1], false));
        assert!(!verifies(&swapped, true));
        swapped[FRAMES - 1][1] = 0.0;
        assert!(!verifies(&swapped, false));
        let bypass: std::vec::Vec<_> = (0..FRAMES).map(input).collect();
        assert!(!verifies(&bypass, false));
    }
}
