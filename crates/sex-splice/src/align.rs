// SPDX-License-Identifier: GPL-3.0-only
//! Acoustic alignment, adaptive room-tone valley snapping, Viterbi CTC word timing,
//! skipped-speech hole detection, acoustic hesitation (`"uhm"`) detection, and
//! cross-correlation waveform sync.
//!
//! Adapted from Scarlet's `ctcAlign.ts`, `dialogue-join.ts`, `gap-sound.ts`,
//! `transcriptHoles.ts`, and `uhm.ts` (MIT) and Rook's `MulticamSyncMethod::Waveform`
//! / `SyncAudio` (GPL-3.0), implemented in deterministic `#![no_std]` integer arithmetic.

use crate::transcript::{SourceCut, Transcript, Word};
use crate::{Micros, Range, SECOND};
use alloc::vec;
use alloc::vec::Vec;

/// Minimum noise floor on the `0..=32768` `i16` scale so pure digital silence never
/// collapses adaptive thresholds to zero.
const MIN_NOISE_FLOOR: u16 = 32;

/// Minimum contiguous duration above [`LevelEnvelope::speech_threshold`] inside a
/// transcript gap to report an unexplained speech hole (`120 ms`).
const MIN_HOLE_SPEECH_US: Micros = 120_000;

/// Minimum duration of a voiced acoustic hesitation filler (`120 ms`).
const MIN_HESITATION_US: Micros = 120_000;

/// Maximum duration of a voiced acoustic hesitation filler (`1_200 ms`).
const MAX_HESITATION_US: Micros = 1_200_000;

/// Analysis bucket width (`10 ms`) used when scanning raw PCM for `"uh"` / `"umm"` fillers.
const HESITATION_BUCKET_US: Micros = 10_000;

// ============================================================================
// 1. Audio Level Envelope & Adaptive Room-Tone Floor
// ============================================================================

/// Bucketed RMS and peak amplitude envelope on the `0..=32768` signed-16-bit PCM scale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelEnvelope {
    pub bucket_us: Micros,
    pub rms: Vec<u16>,
    pub peak: Vec<u16>,
}

impl LevelEnvelope {
    /// Builds a deterministic integer RMS and peak envelope from mono 16-bit PCM.
    pub fn from_mono_i16(pcm: &[i16], sample_rate: u32, bucket_us: Micros) -> Self {
        let safe_bucket = bucket_us.max(1);
        if pcm.is_empty() || sample_rate == 0 || bucket_us <= 0 {
            return Self {
                bucket_us: safe_bucket,
                rms: Vec::new(),
                peak: Vec::new(),
            };
        }

        let sr = sample_rate as u128;
        let bus = safe_bucket as u128;
        let sec = SECOND as u128;
        let est_buckets = ((pcm.len() as u128 * sec) / (sr * bus)) as usize + 1;

        let mut rms = Vec::with_capacity(est_buckets);
        let mut peak = Vec::with_capacity(est_buckets);

        let mut b: u128 = 0;
        let mut start = 0usize;
        while start < pcm.len() {
            let raw_end = (((b + 1) * bus * sr) / sec) as usize;
            let end = raw_end.max(start + 1).min(pcm.len());

            let mut sum_sq: u128 = 0;
            let mut max_abs: u16 = 0;
            for &s in &pcm[start..end] {
                let a = s.unsigned_abs();
                if a > max_abs {
                    max_abs = a;
                }
                let au = a as u128;
                sum_sq += au * au;
            }

            let count = (end - start) as u128;
            let bucket_rms = (sum_sq / count).isqrt() as u16;
            rms.push(bucket_rms);
            peak.push(max_abs);

            start = end;
            b += 1;
        }

        Self {
            bucket_us: safe_bucket,
            rms,
            peak,
        }
    }

    /// Builds a deterministic integer RMS and peak envelope from stereo 16-bit PCM.
    ///
    /// Stereo RMS is computed over combined channel power `((L^2 + R^2) / 2)`, while
    /// peak tracks the maximum absolute sample across both channels.
    pub fn from_stereo_i16(pcm: &[[i16; 2]], sample_rate: u32, bucket_us: Micros) -> Self {
        let safe_bucket = bucket_us.max(1);
        if pcm.is_empty() || sample_rate == 0 || bucket_us <= 0 {
            return Self {
                bucket_us: safe_bucket,
                rms: Vec::new(),
                peak: Vec::new(),
            };
        }

        let sr = sample_rate as u128;
        let bus = safe_bucket as u128;
        let sec = SECOND as u128;
        let est_buckets = ((pcm.len() as u128 * sec) / (sr * bus)) as usize + 1;

        let mut rms = Vec::with_capacity(est_buckets);
        let mut peak = Vec::with_capacity(est_buckets);

        let mut b: u128 = 0;
        let mut start = 0usize;
        while start < pcm.len() {
            let raw_end = (((b + 1) * bus * sr) / sec) as usize;
            let end = raw_end.max(start + 1).min(pcm.len());

            let mut sum_sq: u128 = 0;
            let mut max_abs: u16 = 0;
            for &[l, r] in &pcm[start..end] {
                let al = l.unsigned_abs();
                let ar = r.unsigned_abs();
                let m = al.max(ar);
                if m > max_abs {
                    max_abs = m;
                }
                let alu = al as u128;
                let aru = ar as u128;
                sum_sq += alu * alu + aru * aru;
            }

            let denom = (end - start) as u128 * 2;
            let bucket_rms = (sum_sq / denom).isqrt() as u16;
            rms.push(bucket_rms);
            peak.push(max_abs);

            start = end;
            b += 1;
        }

        Self {
            bucket_us: safe_bucket,
            rms,
            peak,
        }
    }

    /// Adaptive room-tone noise floor adapted from Scarlet's `gap-sound.ts`:
    /// sorts a copy of `self.rms` and selects the quietest 20th-percentile bucket
    /// capped at the 50th-quietest bucket (index `49`), clamped to at least `32`.
    pub fn noise_floor(&self) -> u16 {
        if self.rms.is_empty() {
            return MIN_NOISE_FLOOR;
        }
        let mut sorted = self.rms.clone();
        sorted.sort_unstable();
        let idx = (sorted.len() / 5).min(49).min(sorted.len() - 1);
        sorted[idx].max(MIN_NOISE_FLOOR)
    }

    /// Adaptive RMS threshold above which buckets are treated as active speech.
    pub fn speech_threshold(&self) -> u16 {
        (self.noise_floor() as u32 * 4).clamp(400, 16_000) as u16
    }

    /// Maximum peak sample amplitude overlapping `range`.
    pub fn max_peak_in(&self, range: Range) -> u16 {
        let Some((b0, b1)) = self.bucket_bounds(range) else {
            return 0;
        };
        self.peak[b0..b1].iter().copied().max().unwrap_or(0)
    }

    /// Mean bucket RMS across all buckets overlapping `range`.
    pub fn mean_rms_in(&self, range: Range) -> u16 {
        let Some((b0, b1)) = self.bucket_bounds(range) else {
            return 0;
        };
        let sum: u64 = self.rms[b0..b1].iter().map(|&v| v as u64).sum();
        (sum / (b1 - b0) as u64) as u16
    }

    /// Total duration covered by this envelope in [`Micros`].
    fn duration_us(&self) -> Micros {
        if self.bucket_us <= 0 {
            return 0;
        }
        (self.rms.len() as Micros).saturating_mul(self.bucket_us)
    }

    /// Half-open bucket index slice `(start_idx, end_idx)` overlapping `range`.
    fn bucket_bounds(&self, range: Range) -> Option<(usize, usize)> {
        if range.is_empty() || self.rms.is_empty() || self.bucket_us <= 0 || range.end <= 0 {
            return None;
        }
        let b0 = (range.start.max(0) / self.bucket_us) as usize;
        let b1 = ((range.end.max(0) + self.bucket_us - 1) / self.bucket_us) as usize;
        let b1 = b1.min(self.rms.len());
        if b0 >= b1 || b0 >= self.rms.len() {
            None
        } else {
            Some((b0, b1))
        }
    }

    /// RMS at source timestamp `at_us`, clamping to the nearest valid bucket.
    fn rms_at(&self, at_us: Micros) -> u16 {
        if self.rms.is_empty() || self.bucket_us <= 0 {
            return 0;
        }
        let idx = (at_us.max(0) / self.bucket_us) as usize;
        self.rms[idx.min(self.rms.len() - 1)]
    }
}

