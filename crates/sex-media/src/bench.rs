// SPDX-License-Identifier: GPL-3.0-only
//! SIMD bench (src/bench.c): decode a file held in memory, or encode synthetic
//! pictures, at the SIMD level chosen once at the program's start. The mediabench
//! program runs it once with SSE only and once with AVX/AVX2/FMA3, and its gate
//! checks that both decode to the same pictures.

/// What one decode or encode did.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Run {
    pub frames: i32,
    pub width: i32,
    pub height: i32,
    /// Encoded bytes (encode) or packet bytes read (decode).
    pub bytes: i64,
    /// TSC cycles inside the codec.
    pub cycles: u64,
    /// FNV-1a over every decoded plane row (decode) or every packet (encode).
    pub hash: u64,
    /// Decode with a picture size: TSC cycles scaling pictures into BGRA (bilinear)
    /// and resampling sound to 48 kHz stereo i16, as the player bridge does.
    pub convert_cycles: u64,
    pub codec: [u8; 16],
}

impl Run {
    pub fn codec(&self) -> &str {
        let n = self.codec.iter().position(|b| *b == 0).unwrap_or(16);
        core::str::from_utf8(&self.codec[..n]).unwrap_or("?")
    }
}

#[cfg(ffmpeg)]
mod ffi {
    use core::ffi::{c_char, c_int, c_uint};
    extern "C" {
        pub fn sx_bench_simd(level: c_int);
        pub fn sx_bench_cpu_flags() -> c_uint;
        pub fn sx_bench_decode(data: *const u8, len: usize, show_w: c_int, show_h: c_int, out: *mut super::Run) -> c_int;
        pub fn sx_bench_encode(name: *const c_char, w: c_int, h: c_int, frames: c_int, out: *mut super::Run) -> c_int;
    }
}

/// FFmpeg's AV_CPU_FLAG_* bits for x86.
pub mod flag {
    pub const SSE2: u32 = 0x10;
    pub const SSSE3: u32 = 0x80;
    pub const SSE4: u32 = 0x100;
    pub const SSE42: u32 = 0x200;
    pub const AVX: u32 = 0x4000;
    pub const AVX2: u32 = 0x8000;
    pub const FMA3: u32 = 0x10000;
    pub const AVX512: u32 = 0x100000;
}

/// The SIMD level FFmpeg, dav1d, x264, libvpx and libaom may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Simd {
    /// Everything the CPU and the kernel allow.
    All = 0,
    /// SSE2..SSE4.2 only: AVX, AVX2, FMA3 and AVX-512 hidden.
    Sse = 1,
    /// Up to AVX2 and FMA3: AVX-512 hidden.
    Avx2 = 2,
}

/// Chooses the SIMD level once, before any codec opens.
pub fn simd(level: Simd) {
    #[cfg(ffmpeg)]
    unsafe { ffi::sx_bench_simd(level as i32) };
    #[cfg(not(ffmpeg))]
    let _ = level;
}

/// FFmpeg's CPU flags as it will use them (see `flag`); 0 without FFmpeg.
pub fn cpu_flags() -> u32 {
    #[cfg(ffmpeg)]
    return unsafe { ffi::sx_bench_cpu_flags() };
    #[cfg(not(ffmpeg))]
    0
}

/// Decodes every picture of the first video stream (or every sound frame). With a
/// `show` size (w, h) each picture is also scaled into BGRA at that size and each
/// sound frame resampled to 48 kHz stereo i16 (timed in `convert_cycles`).
pub fn decode(file: &[u8], show: Option<(u32, u32)>) -> Result<Run, i32> {
    let (sw, sh) = show.unwrap_or((0, 0));
    let mut run = Run::default();
    #[cfg(ffmpeg)]
    {
        let r = unsafe { ffi::sx_bench_decode(file.as_ptr(), file.len(), sw as i32, sh as i32, &mut run) };
        if r < 0 { return Err(r); }
        Ok(run)
    }
    #[cfg(not(ffmpeg))]
    { let _ = (file, sw, sh, &mut run); Err(-38) }
}

/// Encodes `frames` synthetic w x h pictures with the named FFmpeg encoder (or, for
/// a sound encoder such as flac, `frames` x 2 s of synthetic stereo sound).
pub fn encode(encoder: &str, w: u32, h: u32, frames: u32) -> Result<Run, i32> {
    let mut run = Run::default();
    let mut name = [0u8; 32];
    if encoder.len() >= name.len() { return Err(-22); }
    name[..encoder.len()].copy_from_slice(encoder.as_bytes());
    #[cfg(ffmpeg)]
    {
        let r = unsafe { ffi::sx_bench_encode(name.as_ptr().cast(), w as i32, h as i32, frames as i32, &mut run) };
        if r < 0 { return Err(r); }
        Ok(run)
    }
    #[cfg(not(ffmpeg))]
    { let _ = (w, h, frames, &mut run, name); Err(-38) }
}
