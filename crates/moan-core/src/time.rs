// SPDX-License-Identifier: GPL-3.0-only
//! Exact rational musical time, time signatures, swing, and analytical
//! TempoMap conversions at Moan's 48 kHz sample clock.

use crate::{
    dsp_math::{abs_f32, exp_f32, ln_f32},
    Error, RATE,
};
use alloc::vec::Vec;
use core::{
    cmp::Ordering,
    hash::{Hash, Hasher},
};

/// Maximum reduced denominator for exact rational quarter-note `Time`.
/// 15_360 = 16 * 960 = 2^10 * 3 * 5, covering 960-PPQ ticks, tuplets (1/3, 1/6, 1/12, 1/5),
/// and 64th rolls (1/16 beat) or finer subdivisions (1/64 beat) without drift.
pub const MAX_DENOMINATOR: u32 = 15_360;

/// Standard MIDI display / export pulses per quarter note.
pub const DISPLAY_PPQ: i64 = 960;

/// Minimum supported tempo in hundredths of a BPM (10.00 BPM).
pub const MIN_BPM_X100: u32 = 1_000;

/// Maximum supported tempo in hundredths of a BPM (999.00 BPM).
pub const MAX_BPM_X100: u32 = 99_900;

/// Maximum global swing percentage delaying odd 16th subdivisions.
pub const MAX_SWING_PCT: u8 = 75;

const MAX_TEMPO_SEGMENTS: usize = 4096;
const MAX_TIME_SIGNATURES: usize = 1024;

#[inline]
const fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[inline]
const fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[inline]
fn floor_f64(x: f64) -> f64 {
    if x.is_nan() || x >= 4_503_599_627_370_496.0 || x <= -4_503_599_627_370_496.0 {
        return x;
    }
    let i = x as i64;
    let fi = i as f64;
    if x < fi {
        fi - 1.0
    } else {
        fi
    }
}

#[inline]
fn round_f64(x: f64) -> f64 {
    floor_f64(x + 0.5)
}

/// Exact reduced rational quarter-note time (`num / den` quarter-note beats).
#[derive(Clone, Copy, Debug)]
pub struct Time {
    pub num: i64,
    pub den: u32,
}

impl Default for Time {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Time {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const ONE: Self = Self { num: 1, den: 1 };
    pub const QUARTER: Self = Self { num: 1, den: 1 };
    pub const EIGHTH: Self = Self { num: 1, den: 2 };
    pub const SIXTEENTH: Self = Self { num: 1, den: 4 };
    pub const THIRTY_SECOND: Self = Self { num: 1, den: 8 };
    pub const SIXTY_FOURTH: Self = Self { num: 1, den: 16 };
    pub const MIN_BEAT_STEP: Self = Self { num: 1, den: 64 };
    pub const TRIPLET_QUARTER: Self = Self { num: 2, den: 3 };
    pub const TRIPLET_EIGHTH: Self = Self { num: 1, den: 3 };
    pub const TRIPLET_SIXTEENTH: Self = Self { num: 1, den: 6 };
    pub const TRIPLET_THIRTY_SECOND: Self = Self { num: 1, den: 12 };

    /// Construct an exact reduced `Time`, returning `None` if `den == 0` or the
    /// reduced denominator exceeds `MAX_DENOMINATOR`.
    pub fn new(num: i64, den: u32) -> Option<Self> {
        if den == 0 {
            return None;
        }
        if num == 0 {
            return Some(Self::ZERO);
        }
        let abs_num = num.unsigned_abs();
        let g = gcd_u64(abs_num, den as u64);
        let r_den = (den as u64) / g;
        if r_den > MAX_DENOMINATOR as u64 {
            return None;
        }
        let r_num = (num as i128) / (g as i128);
        let r_num = i64::try_from(r_num).ok()?;
        Some(Self {
            num: r_num,
            den: r_den as u32,
        })
    }

    #[inline]
    pub const fn from_beats(beats: i64) -> Self {
        Self { num: beats, den: 1 }
    }

    #[inline]
    pub const fn num(self) -> i64 {
        self.num
    }

    #[inline]
    pub const fn den(self) -> u32 {
        self.den
    }

