// SPDX-License-Identifier: GPL-3.0-only
//! `play <wav_path>`: streams a RIFF/WAVE PCM file from `sexfiles` directly into
//! `sexbell` (`OP_BELL_PCM`) with zero heap allocation and zero large static buffers.
//! Supports 8/16-bit, mono/stereo, 8..=192 kHz (linear-interpolated to 48 kHz stereo).

use crate::Scrollback;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::wav::{self, Resampler, WavInfo};

const CHUNK_BYTES: usize = 4096;
/// Audio prefill ahead of real time (ms) kept inside Bell's 341 ms PCM FIFO.
const PREFILL_MS: u64 = 160;
/// Cap PDX sends per main-loop turn so Bell's mailbox never overflows and Spindle stays responsive.
const MAX_CALLS_PER_STEP: usize = 16;
const READ_TIMEOUT_MS: u64 = 15_000;
/// Distinct sequence range so `Play` replies never collide with `fs_commands::FS`.
const SEQ_BASE: u32 = 0x40_0000;
const SEQ_MASK: u32 = 0x3F_FFFF;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    ReadHeader,
    WaitHeader,
    ReadChunk,
    WaitChunk,
    StreamChunk,
    Finish,
}

struct Text { buf: [u8; 160], len: usize }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len]);
    sex_pdx::serial_record!("[spindle.play.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}

fn fs_region() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = s { s = rest; }
    while let [rest @ .., b' ' | b'\t'] = s { s = rest; }
    s
}

const EMPTY_INFO: WavInfo = WavInfo { sample_rate: 48_000, channels: 2, bits_per_sample: 16, data_offset: 0, data_size: 0 };

pub struct Play {
    phase: Phase,
    seq: u32,
    sent_at: u64,
    path_len: u16,
    file_pos: u64,
    data_end: u64,
    chunk_len: usize,
    chunk_at: usize,
    resampler: Resampler,
    start_tick: u64,
    frames_sent: u64,
    /// Up to 6 resampled frames being assembled or retried on `ERR_SERVICE_NOT_READY`.
    pkt: [[i16; 2]; 6],
    pkt_len: u8,
    flushed: bool,
}

pub static mut PLAY: Play = Play {
    phase: Phase::Idle,
    seq: 0,
    sent_at: 0,
    path_len: 0,
    file_pos: 0,
    data_end: 0,
    chunk_len: 0,
    chunk_at: 0,
    resampler: Resampler::new(EMPTY_INFO),
    start_tick: 0,
    frames_sent: 0,
    pkt: [[0; 2]; 6],
    pkt_len: 0,
    flushed: false,
};

