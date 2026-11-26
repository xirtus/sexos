// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Pure `no_std` RIFF/WAVE PCM header parser, streaming linear-interpolating
//! resampler (8/16-bit, mono/stereo, 8..=192 kHz → 48 kHz stereo `i16`), and
//! `OP_BELL_PCM` 6-frame wire packing.

pub const OUT_RATE: u32 = 48_000;
pub const MIN_RATE: u32 = 8_000;
pub const MAX_RATE: u32 = 192_000;

/// Metadata extracted from a RIFF/WAVE PCM header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub data_offset: u32,
    pub data_size: u32,
}

impl WavInfo {
    /// Bytes per input sample frame (`channels * bits_per_sample / 8`).
    pub const fn block_align(&self) -> usize {
        self.channels as usize * (self.bits_per_sample as usize / 8)
    }

    /// Total duration in milliseconds.
    pub const fn duration_ms(&self) -> u64 {
        let align = self.block_align() as u64;
        if align == 0 || self.sample_rate == 0 { return 0; }
        let frames = self.data_size as u64 / align;
        frames * 1000 / self.sample_rate as u64
    }
}

/// Parses a RIFF/WAVE PCM header from the prefix of a WAV file (typically the
/// first 4096 bytes). Skips auxiliary chunks (`LIST`, `JUNK`, `fact`, etc.)
/// until both `fmt ` and `data` chunks are located.
pub fn parse_wav_header(buf: &[u8]) -> Result<WavInfo, &'static str> {
    if buf.len() < 12 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file");
    }
    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits_per_sample = 0u16;
    let mut have_fmt = false;

    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32::from_le_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]) as usize;
        let payload = pos + 8;

        if id == b"fmt " {
            if size < 16 || payload + 16 > buf.len() {
                return Err("truncated WAV fmt chunk");
            }
            let fmt = u16::from_le_bytes([buf[payload], buf[payload + 1]]);
            let is_pcm = fmt == 1 || (fmt == 0xFFFE && size >= 40 && payload + 26 <= buf.len()
                && u16::from_le_bytes([buf[payload + 24], buf[payload + 25]]) == 1);
            if !is_pcm {
                return Err("unsupported WAV format (need uncompressed PCM)");
            }
            channels = u16::from_le_bytes([buf[payload + 2], buf[payload + 3]]);
            sample_rate = u32::from_le_bytes([buf[payload + 4], buf[payload + 5], buf[payload + 6], buf[payload + 7]]);
            bits_per_sample = u16::from_le_bytes([buf[payload + 14], buf[payload + 15]]);
            if !(1..=2).contains(&channels) {
                return Err("unsupported channel count (need mono or stereo)");
            }
            if bits_per_sample != 8 && bits_per_sample != 16 {
                return Err("unsupported bit depth (need 8-bit or 16-bit PCM)");
            }
            if !(MIN_RATE..=MAX_RATE).contains(&sample_rate) {
                return Err("unsupported sample rate (need 8000..=192000 Hz)");
            }
            have_fmt = true;
        } else if id == b"data" {
            if !have_fmt {
                return Err("WAV data chunk before fmt chunk");
            }
            return Ok(WavInfo {
                sample_rate,
                channels,
                bits_per_sample,
                data_offset: payload as u32,
                data_size: size as u32,
            });
        }

        // RIFF chunks are padded to an even byte boundary.
        let padded = size.saturating_add(size & 1);
        let Some(next) = payload.checked_add(padded) else {
            return Err("WAV chunk size overflow");
        };
        pos = next;
    }

    if !have_fmt {
        Err("missing WAV fmt chunk")
    } else {
        Err("missing WAV data chunk in header prefix")
    }
}

/// Decodes one PCM sample frame (8-bit unsigned or 16-bit signed LE, mono or stereo)
/// into a stereo `i16` pair.
#[inline]
pub fn decode_frame(bytes: &[u8], channels: u16, bits_per_sample: u16) -> [i16; 2] {
    match (channels, bits_per_sample) {
        (1, 8) => {
            let s = ((bytes[0] as i16) - 128) << 8;
            [s, s]
        }
        (2, 8) => {
            let l = ((bytes[0] as i16) - 128) << 8;
            let r = ((bytes[1] as i16) - 128) << 8;
            [l, r]
        }
        (1, 16) => {
            let s = i16::from_le_bytes([bytes[0], bytes[1]]);
            [s, s]
        }
        (2, 16) => {
            let l = i16::from_le_bytes([bytes[0], bytes[1]]);
            let r = i16::from_le_bytes([bytes[2], bytes[3]]);
            [l, r]
        }
        _ => [0, 0],
    }
}