    /// Exact conversion from ticks at arbitrary PPQ; fails if reduced denominator > `MAX_DENOMINATOR`.
    pub fn from_ticks(ticks: i64, ppq: u32) -> Option<Self> {
        Self::new(ticks, ppq)
    }

    /// Convert ticks at arbitrary PPQ to `Time`, keeping exactness whenever the reduced
    /// denominator fits within `MAX_DENOMINATOR`, otherwise quantizing to `MAX_DENOMINATOR`.
    pub fn from_ticks_nearest(ticks: i64, ppq: u32) -> Option<Self> {
        if ppq == 0 {
            return None;
        }
        if let Some(exact) = Self::new(ticks, ppq) {
            return Some(exact);
        }
        let scaled = (ticks as i128).checked_mul(MAX_DENOMINATOR as i128)?;
        let half = (ppq as i128) / 2;
        let rounded = if scaled >= 0 {
            (scaled + half) / (ppq as i128)
        } else {
            (scaled - half) / (ppq as i128)
        };
        let r_num = i64::try_from(rounded).ok()?;
        Self::new(r_num, MAX_DENOMINATOR)
    }

    /// Exact conversion from 960-PPQ display ticks.
    pub fn from_display_ticks(ticks: i64) -> Self {
        // DISPLAY_PPQ (960) <= MAX_DENOMINATOR (15360), so this always reduces validly.
        let abs_num = ticks.unsigned_abs();
        if abs_num == 0 {
            return Self::ZERO;
        }
        let g = gcd_u64(abs_num, DISPLAY_PPQ as u64);
        Self {
            num: ((ticks as i128) / (g as i128)) as i64,
            den: ((DISPLAY_PPQ as u64) / g) as u32,
        }
    }

    /// Convert to rounded ticks at `ppq`.
    pub fn to_ticks_rounded(self, ppq: u32) -> Option<i64> {
        if self.den == 0 || ppq == 0 {
            return None;
        }
        let num = (self.num as i128).checked_mul(ppq as i128)?;
        let den = self.den as i128;
        let half = den / 2;
        let rounded = if num >= 0 {
            (num + half) / den
        } else {
            (num - half) / den
        };
        i64::try_from(rounded).ok()
    }

    /// Convert to rounded 960-PPQ display ticks.
    pub fn to_display_ticks(self) -> Option<i64> {
        self.to_ticks_rounded(DISPLAY_PPQ as u32)
    }

    #[inline]
    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    #[inline]
    pub fn is_negative(self) -> bool {
        self.num < 0
    }

    #[inline]
    pub fn to_f64(self) -> f64 {
        if self.den == 0 {
            0.0
        } else {
            self.num as f64 / self.den as f64
        }
    }

    #[inline]
    pub fn to_f32(self) -> f32 {
        self.to_f64() as f32
    }

    /// Canonical `(num, den)` representation; handles malformed `den == 0` defensively.
    #[inline]
    fn canonical(self) -> (i64, u32) {
        if self.den == 0 {
            return (0, 0);
        }
        if self.num == 0 {
            return (0, 1);
        }
        let g = gcd_u64(self.num.unsigned_abs(), self.den as u64);
        (
            ((self.num as i128) / (g as i128)) as i64,
            ((self.den as u64) / g) as u32,
        )
    }

    fn from_i128_reduced(num: i128, den: u128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        if num == 0 {
            return Some(Self::ZERO);
        }
        let g = gcd_u128(num.unsigned_abs(), den);
        let r_den = den / g;
        if r_den > MAX_DENOMINATOR as u128 {
            return None;
        }
        let r_num = num / (g as i128);
        let r_num = i64::try_from(r_num).ok()?;
        Some(Self {
            num: r_num,
            den: r_den as u32,
        })
    }

    fn from_i128_quantized(num: i128, den: u128) -> Option<Self> {
        if let Some(exact) = Self::from_i128_reduced(num, den) {
            return Some(exact);
        }
        if den == 0 {
            return None;
        }
        let scaled = num.checked_mul(MAX_DENOMINATOR as i128)?;
        let den_i = i128::try_from(den).ok()?;
        let half = den_i / 2;
        let q = if scaled >= 0 {
            (scaled + half) / den_i
        } else {
            (scaled - half) / den_i
        };
        Self::new(i64::try_from(q).ok()?, MAX_DENOMINATOR)
    }

