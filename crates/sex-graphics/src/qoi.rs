//! Allocation-free, cooperative QOI 1.0 decoder for small owned assets.
//!
//! Independently implemented from the CC0 specification, version 2022-01-05:
//! <https://qoiformat.org/qoi-specification.pdf>. Pixels are unpremultiplied
//! `0xAARRGGBB`. Channels and colorspace are validated but are informative.
//!
//! The caller keeps the input bytes immutable and the output private across
//! turns. Only `Ready` permits publishing the output; transport authorization
//! and positive RELEASE are separate caller obligations. This state contains
//! no input pointer or image allocation. Every turn checks input length/header;
//! this is not a substitute for the caller's immutable ownership of its body.

pub const MAX_DIMENSION: usize = 48;
pub const MAX_PIXELS: usize = MAX_DIMENSION * MAX_DIMENSION;
pub const MAX_INPUT_BYTES: usize = 12_288;
pub const PIXELS_PER_TURN: usize = 128;
const HEADER_BYTES: usize = 14;
const END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];
fn erase(pixels: &mut [u32]) {
    for pixel in pixels { unsafe { core::ptr::write_volatile(pixel, 0); } }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Pending,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InputTooLarge,
    Truncated,
    InvalidHeader,
    InputChanged,
    RunOverflow,
    ConsecutiveIndex,
    InvalidEnd,
    TrailingData,
    Cancelled,
}

/// Bounded decoder state; the 2,304-pixel output belongs to the caller.
pub struct Decoder {
    header: [u8; HEADER_BYTES],
    input_len: usize,
    width: usize,
    height: usize,
    cursor: usize,
    produced: usize,
    pixel: u32,
    index: [u32; 64],
    run_remaining: usize,
    last_index: Option<u8>,
    ready: bool,
    error: Option<Error>,
    started: bool,
}