/// Streaming linear-interpolating resampler to 48 kHz stereo `i16`.
/// Holds only ~32 bytes of state and maintains exact phase across arbitrary chunk boundaries.
#[derive(Clone, Copy, Debug)]
pub struct Resampler {
    channels: u16,
    bits_per_sample: u16,
    block_align: usize,
    /// Input frames per 48 kHz output frame in 16.16 fixed point.
    step: u64,
    /// Fractional position in [0, 65536) between `prev` and the next input frame.
    pos: u64,
    prev: [i16; 2],
    has_prev: bool,
    /// Partial input frame carried across unaligned chunk reads (max 4 bytes).
    carry: [u8; 4],
    carry_len: u8,
}

impl Resampler {
    pub const fn new(info: WavInfo) -> Self {
        let step = ((info.sample_rate as u64) << 16) / OUT_RATE as u64;
        Self {
            channels: info.channels,
            bits_per_sample: info.bits_per_sample,
            block_align: info.block_align(),
            step: if step == 0 { 1 } else { step },
            pos: 0,
            prev: [0, 0],
            has_prev: false,
            carry: [0; 4],
            carry_len: 0,
        }
    }

    /// Consumes raw WAV PCM bytes from `raw` and writes resampled 48 kHz stereo `i16`
    /// frames into `out`. Returns `(raw_bytes_consumed, out_frames_written)`.
    /// Call repeatedly until `raw` is exhausted (or `out_frames_written == 0`).
    pub fn resample(&mut self, raw: &[u8], out: &mut [[i16; 2]]) -> (usize, usize) {
        let align = self.block_align;
        if align == 0 || out.is_empty() {
            return (0, 0);
        }
        let mut raw_at = 0usize;
        let mut out_at = 0usize;

        while out_at < out.len() {
            // Pull next input frame (completing any carried partial frame first).
            let curr = if self.carry_len > 0 {
                let need = align - self.carry_len as usize;
                if raw.len() - raw_at < need {
                    let take = raw.len() - raw_at;
                    let c = self.carry_len as usize;
                    self.carry[c..c + take].copy_from_slice(&raw[raw_at..raw_at + take]);
                    self.carry_len += take as u8;
                    raw_at += take;
                    break;
                }
                let c = self.carry_len as usize;
                self.carry[c..align].copy_from_slice(&raw[raw_at..raw_at + need]);
                raw_at += need;
                self.carry_len = 0;
                decode_frame(&self.carry[..align], self.channels, self.bits_per_sample)
            } else if raw.len() - raw_at >= align {
                let frame = decode_frame(&raw[raw_at..raw_at + align], self.channels, self.bits_per_sample);
                raw_at += align;
                frame
            } else {
                let rem = raw.len() - raw_at;
                if rem > 0 {
                    self.carry[..rem].copy_from_slice(&raw[raw_at..]);
                    self.carry_len = rem as u8;
                    raw_at += rem;
                }
                break;
            };

            if !self.has_prev {
                self.prev = curr;
                self.has_prev = true;
                continue;
            }

            // Emit all output frames whose position falls in [0, 1.0) between `prev` and `curr`.
            while self.pos < 65_536 {
                if out_at >= out.len() {
                    // Caller's output buffer is full; rewind `raw_at` by one frame?
                    // Instead, we only enter this loop when out_at < out.len(), and if
                    // we fill `out` mid-interpolation we stash `curr` in `carry`? No:
                    // simpler: check room before consuming `curr`!
                    break;
                }
                let t = self.pos as i64;
                let l = self.prev[0] as i64 + (((curr[0] as i64 - self.prev[0] as i64) * t) >> 16);
                let r = self.prev[1] as i64 + (((curr[1] as i64 - self.prev[1] as i64) * t) >> 16);
                out[out_at] = [l.clamp(-32768, 32767) as i16, r.clamp(-32768, 32767) as i16];
                out_at += 1;
                self.pos += self.step;
            }

            if self.pos >= 65_536 {
                self.pos -= 65_536;
                self.prev = curr;
            } else {
                // `out` filled before `pos` crossed 1.0: put `curr` back as carried bytes
                // so the next call resumes at the exact `pos` with the exact same `curr`.
                let reencoded = match (self.channels, self.bits_per_sample) {
                    (1, 16) => {
                        let b = curr[0].to_le_bytes();
                        self.carry[0] = b[0];
                        self.carry[1] = b[1];
                        2
                    }
                    (2, 16) => {
                        let bl = curr[0].to_le_bytes();
                        let br = curr[1].to_le_bytes();
                        self.carry[0] = bl[0];
                        self.carry[1] = bl[1];
                        self.carry[2] = br[0];
                        self.carry[3] = br[1];
                        4
                    }
                    (1, 8) => {
                        self.carry[0] = ((curr[0] >> 8) + 128) as u8;
                        1
                    }
                    (2, 8) => {
                        self.carry[0] = ((curr[0] >> 8) + 128) as u8;
                        self.carry[1] = ((curr[1] >> 8) + 128) as u8;
                        2
                    }
                    _ => 0,
                };
                self.carry_len = reencoded;
                break;
            }
        }

        (raw_at, out_at)
    }