    /// Exact checked addition with GCD reduction.
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        if self.den == 0 || rhs.den == 0 {
            return None;
        }
        let g = gcd_u64(self.den as u64, rhs.den as u64) as i128;
        let d1 = (self.den as i128) / g;
        let d2 = (rhs.den as i128) / g;
        let num = (self.num as i128)
            .checked_mul(d2)?
            .checked_add((rhs.num as i128).checked_mul(d1)?)?;
        let den = (self.den as u128).checked_mul(d2 as u128)?;
        Self::from_i128_reduced(num, den)
    }

    /// Addition that falls back to `MAX_DENOMINATOR` quantization if exact LCM > `MAX_DENOMINATOR`.
    pub fn saturating_quantized_add(self, rhs: Self) -> Option<Self> {
        if let Some(exact) = self.checked_add(rhs) {
            return Some(exact);
        }
        if self.den == 0 || rhs.den == 0 {
            return None;
        }
        let g = gcd_u64(self.den as u64, rhs.den as u64) as i128;
        let d1 = (self.den as i128) / g;
        let d2 = (rhs.den as i128) / g;
        let num = (self.num as i128)
            .checked_mul(d2)?
            .checked_add((rhs.num as i128).checked_mul(d1)?)?;
        let den = (self.den as u128).checked_mul(d2 as u128)?;
        Self::from_i128_quantized(num, den)
    }

    /// Exact checked subtraction with GCD reduction.
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        if self.den == 0 || rhs.den == 0 {
            return None;
        }
        let g = gcd_u64(self.den as u64, rhs.den as u64) as i128;
        let d1 = (self.den as i128) / g;
        let d2 = (rhs.den as i128) / g;
        let num = (self.num as i128)
            .checked_mul(d2)?
            .checked_sub((rhs.num as i128).checked_mul(d1)?)?;
        let den = (self.den as u128).checked_mul(d2 as u128)?;
        Self::from_i128_reduced(num, den)
    }

    /// Exact integer multiplication with GCD reduction.
    pub fn mul_int(self, rhs: i64) -> Option<Self> {
        if self.den == 0 {
            return None;
        }
        if rhs == 0 || self.num == 0 {
            return Some(Self::ZERO);
        }
        let g = gcd_u64(rhs.unsigned_abs(), self.den as u64);
        let r_den = (self.den as u128) / (g as u128);
        let rhs_reduced = (rhs as i128) / (g as i128);
        let r_num = (self.num as i128).checked_mul(rhs_reduced)?;
        Self::from_i128_reduced(r_num, r_den)
    }

    /// Exact integer division with GCD reduction.
    pub fn div_int(self, rhs: i64) -> Option<Self> {
        if self.den == 0 || rhs == 0 {
            return None;
        }
        if self.num == 0 {
            return Some(Self::ZERO);
        }
        let abs_rhs = rhs.unsigned_abs() as u128;
        let sign = if rhs < 0 { -1i128 } else { 1i128 };
        let g = gcd_u128(self.num.unsigned_abs() as u128, abs_rhs);
        let r_num = ((self.num as i128) / (g as i128)).checked_mul(sign)?;
        let r_den = (self.den as u128).checked_mul(abs_rhs / g)?;
        Self::from_i128_reduced(r_num, r_den)
    }

    /// Exact fractional scaling `(self * num) / den`, returning `None` if `den == 0`
    /// or reduced denominator exceeds `MAX_DENOMINATOR`.
    pub fn checked_mul_frac(self, num: i64, den: u32) -> Option<Self> {
        if self.den == 0 || den == 0 {
            return None;
        }
        if self.num == 0 || num == 0 {
            return Some(Self::ZERO);
        }
        let total_num = (self.num as i128).checked_mul(num as i128)?;
        let total_den = (self.den as u128).checked_mul(den as u128)?;
        Self::from_i128_reduced(total_num, total_den)
    }

    /// Fractional scaling with fallback quantization to `MAX_DENOMINATOR`.
    pub fn mul_frac_nearest(self, num: i64, den: u32) -> Option<Self> {
        if self.den == 0 || den == 0 {
            return None;
        }
        if self.num == 0 || num == 0 {
            return Some(Self::ZERO);
        }
        let total_num = (self.num as i128).checked_mul(num as i128)?;
        let total_den = (self.den as u128).checked_mul(den as u128)?;
        Self::from_i128_quantized(total_num, total_den)
    }

    pub fn abs(self) -> Option<Self> {
        let num = self.num.checked_abs()?;
        Self::new(num, self.den)
    }
}