impl Play {
    pub fn busy(&self) -> bool { self.phase != Phase::Idle }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.path_len = 0;
        self.file_pos = 0;
        self.data_end = 0;
        self.chunk_len = 0;
        self.chunk_at = 0;
        self.start_tick = 0;
        self.frames_sent = 0;
        self.pkt = [[0; 2]; 6];
        self.pkt_len = 0;
        self.flushed = false;
    }

    fn next_seq(&mut self) -> u32 {
        self.seq = (self.seq + 1) & SEQ_MASK;
        SEQ_BASE | self.seq
    }

    /// `play <wav_path>` or `play stop`.
    pub fn command(&mut self, args: &[u8], sb: &mut Scrollback) {
        let arg = trim(args);
        if arg.is_empty() {
            sb.push(b"Usage: play <wav_path> | play stop");
            return;
        }
        if arg == b"stop" {
            if self.busy() {
                self.reset();
                sb.push(b"play: stopped.");
            } else {
                sb.push(b"play: nothing playing.");
            }
            return;
        }
        if self.busy() {
            sb.push(b"play: already playing (run `play stop` first).");
            return;
        }
        if unsafe { crate::fs_commands::FS.busy() } {
            sb.push(b"play: disk is busy; try again in a moment.");
            return;
        }
        let Some(region) = fs_region() else {
            sb.push(b"play: filesystem unavailable.");
            return;
        };
        let mut full = [0u8; w::PATH_BYTES];
        let Some(n) = (unsafe { crate::fs_commands::FS.join(arg, &mut full) }) else {
            sb.push(b"play: path too long.");
            return;
        };
        if n == 0 {
            sb.push(b"Usage: play <wav_path> | play stop");
            return;
        }
        region[..n].copy_from_slice(&full[..n]);
        self.reset();
        self.path_len = n as u16;
        self.phase = Phase::ReadHeader;
    }

    /// Advances WAV reading and paced `OP_BELL_PCM` streaming. Returns true if scrollback changed.
    pub fn step(&mut self, sb: &mut Scrollback) -> bool {
        if self.phase == Phase::Idle { return false; }
        let now = sex_pdx::get_ticks();
        match self.phase {
            Phase::Idle => false,
            Phase::ReadHeader => {
                let seq = self.next_seq();
                let r = w::Request { seq, kind: w::READ, path_len: self.path_len, offset: 0, length: CHUNK_BYTES as u64 };
                let a = r.args();
                match sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]) {
                    Ok(_) => { self.sent_at = now; self.phase = Phase::WaitHeader; }
                    Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
                    Err(_) => { sb.push(b"play: could not reach sexfiles."); self.reset(); return true; }
                }
                false
            }
            Phase::WaitHeader | Phase::WaitChunk => {
                if now.saturating_sub(self.sent_at) >= READ_TIMEOUT_MS {
                    sb.push(b"play: timed out reading WAV file.");
                    self.reset();
                    return true;
                }
                false
            }
            Phase::ReadChunk => {
                if self.file_pos >= self.data_end {
                    self.phase = Phase::Finish;
                    return self.pump_audio(now, sb);
                }
                let want = (self.data_end - self.file_pos).min(CHUNK_BYTES as u64);
                let seq = self.next_seq();
                let r = w::Request { seq, kind: w::READ, path_len: self.path_len, offset: self.file_pos, length: want };
                let a = r.args();
                match sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]) {
                    Ok(_) => { self.sent_at = now; self.phase = Phase::WaitChunk; }
                    Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
                    Err(_) => { sb.push(b"play: read failed."); self.reset(); return true; }
                }
                false
            }
            Phase::StreamChunk | Phase::Finish => self.pump_audio(now, sb),
        }
    }

    fn pump_audio(&mut self, now: u64, sb: &mut Scrollback) -> bool {
        if self.start_tick == 0 {
            self.start_tick = now;
        }
        let target_frames = (now.saturating_sub(self.start_tick) + PREFILL_MS) * (wav::OUT_RATE as u64 / 1000);
        let mut calls = 0usize;

        while calls < MAX_CALLS_PER_STEP && self.frames_sent < target_frames {
            // Fill up to 6 frames in `self.pkt` if not yet full.
            if self.pkt_len < 6 {
                if self.phase == Phase::StreamChunk {
                    let Some(region) = fs_region() else {
                        sb.push(b"play: filesystem view lost.");
                        self.reset();
                        return true;
                    };
                    let raw = &region[w::PATH_BYTES + self.chunk_at..w::PATH_BYTES + self.chunk_len];
                    let need = 6 - self.pkt_len as usize;
                    let (used, made) = self.resampler.resample(raw, &mut self.pkt[self.pkt_len as usize..self.pkt_len as usize + need]);
                    self.chunk_at += used;
                    self.pkt_len += made as u8;
                    if self.pkt_len < 6 && self.chunk_at >= self.chunk_len {
                        // Need next chunk from `sexfiles` before completing this 6-frame packet.
                        self.phase = if self.file_pos >= self.data_end { Phase::Finish } else { Phase::ReadChunk };
                        break;
                    }
                    if used == 0 && made == 0 {
                        self.phase = if self.file_pos >= self.data_end { Phase::Finish } else { Phase::ReadChunk };
                        break;
                    }
                } else {
                    // Phase::Finish — drain any carried/final sample and zero-pad the tail packet.
                    if !self.flushed {
                        let need = 6 - self.pkt_len as usize;
                        let (_, made) = self.resampler.resample(&[], &mut self.pkt[self.pkt_len as usize..self.pkt_len as usize + need]);
                        self.pkt_len += made as u8;
                        if self.pkt_len < 6 {
                            let need2 = 6 - self.pkt_len as usize;
                            let f = self.resampler.flush(&mut self.pkt[self.pkt_len as usize..self.pkt_len as usize + need2]);
                            self.pkt_len += f as u8;
                            self.flushed = true;
                        }
                    }
                    if self.pkt_len == 0 {
                        say!(sb, "play: done ({} frames @ 48 kHz stereo)", self.frames_sent);
                        self.reset();
                        return true;
                    }
                    while self.pkt_len < 6 {
                        self.pkt[self.pkt_len as usize] = [0, 0];
                        self.pkt_len += 1;
                    }
                }
            }

            // Send the 6-frame packet to Bell (`OP_BELL_PCM`).
            let words = wav::pack_pcm6(&self.pkt);
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_BELL, sex_pdx::OP_BELL_PCM, words[0], words[1], words[2]) {
                Ok(_) => {
                    self.pkt_len = 0;
                    self.frames_sent += 6;
                    calls += 1;
                }
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => break,
                Err(_) => {
                    sb.push(b"play: sexbell unreachable.");
                    self.reset();
                    return true;
                }
            }
        }
        false
    }

    /// Handles a reply from `sexfiles` (PD 11). Returns true if it belonged to `Play`.
    pub fn receive(&mut self, word: u64, sb: &mut Scrollback) -> bool {
        if !matches!(self.phase, Phase::WaitHeader | Phase::WaitChunk) { return false; }
        let Some((seq, status, value)) = w::decode_reply(word) else { return false };
        if seq != (SEQ_BASE | self.seq) { return false; }
        let Some(region) = fs_region() else { self.reset(); return true };

        if status != w::OK {
            say!(sb, "play: {}", core::str::from_utf8(w::message(status)).unwrap_or("read error"));
            self.reset();
            return true;
        }
        let n = (value as usize).min(CHUNK_BYTES);
        match self.phase {
            Phase::WaitHeader => {
                let hdr_bytes = &region[w::PATH_BYTES..w::PATH_BYTES + n];
                match wav::parse_wav_header(hdr_bytes) {
                    Ok(info) => {
                        let path_str = core::str::from_utf8(&region[..self.path_len as usize]).unwrap_or("?");
                        let secs = info.duration_ms() / 1000;
                        let tenths = (info.duration_ms() % 1000) / 100;
                        say!(sb, "play: {} ({} Hz, {} ch, {}-bit, {}.{}s)", path_str, info.sample_rate, info.channels, info.bits_per_sample, secs, tenths);
                        self.resampler = Resampler::new(info);
                        self.data_end = info.data_offset as u64 + info.data_size as u64;
                        if (info.data_offset as usize) < n {
                            // First PCM bytes are already sitting right here in `region`!
                            self.chunk_at = info.data_offset as usize;
                            self.chunk_len = n.min(self.data_end as usize);
                            self.file_pos = self.chunk_len as u64;
                            self.phase = Phase::StreamChunk;
                        } else {
                            self.file_pos = info.data_offset as u64;
                            self.phase = Phase::ReadChunk;
                        }
                    }
                    Err(msg) => {
                        say!(sb, "play: invalid WAV ({})", msg);
                        self.reset();
                    }
                }
            }
            Phase::WaitChunk => {
                if n == 0 {
                    self.phase = Phase::Finish;
                } else {
                    self.chunk_at = 0;
                    self.chunk_len = n;
                    self.file_pos += n as u64;
                    self.phase = Phase::StreamChunk;
                }
            }
            _ => {}
        }
        true
    }
}