// ============================================================================
// 2. Room-Tone Valley Snapping, Whole-Frame Placement & Audible-Edge Refusal
// ============================================================================

/// Refusal returned when a proposed cut edge lies inside continuous audible speech and
/// no room-tone valley exists within `max_search_us`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudibleEdgeRefusal {
    pub at_source: Micros,
    pub is_end_edge: bool,
    pub rms_at_edge: u16,
    pub quiet_threshold: u16,
}

/// Summary of how far [`snap_cut_to_quiet_frames`] moved a cut's start and end edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeSnapReport {
    pub original: Range,
    pub snapped: Range,
    /// Signed delta `snapped.start - original.start`.
    pub start_moved_us: Micros,
    /// Signed delta `snapped.end - original.end`.
    pub end_moved_us: Micros,
}

/// Exact source timestamp of whole video frame `frame_idx`:
/// `frame_idx * fps_den * SECOND / fps_num`.
#[inline]
fn frame_time_us(frame_idx: i64, fps_num: u32, fps_den: u32) -> Micros {
    if frame_idx <= 0 {
        return 0;
    }
    let num = fps_num.max(1) as i128;
    let den = fps_den.max(1) as i128;
    ((frame_idx as i128 * den * SECOND as i128) / num) as Micros
}

/// Largest `frame_idx >= 0` such that `frame_time_us(frame_idx) <= t`.
fn floor_frame_idx(t: Micros, fps_num: u32, fps_den: u32) -> i64 {
    if t <= 0 {
        return 0;
    }
    let num = fps_num.max(1) as i128;
    let den = fps_den.max(1) as i128;
    let mut k = ((t as i128 * num) / (den * SECOND as i128)) as i64;
    while frame_time_us(k + 1, fps_num, fps_den) <= t {
        k += 1;
    }
    while k > 0 && frame_time_us(k, fps_num, fps_den) > t {
        k -= 1;
    }
    k
}

/// Smallest `frame_idx >= 0` such that `frame_time_us(frame_idx) >= t`.
fn ceil_frame_idx(t: Micros, fps_num: u32, fps_den: u32) -> i64 {
    if t <= 0 {
        return 0;
    }
    let k = floor_frame_idx(t, fps_num, fps_den);
    if frame_time_us(k, fps_num, fps_den) < t {
        k + 1
    } else {
        k
    }
}

/// Contiguous run `[b0..b1)` of quiet buckets (`rms <= quiet_threshold`).
#[derive(Clone, Copy, Debug)]
struct QuietRun {
    b0: usize,
    b1: usize,
    start_us: Micros,
    end_us: Micros,
}

fn collect_quiet_runs(env: &LevelEnvelope, quiet_threshold: u16) -> Vec<QuietRun> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < env.rms.len() {
        if env.rms[i] <= quiet_threshold {
            let b0 = i;
            while i < env.rms.len() && env.rms[i] <= quiet_threshold {
                i += 1;
            }
            let b1 = i;
            runs.push(QuietRun {
                b0,
                b1,
                start_us: b0 as Micros * env.bucket_us,
                end_us: b1 as Micros * env.bucket_us,
            });
        } else {
            i += 1;
        }
    }
    runs
}

/// Local RMS around candidate time `t_f` clamped inside `run`.
fn valley_rms_at(env: &LevelEnvelope, run: &QuietRun, t_f: Micros) -> u16 {
    let b_right = ((t_f / env.bucket_us) as usize).clamp(run.b0, run.b1 - 1);
    let b_left = if t_f > 0 {
        (((t_f - 1) / env.bucket_us) as usize).clamp(run.b0, run.b1 - 1)
    } else {
        b_right
    };
    ((env.rms[b_left] as u32 + env.rms[b_right] as u32) / 2) as u16
}

/// Searches `[win_lo, win_hi]` around `edge_us` for the quietest room-tone valley and
/// selects an exact whole-frame boundary inside that quiet run (`edgeOnFrame`).
#[allow(clippy::too_many_arguments)]
fn snap_single_edge(
    env: &LevelEnvelope,
    runs: &[QuietRun],
    fps_num: u32,
    fps_den: u32,
    edge_us: Micros,
    win_lo: Micros,
    win_hi: Micros,
    min_frame_us: Option<Micros>,
    is_end_edge: bool,
    quiet_threshold: u16,
) -> Result<Micros, AudibleEdgeRefusal> {
    let env_end = env.duration_us();

    // Keep quiet runs that overlap `[win_lo, win_hi]` with positive overlap (or touch when
    // the window collapses at a boundary).
    let overlapping: Vec<&QuietRun> = runs
        .iter()
        .filter(|r| {
            let lo = r.start_us.max(win_lo);
            let hi = r.end_us.min(win_hi);
            hi > lo || (win_lo == win_hi && r.start_us <= win_lo && win_lo <= r.end_us)
        })
        .collect();

    if overlapping.is_empty() {
        return Err(AudibleEdgeRefusal {
            at_source: edge_us,
            is_end_edge,
            rms_at_edge: env.rms_at(edge_us),
            quiet_threshold,
        });
    }

    // Score whole-frame boundaries `t_f` inside each overlapping quiet run.
    // Lower cost wins:
    //   1. Prefer frames inside `[win_lo, win_hi]`.
    //   2. Prefer frames strictly interior to the quiet run (both sides silent) unless at
    //      media start/end.
    //   3. Minimize local valley RMS (`valley_rms * 1000`), breaking flat room-tone ties by
    //      proximity to `edge_us`.
    let mut best: Option<(i64, Micros)> = None;

    for run in &overlapping {
        let k_start = ceil_frame_idx(run.start_us, fps_num, fps_den);
        let k_end = floor_frame_idx(run.end_us, fps_num, fps_den);

        if k_start <= k_end {
            for k in k_start..=k_end {
                let t_f = frame_time_us(k, fps_num, fps_den);
                if let Some(min_t) = min_frame_us {
                    if t_f <= min_t {
                        continue;
                    }
                }

                let outside_win_penalty = if t_f < win_lo || t_f > win_hi {
                    500_000i64
                } else {
                    0
                };
                let touches_left_loud = t_f == run.start_us && run.b0 > 0;
                let touches_right_loud = t_f == run.end_us && run.b1 < env.rms.len();
                let boundary_penalty = if touches_left_loud || touches_right_loud {
                    50_000i64
                } else {
                    0
                };
                let local_rms = valley_rms_at(env, run, t_f) as i64;
                let dist_cost = (t_f - edge_us).abs() / 1_000;
                let cost = outside_win_penalty + boundary_penalty + local_rms * 1_000 + dist_cost;

                match best {
                    None => best = Some((cost, t_f)),
                    Some((best_cost, _)) if cost < best_cost => best = Some((cost, t_f)),
                    _ => {}
                }
            }
        } else {
            // Sub-frame quiet run: round OUTWARD from the cut so inward rounding never
            // leaves a fraction of a cut word audible at the join (`edgeOnFrame`).
            let k_out = if is_end_edge {
                ceil_frame_idx(run.start_us, fps_num, fps_den)
            } else {
                floor_frame_idx(run.end_us, fps_num, fps_den)
            };
            let t_f = frame_time_us(k_out, fps_num, fps_den).clamp(0, env_end);
            if min_frame_us.is_none_or(|min_t| t_f > min_t) {
                let local_rms = valley_rms_at(env, run, run.start_us) as i64;
                let dist_cost = (t_f - edge_us).abs() / 1_000;
                let cost = 1_000_000i64 + local_rms * 1_000 + dist_cost;
                match best {
                    None => best = Some((cost, t_f)),
                    Some((best_cost, _)) if cost < best_cost => best = Some((cost, t_f)),
                    _ => {}
                }
            }
        }
    }

    best.map(|(_, t_f)| t_f).ok_or(AudibleEdgeRefusal {
        at_source: edge_us,
        is_end_edge,
        rms_at_edge: env.rms_at(edge_us),
        quiet_threshold,
    })
}