impl PartialEq for Time {
    fn eq(&self, other: &Self) -> bool {
        self.canonical() == other.canonical()
    }
}

impl Eq for Time {}

impl Hash for Time {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (n, d) = self.canonical();
        n.hash(state);
        d.hash(state);
    }
}

impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.den == 0, other.den == 0) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => {
                let lhs = (self.num as i128) * (other.den as i128);
                let rhs = (other.num as i128) * (self.den as i128);
                lhs.cmp(&rhs)
            }
        }
    }
}

/// Musical time signature (`numerator / denominator`, e.g. `4/4`, `3/4`, `5/4`, `6/8`, `7/8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self::FOUR_FOUR
    }
}

impl TimeSignature {
    pub const FOUR_FOUR: Self = Self {
        numerator: 4,
        denominator: 4,
    };
    pub const THREE_FOUR: Self = Self {
        numerator: 3,
        denominator: 4,
    };
    pub const FIVE_FOUR: Self = Self {
        numerator: 5,
        denominator: 4,
    };
    pub const SIX_EIGHT: Self = Self {
        numerator: 6,
        denominator: 8,
    };
    pub const SEVEN_EIGHT: Self = Self {
        numerator: 7,
        denominator: 8,
    };

    pub fn new(numerator: u8, denominator: u8) -> Result<Self, Error> {
        if numerator == 0 || denominator == 0 || !denominator.is_power_of_two() || denominator > 64
        {
            return Err(Error::Range);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn is_valid(&self) -> bool {
        self.numerator > 0
            && self.denominator > 0
            && self.denominator.is_power_of_two()
            && self.denominator <= 64
    }

    /// Exact bar duration in quarter-note beats (`numerator * 4 / denominator`).
    pub fn beats_per_bar(&self) -> Time {
        if !self.is_valid() {
            return Time::from_beats(4);
        }
        Time::new(self.numerator as i64 * 4, self.denominator as u32)
            .unwrap_or_else(|| Time::from_beats(4))
    }

    /// Power-of-two exponent `log2(denominator)` for SMF `FF 58` serialization.
    pub fn denominator_log2(&self) -> u8 {
        self.denominator.trailing_zeros() as u8
    }
}

/// Tempo anchor or linear BPM ramp segment starting at `start` quarter-note beats.
/// When `ramp_to_bpm_x100` is `Some(target)` and a subsequent segment exists at `next_start`,
/// BPM varies linearly from `bpm_x100` at `start` to `target` at `next_start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempoSegment {
    pub start: Time,
    pub bpm_x100: u32,
    pub ramp_to_bpm_x100: Option<u32>,
}

impl TempoSegment {
    pub fn validate(&self) -> Result<(), Error> {
        if self.start.is_negative()
            || !(MIN_BPM_X100..=MAX_BPM_X100).contains(&self.bpm_x100)
        {
            return Err(Error::Range);
        }
        if let Some(target) = self.ramp_to_bpm_x100 {
            if !(MIN_BPM_X100..=MAX_BPM_X100).contains(&target) {
                return Err(Error::Range);
            }
        }
        Ok(())
    }
}

/// Piecewise-constant and piecewise-linear-ramp tempo map with time signature tracking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TempoMap {
    segments: Vec<TempoSegment>,
    time_signatures: Vec<(Time, TimeSignature)>,
}

impl Default for TempoMap {
    fn default() -> Self {
        let mut segments = Vec::new();
        let _ = segments.try_reserve_exact(1);
        segments.push(TempoSegment {
            start: Time::ZERO,
            bpm_x100: 12_000,
            ramp_to_bpm_x100: None,
        });
        let mut time_signatures = Vec::new();
        let _ = time_signatures.try_reserve_exact(1);
        time_signatures.push((Time::ZERO, TimeSignature::FOUR_FOUR));
        Self {
            segments,
            time_signatures,
        }
    }
}

