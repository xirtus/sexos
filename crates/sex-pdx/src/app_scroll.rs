// SPDX-License-Identifier: GPL-3.0-only
//! Bounded vertical wheel data. Decoding authenticates no sender: Input,
//! Shell and apps separately check their actual BOOT-sanctioned IPC peer.
//! Positive steps move toward the document start. Zero is not an event.
pub const CLASS: u64 = 5;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll { vertical: i8 }
impl Scroll {
    pub const fn new(vertical: i8) -> Option<Self> {
        if vertical == 0 { None } else { Some(Self { vertical }) }
    }
    pub const fn vertical(self) -> i8 { self.vertical }
    pub const fn wire(self) -> u64 { self.vertical as i64 as u64 }
    pub fn decode(word: u64, reserved: u64, class: u64) -> Option<Self> {
        if reserved != 0 || class != CLASS { return None; }
        let event = Self::new(word as i8)?;
        (event.wire() == word).then_some(event)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn every_nonzero_step_round_trips_with_one_canonical_word() {
        for vertical in i8::MIN..=i8::MAX {
            let Some(event) = Scroll::new(vertical) else { assert_eq!(vertical,0); continue; };
            assert_eq!(Scroll::decode(event.wire(),0,CLASS),Some(event));
            assert_eq!(event.vertical(),vertical);
            for bit in 8..64 { assert_eq!(Scroll::decode(event.wire() ^ (1u64 << bit),0,CLASS),None); }
        }
    }
    #[test] fn rejects_aliases_zero_reserved_data_and_other_classes() {
        for word in [0,128,255,256,65535,u32::MAX as u64] { assert_eq!(Scroll::decode(word,0,CLASS),None); }
        for reserved in [1,2,u64::MAX] { assert_eq!(Scroll::decode(1,reserved,CLASS),None); }
        for class in [0,1,2,3,4,6,u64::MAX] { assert_eq!(Scroll::decode(1,0,class),None); }
    }
}