/// Snaps both edges of `cut` into the quietest room-tone valley within `±max_search_us`,
/// placing both edges on exact whole-frame boundaries (`frame_idx * fps_den * SECOND / fps_num`)
/// inside the quiet run so inward rounding never leaves a frame of a cut word sounding at
/// the join (`edgeOnFrame` from Scarlet).
///
/// Returns `Err(AudibleEdgeRefusal)` if either edge sits in continuous loud speech with no
/// quiet bucket within `max_search_us`.
pub fn snap_cut_to_quiet_frames(
    env: &LevelEnvelope,
    fps_num: u32,
    fps_den: u32,
    cut: SourceCut,
    max_search_us: Micros,
) -> Result<(SourceCut, EdgeSnapReport), AudibleEdgeRefusal> {
    let fps_num = fps_num.max(1);
    let fps_den = fps_den.max(1);
    let search = max_search_us.max(0);
    let original = cut.range;

    if env.rms.is_empty() || env.bucket_us <= 0 {
        let s_idx = floor_frame_idx(original.start.max(0), fps_num, fps_den);
        let e_idx = ceil_frame_idx(original.end.max(0), fps_num, fps_den).max(s_idx + 1);
        let snapped = Range::new(
            frame_time_us(s_idx, fps_num, fps_den),
            frame_time_us(e_idx, fps_num, fps_den),
        );
        return Ok((
            SourceCut {
                media: cut.media,
                range: snapped,
            },
            EdgeSnapReport {
                original,
                snapped,
                start_moved_us: snapped.start - original.start,
                end_moved_us: snapped.end - original.end,
            },
        ));
    }

    let quiet_threshold = (env.noise_floor() as u32 * 2).max(180).min(u16::MAX as u32) as u16;
    let env_end = env.duration_us();
    let runs = collect_quiet_runs(env, quiet_threshold);

    // Prevent `start` from jumping forward over the loud cut body into post-cut silence,
    // and prevent `end` from jumping backward over the loud cut body into pre-cut silence.
    let mut start_hi_cap = original.end.min(env_end);
    let mut end_lo_cap = original.start.max(0);
    if let Some((b0, b1)) = env.bucket_bounds(original) {
        let first_loud = (b0..b1).find(|&b| env.rms[b] > quiet_threshold);
        let last_loud = (b0..b1).rev().find(|&b| env.rms[b] > quiet_threshold);
        if let (Some(fl), Some(ll)) = (first_loud, last_loud) {
            let loud_end_us = (ll + 1) as Micros * env.bucket_us;
            let loud_start_us = fl as Micros * env.bucket_us;
            // Cap `start` strictly before the end of the last loud run in `cut`, and `end`
            // strictly after the start of the first loud run in `cut`.
            start_hi_cap = start_hi_cap.min(loud_end_us);
            end_lo_cap = end_lo_cap.max(loud_start_us);
        }
    }

    let start_lo = (original.start - search).max(0).min(env_end);
    let start_hi = (original.start + search).clamp(start_lo, start_hi_cap.max(start_lo));

    let snapped_start = snap_single_edge(
        env,
        &runs,
        fps_num,
        fps_den,
        original.start,
        start_lo,
        start_hi,
        None,
        false,
        quiet_threshold,
    )?;

    let end_lo_raw = (original.end - search).max(end_lo_cap).max(0).min(env_end);
    let end_hi = (original.end + search).max(end_lo_raw).min(env_end);

    let snapped_end = snap_single_edge(
        env,
        &runs,
        fps_num,
        fps_den,
        original.end,
        end_lo_raw,
        end_hi,
        Some(snapped_start),
        true,
        quiet_threshold,
    )?;

    let snapped = Range::new(snapped_start, snapped_end);
    let report = EdgeSnapReport {
        original,
        snapped,
        start_moved_us: snapped.start - original.start,
        end_moved_us: snapped.end - original.end,
    };
    Ok((
        SourceCut {
            media: cut.media,
            range: snapped,
        },
        report,
    ))
}

/// Batch helper applying [`snap_cut_to_quiet_frames`] to a slice of [`SourceCut`]s.
pub fn snap_cut_edges(
    env: &LevelEnvelope,
    fps_num: u32,
    fps_den: u32,
    cuts: &[SourceCut],
    max_search_us: Micros,
) -> Result<Vec<(SourceCut, EdgeSnapReport)>, AudibleEdgeRefusal> {
    let mut out = Vec::with_capacity(cuts.len());
    for &cut in cuts {
        out.push(snap_cut_to_quiet_frames(
            env,
            fps_num,
            fps_den,
            cut,
            max_search_us,
        )?);
    }
    Ok(out)
}

// ============================================================================
// 3. Viterbi CTC Forced Word Alignment
// ============================================================================

/// wav2vec2 CTC `<pad>` / blank token index.
pub const CTC_BLANK: usize = 0;

/// wav2vec2 CTC `|` word-separator token index.
pub const CTC_SEP: usize = 4;

/// wav2vec2 27-character uppercase + apostrophe alphabet mapped to class indices `5..32`.
pub const CTC_ALPHABET: &[u8] = b"ETAONIHSRDLUMWCFGYPBVK'XJQZ";

/// Default wav2vec2 acoustic frame stride (`20 ms` = `20_000` µs, 50 Hz).
pub const DEFAULT_CTC_FRAME_US: Micros = 20_000;