impl TempoMap {
    /// Constant-tempo map at `bpm_x100` (e.g. `12000` = `120.00 BPM`) in `4/4`.
    pub fn constant(bpm_x100: u32) -> Result<Self, Error> {
        if !(MIN_BPM_X100..=MAX_BPM_X100).contains(&bpm_x100) {
            return Err(Error::Range);
        }
        let mut segments = Vec::new();
        segments.try_reserve_exact(1).map_err(|_| Error::Memory)?;
        segments.push(TempoSegment {
            start: Time::ZERO,
            bpm_x100,
            ramp_to_bpm_x100: None,
        });
        let mut time_signatures = Vec::new();
        time_signatures
            .try_reserve_exact(1)
            .map_err(|_| Error::Memory)?;
        time_signatures.push((Time::ZERO, TimeSignature::FOUR_FOUR));
        Ok(Self {
            segments,
            time_signatures,
        })
    }

    pub fn new(segments: Vec<TempoSegment>, time_sig: TimeSignature) -> Result<Self, Error> {
        if !time_sig.is_valid() {
            return Err(Error::Range);
        }
        let mut sigs = Vec::new();
        sigs.try_reserve_exact(1).map_err(|_| Error::Memory)?;
        sigs.push((Time::ZERO, time_sig));
        Self::with_time_signatures(segments, sigs)
    }

    pub fn with_time_signatures(
        mut segments: Vec<TempoSegment>,
        mut time_signatures: Vec<(Time, TimeSignature)>,
    ) -> Result<Self, Error> {
        if segments.len() > MAX_TEMPO_SEGMENTS || time_signatures.len() > MAX_TIME_SIGNATURES {
            return Err(Error::Range);
        }
        for seg in &segments {
            seg.validate()?;
        }
        for (at, sig) in &time_signatures {
            if at.is_negative() || !sig.is_valid() {
                return Err(Error::Range);
            }
        }
        if segments.is_empty() {
            segments.try_reserve_exact(1).map_err(|_| Error::Memory)?;
            segments.push(TempoSegment {
                start: Time::ZERO,
                bpm_x100: 12_000,
                ramp_to_bpm_x100: None,
            });
        } else {
            segments.sort_by(|a, b| a.start.cmp(&b.start));
            // Keep latest segment when multiple share identical start beats.
            let mut deduped: Vec<TempoSegment> = Vec::new();
            deduped
                .try_reserve(segments.len())
                .map_err(|_| Error::Memory)?;
            for seg in segments {
                if let Some(last) = deduped.last_mut() {
                    if last.start == seg.start {
                        *last = seg;
                        continue;
                    }
                }
                deduped.push(seg);
            }
            segments = deduped;
        }
        if time_signatures.is_empty() {
            time_signatures
                .try_reserve_exact(1)
                .map_err(|_| Error::Memory)?;
            time_signatures.push((Time::ZERO, TimeSignature::FOUR_FOUR));
        } else {
            time_signatures.sort_by(|a, b| a.0.cmp(&b.0));
            let mut deduped: Vec<(Time, TimeSignature)> = Vec::new();
            deduped
                .try_reserve(time_signatures.len())
                .map_err(|_| Error::Memory)?;
            for (at, sig) in time_signatures {
                if let Some(last) = deduped.last_mut() {
                    if last.0 == at {
                        last.1 = sig;
                        continue;
                    }
                }
                deduped.push((at, sig));
            }
            time_signatures = deduped;
        }
        Ok(Self {
            segments,
            time_signatures,
        })
    }

    pub fn add_segment(&mut self, seg: TempoSegment) -> Result<(), Error> {
        seg.validate()?;
        if let Some(existing) = self.segments.iter_mut().find(|s| s.start == seg.start) {
            *existing = seg;
            return Ok(());
        }
        if self.segments.len() >= MAX_TEMPO_SEGMENTS {
            return Err(Error::Range);
        }
        self.segments.try_reserve(1).map_err(|_| Error::Memory)?;
        let idx = self.segments.partition_point(|s| s.start < seg.start);
        self.segments.insert(idx, seg);
        Ok(())
    }