impl Decoder {
    pub fn new(input: &[u8]) -> Result<Self, Error> {
        if input.len() > MAX_INPUT_BYTES {
            return Err(Error::InputTooLarge);
        }
        if input.len() < HEADER_BYTES + END.len() {
            return Err(Error::Truncated);
        }
        let width = u32::from_be_bytes([input[4], input[5], input[6], input[7]]) as usize;
        let height = u32::from_be_bytes([input[8], input[9], input[10], input[11]]) as usize;
        if &input[..4] != b"qoif"
            || width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || !matches!(input[12], 3 | 4)
            || input[13] > 1
        {
            return Err(Error::InvalidHeader);
        }
        let mut header = [0; HEADER_BYTES];
        header.copy_from_slice(&input[..HEADER_BYTES]);
        Ok(Self {
            header,
            input_len: input.len(),
            width,
            height,
            cursor: HEADER_BYTES,
            produced: 0,
            pixel: 0xff00_0000,
            index: [0; 64],
            run_remaining: 0,
            last_index: None,
            ready: false,
            error: None,
            started: false,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn produced(&self) -> usize {
        self.produced
    }

    /// Decode at most 128 pixels, including pixels expanded from RUN chunks.
    /// Any error erases the entire output and permanently fails this decoder.
    pub fn advance(&mut self, input: &[u8], out: &mut [u32; MAX_PIXELS]) -> Result<State, Error> {
        if let Some(error) = self.error {
            erase(out);
            return Err(error);
        }
        if input.len() != self.input_len || input.get(..HEADER_BYTES) != Some(&self.header[..]) {
            return self.fail(out, Error::InputChanged);
        }
        if self.ready {
            return Ok(State::Ready);
        }
        if !self.started {
            out.fill(0);
            self.started = true;
        }
        let total = self.width * self.height;
        let data_end = input.len() - END.len();
        let mut turn_pixels = 0;
        while self.produced < total && turn_pixels < PIXELS_PER_TURN {
            if self.run_remaining != 0 {
                self.run_remaining -= 1;
            } else {
                if self.cursor >= data_end {
                    return self.fail(out, Error::Truncated);
                }
                let tag = input[self.cursor];
                self.cursor += 1;
                let previous_index = self.last_index.take();
                let (r, g, b, a) = channels(self.pixel);
                match tag {
                    0xfe | 0xff => {
                        let needed = if tag == 0xfe { 3 } else { 4 };
                        if data_end - self.cursor < needed {
                            return self.fail(out, Error::Truncated);
                        }
                        self.pixel = argb(
                            input[self.cursor],
                            input[self.cursor + 1],
                            input[self.cursor + 2],
                            if needed == 4 {
                                input[self.cursor + 3]
                            } else {
                                a
                            },
                        );
                        self.cursor += needed;
                    }
                    _ => match tag >> 6 {
                        0 => {
                            let slot = tag & 63;
                            if previous_index == Some(slot) {
                                return self.fail(out, Error::ConsecutiveIndex);
                            }
                            self.pixel = self.index[slot as usize];
                            self.last_index = Some(slot);
                        }
                        1 => {
                            self.pixel = argb(
                                add(r, ((tag >> 4) & 3) as i16 - 2),
                                add(g, ((tag >> 2) & 3) as i16 - 2),
                                add(b, (tag & 3) as i16 - 2),
                                a,
                            );
                        }
                        2 => {
                            if self.cursor >= data_end {
                                return self.fail(out, Error::Truncated);
                            }
                            let second = input[self.cursor];
                            self.cursor += 1;
                            let dg = (tag & 63) as i16 - 32;
                            self.pixel = argb(
                                add(r, dg + (second >> 4) as i16 - 8),
                                add(g, dg),
                                add(b, dg + (second & 15) as i16 - 8),
                                a,
                            );
                        }
                        3 => {
                            let run = (tag & 63) as usize + 1;
                            if run > total - self.produced {
                                return self.fail(out, Error::RunOverflow);
                            }
                            // This iteration emits the first pixel of the run.
                            self.run_remaining = run - 1;
                        }
                        _ => unreachable!(),
                    },
                }
            }
            out[self.produced] = self.pixel;
            let (r, g, b, a) = channels(self.pixel);
            let hash = (r as usize * 3 + g as usize * 5 + b as usize * 7 + a as usize * 11) % 64;
            self.index[hash] = self.pixel;
            self.produced += 1;
            turn_pixels += 1;
        }
        if self.produced == total {
            if self.cursor != data_end {
                return self.fail(out, Error::TrailingData);
            }
            if input[data_end..] != END {
                return self.fail(out, Error::InvalidEnd);
            }
            self.ready = true;
            Ok(State::Ready)
        } else {
            Ok(State::Pending)
        }
    }

    /// Terminal cancellation: no partially decoded or previously ready pixels survive.
    pub fn cancel(&mut self, out: &mut [u32; MAX_PIXELS]) {
        let _ = self.fail(out, Error::Cancelled);
    }

    fn fail(&mut self, out: &mut [u32; MAX_PIXELS], error: Error) -> Result<State, Error> {
        erase(out);
        erase(&mut self.index);
        unsafe { core::ptr::write_volatile(&mut self.pixel, 0); }
        self.run_remaining = 0;
        self.last_index = None;
        self.produced = 0;
        self.ready = false;
        self.error = Some(error);
        Err(error)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        erase(&mut self.index);
        unsafe { core::ptr::write_volatile(&mut self.pixel, 0); }
    }
}

fn add(value: u8, delta: i16) -> u8 {
    value.wrapping_add(delta as u8)
}
fn argb(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}
fn channels(pixel: u32) -> (u8, u8, u8, u8) {
    (
        (pixel >> 16) as u8,
        (pixel >> 8) as u8,
        pixel as u8,
        (pixel >> 24) as u8,
    )
}

#[cfg(test)]
mod tests {
    extern crate std;
    use self::std::{vec, vec::Vec};
    use super::*;

    fn image(width: u32, height: u32, chunks: &[u8]) -> Vec<u8> {
        let mut input = b"qoif".to_vec();
        input.extend_from_slice(&width.to_be_bytes());
        input.extend_from_slice(&height.to_be_bytes());
        input.extend_from_slice(&[4, 0]);
        input.extend_from_slice(chunks);
        input.extend_from_slice(&END);
        input
    }
    fn decode(input: &[u8]) -> (Decoder, [u32; MAX_PIXELS]) {
        let mut decoder = Decoder::new(input).unwrap();
        let mut out = [0xdead_beef; MAX_PIXELS];
        let mut turns = 0;
        loop {
            let before = decoder.produced();
            let state = decoder.advance(input, &mut out).unwrap();
            assert!(decoder.produced() - before <= PIXELS_PER_TURN);
            turns += 1;
            assert!(turns <= MAX_PIXELS / PIXELS_PER_TURN + 1);
            if state == State::Ready {
                break;
            }
        }
        assert!(out[decoder.produced()..].iter().all(|&pixel| pixel == 0));
        (decoder, out)
    }
    fn fails(input: &[u8], expected: Error) {
        let mut decoder = Decoder::new(input).unwrap();
        let mut out = [0xdead_beef; MAX_PIXELS];
        loop {
            match decoder.advance(input, &mut out) {
                Ok(State::Pending) => (),
                other => {
                    assert_eq!(other, Err(expected));
                    break;
                }
            }
        }
        assert!(out.iter().all(|&pixel| pixel == 0));
        assert_eq!(decoder.produced(), 0);
        out[0] = 1;
        assert_eq!(decoder.advance(input, &mut out), Err(expected));
        assert!(out.iter().all(|&pixel| pixel == 0));
    }

    #[test]
    fn rgb_and_rgba_precede_run_tags_and_preserve_unpremultiplied_alpha() {
        let (_, out) = decode(&image(
            3,
            1,
            &[0xff, 200, 100, 50, 7, 0xfe, 1, 2, 3, 0xff, 9, 8, 7, 0],
        ));
        assert_eq!(&out[..3], &[0x07c8_6432, 0x0701_0203, 0x0009_0807]);
    }

    #[test]
    fn initial_pixel_is_opaque_black_but_index_starts_transparent() {
        let (_, out) = decode(&image(3, 1, &[0xc0, 0x00, 0x35]));
        // Black alpha255 hashes to53; transparent black hashes to0.
        assert_eq!(&out[..3], &[0xff00_0000, 0x0000_0000, 0xff00_0000]);
    }

    #[test]
    fn index_hash_covers_all_channels_and_collisions_replace() {
        // (1*3+2*5+3*7+4*11)%64=14; r+64 preserves slot14.
        let (_, out) = decode(&image(4, 1, &[0xff, 1, 2, 3, 4, 0xfe, 65, 2, 3, 14, 0xc0]));
        assert_eq!(
            &out[..4],
            &[0x0401_0203, 0x0441_0203, 0x0441_0203, 0x0441_0203]
        );
    }

    #[test]
    fn diff_wraps_each_channel_without_changing_alpha() {
        let (_, out) = decode(&image(3, 1, &[0xff, 0, 255, 1, 9, 0x4c, 0x7f]));
        // DIFF(-2,+1,-2), then DIFF(+1,+1,+1).
        assert_eq!(&out[..3], &[0x0900_ff01, 0x09fe_00ff, 0x09ff_0100]);
    }

    #[test]
    fn luma_wraps_and_uses_green_relative_biases() {
        let (_, out) = decode(&image(
            3,
            1,
            &[0xff, 10, 250, 1, 99, 0x80, 0x0f, 0xbf, 0xf0],
        ));
        // dg=-32, dr=-40, db=-25; then dg=31, dr=38, db=23.
        assert_eq!(&out[..3], &[0x630a_fa01, 0x63e2_dae8, 0x6308_f9ff]);
    }

    #[test]
    fn all_diff_values_match_hand_derived_bias_ranges() {
        for tag in 0x40..=0x7f {
            let (_, out) = decode(&image(1, 1, &[tag]));
            let r = (((tag >> 4) & 3) as i16 - 2) as u8;
            let g = (((tag >> 2) & 3) as i16 - 2) as u8;
            let b = ((tag & 3) as i16 - 2) as u8;
            assert_eq!(
                out[0],
                0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
            );
        }
    }

    #[test]
    fn channels_and_colorspace_are_informative() {
        for channels in [3, 4] {
            for colorspace in [0, 1] {
                let mut input = image(1, 1, &[0xff, 1, 2, 3, 4]);
                input[12] = channels;
                input[13] = colorspace;
                assert_eq!(decode(&input).1[0], 0x0401_0203);
            }
        }
    }

    #[test]
    fn maximum_runs_are_split_by_pixel_budget_not_chunk_budget() {
        // 48*48=2304; 37 runs of62 plus one run10.
        let mut chunks = vec![0xfd; 37];
        chunks.push(0xc9);
        let input = image(48, 48, &chunks);
        let mut decoder = Decoder::new(&input).unwrap();
        let mut out = [0xdead_beef; MAX_PIXELS];
        for turn in 1..=18 {
            let state = decoder.advance(&input, &mut out).unwrap();
            assert_eq!(decoder.produced(), turn * 128);
            assert_eq!(
                state,
                if turn == 18 {
                    State::Ready
                } else {
                    State::Pending
                }
            );
            assert!(out[..decoder.produced()].iter().all(|&p| p == 0xff00_0000));
            assert!(out[decoder.produced()..].iter().all(|&p| p == 0));
        }
        assert_eq!((decoder.width(), decoder.height()), (48, 48));
        assert_eq!(decoder.advance(&input, &mut out), Ok(State::Ready));
    }

    #[test]
    fn final_turn_below_budget_and_row_order() {
        let input = image(3, 2, &[0xfe, 1, 2, 3, 0xc2, 0xfe, 4, 5, 6, 0xc0]);
        let (decoder, out) = decode(&input);
        assert_eq!(decoder.produced(), 6);
        assert_eq!(
            &out[..6],
            &[
                0xff01_0203,
                0xff01_0203,
                0xff01_0203,
                0xff01_0203,
                0xff04_0506,
                0xff04_0506
            ]
        );
    }

    #[test]
    fn invalid_magic_dimensions_and_metadata_are_rejected() {
        for (offset, value) in [
            (0, b'x'),
            (7, 0),
            (7, 49),
            (11, 0),
            (11, 49),
            (12, 0),
            (12, 2),
            (12, 5),
            (13, 2),
        ] {
            let mut input = image(1, 1, &[0xc0]);
            input[offset] = value;
            assert!(matches!(Decoder::new(&input), Err(Error::InvalidHeader)));
        }
        for (width, height) in [(u32::MAX, 1), (1, u32::MAX), (256, 1), (1, 256)] {
            assert!(matches!(
                Decoder::new(&image(width, height, &[0xc0])),
                Err(Error::InvalidHeader)
            ));
        }
    }

    #[test]
    fn short_header_marker_and_oversize_are_rejected_at_construction() {
        for len in 0..22 {
            assert!(matches!(Decoder::new(&vec![0; len]), Err(Error::Truncated)));
        }
        assert!(matches!(
            Decoder::new(&vec![0; MAX_INPUT_BYTES + 1]),
            Err(Error::InputTooLarge)
        ));
    }

    #[test]
    fn input_size_limit_is_inclusive_then_exact_eof_is_enforced() {
        // Maximum supported all-RGBA image occupies11542bytes; padding cannot
        // turn an otherwise valid image into a valid12288byte image.
        let mut chunks = Vec::new();
        for _ in 0..MAX_PIXELS {
            chunks.extend_from_slice(&[0xff, 1, 2, 3, 4]);
        }
        let input = image(48, 48, &chunks);
        assert_eq!(input.len(), 11_542);
        assert_eq!(decode(&input).1[MAX_PIXELS - 1], 0x0401_0203);
        let mut padded = input;
        padded.resize(MAX_INPUT_BYTES, 0);
        assert!(Decoder::new(&padded).is_ok());
        fails(&padded, Error::TrailingData);
    }

    #[test]
    fn every_multibyte_chunk_truncation_fails_without_reading_marker() {
        for chunk in [
            &[0xfe][..],
            &[0xfe, 1],
            &[0xfe, 1, 2],
            &[0xff],
            &[0xff, 1],
            &[0xff, 1, 2],
            &[0xff, 1, 2, 3],
            &[0x80],
        ] {
            fails(&image(1, 1, chunk), Error::Truncated);
        }
        fails(&image(1, 1, &[]), Error::Truncated);
    }

    #[test]
    fn run_overflow_rejected_before_writing_outside_declared_image() {
        fails(&image(1, 1, &[0xc1]), Error::RunOverflow);
        fails(&image(48, 1, &[0xfd]), Error::RunOverflow);
        fails(&image(2, 1, &[0xc0, 0xc1]), Error::RunOverflow);
    }

    #[test]
    fn consecutive_same_index_is_invalid_but_run_or_other_chunk_separates() {
        fails(&image(2, 1, &[0x00, 0x00]), Error::ConsecutiveIndex);
        assert_eq!(&decode(&image(3, 1, &[0, 0xc0, 0])).1[..3], &[0, 0, 0]);
        assert_eq!(&decode(&image(3, 1, &[0, 1, 0])).1[..3], &[0, 0, 0]);
    }

    #[test]
    fn invalid_marker_or_any_trailing_bytes_erase_all_pixels() {
        for offset in 0..8 {
            let mut input = image(1, 1, &[0xc0]);
            let index = input.len() - 8 + offset;
            input[index] ^= 1;
            fails(&input, Error::InvalidEnd);
        }
        let mut input = image(1, 1, &[0xc0]);
        input.push(0);
        fails(&input, Error::TrailingData);
        let input = image(1, 1, &[0xc0, 0xc0]);
        fails(&input, Error::TrailingData);
    }

    #[test]
    fn errors_after_a_successful_pending_turn_erase_earlier_pixels() {
        let mut chunks = vec![0xc0; 128];
        chunks.push(0xfe); // Pixel129 truncates; dimensions129=43*3.
        let input = image(43, 3, &chunks);
        let mut decoder = Decoder::new(&input).unwrap();
        let mut out = [0; MAX_PIXELS];
        assert_eq!(decoder.advance(&input, &mut out), Ok(State::Pending));
        assert_eq!(decoder.produced(), 128);
        assert_ne!(out[127], 0);
        assert_eq!(decoder.advance(&input, &mut out), Err(Error::Truncated));
        assert!(out.iter().all(|&p| p == 0));
    }

    #[test]
    fn metadata_length_changes_fail_even_after_ready() {
        let input = image(1, 1, &[0xc0]);
        for offset in 0..HEADER_BYTES {
            let (mut decoder, mut out) = decode(&input);
            let mut changed = input.clone();
            changed[offset] ^= 1;
            assert_eq!(
                decoder.advance(&changed, &mut out),
                Err(Error::InputChanged)
            );
            assert!(out.iter().all(|&p| p == 0));
        }
        let (mut decoder, mut out) = decode(&input);
        assert_eq!(
            decoder.advance(&input[..input.len() - 1], &mut out),
            Err(Error::InputChanged)
        );
        assert!(out.iter().all(|&p| p == 0));
    }

    #[test]
    fn changed_metadata_during_pending_erases_previous_turn() {
        let input = image(16, 16, &[0xfd, 0xfd, 0xfd, 0xfd, 0xc7]);
        for offset in 0..HEADER_BYTES {
            let mut decoder = Decoder::new(&input).unwrap();
            let mut out = [0; MAX_PIXELS];
            assert_eq!(decoder.advance(&input, &mut out), Ok(State::Pending));
            assert_eq!(decoder.produced(), 128);
            let mut changed = input.clone();
            changed[offset] ^= 1;
            assert_eq!(
                decoder.advance(&changed, &mut out),
                Err(Error::InputChanged)
            );
            assert_eq!(decoder.produced(), 0);
            assert!(out.iter().all(|&p| p == 0));
        }
        let mut decoder = Decoder::new(&input).unwrap();
        let mut out = [1; MAX_PIXELS];
        assert_eq!(
            decoder.advance(&input[..13], &mut out),
            Err(Error::InputChanged)
        );
        assert!(out.iter().all(|&p| p == 0));
    }

    #[test]
    fn cancel_before_pending_and_after_ready_is_terminal_and_zeroes() {
        for pending_turns in [0, 1, 2] {
            let input = image(16, 16, &[0xfd, 0xfd, 0xfd, 0xfd, 0xc7]);
            let mut decoder = Decoder::new(&input).unwrap();
            let mut out = [0xdead_beef; MAX_PIXELS];
            for _ in 0..pending_turns {
                decoder.advance(&input, &mut out).unwrap();
            }
            decoder.cancel(&mut out);
            assert_eq!(decoder.produced(), 0);
            assert!(out.iter().all(|&p| p == 0));
            assert_eq!(decoder.advance(&input, &mut out), Err(Error::Cancelled));
        }
    }

    #[test]
    fn bounded_adversarial_inputs_never_panic_or_overrun_turn_budget() {
        let mut random = 0x1234_5678_u32;
        for case in 0..4096 {
            let mut bytes = vec![0; case % 192];
            for byte in &mut bytes {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                *byte = random as u8;
            }
            let input = if case % 2 == 0 {
                image((case % 48 + 1) as u32, (case % 17 + 1) as u32, &bytes)
            } else {
                bytes
            };
            if let Ok(mut decoder) = Decoder::new(&input) {
                let mut out = [0xdead_beef; MAX_PIXELS];
                for _ in 0..=MAX_PIXELS / PIXELS_PER_TURN {
                    let before = decoder.produced();
                    match decoder.advance(&input, &mut out) {
                        Ok(state) => {
                            assert!(decoder.produced() >= before);
                            assert!(decoder.produced() - before <= PIXELS_PER_TURN);
                            assert!(decoder.produced() <= decoder.width() * decoder.height());
                            if state == State::Ready {
                                break;
                            }
                        }
                        Err(_) => {
                            assert!(out.iter().all(|&p| p == 0));
                            break;
                        }
                    }
                }
            }
        }
    }
}