/// Converts a transcript word into wav2vec2 CTC class indices (`5 + idx` in [`CTC_ALPHABET`]).
///
/// Keeps ASCII letters (uppercased) and internal apostrophes (`'` flanked by letters, such
/// as `"don't"`), returning `None` for punctuation-only tokens, numbers, or bracketed tags
/// such as `"[music]"` or `"<laughter>"`.
pub fn word_ctc_labels(text: &str) -> Option<Vec<usize>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('<') && trimmed.ends_with('>'))
    {
        return None;
    }

    // Collect normalized ASCII letters and candidate apostrophes first so we can keep
    // only internal apostrophes (`'` preceded and followed by at least one ASCII letter).
    let mut cleaned: Vec<u8> = Vec::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphabetic() {
            cleaned.push(ch.to_ascii_uppercase() as u8);
        } else if ch == '\'' || ch == '\u{2019}' {
            cleaned.push(b'\'');
        }
    }

    let mut out = Vec::with_capacity(cleaned.len());
    let mut seen_letter = false;
    for (i, &b) in cleaned.iter().enumerate() {
        if b == b'\'' {
            let prev_is_letter = seen_letter && i > 0 && cleaned[i - 1].is_ascii_alphabetic();
            let has_later_letter = cleaned[i + 1..].iter().any(|c| c.is_ascii_alphabetic());
            if !prev_is_letter || !has_later_letter {
                continue;
            }
        } else {
            seen_letter = true;
        }
        if let Some(pos) = CTC_ALPHABET.iter().position(|&a| a == b) {
            out.push(5 + pos);
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Trims trailing room-tone silence buckets off a word's `Range` without collapsing the word.
fn trim_trailing_silence(range: Range, min_len_us: Micros, env: &LevelEnvelope) -> Range {
    if range.is_empty() || env.rms.is_empty() || env.bucket_us <= 0 {
        return range;
    }
    let quiet_thresh = (env.noise_floor() as u32 * 2).max(180).min(u16::MAX as u32) as u16;
    let Some((b0, b1)) = env.bucket_bounds(range) else {
        return range;
    };

    if let Some(last_audible) = (b0..b1).rev().find(|&b| env.rms[b] > quiet_thresh) {
        let audible_end = ((last_audible + 1) as Micros * env.bucket_us).min(range.end);
        let floor_end = (range.start + min_len_us.max(1)).min(range.end);
        Range::new(range.start, audible_end.max(floor_end))
    } else {
        range
    }
}

/// Runs exact integer Viterbi CTC forced word alignment over `log_probs` (`32` classes per
/// acoustic frame of `frame_us` microseconds), backtracking the exact first and last
/// non-blank emission frame of each word.
///
/// If `env` is provided, trailing quiet room-tone buckets are trimmed off stretched or
/// unconfirmed word tails (preventing Whisper's multi-second silence stretch bug).
/// Falls back gracefully to original word timestamps when `log_probs` is shorter than the
/// minimum CTC frame count required by `words`.
pub fn ctc_align_words(
    log_probs: &[[i16; 32]],
    frame_us: Micros,
    media: u32,
    words: &[Word],
    env: Option<&LevelEnvelope>,
) -> Transcript {
    if words.is_empty() || frame_us <= 0 {
        return Transcript {
            media,
            words: words.to_vec(),
        };
    }

    // Build `(class_idx, Option<word_idx>)` for every alignable word separated by `CTC_SEP`.
    let mut raw_tokens: Vec<(usize, Option<usize>)> = Vec::new();
    let mut alignable_indices: Vec<usize> = Vec::new();
    for (wi, w) in words.iter().enumerate() {
        if let Some(labels) = word_ctc_labels(&w.text) {
            if !alignable_indices.is_empty() {
                raw_tokens.push((CTC_SEP, None));
            }
            alignable_indices.push(wi);
            for c in labels {
                raw_tokens.push((c, Some(wi)));
            }
        }
    }

    if raw_tokens.is_empty() {
        let mut fallback = words.to_vec();
        if let Some(e) = env {
            for w in &mut fallback {
                w.source = trim_trailing_silence(w.source, frame_us, e);
            }
        }
        return Transcript {
            media,
            words: fallback,
        };
    }

    // Minimum frames needed by CTC: 1 per token plus 1 mandatory blank frame between
    // identical adjacent non-blank tokens.
    let mut min_frames = 1usize;
    for pair in raw_tokens.windows(2) {
        min_frames += if pair[0].0 == pair[1].0 { 2 } else { 1 };
    }
    let t_len = log_probs.len();
    if t_len < min_frames {
        return Transcript {
            media,
            words: words.to_vec(),
        };
    }

    // Expanded CTC state graph: `[BLANK, tok_0, BLANK, tok_1, ..., tok_{K-1}, BLANK]`.
    let l_len = raw_tokens.len() * 2 + 1;
    let mut states: Vec<(usize, Option<usize>)> = Vec::with_capacity(l_len);
    states.push((CTC_BLANK, None));
    for &(tok, owner) in &raw_tokens {
        states.push((tok, owner));
        states.push((CTC_BLANK, None));
    }

    const NEG_INF: i64 = i64::MIN / 4;
    let mut prev_dp = vec![NEG_INF; l_len];
    let mut curr_dp = vec![NEG_INF; l_len];
    // Backpointer `bp[t * l_len + s]` stores `s - prev_s` in `{0, 1, 2}`.
    let mut bp = vec![0u8; t_len * l_len];

    prev_dp[0] = log_probs[0][states[0].0] as i64;
    if l_len > 1 {
        prev_dp[1] = log_probs[0][states[1].0] as i64;
    }

    for t in 1..t_len {
        let probs = &log_probs[t];
        // States beyond `2 * t + 1` cannot be reached yet; states that cannot reach `l_len - 2`
        // in the remaining `t_len - 1 - t` steps can still be pruned naturally or evaluated.
        let s_max = (2 * t + 1).min(l_len - 1);
        curr_dp[..=s_max].fill(NEG_INF);
        for s in 0..=s_max {
            let mut best = prev_dp[s];
            let mut step = 0u8;

            if s >= 1 && prev_dp[s - 1] > best {
                best = prev_dp[s - 1];
                step = 1;
            }
            // Skipping a blank (`s - 2 -> s`) is allowed ONLY when entering a non-blank
            // token (`s` odd) that differs from the preceding non-blank token (`s - 2`).
            if s >= 2 && (s & 1) == 1 && states[s].0 != states[s - 2].0 && prev_dp[s - 2] > best {
                best = prev_dp[s - 2];
                step = 2;
            }

            if best > NEG_INF / 2 {
                curr_dp[s] = best + probs[states[s].0] as i64;
                bp[t * l_len + s] = step;
            }
        }
        core::mem::swap(&mut prev_dp, &mut curr_dp);
    }

    let end_state = if l_len >= 2 && prev_dp[l_len - 2] > prev_dp[l_len - 1] {
        l_len - 2
    } else {
        l_len - 1
    };
    if prev_dp[end_state] <= NEG_INF / 2 {
        return Transcript {
            media,
            words: words.to_vec(),
        };
    }

    let mut first_frame: Vec<Option<usize>> = vec![None; words.len()];
    let mut last_frame: Vec<Option<usize>> = vec![None; words.len()];

    let mut s = end_state;
    for t in (0..t_len).rev() {
        if let Some(wi) = states[s].1 {
            first_frame[wi] = Some(first_frame[wi].map_or(t, |f| f.min(t)));
            last_frame[wi] = Some(last_frame[wi].map_or(t, |l| l.max(t)));
        }
        if t > 0 {
            s -= bp[t * l_len + s] as usize;
        }
    }

    let mut aligned_words: Vec<Word> = Vec::with_capacity(words.len());
    for (wi, w) in words.iter().enumerate() {
        let mut range = match (first_frame[wi], last_frame[wi]) {
            (Some(f0), Some(f1)) => Range::new(f0 as Micros * frame_us, (f1 + 1) as Micros * frame_us),
            _ => w.source,
        };
        if let Some(e) = env {
            range = trim_trailing_silence(range, frame_us, e);
        }
        aligned_words.push(Word {
            text: w.text.clone(),
            source: range,
        });
    }

    // Keep unaligned tags (e.g. `[music]`) monotonic and non-overlapping with neighbors.
    for wi in 0..aligned_words.len() {
        if first_frame[wi].is_none() {
            let lo = if wi > 0 {
                aligned_words[wi - 1].source.end
            } else {
                0
            };
            let hi = aligned_words[wi + 1..]
                .iter()
                .enumerate()
                .find_map(|(off, nw)| first_frame[wi + 1 + off].map(|_| nw.source.start));
            let mut r = aligned_words[wi].source;
            if r.start < lo {
                r.start = lo;
            }
            if let Some(max_end) = hi {
                if r.end > max_end {
                    r.end = max_end;
                }
            }
            if r.end < r.start {
                r.end = r.start;
            }
            aligned_words[wi].source = r;
        }
    }

    Transcript {
        media,
        words: aligned_words,
    }
}

// ============================================================================
// 4. Transcript Holes & Acoustic Hesitation (`uhm`) Detection
// ============================================================================

/// Collects wordless source spans (`0..first`, inter-word gaps, and `last..media_duration`).
fn wordless_gaps(transcript: &Transcript, media_duration: Micros) -> Vec<Range> {
    if media_duration <= 0 {
        return Vec::new();
    }
    if transcript.words.is_empty() {
        return vec![Range::new(0, media_duration)];
    }

    let mut gaps = Vec::with_capacity(transcript.words.len() + 1);
    let first_start = transcript.words[0].source.start.clamp(0, media_duration);
    if first_start > 0 {
        gaps.push(Range::new(0, first_start));
    }
    for pair in transcript.words.windows(2) {
        let a = pair[0].source.end.clamp(0, media_duration);
        let b = pair[1].source.start.clamp(0, media_duration);
        if b > a {
            gaps.push(Range::new(a, b));
        }
    }
    let last_end = transcript
        .words
        .last()
        .map(|w| w.source.end.clamp(0, media_duration))
        .unwrap_or(0);
    if media_duration > last_end {
        gaps.push(Range::new(last_end, media_duration));
    }
    gaps
}

/// Scans transcript gaps `>= min_gap_us` (including `0..first` and `last..media_duration`)
/// and returns sub-spans where `env` exceeds [`LevelEnvelope::speech_threshold`] for at
/// least `120_000` µs (`120 ms`), catching spoken remarks Whisper skipped after long pauses.
pub fn find_unexplained_speech_holes(
    transcript: &Transcript,
    env: &LevelEnvelope,
    media_duration: Micros,
    min_gap_us: Micros,
) -> Vec<Range> {
    if env.rms.is_empty() || env.bucket_us <= 0 || media_duration <= 0 {
        return Vec::new();
    }

    let thresh = env.speech_threshold();
    let min_gap = min_gap_us.max(0);
    // Allow bridging brief stop-consonant closures (`<= 30 ms`) inside a single remark.
    let max_bridge_buckets = (30_000 / env.bucket_us).max(0) as usize;

    let mut holes = Vec::new();
    for gap in wordless_gaps(transcript, media_duration) {
        if gap.len() < min_gap {
            continue;
        }
        let Some((b0, b1)) = env.bucket_bounds(gap) else {
            continue;
        };

        let mut i = b0;
        while i < b1 {
            if env.rms[i] > thresh {
                let run_start = i;
                let mut run_end = i + 1;
                let mut j = run_end;
                while j < b1 {
                    if env.rms[j] > thresh {
                        run_end = j + 1;
                        j += 1;
                    } else if j - run_end < max_bridge_buckets {
                        j += 1;
                    } else {
                        break;
                    }
                }

                let start_us = (run_start as Micros * env.bucket_us).max(gap.start);
                let end_us = (run_end as Micros * env.bucket_us).min(gap.end);
                if end_us - start_us >= MIN_HOLE_SPEECH_US {
                    holes.push(Range::new(start_us, end_us));
                }
                i = run_end;
            } else {
                i += 1;
            }
        }
    }

    holes
}

/// Scans wordless gaps in `transcript` for sustained voiced hesitation fillers (`"uh"`,
/// `"umm"`, `"er"` that ASR omitted), returning [`SourceCut`] candidates.
///
/// Candidate islands must satisfy all four acoustic criteria adapted from Scarlet's `uhm.ts`:
/// 1. Duration in `120 ms ..= 1_200 ms`.
/// 2. Moderate sustained voiced energy well above the adaptive room-tone floor.
/// 3. Low zero-crossing rate (`< sample_rate / 6`), separating voiced vowels/nasals from
///    unvoiced sibilants, breath hiss, and clicks.
/// 4. Low bucket-to-bucket amplitude variance (`CV <= 450` per-mille), separating steady
///    hesitation vowels from multi-syllable speech and transient impacts.
pub fn detect_acoustic_hesitations(
    pcm: &[i16],
    sample_rate: u32,
    transcript: &Transcript,
) -> Vec<SourceCut> {
    if pcm.is_empty() || sample_rate == 0 {
        return Vec::new();
    }

    let env = LevelEnvelope::from_mono_i16(pcm, sample_rate, HESITATION_BUCKET_US);
    if env.rms.is_empty() {
        return Vec::new();
    }

    let media_duration = ((pcm.len() as i128 * SECOND as i128) / sample_rate as i128) as Micros;
    let voiced_min_rms = (env.noise_floor() as u32 * 3).clamp(250, 12_000) as u16;
    let max_zcr = (sample_rate / 6) as u64;

    let mut cuts = Vec::new();

    for gap in wordless_gaps(transcript, media_duration) {
        if gap.len() < MIN_HESITATION_US {
            continue;
        }
        let Some((b0, b1)) = env.bucket_bounds(gap) else {
            continue;
        };

        let mut i = b0;
        while i < b1 {
            if env.rms[i] >= voiced_min_rms {
                let run_b0 = i;
                let mut run_b1 = i + 1;
                let mut j = run_b1;
                // Bridge at most 1 single `10 ms` bucket micro-dip if immediately resumed.
                while j < b1 {
                    if env.rms[j] >= voiced_min_rms {
                        run_b1 = j + 1;
                        j += 1;
                    } else if j == run_b1 && j + 1 < b1 && env.rms[j + 1] >= voiced_min_rms {
                        j += 1;
                    } else {
                        break;
                    }
                }

                let start_us = (run_b0 as Micros * HESITATION_BUCKET_US).max(gap.start);
                let end_us = (run_b1 as Micros * HESITATION_BUCKET_US).min(gap.end);
                let dur_us = end_us - start_us;

                if (MIN_HESITATION_US..=MAX_HESITATION_US).contains(&dur_us) {
                    let s0 = ((start_us as u128 * sample_rate as u128) / SECOND as u128) as usize;
                    let s1 = (((end_us as u128 * sample_rate as u128) / SECOND as u128) as usize)
                        .min(pcm.len());

                    if s1 > s0 + 1 {
                        // Zero-crossing rate check: steady vowel/nasal < sample_rate / 6.
                        let mut crossings: u64 = 0;
                        for w in pcm[s0..s1].windows(2) {
                            if (w[0] >= 0 && w[1] < 0) || (w[0] < 0 && w[1] >= 0) {
                                crossings += 1;
                            }
                        }
                        let zcr = crossings * sample_rate as u64 / (s1 - s0) as u64;

                        if crossings > 0 && zcr < max_zcr {
                            // Low amplitude variance check on core buckets (excluding 1
                            // attack/release edge bucket on each side when long enough).
                            let slice = &env.rms[run_b0..run_b1];
                            let core = if slice.len() >= 4 {
                                &slice[1..slice.len() - 1]
                            } else {
                                slice
                            };
                            let mean = core.iter().map(|&v| v as u64).sum::<u64>()
                                / core.len().max(1) as u64;
                            if mean > 0 {
                                let var = core
                                    .iter()
                                    .map(|&v| {
                                        let d = v as i64 - mean as i64;
                                        (d * d) as u64
                                    })
                                    .sum::<u64>()
                                    / core.len() as u64;
                                let std_dev = var.isqrt();
                                let cv_permille = std_dev * 1_000 / mean;
                                if cv_permille <= 450 {
                                    cuts.push(SourceCut {
                                        media: transcript.media,
                                        range: Range::new(start_us, end_us),
                                    });
                                }
                            }
                        }
                    }
                }

                i = run_b1;
            } else {
                i += 1;
            }
        }
    }

    cuts
}

// ============================================================================
// 5. Waveform Cross-Correlation Audio Sync (`MulticamSyncMethod::Waveform`)
// ============================================================================

/// Resamples an envelope's RMS series onto `step_us` buckets when two envelopes use
/// different bucket widths.
fn resample_rms(env: &LevelEnvelope, step_us: Micros) -> Vec<u16> {
    if env.bucket_us == step_us || env.rms.is_empty() || step_us <= 0 {
        return env.rms.clone();
    }
    let total_us = env.duration_us();
    let count = ((total_us + step_us - 1) / step_us) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let r = Range::new(i as Micros * step_us, (i as Micros + 1) * step_us);
        out.push(env.mean_rms_in(r));
    }
    out
}

/// Finds the lag in `[-max_lag_us, +max_lag_us]` maximizing normalized integer
/// cross-correlation between mean-centered `reference` and `target` RMS envelopes.
///
/// Sign convention matches [`crate::Timeline`] clip placement: a returned `offset` means an
/// acoustic event at `t_tgt` in `target` corresponds to `t_ref = t_tgt + offset` in
/// `reference` (so placing `target` at timeline `at = offset` aligns external mic audio
/// with a camera clip starting at `0`).
pub fn sync_offset_by_envelope(
    reference: &LevelEnvelope,
    target: &LevelEnvelope,
    max_lag_us: Micros,
) -> Micros {
    if reference.rms.len() < 2
        || target.rms.len() < 2
        || reference.bucket_us <= 0
        || target.bucket_us <= 0
    {
        return 0;
    }

    let step_us = reference.bucket_us.min(target.bucket_us);
    let ref_cow;
    let tgt_cow;
    let ref_rms: &[u16] = if reference.bucket_us == step_us {
        &reference.rms
    } else {
        ref_cow = resample_rms(reference, step_us);
        &ref_cow
    };
    let tgt_rms: &[u16] = if target.bucket_us == step_us {
        &target.rms
    } else {
        tgt_cow = resample_rms(target, step_us);
        &tgt_cow
    };

    if ref_rms.len() < 2 || tgt_rms.len() < 2 {
        return 0;
    }

    // Scale by 4 before integer mean subtraction to preserve quarter-LSB mean precision.
    let ref_sum: i64 = ref_rms.iter().map(|&v| (v as i64) * 4).sum();
    let tgt_sum: i64 = tgt_rms.iter().map(|&v| (v as i64) * 4).sum();
    let ref_mean = ref_sum / ref_rms.len() as i64;
    let tgt_mean = tgt_sum / tgt_rms.len() as i64;

    let ref_centered: Vec<i64> = ref_rms.iter().map(|&v| (v as i64) * 4 - ref_mean).collect();
    let tgt_centered: Vec<i64> = tgt_rms.iter().map(|&v| (v as i64) * 4 - tgt_mean).collect();

    let max_lag_steps = (max_lag_us.max(0) / step_us) as isize;
    let ref_len = ref_centered.len() as isize;
    let tgt_len = tgt_centered.len() as isize;
    let min_overlap = (ref_centered.len().min(tgt_centered.len()) / 4).max(2);

    let mut best_lag: isize = 0;
    let mut best_score: i128 = i128::MIN;

    for lag in -max_lag_steps..=max_lag_steps {
        // `i_ref = i_tgt + lag`
        let j_start = (-lag).max(0) as usize;
        let j_end = (ref_len - lag).min(tgt_len).max(0) as usize;
        if j_end <= j_start || (j_end - j_start) < min_overlap {
            continue;
        }

        let i_start = (j_start as isize + lag) as usize;
        let overlap = j_end - j_start;
        let r_slice = &ref_centered[i_start..i_start + overlap];
        let t_slice = &tgt_centered[j_start..j_end];

        let mut dot: i128 = 0;
        let mut norm_r: u128 = 0;
        let mut norm_t: u128 = 0;
        for k in 0..overlap {
            let rv = r_slice[k];
            let tv = t_slice[k];
            dot += rv as i128 * tv as i128;
            norm_r += (rv * rv) as u128;
            norm_t += (tv * tv) as u128;
        }

        let denom = (norm_r * norm_t).isqrt();
        if denom == 0 {
            continue;
        }

        // Fixed-point correlation score in parts-per-billion (`[-1e9, +1e9]`).
        let score = (dot * 1_000_000_000i128) / denom as i128;
        if score > best_score || (score == best_score && lag.abs() < best_lag.abs()) {
            best_score = score;
            best_lag = lag;
        }
    }

    best_lag as Micros * step_us
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    /// Deterministic integer triangle-wave oscillator at `freq_hz` with peak `amp`
    /// (`0..=32767`) — pure integer `#![no_std]` test signal synthesis!
    fn synth_triangle_tone(
        out: &mut [i16],
        sample_rate: u32,
        freq_hz: u32,
        amp: i32,
        phase_num: u64,
    ) {
        let sr = sample_rate as u64;
        let f = freq_hz.max(1) as u64;
        for (i, s) in out.iter_mut().enumerate() {
            // Phase `p` in `0..4*sr` per cycle.
            let p = ((i as u64 * 4 * f) + phase_num) % (4 * sr);
            let tri = if p < sr {
                (p as i64 * amp as i64) / sr as i64
            } else if p < 3 * sr {
                amp as i64 - ((p - sr) as i64 * amp as i64) / sr as i64
            } else {
                -amp as i64 + ((p - 3 * sr) as i64 * amp as i64) / sr as i64
            };
            *s = (*s as i32 + tri as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    /// Deterministic xorshift32 pseudo-noise bounded to `±amp`.
    fn add_room_noise(out: &mut [i16], amp: i16, mut seed: u32) {
        if amp <= 0 {
            return;
        }
        let span = (amp as i32) * 2 + 1;
        for s in out.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = (seed % span as u32) as i32 - amp as i32;
            *s = (*s as i32 + n).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    fn word(text: &str, start: Micros, end: Micros) -> Word {
        Word {
            text: String::from(text),
            source: Range::new(start, end),
        }
    }

    // ------------------------------------------------------------------------
    // 1. LevelEnvelope & Adaptive Room-Tone Floor Tests
    // ------------------------------------------------------------------------

    #[test]
    fn adaptive_noise_floor_quiet_vs_noisy_rooms_and_extremes() {
        let sr = 16_000u32;
        let bucket_us = 10_000; // 10 ms -> 160 samples/bucket

        // Pure digital silence clamps to `32` rather than collapsing to `0`.
        let silent = vec![0i16; sr as usize * 2];
        let env_silent = LevelEnvelope::from_mono_i16(&silent, sr, bucket_us);
        assert_eq!(env_silent.rms.len(), 200);
        assert_eq!(env_silent.noise_floor(), 32);
        assert_eq!(env_silent.speech_threshold(), 400);

        // `i16::MIN` (-32768) must never overflow `abs` and yields exact 32768 RMS/peak.
        let full_scale = vec![i16::MIN; 320];
        let env_fs = LevelEnvelope::from_mono_i16(&full_scale, sr, bucket_us);
        assert_eq!(env_fs.rms, vec![32768, 32768]);
        assert_eq!(env_fs.peak, vec![32768, 32768]);

        // Quiet studio room (~60 peak noise) with loud speech bursts (70% speech, 30% pauses).
        let mut quiet_room = vec![0i16; sr as usize * 4]; // 4s = 400 buckets
        add_room_noise(&mut quiet_room, 90, 0x1234_5678);
        synth_triangle_tone(&mut quiet_room[0..sr as usize], sr, 180, 6000, 0);
        synth_triangle_tone(
            &mut quiet_room[(sr as usize * 3 / 2)..(sr as usize * 7 / 2)],
            sr,
            210,
            6500,
            0,
        );
        let env_quiet = LevelEnvelope::from_mono_i16(&quiet_room, sr, bucket_us);

        // Noisy HVAC room (~900 peak noise) with the same speech layout.
        let mut noisy_room = vec![0i16; sr as usize * 4];
        add_room_noise(&mut noisy_room, 900, 0xDEAD_BEEF);
        synth_triangle_tone(&mut noisy_room[0..sr as usize], sr, 180, 6000, 0);
        synth_triangle_tone(
            &mut noisy_room[(sr as usize * 3 / 2)..(sr as usize * 7 / 2)],
            sr,
            210,
            6500,
            0,
        );
        let env_noisy = LevelEnvelope::from_mono_i16(&noisy_room, sr, bucket_us);

        let floor_q = env_quiet.noise_floor();
        let floor_n = env_noisy.noise_floor();
        assert!(
            (40..=80).contains(&floor_q),
            "expected quiet floor ~52, got {floor_q}"
        );
        assert!(
            (450..=600).contains(&floor_n),
            "expected noisy floor ~520, got {floor_n}"
        );
        assert!(env_noisy.speech_threshold() > env_quiet.speech_threshold() * 3);

        // Query helpers `max_peak_in` and `mean_rms_in` distinguish speech vs room-tone gap.
        let speech_rms = env_quiet.mean_rms_in(Range::new(100_000, 900_000));
        let gap_rms = env_quiet.mean_rms_in(Range::new(1_100_000, 1_400_000));
        assert!(speech_rms > 3000, "speech_rms={speech_rms}");
        assert!(gap_rms < 100, "gap_rms={gap_rms}");
        assert!(env_quiet.max_peak_in(Range::new(100_000, 900_000)) >= 5900);
    }

    // ------------------------------------------------------------------------
    // 2. Room-Tone Valley Snapping & Audible-Edge Refusal Tests
    // ------------------------------------------------------------------------

    #[test]
    fn snap_cut_places_edges_on_exact_quiet_frames_and_never_leaks_cut_word() {
        // 30 fps (`frame_idx * 1_000_000 / 30`), 10 ms envelope buckets over 2.0 s.
        let bucket_us = 10_000;
        let mut rms = vec![50u16; 200];
        let mut peak = vec![80u16; 200];

        // Kept word 1: `0..490ms` (`0..49`).
        for b in 0..49 {
            rms[b] = 3200;
            peak[b] = 5500;
        }
        // Pre-cut room-tone pause: `490ms..620ms` (`49..62`), with deepest valley `38`
        // around `560ms..580ms` (near frame 17 = `566_666` µs!).
        for b in 49..62 {
            rms[b] = 110;
        }
        rms[55] = 38;
        rms[56] = 38;
        rms[57] = 38;

        // Filler word `"um"` to remove: `620ms..910ms` (`62..91`).
        for b in 62..91 {
            rms[b] = 3600;
            peak[b] = 6000;
        }

        // Post-cut room-tone pause: `910ms..1050ms` (`91..105`), deepest valley `40`
        // around `990ms..1010ms` (frame 30 = `1_000_000` µs!).
        for b in 91..105 {
            rms[b] = 115;
        }
        rms[99] = 40;
        rms[100] = 40;

        // Kept word 2: `1050ms..1800ms` (`105..180`).
        for b in 105..180 {
            rms[b] = 3400;
            peak[b] = 5800;
        }

        let env = LevelEnvelope {
            bucket_us,
            rms,
            peak,
        };

        // Raw `Transcript::cut_words`-style cut: starts right at `"um"` onset (`620_000`,
        // where naive inward rounding to frame 19 = `633_333` would leak 13.3ms of `"um"`!)
        // and ends slightly inside `"um"` tail (`905_000`, where naive inward floor to
        // frame 27 = `900_000` would leak 5ms of `"um"`!).
        let raw_cut = SourceCut {
            media: 1,
            range: Range::new(620_000, 905_000),
        };

        let (snapped_cut, report) =
            snap_cut_to_quiet_frames(&env, 30, 1, raw_cut, 150_000).expect("should snap cleanly");

        // Verify exact whole-frame alignment at 30 fps (`frame_idx * 1_000_000 / 30`).
        let s_frame = floor_frame_idx(snapped_cut.range.start, 30, 1);
        let e_frame = floor_frame_idx(snapped_cut.range.end, 30, 1);
        assert_eq!(snapped_cut.range.start, frame_time_us(s_frame, 30, 1));
        assert_eq!(snapped_cut.range.end, frame_time_us(e_frame, 30, 1));

        // Must sit strictly inside the two quiet valleys (`490ms..=620ms` and `910ms..=1050ms`)
        // so ZERO microseconds of `"um"` (`620ms..910ms`) survive and neither neighbor word is clipped!
        assert_eq!(snapped_cut.range.start, 566_666); // frame 17 in deepest valley!
        assert_eq!(snapped_cut.range.end, 1_000_000); // frame 30 in deepest valley!
        assert_eq!(
            report.start_moved_us,
            snapped_cut.range.start - raw_cut.range.start
        );
        assert_eq!(
            report.end_moved_us,
            snapped_cut.range.end - raw_cut.range.end
        );
    }

    #[test]
    fn snap_cut_refuses_audible_edge_in_continuous_speech() {
        let bucket_us = 10_000;
        // 30% room tone (`50`) at the end so noise_floor = 50 (`quiet_threshold = 180`),
        // and continuous loud monologue (`3000`) throughout `0..1_400_000`.
        let mut rms = vec![3000u16; 200];
        for r in &mut rms[140..200] {
            *r = 50;
        }
        let env = LevelEnvelope {
            bucket_us,
            peak: rms.clone(),
            rms,
        };

        // Start edge at `500_000` sits inside uninterrupted loud speech -> refuses start edge.
        let bad_start = SourceCut {
            media: 1,
            range: Range::new(500_000, 1_600_000),
        };
        let err = snap_cut_to_quiet_frames(&env, 24, 1, bad_start, 100_000)
            .expect_err("must refuse audible start edge");
        assert_eq!(
            err,
            AudibleEdgeRefusal {
                at_source: 500_000,
                is_end_edge: false,
                rms_at_edge: 3000,
                quiet_threshold: 180,
            }
        );

        // Start edge in quiet (`1_500_000`), end edge in loud speech if we flip layout -> refuses end edge.
        let mut rms2 = vec![3000u16; 200];
        for r in &mut rms2[0..60] {
            *r = 50;
        }
        let env2 = LevelEnvelope {
            bucket_us,
            peak: rms2.clone(),
            rms: rms2,
        };
        let bad_end = SourceCut {
            media: 1,
            range: Range::new(300_000, 1_200_000),
        };
        let err_end = snap_cut_to_quiet_frames(&env2, 24, 1, bad_end, 100_000)
            .expect_err("must refuse audible end edge");
        assert!(err_end.is_end_edge);
        assert_eq!(err_end.at_source, 1_200_000);
        assert_eq!(err_end.rms_at_edge, 3000);
    }

    // ------------------------------------------------------------------------
    // 3. Viterbi CTC Forced Word Alignment Tests
    // ------------------------------------------------------------------------

    #[test]
    fn word_ctc_labels_handles_apostrophes_punctuation_and_tags() {
        // 'E'=0 -> 5, 'T'=1 -> 6, 'A'=2 -> 7
        assert_eq!(word_ctc_labels("Tea!"), Some(vec![6, 5, 7]));
        // Internal apostrophe in "don't" is preserved (`'` is index 22 -> 5 + 22 = 27),
        // while outer single quotes in "'hello'" are stripped.
        let dont = word_ctc_labels("don't,").expect("don't");
        assert_eq!(dont.len(), 5);
        assert_eq!(dont[3], 5 + 22);

        let quoted = word_ctc_labels("'Hi'").expect("quoted Hi");
        assert_eq!(quoted, word_ctc_labels("hi").unwrap());

        assert_eq!(word_ctc_labels("[music]"), None);
        assert_eq!(word_ctc_labels("<laughter>"), None);
        assert_eq!(word_ctc_labels("..."), None);
        assert_eq!(word_ctc_labels("2026"), None);
    }

    #[test]
    fn viterbi_ctc_tightens_drifted_whisper_words_and_trims_stretched_silent_tails() {
        // 100 CTC frames at 20 ms = 2.0 seconds (`0..2_000_000` µs).
        // True acoustics:
        //   - Word 0 `"HELLO"` (note repeated `'L'` testing mandatory blank transition!):
        //     frames `10..=18` (`200_000..380_000` µs)
        //   - Separator / pause: frames `19..=39`
        //   - Word 1 `"WORLD"`: frames `40..=49` (`800_000..1_000_000` µs), where final `'D'`
        //     keeps weak ambiguous logits through frame `69` (`1_400_000` µs) while actual
        //     audio goes completely silent at `1_000_000` µs!
        let mut log_probs = [[-1000i16; 32]; 100];
        for f in &mut log_probs {
            f[CTC_BLANK] = -10;
        }

        let hello_lbls = word_ctc_labels("Hello").unwrap(); // [H, E, L, L, O]
        // Emit H(10), E(11), L(12,13), BLANK(14), L(15,16), O(17,18)
        let schedule_hello = [
            (10, hello_lbls[0]),
            (11, hello_lbls[1]),
            (12, hello_lbls[2]),
            (13, hello_lbls[2]),
            (14, CTC_BLANK),
            (15, hello_lbls[3]),
            (16, hello_lbls[3]),
            (17, hello_lbls[4]),
            (18, hello_lbls[4]),
        ];
        for (fr, cls) in schedule_hello {
            log_probs[fr] = [-1000i16; 32];
            log_probs[fr][cls] = 0;
        }
        log_probs[25][CTC_SEP] = 0;

        let world_lbls = word_ctc_labels("world").unwrap(); // [W, O, R, L, D]
        let schedule_world = [
            (40, world_lbls[0]),
            (41, world_lbls[0]),
            (42, world_lbls[1]),
            (43, world_lbls[1]),
            (44, world_lbls[2]),
            (45, world_lbls[2]),
            (46, world_lbls[3]),
            (47, world_lbls[3]),
            (48, world_lbls[4]),
            (49, world_lbls[4]),
        ];
        for (fr, cls) in schedule_world {
            log_probs[fr] = [-1000i16; 32];
            log_probs[fr][cls] = 0;
        }
        // Simulate stretched CTC tail on `'D'` across silent frames `50..65` (`1.0s..1.32s`).
        for fr in 50..=65 {
            log_probs[fr][world_lbls[4]] = -5;
            log_probs[fr][CTC_BLANK] = -20;
        }

        // Level envelope (10ms buckets, 200 buckets = 2.0s): audible ONLY on `200ms..380ms`
        // and `800ms..1000ms`; pure room tone (`45`) everywhere else!
        let mut rms = vec![45u16; 200];
        for b in 20..38 {
            rms[b] = 2800;
        }
        for b in 80..100 {
            rms[b] = 3100;
        }
        let env = LevelEnvelope {
            bucket_us: 10_000,
            peak: rms.clone(),
            rms,
        };

        // Sloppy/drifted Whisper input timestamps (shifted by hundreds of ms + 2s silence stretch!).
        let drifted_words = vec![
            word("Hello,", 0, 750_000),
            word("world!", 750_000, 2_000_000),
        ];

        let aligned = ctc_align_words(
            &log_probs,
            DEFAULT_CTC_FRAME_US,
            7,
            &drifted_words,
            Some(&env),
        );
        assert_eq!(aligned.media, 7);
        assert_eq!(aligned.words[0].source, Range::new(200_000, 380_000));
        // Even though CTC kept emitting `'D'` through frame 65 (`1_320_000`), `env` trims the
        // silent tail cleanly back to `1_000_000`!
        assert_eq!(aligned.words[1].source, Range::new(800_000, 1_000_000));

        // Graceful fallback when `log_probs` is shorter than the label sequence requires.
        let fallback = ctc_align_words(
            &log_probs[..3],
            DEFAULT_CTC_FRAME_US,
            7,
            &drifted_words,
            Some(&env),
        );
        assert_eq!(fallback.words, drifted_words);
    }

    // ------------------------------------------------------------------------
    // 4. Unexplained Speech Holes & Acoustic Hesitation (`uhm`) Tests
    // ------------------------------------------------------------------------

    #[test]
    fn finds_skipped_speech_holes_in_leading_interword_and_trailing_gaps() {
        // 5.0 seconds (`500` x 10ms buckets), baseline room tone = `50` (`speech_threshold = 400`).
        let mut rms = vec![50u16; 500];
        // Leading skipped remark: `100ms..300ms` (200ms >= 120ms) -> hole!
        for b in 10..30 {
            rms[b] = 2200;
        }
        // Transcribed word 1: `700ms..1_100ms`.
        for b in 70..110 {
            rms[b] = 3000;
        }
        // Brief 60ms mic click at `1_500ms..1_560ms` (< 120ms) -> NOT a speech hole!
        for b in 150..156 {
            rms[b] = 2500;
        }
        // Skipped remark after long pause at `2_000ms..2_350ms` (with a 20ms stop-consonant dip
        // at `2_150ms..2_170ms` bridged into a single 350ms hole!).
        for b in 200..215 {
            rms[b] = 2600;
        }
        for b in 217..235 {
            rms[b] = 2400;
        }
        // Transcribed word 2: `3_000ms..3_400ms`.
        for b in 300..340 {
            rms[b] = 3100;
        }
        // Trailing skipped remark: `4_200ms..4_500ms` (300ms) -> hole!
        for b in 420..450 {
            rms[b] = 1900;
        }

        let env = LevelEnvelope {
            bucket_us: 10_000,
            peak: rms.clone(),
            rms,
        };
        let transcript = Transcript {
            media: 3,
            words: vec![
                word("first", 700_000, 1_100_000),
                word("second", 3_000_000, 3_400_000),
            ],
        };

        let holes = find_unexplained_speech_holes(&transcript, &env, 5_000_000, 500_000);
        assert_eq!(
            holes,
            vec![
                Range::new(100_000, 300_000),
                Range::new(2_000_000, 2_350_000),
                Range::new(4_200_000, 4_500_000),
            ]
        );
    }

    #[test]
    fn detects_voiced_acoustic_hesitations_while_rejecting_clicks_sibilants_and_modulated_speech() {
        let sr = 16_000u32;
        // 5.0 s buffer (`80_000` samples) with gentle room tone.
        let mut pcm = vec![0i16; sr as usize * 5];
        add_room_noise(&mut pcm, 45, 0xCAFE_BABE);

        // Spoken word 1: `200ms..600ms`.
        synth_triangle_tone(
            &mut pcm[(sr as usize / 5)..(sr as usize * 3 / 5)],
            sr,
            190,
            5000,
            0,
        );

        // Gap candidate A (`900ms..1_220ms`, 320 ms): genuine steady voiced `"uhm"`
        // (`f0 = 130 Hz` + nasal formant `390 Hz`, low ZCR, steady envelope) -> DETECT!
        let uhm_range = (sr as usize * 9 / 10)..(sr as usize * 122 / 100);
        synth_triangle_tone(&mut pcm[uhm_range.clone()], sr, 130, 2600, 0);
        synth_triangle_tone(&mut pcm[uhm_range], sr, 390, 900, 0);

        // Gap candidate B (`1_500ms..1_550ms`, 50 ms): short click/pop -> REJECT (< 120 ms)!
        synth_triangle_tone(
            &mut pcm[(sr as usize * 150 / 100)..(sr as usize * 155 / 100)],
            sr,
            150,
            4000,
            0,
        );

        // Gap candidate C (`1_800ms..2_100ms`, 300 ms): unvoiced sibilant `"sss"` / hiss
        // (`3_500 Hz` -> ZCR = `7_000 >= sr / 6 = 2_666`) -> REJECT (high ZCR)!
        synth_triangle_tone(
            &mut pcm[(sr as usize * 180 / 100)..(sr as usize * 210 / 100)],
            sr,
            3500,
            2500,
            0,
        );

        // Gap candidate D (`2_400ms..2_900ms`, 500 ms): strongly amplitude-modulated
        // syllabic speech (`6000 -> 450 -> 6500 -> 450 -> 6000`) -> REJECT (high variance)!
        for (k, amp) in [6000, 450, 6500, 450, 6000].into_iter().enumerate() {
            let s0 = sr as usize * (240 + k * 10) / 100;
            let s1 = sr as usize * (240 + (k + 1) * 10) / 100;
            synth_triangle_tone(&mut pcm[s0..s1], sr, 160, amp, 0);
        }

        // Spoken word 2: `3_400ms..3_900ms`.
        synth_triangle_tone(
            &mut pcm[(sr as usize * 34 / 10)..(sr as usize * 39 / 10)],
            sr,
            185,
            5200,
            0,
        );

        let transcript = Transcript {
            media: 9,
            words: vec![
                word("Well", 200_000, 600_000),
                word("actually", 3_400_000, 3_900_000),
            ],
        };

        let hesitations = detect_acoustic_hesitations(&pcm, sr, &transcript);
        assert_eq!(hesitations.len(), 1, "got: {hesitations:?}");
        assert_eq!(hesitations[0].media, 9);
        assert!(
            (hesitations[0].range.start - 900_000).abs() <= 10_000
                && (hesitations[0].range.end - 1_220_000).abs() <= 10_000,
            "unexpected hesitation span: {:?}",
            hesitations[0].range
        );
    }

    // ------------------------------------------------------------------------
    // 5. Waveform Cross-Correlation Audio Sync Tests
    // ------------------------------------------------------------------------

    #[test]
    fn sync_offset_recovers_exact_positive_and_negative_micros_lag_across_mono_and_stereo() {
        let sr = 16_000u32;
        let bucket_us = 10_000; // 10 ms

        // Build a 3.0s asymmetric speech-like cadence so lag is uniquely identifiable.
        let mut base = vec![0i16; sr as usize * 3];
        add_room_noise(&mut base, 40, 0x1111_2222);
        for &(start_ms, end_ms, freq, amp) in &[
            (350usize, 520usize, 170u32, 3000i32),
            (680, 990, 210, 5200),
            (1320, 1460, 150, 2400),
            (1850, 2280, 195, 4600),
        ] {
            let s0 = sr as usize * start_ms / 1000;
            let s1 = sr as usize * end_ms / 1000;
            synth_triangle_tone(&mut base[s0..s1], sr, freq, amp, 0);
        }

        // Camera scratch mic (`reference`): quiet mono gain (`0.5x`) + noisier camera preamps,
        // starting at `t = 0`.
        let mut cam_mono: Vec<i16> = base.iter().map(|&s| s / 2).collect();
        add_room_noise(&mut cam_mono, 180, 0x9999_AAAA);
        let env_cam = LevelEnvelope::from_mono_i16(&cam_mono, sr, bucket_us);

        // External stereo recorder (`target`) started `230_000` µs (230 ms = 23 buckets) LATER
        // than camera (`t_ref = t_tgt + 230_000`) with louder gain (`1.5x`).
        let shift_samples = (sr as usize * 230) / 1000;
        let ext_stereo: Vec<[i16; 2]> = base[shift_samples..]
            .iter()
            .map(|&s| {
                let hot = ((s as i32 * 3) / 2).clamp(-32767, 32767) as i16;
                [hot, hot]
            })
            .collect();
        let env_ext = LevelEnvelope::from_stereo_i16(&ext_stereo, sr, bucket_us);

        let offset_pos = sync_offset_by_envelope(&env_cam, &env_ext, 600_000);
        assert_eq!(offset_pos, 230_000);

        // Reversing reference and target recovers exact `-230_000` µs lag!
        let offset_neg = sync_offset_by_envelope(&env_ext, &env_cam, 600_000);
        assert_eq!(offset_neg, -230_000);

        // Also recovers exact lag when bucket widths differ (10 ms vs 20 ms).
        let env_ext_20ms = LevelEnvelope::from_stereo_i16(&ext_stereo, sr, 20_000);
        let offset_mixed = sync_offset_by_envelope(&env_cam, &env_ext_20ms, 600_000);
        assert!((offset_mixed - 230_000).abs() <= 10_000, "got {offset_mixed}");
    }
}