    pub fn add_time_signature(&mut self, at: Time, sig: TimeSignature) -> Result<(), Error> {
        if at.is_negative() || !sig.is_valid() {
            return Err(Error::Range);
        }
        if let Some(existing) = self.time_signatures.iter_mut().find(|t| t.0 == at) {
            existing.1 = sig;
            return Ok(());
        }
        if self.time_signatures.len() >= MAX_TIME_SIGNATURES {
            return Err(Error::Range);
        }
        self.time_signatures
            .try_reserve(1)
            .map_err(|_| Error::Memory)?;
        let idx = self.time_signatures.partition_point(|t| t.0 < at);
        self.time_signatures.insert(idx, (at, sig));
        Ok(())
    }

    pub fn segments(&self) -> &[TempoSegment] {
        &self.segments
    }

    pub fn time_signatures(&self) -> &[(Time, TimeSignature)] {
        &self.time_signatures
    }

    pub fn time_signature_at(&self, beat: Time) -> TimeSignature {
        if self.time_signatures.is_empty() {
            return TimeSignature::FOUR_FOUR;
        }
        let idx = self.time_signatures.partition_point(|t| t.0 <= beat);
        if idx == 0 {
            self.time_signatures[0].1
        } else {
            self.time_signatures[idx - 1].1
        }
    }

    /// Instantaneous BPM * 100 at `beat`.
    pub fn bpm_x100_at(&self, beat: Time) -> u32 {
        if self.segments.is_empty() {
            return 12_000;
        }
        let b = beat.max(Time::ZERO);
        let idx = self.segments.partition_point(|s| s.start <= b);
        if idx == 0 {
            return self.segments[0].bpm_x100;
        }
        let seg_i = idx - 1;
        let seg = self.segments[seg_i];
        if let (Some(target), Some(next)) = (seg.ramp_to_bpm_x100, self.segments.get(seg_i + 1)) {
            let span = (next.start.to_f64() - seg.start.to_f64()).max(0.0);
            if span > 0.0 {
                let frac = ((b.to_f64() - seg.start.to_f64()) / span).clamp(0.0, 1.0);
                let val = seg.bpm_x100 as f64 + (target as f64 - seg.bpm_x100 as f64) * frac;
                return round_f64(val).clamp(MIN_BPM_X100 as f64, MAX_BPM_X100 as f64) as u32;
            }
        }
        seg.bpm_x100
    }

    /// Apply global `swing_pct` (`0..=75%`) piecewise-linearly within every half-beat
    /// (eighth-note pair), delaying odd 16th subdivisions (`0.25` -> `0.25 + 0.25 * pct/100`)
    /// while keeping all even 16th boundaries fixed and monotonicity strict.
    #[inline]
    pub fn swing_beat_f64(beat: f64, swing_pct: u8) -> f64 {
        if beat <= 0.0 || swing_pct == 0 {
            return beat;
        }
        let pct = swing_pct.min(MAX_SWING_PCT) as f64 / 100.0;
        let pair_idx = floor_f64(beat * 2.0);
        let pair_start = pair_idx * 0.5;
        let local = beat - pair_start; // in [0.0, 0.5)
        let delay = 0.25 * pct;
        let warped = if local <= 0.25 {
            local * ((0.25 + delay) / 0.25)
        } else {
            (0.25 + delay) + (local - 0.25) * ((0.25 - delay) / 0.25)
        };
        pair_start + warped
    }

    /// Exact inverse of `swing_beat_f64`.
    #[inline]
    pub fn unswing_beat_f64(swung: f64, swing_pct: u8) -> f64 {
        if swung <= 0.0 || swing_pct == 0 {
            return swung;
        }
        let pct = swing_pct.min(MAX_SWING_PCT) as f64 / 100.0;
        let pair_idx = floor_f64(swung * 2.0);
        let pair_start = pair_idx * 0.5;
        let local = swung - pair_start; // in [0.0, 0.5)
        let delay = 0.25 * pct;
        let swung_mid = 0.25 + delay;
        let unwarped = if local <= swung_mid {
            local * (0.25 / swung_mid)
        } else {
            0.25 + (local - swung_mid) * (0.25 / (0.25 - delay))
        };
        pair_start + unwarped
    }