    /// Emits the final held sample at EOF if any.
    pub fn flush(&mut self, out: &mut [[i16; 2]]) -> usize {
        if self.has_prev && self.pos == 0 && !out.is_empty() {
            out[0] = self.prev;
            self.has_prev = false;
            1
        } else {
            self.has_prev = false;
            0
        }
    }
}

/// Packs 6 stereo `i16` frames into 3 `u64` PDX arguments for `OP_BELL_PCM`.
#[inline]
pub fn pack_pcm6(frames: &[[i16; 2]; 6]) -> [u64; 3] {
    let pack2 = |a: [i16; 2], b: [i16; 2]| -> u64 {
        let f0 = (a[0] as u16 as u32) | ((a[1] as u16 as u32) << 16);
        let f1 = (b[0] as u16 as u32) | ((b[1] as u16 as u32) << 16);
        (f0 as u64) | ((f1 as u64) << 32)
    };
    [
        pack2(frames[0], frames[1]),
        pack2(frames[2], frames[3]),
        pack2(frames[4], frames[5]),
    ]
}

/// Unpacks 3 `u64` PDX arguments from `OP_BELL_PCM` into 6 stereo `i16` frames.
#[inline]
pub fn unpack_pcm6(words: [u64; 3]) -> [[i16; 2]; 6] {
    let unpack = |f: u32| -> [i16; 2] { [f as u16 as i16, (f >> 16) as u16 as i16] };
    [
        unpack(words[0] as u32),
        unpack((words[0] >> 32) as u32),
        unpack(words[1] as u32),
        unpack((words[1] >> 32) as u32),
        unpack(words[2] as u32),
        unpack((words[2] >> 32) as u32),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    fn make_wav(sample_rate: u32, channels: u16, bits: u16, extra_chunk: Option<(&[u8; 4], &[u8])>, pcm: &[u8]) -> Vec<u8> {
        let block_align = channels * (bits / 8);
        let byte_rate = sample_rate * block_align as u32;
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes()); // PCM
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&sample_rate.to_le_bytes());
        body.extend_from_slice(&byte_rate.to_le_bytes());
        body.extend_from_slice(&block_align.to_le_bytes());
        body.extend_from_slice(&bits.to_le_bytes());
        if let Some((tag, data)) = extra_chunk {
            body.extend_from_slice(tag);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() & 1 == 1 {
                body.push(0);
            }
        }
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        body.extend_from_slice(pcm);

        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parse_24khz_mono_16bit_wav() {
        let pcm = [0u8; 48000]; // 1.0 s at 24 kHz mono 16-bit
        let wav = make_wav(24_000, 1, 16, None, &pcm);
        let info = parse_wav_header(&wav).unwrap();
        assert_eq!(info.sample_rate, 24_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.data_offset, 44);
        assert_eq!(info.data_size, 48_000);
        assert_eq!(info.block_align(), 2);
        assert_eq!(info.duration_ms(), 1000);
    }

    #[test]
    fn parse_wav_with_odd_list_chunk_before_data() {
        let wav = make_wav(44_100, 2, 16, Some((b"JUNK", b"abc")), &[1, 2, 3, 4]);
        let info = parse_wav_header(&wav).unwrap();
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.data_offset, 44 + 8 + 4);
        assert_eq!(info.data_size, 4);
    }

    #[test]
    fn parse_wav_rejects_invalid() {
        assert!(parse_wav_header(b"OggS12345678").is_err());
        let mut wav = make_wav(24_000, 1, 16, None, &[0; 4]);
        wav[20] = 3; // IEEE float, not PCM
        assert!(parse_wav_header(&wav).is_err());
        let bad_rate = make_wav(4_000, 1, 16, None, &[0; 4]);
        assert!(parse_wav_header(&bad_rate).is_err());
        let bad_bits = make_wav(48_000, 1, 24, None, &[0; 6]);
        assert!(parse_wav_header(&bad_bits).is_err());
    }

    #[test]
    fn resample_24khz_mono_to_48khz_stereo_linear_interpolation() {
        let samples: [i16; 4] = [0, 1000, 2000, -2000];
        let mut pcm = Vec::new();
        for s in samples {
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        let info = WavInfo { sample_rate: 24_000, channels: 1, bits_per_sample: 16, data_offset: 44, data_size: pcm.len() as u32 };
        let mut r = Resampler::new(info);
        let mut out = [[0i16; 2]; 16];
        let (used, mut written) = r.resample(&pcm, &mut out);
        assert_eq!(used, pcm.len());
        written += r.flush(&mut out[written..]);
        // 0 -> 500 -> 1000 -> 1500 -> 2000 -> 0 -> -2000
        let expected: [[i16; 2]; 7] = [
            [0, 0], [500, 500], [1000, 1000], [1500, 1500], [2000, 2000], [0, 0], [-2000, -2000],
        ];
        assert_eq!(&out[..written], &expected);
    }

    #[test]
    fn resample_chunk_boundary_matches_oneshot() {
        let mut pcm = Vec::new();
        for i in 0..64i16 {
            pcm.extend_from_slice(&(i * 300 - 8000).to_le_bytes());
        }
        let info = WavInfo { sample_rate: 24_000, channels: 1, bits_per_sample: 16, data_offset: 44, data_size: pcm.len() as u32 };

        // One-shot
        let mut r1 = Resampler::new(info);
        let mut whole = std::vec![[0i16; 2]; 256];
        let (_, mut n1) = r1.resample(&pcm, &mut whole);
        n1 += r1.flush(&mut whole[n1..]);

        // Small unaligned input chunks (7 bytes) and tiny output buffer (5 frames)
        let mut r2 = Resampler::new(info);
        let mut streamed = Vec::new();
        let mut small_out = [[0i16; 2]; 5];
        for chunk in pcm.chunks(7) {
            let mut off = 0;
            while off < chunk.len() {
                let (u, w) = r2.resample(&chunk[off..], &mut small_out);
                streamed.extend_from_slice(&small_out[..w]);
                off += u;
                if u == 0 && w == 0 { break; }
            }
        }
        // Drain any carried frame + flush
        loop {
            let (_, w) = r2.resample(&[], &mut small_out);
            if w == 0 { break; }
            streamed.extend_from_slice(&small_out[..w]);
        }
        let w = r2.flush(&mut small_out);
        streamed.extend_from_slice(&small_out[..w]);

        assert_eq!(streamed.as_slice(), &whole[..n1]);
    }

    #[test]
    fn resample_8bit_mono_and_48khz_stereo_passthrough() {
        // 8-bit unsigned mono: 128 -> 0, 255 -> 32512
        let info8 = WavInfo { sample_rate: 48_000, channels: 1, bits_per_sample: 8, data_offset: 44, data_size: 2 };
        let mut r8 = Resampler::new(info8);
        let mut out = [[0i16; 2]; 4];
        let (_, mut n) = r8.resample(&[128, 255], &mut out);
        n += r8.flush(&mut out[n..]);
        assert_eq!(&out[..n], &[[0, 0], [32512, 32512]]);

        // 48 kHz stereo 16-bit passthrough
        let mut pcm = Vec::new();
        for (l, r) in [(-1234i16, 5678i16), (32767, -32768)] {
            pcm.extend_from_slice(&l.to_le_bytes());
            pcm.extend_from_slice(&r.to_le_bytes());
        }
        let info48 = WavInfo { sample_rate: 48_000, channels: 2, bits_per_sample: 16, data_offset: 44, data_size: pcm.len() as u32 };
        let mut r48 = Resampler::new(info48);
        let (_, mut n) = r48.resample(&pcm, &mut out);
        n += r48.flush(&mut out[n..]);
        assert_eq!(&out[..n], &[[-1234, 5678], [32767, -32768]]);
    }

    #[test]
    fn pack_unpack_pcm6_roundtrip() {
        let frames: [[i16; 2]; 6] = [
            [0, 0],
            [i16::MIN, i16::MAX],
            [-1, 1],
            [12345, -23456],
            [-32000, 32000],
            [42, -42],
        ];
        assert_eq!(unpack_pcm6(pack_pcm6(&frames)), frames);
    }
}