    /// Integrate segment duration over `[0, dq]` beats starting at `seg_start` with slope `k` BPM/beat.
    #[inline]
    fn segment_seconds(b0: f64, k: f64, dq: f64) -> f64 {
        if dq <= 0.0 {
            return 0.0;
        }
        if abs_f32(k as f32) < 1e-4 {
            return 60.0 * dq / b0;
        }
        let ratio = (b0 + k * dq) / b0;
        if ratio <= 0.0 {
            return 60.0 * dq / b0;
        }
        (60.0 / k) * (ln_f32(ratio as f32) as f64)
    }

    /// Analytical inverse of `segment_seconds`: elapsed beats `dq` for elapsed seconds `dt`.
    #[inline]
    fn segment_beats_for_seconds(b0: f64, k: f64, dt: f64) -> f64 {
        if dt <= 0.0 {
            return 0.0;
        }
        if abs_f32(k as f32) < 1e-4 {
            return dt * b0 / 60.0;
        }
        let arg = (k * dt / 60.0) as f32;
        b0 * ((exp_f32(arg) as f64) - 1.0) / k
    }

    /// Analytical beat-to-seconds conversion with optional swing.
    pub fn beat_to_seconds(&self, beat: Time, swing_pct: u8) -> f64 {
        if beat <= Time::ZERO {
            return 0.0;
        }
        let target = Self::swing_beat_f64(beat.to_f64(), swing_pct);
        if target <= 0.0 {
            return 0.0;
        }
        if self.segments.is_empty() {
            return 60.0 * target / 120.0;
        }
        let mut secs = 0.0f64;
        let first_start = self.segments[0].start.to_f64();
        if target <= first_start {
            let b0 = self.segments[0].bpm_x100 as f64 / 100.0;
            return 60.0 * target / b0;
        }
        if first_start > 0.0 {
            let b0 = self.segments[0].bpm_x100 as f64 / 100.0;
            secs += 60.0 * first_start / b0;
        }
        for i in 0..self.segments.len() {
            let seg = self.segments[i];
            let s_start = seg.start.to_f64();
            let b0 = seg.bpm_x100 as f64 / 100.0;
            if let Some(next) = self.segments.get(i + 1) {
                let s_end = next.start.to_f64();
                let span = s_end - s_start;
                if span <= 0.0 {
                    continue;
                }
                let k = match seg.ramp_to_bpm_x100 {
                    Some(target_x100) => ((target_x100 as f64 / 100.0) - b0) / span,
                    None => 0.0,
                };
                if target <= s_end {
                    secs += Self::segment_seconds(b0, k, target - s_start);
                    return secs;
                }
                secs += Self::segment_seconds(b0, k, span);
            } else {
                secs += 60.0 * (target - s_start) / b0;
                return secs;
            }
        }
        secs
    }

    /// Exact analytical beat-to-frame conversion at `RATE = 48_000`.
    pub fn beat_to_frame(&self, beat: Time, swing_pct: u8) -> u64 {
        if beat <= Time::ZERO {
            return 0;
        }
        let secs = self.beat_to_seconds(beat, swing_pct);
        if secs <= 0.0 || secs.is_nan() {
            return 0;
        }
        let frames = round_f64(secs * RATE as f64);
        if frames <= 0.0 {
            0
        } else if frames >= u64::MAX as f64 {
            u64::MAX
        } else {
            frames as u64
        }
    }

    /// Exact analytical frame-to-beat conversion at `RATE = 48_000`.
    pub fn frame_to_beat(&self, frame: u64, swing_pct: u8) -> Time {
        if frame == 0 {
            return Time::ZERO;
        }
        let target_secs = frame as f64 / RATE as f64;
        let swung_beat = self.seconds_to_swung_beat_f64(target_secs);
        let beat_f64 = Self::unswing_beat_f64(swung_beat, swing_pct);
        let ticks = round_f64(beat_f64 * MAX_DENOMINATOR as f64);
        let clamped = if ticks <= 0.0 {
            0i64
        } else if ticks >= i64::MAX as f64 {
            i64::MAX
        } else {
            ticks as i64
        };
        Time::new(clamped, MAX_DENOMINATOR).unwrap_or(Time::ZERO)
    }

    fn seconds_to_swung_beat_f64(&self, target_secs: f64) -> f64 {
        if target_secs <= 0.0 {
            return 0.0;
        }
        if self.segments.is_empty() {
            return target_secs * 120.0 / 60.0;
        }
        let mut secs_acc = 0.0f64;
        let first_start = self.segments[0].start.to_f64();
        if first_start > 0.0 {
            let b0 = self.segments[0].bpm_x100 as f64 / 100.0;
            let pre_secs = 60.0 * first_start / b0;
            if target_secs <= pre_secs {
                return target_secs * b0 / 60.0;
            }
            secs_acc += pre_secs;
        }
        for i in 0..self.segments.len() {
            let seg = self.segments[i];
            let s_start = seg.start.to_f64();
            let b0 = seg.bpm_x100 as f64 / 100.0;
            if let Some(next) = self.segments.get(i + 1) {
                let s_end = next.start.to_f64();
                let span = s_end - s_start;
                if span <= 0.0 {
                    continue;
                }
                let k = match seg.ramp_to_bpm_x100 {
                    Some(target_x100) => ((target_x100 as f64 / 100.0) - b0) / span,
                    None => 0.0,
                };
                let seg_secs = Self::segment_seconds(b0, k, span);
                if target_secs <= secs_acc + seg_secs {
                    let dt = target_secs - secs_acc;
                    return s_start + Self::segment_beats_for_seconds(b0, k, dt);
                }
                secs_acc += seg_secs;
            } else {
                let dt = target_secs - secs_acc;
                return s_start + dt * b0 / 60.0;
            }
        }
        0.0
    }

    /// Compute the bar `[bar_start, bar_end)` containing `beat`, honoring any time-signature changes.
    pub fn bar_bounds_at_beat(&self, beat: Time) -> (Time, Time) {
        let target = beat.max(Time::ZERO);
        let mut cur_start = Time::ZERO;
        let sigs = &self.time_signatures;
        if sigs.is_empty() {
            let bar_len = TimeSignature::FOUR_FOUR.beats_per_bar();
            let bar_idx = (target.num as i128 * bar_len.den as i128)
                / (target.den as i128 * bar_len.num as i128);
            let s = bar_len.mul_int(bar_idx as i64).unwrap_or(Time::ZERO);
            let e = s.checked_add(bar_len).unwrap_or(s);
            return (s, e);
        }

        for i in 0..sigs.len() {
            let cur_sig = sigs[i].1;
            let bar_len = cur_sig.beats_per_bar();
            let region_end = sigs.get(i + 1).map(|s| s.0);
            if let Some(reg_end) = region_end {
                if target >= reg_end {
                    cur_start = reg_end;
                    continue;
                }
            }
            let diff = target.checked_sub(cur_start).unwrap_or(Time::ZERO);
            let bar_idx = if diff <= Time::ZERO {
                0
            } else {
                (diff.num as i128 * bar_len.den as i128)
                    / (diff.den as i128 * bar_len.num as i128)
            };
            let offset = bar_len.mul_int(bar_idx as i64).unwrap_or(Time::ZERO);
            let bar_start = cur_start.checked_add(offset).unwrap_or(cur_start);
            let mut bar_end = bar_start.checked_add(bar_len).unwrap_or(bar_start);
            if let Some(reg_end) = region_end {
                if reg_end < bar_end {
                    bar_end = reg_end;
                }
            }
            return (bar_start, bar_end);
        }
        (cur_start, cur_start)
    }

    /// Return the exact next bar boundary strictly greater than `beat`.
    pub fn next_bar_beat(&self, beat: Time) -> Time {
        let (_start, end) = self.bar_bounds_at_beat(beat);
        end
    }

    /// Return the exact next bar boundary `(beat, frame)` strictly after `current_frame`.
    pub fn next_bar_at_frame(&self, current_frame: u64, swing_pct: u8) -> (Time, u64) {
        let current_beat = self.frame_to_beat(current_frame, swing_pct);
        let next_beat = self.next_bar_beat(current_beat);
        let next_frame = self.beat_to_frame(next_beat, swing_pct);
        (next_beat, next_frame)
    }
}
