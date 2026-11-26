// SPDX-License-Identifier: GPL-3.0-only
//! Modern `#![no_std]`, integer/fixed-point TCP Congestion Control (no FPU/SSE):
//! - **CUBIC** (RFC 9438): cubic growth `W_cubic(t) = C*(t-K)^3 + W_max`,
//!   TCP-friendly region, beta=0.7 multiplicative decrease, fast convergence,
//!   and HyStart++ (RFC 9406) slow-start exit.
//! - **BBR**: windowed max-bandwidth filter, min-RTT filter,
//!   Startup / Drain / ProbeBW / ProbeRTT state machine, pacing rate & cwnd.

/// Default Maximum Segment Size (Ethernet 1500 - 20 IPv4 - 20 TCP).
pub const DEFAULT_MSS: u32 = 1460;

/// RFC 6928 Initial Window (10 segments).
pub const INIT_CWND_SEGMENTS: u32 = 10;

/// Minimum congestion window in segments.
pub const MIN_CWND_SEGMENTS: u32 = 2;

/// Maximum congestion window cap (64 MiB) to prevent unbounded integer overflow.
pub const MAX_CWND_BYTES: u32 = 64 * 1024 * 1024;

/// Supported congestion control algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CongestionAlgorithm {
    /// RFC 9438 CUBIC (default).
    Cubic,
    /// Bottleneck Bandwidth and Round-trip propagation time (BBR).
    Bbr,
}

impl Default for CongestionAlgorithm {
    fn default() -> Self {
        Self::Cubic
    }
}

/// Telemetry sample provided on each incoming acknowledgment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AckSample {
    /// Newly acknowledged bytes in this ACK.
    pub acked_bytes: u32,
    /// Latest measured RTT in microseconds (0 if unavailable).
    pub rtt_us: u64,
    /// Current monotonic timestamp in microseconds.
    pub now_us: u64,
    /// Cumulative delivered bytes over the connection lifetime.
    pub delivered_bytes: u64,
    /// Delivery interval in microseconds for the sampled bytes (0 if unavailable).
    pub delivered_interval_us: u64,
    /// Whether the sender was application-limited during this delivery sample.
    pub is_app_limited: bool,
    /// Current estimated bytes in flight.
    pub bytes_in_flight: u32,
}

/// Output control parameters produced by a congestion controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CongestionAction {
    /// Congestion window in bytes.
    pub cwnd: u32,
    /// Slow-start threshold in bytes.
    pub ssthresh: u32,
    /// Recommended pacing rate in bytes per second (0 if unpaced).
    pub pacing_rate_bps: u64,
}

/// Common congestion control interface implemented by CUBIC, BBR, and the enum dispatcher.
pub trait CongestionControl {
    fn on_ack(&mut self, sample: AckSample) -> CongestionAction;
    fn on_loss(&mut self, now_us: u64, bytes_in_flight: u32) -> CongestionAction;
    fn on_timeout(&mut self, now_us: u64) -> CongestionAction;
    fn cwnd(&self) -> u32;
    fn ssthresh(&self) -> u32;
    fn pacing_rate_bps(&self) -> u64;
}

/// Exact floor integer cube root `floor(x^(1/3))` using pure integer binary search.
/// Zero floating-point or SSE instructions.
pub const fn icbrt_u128(x: u128) -> u64 {
    if x == 0 {
        return 0;
    }
    let mut lo: u128 = 0;
    // 2^42 cubed is 2^126 < u128::MAX; upper bound 2_642_245^3 < 2^64,
    // and 4_194_304^3 = 2^66, while (u64::MAX as u128) covers up to 2^128 via 2^42.
    let mut hi: u128 = 1u128 << 42;
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if let Some(m2) = mid.checked_mul(mid) {
            if let Some(m3) = m2.checked_mul(mid) {
                if m3 <= x {
                    lo = mid;
                    continue;
                }
            }
        }
        hi = mid - 1;
    }
    lo as u64
}

// ============================================================================
// CUBIC (RFC 9438) + HyStart++ (RFC 9406)
// ============================================================================

/// HyStart++ minimum RTT samples per round before evaluating delay increase.
const HYSTART_MIN_SAMPLES: u32 = 8;
/// HyStart++ delay threshold lower bound (4 ms in microseconds).
const HYSTART_MIN_THRESH_US: u64 = 4_000;
/// HyStart++ delay threshold upper bound (16 ms in microseconds).
const HYSTART_MAX_THRESH_US: u64 = 16_000;

/// Integer/fixed-point RFC 9438 CUBIC congestion controller with HyStart++ slow-start exit.
#[derive(Clone, Debug)]
pub struct Cubic {
    mss: u32,
    cwnd: u32,
    ssthresh: u32,
    /// `W_max`: congestion window (in bytes) just before the last congestion event.
    w_max: u32,
    /// `W_last_max`: previous `W_max` used for Fast Convergence (RFC 9438 §4.7).
    w_last_max: u32,
    /// `K` in milliseconds: `cbrt(W_max_segs * (1 - beta) / C) * 1000`.
    k_ms: u64,
    /// Start of the current congestion avoidance epoch in microseconds.
    epoch_start_us: Option<u64>,
    /// Minimum observed RTT in microseconds.
    min_rtt_us: u64,
    /// Latest RTT in microseconds.
    srtt_us: u64,
    /// Current pacing rate in bytes/sec.
    pacing_rate_bps: u64,
    /// HyStart++ state:
    hystart_enabled: bool,
    hystart_round_delivered: u64,
    hystart_last_round_min_rtt: u64,
    hystart_curr_round_min_rtt: u64,
    hystart_sample_count: u32,
    hystart_exited: bool,
}

impl Cubic {
    /// Construct a new CUBIC controller with `mss` (defaults to `DEFAULT_MSS` if 0).
    pub const fn new(mss: u32) -> Self {
        let mss = if mss == 0 { DEFAULT_MSS } else { mss };
        let init_cwnd = INIT_CWND_SEGMENTS * mss;
        Self {
            mss,
            cwnd: init_cwnd,
            ssthresh: u32::MAX,
            w_max: 0,
            w_last_max: 0,
            k_ms: 0,
            epoch_start_us: None,
            min_rtt_us: u64::MAX,
            srtt_us: 0,
            pacing_rate_bps: 0,
            hystart_enabled: true,
            hystart_round_delivered: 0,
            hystart_last_round_min_rtt: u64::MAX,
            hystart_curr_round_min_rtt: u64::MAX,
            hystart_sample_count: 0,
            hystart_exited: false,
        }
    }

    /// Enable or disable HyStart++ slow-start exit.
    pub fn set_hystart(&mut self, enabled: bool) {
        self.hystart_enabled = enabled;
    }

    /// Whether HyStart++ triggered an early slow-start exit.
    pub const fn hystart_exited(&self) -> bool {
        self.hystart_exited
    }

    /// Current `W_max` in bytes.
    pub const fn w_max(&self) -> u32 {
        self.w_max
    }

    /// Current `K` in milliseconds.
    pub const fn k_ms(&self) -> u64 {
        self.k_ms
    }

    /// Compute `K` in milliseconds: `cbrt(W_max_segs * (1 - 0.7) / 0.4) * 1000`
    /// = `cbrt(W_max_segs * 750_000_000)`.
    fn compute_k_ms(&self, w_max_bytes: u32) -> u64 {
        let w_max_segs = (w_max_bytes as u128) / (self.mss as u128);
        if w_max_segs == 0 {
            return 0;
        }
        icbrt_u128(w_max_segs.saturating_mul(750_000_000))
    }

    /// Evaluate `W_cubic(t_ms) = W_max + C * (t - K)^3` in bytes (RFC 9438 §4.2).
    /// With `C = 0.4` and `t, K` in milliseconds:
    /// `delta_segs = 4 * (t_ms - K_ms)^3 / 10_000_000_000`.
    pub fn w_cubic_bytes(&self, t_ms: u64) -> u32 {
        let dt: i128 = (t_ms as i128) - (self.k_ms as i128);
        // Clamp |dt| to prevent overflow when cubing (1_000_000 ms = 1000 s -> 10^18).
        let dt_clamped = dt.clamp(-1_000_000, 1_000_000);
        let dt3 = dt_clamped * dt_clamped * dt_clamped;
        let delta_bytes = (dt3.saturating_mul(4).saturating_mul(self.mss as i128)) / 10_000_000_000;
        let w = (self.w_max as i128).saturating_add(delta_bytes);
        let min_w = (MIN_CWND_SEGMENTS * self.mss) as i128;
        w.clamp(min_w, MAX_CWND_BYTES as i128) as u32
    }

    /// Evaluate the RFC 9438 §4.3 TCP-friendly window estimate `W_est(t)` in bytes:
    /// `W_est(t) = W_max * beta + [3 * (1 - beta) / (1 + beta)] * (t / RTT) * MSS`
    /// With `beta = 0.7`, `3 * 0.3 / 1.7 = 9 / 17`.
    pub fn w_est_bytes(&self, t_us: u64, rtt_us: u64) -> u32 {
        let base = ((self.w_max as u128) * 7) / 10;
        if rtt_us == 0 {
            return base.min(MAX_CWND_BYTES as u128) as u32;
        }
        let inc = (9u128)
            .saturating_mul(self.mss as u128)
            .saturating_mul(t_us as u128)
            / (17u128.saturating_mul(rtt_us as u128));
        base.saturating_add(inc)
            .clamp((MIN_CWND_SEGMENTS * self.mss) as u128, MAX_CWND_BYTES as u128) as u32
    }

    fn update_pacing(&mut self) {
        if self.srtt_us == 0 {
            return;
        }
        // 2.0x pacing gain in slow start, 1.2x in congestion avoidance.
        let gain_num: u128 = if self.cwnd < self.ssthresh { 200 } else { 120 };
        let bps = (self.cwnd as u128)
            .saturating_mul(1_000_000)
            .saturating_mul(gain_num)
            / ((self.srtt_us as u128).saturating_mul(100));
        self.pacing_rate_bps = bps.min(u64::MAX as u128) as u64;
    }

    fn hystart_update(&mut self, sample: &AckSample) {
        if !self.hystart_enabled || sample.rtt_us == 0 {
            return;
        }
        // Start a new RTT round when delivered bytes advance past the round boundary.
        if sample.delivered_bytes >= self.hystart_round_delivered {
            if self.hystart_curr_round_min_rtt != u64::MAX {
                self.hystart_last_round_min_rtt = self.hystart_curr_round_min_rtt;
            }
            self.hystart_curr_round_min_rtt = u64::MAX;
            self.hystart_sample_count = 0;
            self.hystart_round_delivered = sample
                .delivered_bytes
                .saturating_add(self.cwnd as u64);
        }

        if sample.rtt_us < self.hystart_curr_round_min_rtt {
            self.hystart_curr_round_min_rtt = sample.rtt_us;
        }
        self.hystart_sample_count = self.hystart_sample_count.saturating_add(1);

        // RFC 9406 §4.2 delay-increase check.
        if self.hystart_sample_count >= HYSTART_MIN_SAMPLES
            && self.hystart_last_round_min_rtt != u64::MAX
        {
            let thresh = (self.hystart_last_round_min_rtt / 8)
                .clamp(HYSTART_MIN_THRESH_US, HYSTART_MAX_THRESH_US);
            if self.hystart_curr_round_min_rtt
                >= self.hystart_last_round_min_rtt.saturating_add(thresh)
            {
                // Exit slow start early!
                self.ssthresh = self.cwnd;
                self.hystart_exited = true;
            }
        }
    }

    fn action(&self) -> CongestionAction {
        CongestionAction {
            cwnd: self.cwnd,
            ssthresh: self.ssthresh,
            pacing_rate_bps: self.pacing_rate_bps,
        }
    }
}

impl CongestionControl for Cubic {
    fn on_ack(&mut self, sample: AckSample) -> CongestionAction {
        if sample.rtt_us > 0 {
            if sample.rtt_us < self.min_rtt_us {
                self.min_rtt_us = sample.rtt_us;
            }
            if self.srtt_us == 0 {
                self.srtt_us = sample.rtt_us;
            } else {
                // RFC 6298 EWMA: srtt = 7/8 * srtt + 1/8 * rtt
                self.srtt_us = (self.srtt_us * 7 + sample.rtt_us) / 8;
            }
        }

        if sample.acked_bytes == 0 {
            self.update_pacing();
            return self.action();
        }

        if self.cwnd < self.ssthresh {
            // Slow Start (with HyStart++ early exit check).
            self.hystart_update(&sample);
            if self.cwnd < self.ssthresh {
                let inc = sample.acked_bytes.min(self.mss.saturating_mul(2));
                self.cwnd = self.cwnd.saturating_add(inc).min(MAX_CWND_BYTES);
            }
        } else {
            // Congestion Avoidance (RFC 9438).
            let epoch_start = match self.epoch_start_us {
                Some(t0) => t0,
                None => {
                    self.epoch_start_us = Some(sample.now_us);
                    if self.cwnd < self.w_max {
                        self.k_ms = self.compute_k_ms(self.w_max);
                    } else {
                        self.k_ms = 0;
                        self.w_max = self.cwnd;
                    }
                    sample.now_us
                }
            };

            let t_us = sample.now_us.saturating_sub(epoch_start);
            let rtt_us = if self.min_rtt_us != u64::MAX {
                self.min_rtt_us
            } else if sample.rtt_us > 0 {
                sample.rtt_us
            } else {
                100_000
            };

            let t_plus_rtt_ms = t_us.saturating_add(rtt_us) / 1_000;
            let w_cubic_target = self.w_cubic_bytes(t_plus_rtt_ms);
            let w_est = self.w_est_bytes(t_us, rtt_us);

            let target = w_cubic_target.max(w_est);
            if target > self.cwnd {
                // RFC 9438 §4.4: cap target at 1.5 * cwnd, then scale increase by acked_bytes / cwnd.
                let capped_target = target.min(self.cwnd.saturating_add(self.cwnd / 2));
                let diff = capped_target.saturating_sub(self.cwnd);
                let inc = ((diff as u128)
                    .saturating_mul(sample.acked_bytes as u128)
                    / (self.cwnd.max(1) as u128)) as u32;
                self.cwnd = self.cwnd.saturating_add(inc.max(1)).min(MAX_CWND_BYTES);
            } else {
                // Gentle increment when already at/above target (1 byte per 100 cwnd).
                let inc = ((sample.acked_bytes as u128)
                    / ((self.cwnd.max(1) as u128).saturating_mul(100))) as u32;
                self.cwnd = self.cwnd.saturating_add(inc).min(MAX_CWND_BYTES);
            }
        }

        self.update_pacing();
        self.action()
    }

    fn on_loss(&mut self, _now_us: u64, _bytes_in_flight: u32) -> CongestionAction {
        self.epoch_start_us = None;
        // RFC 9438 §4.7 Fast Convergence.
        if self.cwnd < self.w_last_max {
            self.w_last_max = self.cwnd;
            self.w_max = ((self.cwnd as u64 * 17) / 20) as u32;
        } else {
            self.w_last_max = self.cwnd;
            self.w_max = self.cwnd;
        }
        // RFC 9438 §4.6 Multiplicative Decrease: beta = 0.7.
        let min_cwnd = MIN_CWND_SEGMENTS * self.mss;
        self.ssthresh = (((self.cwnd as u64) * 7) / 10).max(min_cwnd as u64) as u32;
        self.cwnd = self.ssthresh;
        self.k_ms = self.compute_k_ms(self.w_max);
        self.update_pacing();
        self.action()
    }

    fn on_timeout(&mut self, _now_us: u64) -> CongestionAction {
        self.epoch_start_us = None;
        self.w_last_max = self.cwnd;
        self.w_max = self.cwnd;
        let min_cwnd = MIN_CWND_SEGMENTS * self.mss;
        self.ssthresh = (((self.cwnd as u64) * 7) / 10).max(min_cwnd as u64) as u32;
        self.cwnd = self.mss;
        self.k_ms = self.compute_k_ms(self.w_max);
        self.hystart_exited = false;
        self.update_pacing();
        self.action()
    }

    fn cwnd(&self) -> u32 {
        self.cwnd
    }

    fn ssthresh(&self) -> u32 {
        self.ssthresh
    }

    fn pacing_rate_bps(&self) -> u64 {
        self.pacing_rate_bps
    }
}

// ============================================================================
// BBR (Bottleneck Bandwidth and Round-trip propagation time)
// ============================================================================

/// BBR state machine phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BbrState {
    Startup,
    Drain,
    ProbeBw,
    ProbeRtt,
}

/// Window length for max-bandwidth filter (10 packet-timed round trips).
const BBR_BW_FILTER_ROUNDS: u64 = 10;
/// Window length for min-RTT filter (10 seconds in microseconds).
const BBR_MIN_RTT_WINDOW_US: u64 = 10_000_000;
/// Duration spent in ProbeRTT (200 ms in microseconds).
const BBR_PROBE_RTT_DURATION_US: u64 = 200_000;
/// Minimum cwnd during ProbeRTT (4 segments).
const BBR_PROBE_RTT_CWND_SEGMENTS: u32 = 4;
/// Startup pacing gain: 2 / ln(2) ≈ 2.885 (scaled per-mille).
const BBR_STARTUP_PACING_GAIN: u32 = 2885;
/// Drain pacing gain: ln(2) / 2 ≈ 0.347 (scaled per-mille).
const BBR_DRAIN_PACING_GAIN: u32 = 347;
/// Steady-state cwnd gain: 2.0 (scaled per-mille).
const BBR_CWND_GAIN: u32 = 2000;
/// ProbeBW 8-phase pacing gain cycle (scaled per-mille: 5/4, 3/4, 1, 1, 1, 1, 1, 1).
const BBR_PROBE_BW_GAINS: [u32; 8] = [1250, 750, 1000, 1000, 1000, 1000, 1000, 1000];

#[derive(Clone, Copy, Debug)]
struct BwSample {
    bw_bps: u64,
    round: u64,
}

/// 3-stage Kathleen Nichols windowed maximum filter (pure integer, O(1)).
#[derive(Clone, Debug)]
pub struct MaxBwFilter {
    window_rounds: u64,
    estimates: [BwSample; 3],
}

impl MaxBwFilter {
    pub const fn new(window_rounds: u64) -> Self {
        let zero = BwSample { bw_bps: 0, round: 0 };
        Self {
            window_rounds,
            estimates: [zero; 3],
        }
    }

    pub const fn max_bw(&self) -> u64 {
        self.estimates[0].bw_bps
    }

    pub fn update(&mut self, bw_bps: u64, round: u64) {
        let sample = BwSample { bw_bps, round };
        if self.estimates[0].bw_bps == 0
            || bw_bps >= self.estimates[0].bw_bps
            || round.saturating_sub(self.estimates[2].round) > self.window_rounds
        {
            self.estimates = [sample; 3];
            return;
        }
        if bw_bps >= self.estimates[1].bw_bps {
            self.estimates[1] = sample;
            self.estimates[2] = sample;
        } else if bw_bps >= self.estimates[2].bw_bps {
            self.estimates[2] = sample;
        }

        // Expire stale maxima as rounds advance.
        if round.saturating_sub(self.estimates[0].round) > self.window_rounds {
            self.estimates[0] = self.estimates[1];
            self.estimates[1] = self.estimates[2];
            self.estimates[2] = sample;
            if round.saturating_sub(self.estimates[0].round) > self.window_rounds {
                self.estimates[0] = self.estimates[1];
                self.estimates[1] = self.estimates[2];
                self.estimates[2] = sample;
            }
        } else if self.estimates[1].round == self.estimates[0].round
            && round.saturating_sub(self.estimates[0].round) > self.window_rounds / 4
        {
            self.estimates[1] = sample;
            self.estimates[2] = sample;
        } else if self.estimates[2].round == self.estimates[1].round
            && round.saturating_sub(self.estimates[0].round) > self.window_rounds / 2
        {
            self.estimates[2] = sample;
        }
    }
}

/// Integer/fixed-point BBR congestion controller.
#[derive(Clone, Debug)]
pub struct Bbr {
    mss: u32,
    state: BbrState,
    cwnd: u32,
    pacing_rate_bps: u64,
    pacing_gain: u32,
    cwnd_gain: u32,
    max_bw: MaxBwFilter,
    min_rtt_us: u64,
    min_rtt_stamp_us: u64,
    round_count: u64,
    next_round_delivered: u64,
    /// Startup full-pipe detection.
    filled_pipe: bool,
    full_bw_bps: u64,
    full_bw_count: u32,
    /// ProbeBW 8-phase cycle.
    cycle_index: usize,
    cycle_stamp_us: u64,
    /// ProbeRTT exit timer.
    probe_rtt_done_stamp_us: Option<u64>,
    probe_rtt_round_done: bool,
    prior_cwnd: u32,
}

impl Bbr {
    pub const fn new(mss: u32) -> Self {
        let mss = if mss == 0 { DEFAULT_MSS } else { mss };
        let init_cwnd = INIT_CWND_SEGMENTS * mss;
        Self {
            mss,
            state: BbrState::Startup,
            cwnd: init_cwnd,
            pacing_rate_bps: 0,
            pacing_gain: BBR_STARTUP_PACING_GAIN,
            cwnd_gain: BBR_CWND_GAIN,
            max_bw: MaxBwFilter::new(BBR_BW_FILTER_ROUNDS),
            min_rtt_us: u64::MAX,
            min_rtt_stamp_us: 0,
            round_count: 0,
            next_round_delivered: 0,
            filled_pipe: false,
            full_bw_bps: 0,
            full_bw_count: 0,
            cycle_index: 0,
            cycle_stamp_us: 0,
            probe_rtt_done_stamp_us: None,
            probe_rtt_round_done: false,
            prior_cwnd: init_cwnd,
        }
    }

    pub const fn state(&self) -> BbrState {
        self.state
    }

    pub const fn max_bw_bps(&self) -> u64 {
        self.max_bw.max_bw()
    }

    pub const fn min_rtt_us(&self) -> u64 {
        self.min_rtt_us
    }

    pub const fn filled_pipe(&self) -> bool {
        self.filled_pipe
    }

    pub const fn bdp_bytes(&self) -> u32 {
        if self.min_rtt_us == u64::MAX || self.max_bw.max_bw() == 0 {
            return INIT_CWND_SEGMENTS * self.mss;
        }
        let bdp = (self.max_bw.max_bw() as u128)
            .saturating_mul(self.min_rtt_us as u128)
            / 1_000_000;
        let max = MAX_CWND_BYTES as u128;
        if bdp > max { MAX_CWND_BYTES } else { bdp as u32 }
    }

    fn update_model(&mut self, sample: &AckSample) -> bool {
        // 1. Packet-timed round-trip counter.
        let mut round_start = false;
        if sample.delivered_bytes >= self.next_round_delivered {
            self.next_round_delivered = sample
                .delivered_bytes
                .saturating_add(self.cwnd.max(self.mss) as u64);
            self.round_count = self.round_count.saturating_add(1);
            round_start = true;
        }

        // 2. Bandwidth sample -> windowed max filter.
        if sample.acked_bytes > 0 && sample.delivered_interval_us > 0 {
            let bw_bps = ((sample.acked_bytes as u128)
                .saturating_mul(1_000_000)
                / (sample.delivered_interval_us as u128)) as u64;
            if !sample.is_app_limited || bw_bps >= self.max_bw.max_bw() {
                self.max_bw.update(bw_bps, self.round_count);
            }
        }

        // 3. Min-RTT filter (10-second window).
        let rtt_expired = self.min_rtt_us != u64::MAX
            && sample.now_us.saturating_sub(self.min_rtt_stamp_us) >= BBR_MIN_RTT_WINDOW_US;
        if sample.rtt_us > 0 && (sample.rtt_us <= self.min_rtt_us || rtt_expired) {
            self.min_rtt_us = sample.rtt_us;
            self.min_rtt_stamp_us = sample.now_us;
        }

        // 4. Check ProbeRTT entry.
        if rtt_expired && self.state != BbrState::ProbeRtt {
            self.prior_cwnd = self.cwnd;
            self.state = BbrState::ProbeRtt;
            self.pacing_gain = 1000;
            self.cwnd_gain = 1000;
            self.probe_rtt_done_stamp_us = None;
            self.probe_rtt_round_done = false;
        }

        round_start
    }

    fn advance_state_machine(&mut self, sample: &AckSample, round_start: bool) {
        match self.state {
            BbrState::Startup => {
                self.pacing_gain = BBR_STARTUP_PACING_GAIN;
                self.cwnd_gain = BBR_CWND_GAIN;
                if round_start && !sample.is_app_limited {
                    let bw = self.max_bw.max_bw();
                    // Check for >= 25% bandwidth growth (5/4).
                    let thresh = self.full_bw_bps.saturating_mul(5) / 4;
                    if bw >= thresh && bw > 0 {
                        self.full_bw_bps = bw;
                        self.full_bw_count = 0;
                    } else if self.full_bw_bps > 0 {
                        self.full_bw_count += 1;
                        if self.full_bw_count >= 3 {
                            self.filled_pipe = true;
                            self.state = BbrState::Drain;
                            self.pacing_gain = BBR_DRAIN_PACING_GAIN;
                            self.cwnd_gain = BBR_CWND_GAIN;
                        }
                    }
                }
            }
            BbrState::Drain => {
                self.pacing_gain = BBR_DRAIN_PACING_GAIN;
                self.cwnd_gain = BBR_CWND_GAIN;
                if sample.bytes_in_flight <= self.bdp_bytes() {
                    self.state = BbrState::ProbeBw;
                    self.cycle_index = 0;
                    self.cycle_stamp_us = sample.now_us;
                    self.pacing_gain = BBR_PROBE_BW_GAINS[0];
                    self.cwnd_gain = BBR_CWND_GAIN;
                }
            }
            BbrState::ProbeBw => {
                let min_rtt = if self.min_rtt_us == u64::MAX {
                    100_000
                } else {
                    self.min_rtt_us
                };
                if sample.now_us.saturating_sub(self.cycle_stamp_us) >= min_rtt {
                    self.cycle_index = (self.cycle_index + 1) % BBR_PROBE_BW_GAINS.len();
                    self.cycle_stamp_us = sample.now_us;
                }
                self.pacing_gain = BBR_PROBE_BW_GAINS[self.cycle_index];
                self.cwnd_gain = BBR_CWND_GAIN;
            }
            BbrState::ProbeRtt => {
                self.pacing_gain = 1000;
                self.cwnd_gain = 1000;
                let probe_cwnd = BBR_PROBE_RTT_CWND_SEGMENTS * self.mss;
                match self.probe_rtt_done_stamp_us {
                    None => {
                        if sample.bytes_in_flight <= probe_cwnd {
                            self.probe_rtt_done_stamp_us =
                                Some(sample.now_us.saturating_add(BBR_PROBE_RTT_DURATION_US));
                            self.probe_rtt_round_done = false;
                            self.next_round_delivered =
                                sample.delivered_bytes.saturating_add(probe_cwnd as u64);
                        }
                    }
                    Some(done_at) => {
                        if round_start {
                            self.probe_rtt_round_done = true;
                        }
                        if self.probe_rtt_round_done && sample.now_us >= done_at {
                            self.min_rtt_stamp_us = sample.now_us;
                            self.cwnd = self.cwnd.max(self.prior_cwnd);
                            if self.filled_pipe {
                                self.state = BbrState::ProbeBw;
                                self.cycle_index = 0;
                                self.cycle_stamp_us = sample.now_us;
                                self.pacing_gain = BBR_PROBE_BW_GAINS[0];
                            } else {
                                self.state = BbrState::Startup;
                                self.pacing_gain = BBR_STARTUP_PACING_GAIN;
                            }
                            self.cwnd_gain = BBR_CWND_GAIN;
                        }
                    }
                }
            }
        }
    }

    fn update_controls(&mut self, sample: &AckSample) {
        let bw = self.max_bw.max_bw();
        if bw > 0 {
            // 1% pacing headroom (99 / 100) to avoid standing queues at the bottleneck.
            let rate = (bw as u128)
                .saturating_mul(self.pacing_gain as u128)
                .saturating_mul(99)
                / 100_000;
            self.pacing_rate_bps = rate.max(self.mss as u128).min(u64::MAX as u128) as u64;
        }

        let min_cwnd = BBR_PROBE_RTT_CWND_SEGMENTS * self.mss;
        if self.state == BbrState::ProbeRtt {
            self.cwnd = self.cwnd.min(min_cwnd).max(min_cwnd);
            return;
        }

        let bdp = self.bdp_bytes();
        let target_cwnd = ((bdp as u128)
            .saturating_mul(self.cwnd_gain as u128)
            / 1000)
            .clamp(min_cwnd as u128, MAX_CWND_BYTES as u128) as u32;

        if self.filled_pipe {
            self.cwnd = self.cwnd.saturating_add(sample.acked_bytes).min(target_cwnd);
        } else if self.cwnd < target_cwnd || sample.delivered_bytes < (INIT_CWND_SEGMENTS * self.mss) as u64 {
            self.cwnd = self.cwnd.saturating_add(sample.acked_bytes).min(MAX_CWND_BYTES);
        }
        self.cwnd = self.cwnd.max(min_cwnd);
    }

    fn action(&self) -> CongestionAction {
        CongestionAction {
            cwnd: self.cwnd,
            ssthresh: MAX_CWND_BYTES,
            pacing_rate_bps: self.pacing_rate_bps,
        }
    }
}

impl CongestionControl for Bbr {
    fn on_ack(&mut self, sample: AckSample) -> CongestionAction {
        let round_start = self.update_model(&sample);
        self.advance_state_machine(&sample, round_start);
        self.update_controls(&sample);
        self.action()
    }

    fn on_loss(&mut self, _now_us: u64, bytes_in_flight: u32) -> CongestionAction {
        // BBR does not halve cwnd on isolated packet loss; it bounds cwnd to
        // preserve packet conservation during recovery.
        self.prior_cwnd = self.cwnd.max(self.prior_cwnd);
        let min_cwnd = BBR_PROBE_RTT_CWND_SEGMENTS * self.mss;
        self.cwnd = bytes_in_flight.saturating_add(self.mss).max(min_cwnd);
        self.action()
    }

    fn on_timeout(&mut self, _now_us: u64) -> CongestionAction {
        self.prior_cwnd = self.cwnd.max(self.prior_cwnd);
        self.cwnd = self.mss;
        self.action()
    }

    fn cwnd(&self) -> u32 {
        self.cwnd
    }

    fn ssthresh(&self) -> u32 {
        MAX_CWND_BYTES
    }

    fn pacing_rate_bps(&self) -> u64 {
        self.pacing_rate_bps
    }
}

// ============================================================================
// Unified Controller Wrapper
// ============================================================================

/// Runtime-selectable congestion controller (defaults to CUBIC).
#[derive(Clone, Debug)]
pub enum CongestionController {
    Cubic(Cubic),
    Bbr(Bbr),
}

impl CongestionController {
    pub const fn new(algo: CongestionAlgorithm, mss: u32) -> Self {
        match algo {
            CongestionAlgorithm::Cubic => Self::Cubic(Cubic::new(mss)),
            CongestionAlgorithm::Bbr => Self::Bbr(Bbr::new(mss)),
        }
    }

    pub const fn algorithm(&self) -> CongestionAlgorithm {
        match self {
            Self::Cubic(_) => CongestionAlgorithm::Cubic,
            Self::Bbr(_) => CongestionAlgorithm::Bbr,
        }
    }
}

impl Default for CongestionController {
    fn default() -> Self {
        Self::Cubic(Cubic::new(DEFAULT_MSS))
    }
}

impl CongestionControl for CongestionController {
    fn on_ack(&mut self, sample: AckSample) -> CongestionAction {
        match self {
            Self::Cubic(c) => c.on_ack(sample),
            Self::Bbr(b) => b.on_ack(sample),
        }
    }

    fn on_loss(&mut self, now_us: u64, bytes_in_flight: u32) -> CongestionAction {
        match self {
            Self::Cubic(c) => c.on_loss(now_us, bytes_in_flight),
            Self::Bbr(b) => b.on_loss(now_us, bytes_in_flight),
        }
    }

    fn on_timeout(&mut self, now_us: u64) -> CongestionAction {
        match self {
            Self::Cubic(c) => c.on_timeout(now_us),
            Self::Bbr(b) => b.on_timeout(now_us),
        }
    }

    fn cwnd(&self) -> u32 {
        match self {
            Self::Cubic(c) => c.cwnd(),
            Self::Bbr(b) => b.cwnd(),
        }
    }

    fn ssthresh(&self) -> u32 {
        match self {
            Self::Cubic(c) => c.ssthresh(),
            Self::Bbr(b) => b.ssthresh(),
        }
    }

    fn pacing_rate_bps(&self) -> u64 {
        match self {
            Self::Cubic(c) => c.pacing_rate_bps(),
            Self::Bbr(b) => b.pacing_rate_bps(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_cube_root_is_exact() {
        assert_eq!(icbrt_u128(0), 0);
        assert_eq!(icbrt_u128(1), 1);
        assert_eq!(icbrt_u128(27), 3);
        assert_eq!(icbrt_u128(1_000_000_000), 1_000);
        assert_eq!(icbrt_u128(1_000_000_000_000), 10_000);
        // 100 segments * 750_000_000 = 75_000_000_000 -> cbrt ≈ 4217.16 ms
        assert_eq!(icbrt_u128(75_000_000_000), 4217);
    }

    #[test]
    fn cubic_multiplicative_decrease_concave_and_convex_growth() {
        let mut cubic = Cubic::new(1460);
        cubic.set_hystart(false);
        let mut now_us = 100_000u64;
        let mut delivered = 0u64;

        // Grow cwnd in slow start to ~100 segments.
        for _ in 0..90 {
            delivered += 1460;
            now_us += 1_000;
            cubic.on_ack(AckSample {
                acked_bytes: 1460,
                rtt_us: 20_000,
                now_us,
                delivered_bytes: delivered,
                delivered_interval_us: 1_000,
                is_app_limited: false,
                bytes_in_flight: cubic.cwnd(),
            });
        }
        let pre_loss_cwnd = cubic.cwnd();
        assert!(pre_loss_cwnd >= 90 * 1460);

        // Trigger loss -> beta = 0.7 multiplicative decrease.
        let loss = cubic.on_loss(now_us, pre_loss_cwnd);
        let expected_after_loss = ((pre_loss_cwnd as u64) * 7 / 10) as u32;
        assert_eq!(loss.cwnd, expected_after_loss);
        assert_eq!(loss.ssthresh, expected_after_loss);
        assert_eq!(cubic.w_max(), pre_loss_cwnd);
        assert!(cubic.k_ms() > 0);

        // Verify W_cubic(0) ≈ 0.7 * W_max, W_cubic(K) == W_max, W_cubic(K + 2000ms) > W_max.
        let w0 = cubic.w_cubic_bytes(0);
        assert!((w0 as i64 - expected_after_loss as i64).abs() <= 1460);
        let wk = cubic.w_cubic_bytes(cubic.k_ms());
        assert_eq!(wk, cubic.w_max());
        let w_convex = cubic.w_cubic_bytes(cubic.k_ms() + 2_000);
        assert!(w_convex > cubic.w_max());

        // Advance ACKs through congestion avoidance and verify cwnd recovers past W_max.
        let k_us = cubic.k_ms() * 1_000;
        for step in 0..400u64 {
            now_us += (k_us / 150).max(5_000);
            delivered += 1460;
            cubic.on_ack(AckSample {
                acked_bytes: 1460,
                rtt_us: 20_000,
                now_us,
                delivered_bytes: delivered,
                delivered_interval_us: 5_000,
                is_app_limited: false,
                bytes_in_flight: cubic.cwnd(),
            });
            if step == 399 {
                assert!(cubic.cwnd() > pre_loss_cwnd, "cwnd should enter convex growth past W_max");
            }
        }
    }

    #[test]
    fn cubic_hystart_exits_slow_start_on_delay_increase() {
        let mut cubic = Cubic::new(1460);
        let mut now_us = 10_000u64;
        let mut delivered = 0u64;

        // Round 1: steady 20ms RTT.
        for _ in 0..12 {
            delivered += 1460;
            now_us += 2_000;
            cubic.on_ack(AckSample {
                acked_bytes: 1460,
                rtt_us: 20_000,
                now_us,
                delivered_bytes: delivered,
                delivered_interval_us: 2_000,
                is_app_limited: false,
                bytes_in_flight: cubic.cwnd(),
            });
        }
        assert!(!cubic.hystart_exited());

        // Round 2: RTT jumps by +10ms (from 20ms to 30ms, exceeding 4ms min threshold).
        for _ in 0..30 {
            delivered += 1460;
            now_us += 2_000;
            cubic.on_ack(AckSample {
                acked_bytes: 1460,
                rtt_us: 30_000,
                now_us,
                delivered_bytes: delivered,
                delivered_interval_us: 2_000,
                is_app_limited: false,
                bytes_in_flight: cubic.cwnd(),
            });
        }
        assert!(cubic.hystart_exited());
        assert!(cubic.ssthresh() < u32::MAX);
    }

    #[test]
    fn bbr_startup_drain_probebw_and_probertt_state_machine() {
        let mut bbr = Bbr::new(1460);
        assert_eq!(bbr.state(), BbrState::Startup);

        let mut now_us = 100_000u64;
        let mut delivered = 0u64;
        // Simulate constant 10 MB/s bottleneck with 20ms min RTT -> BDP = 200 KB.
        // 1460 bytes delivered every 146 us = 10_000_000 bytes/sec.
        let mut saw_drain = false;
        let mut saw_probe_bw = false;
        for _ in 0..2000 {
            delivered += 1460;
            now_us += 146;
            let in_flight = if bbr.state() == BbrState::Drain {
                // Simulate draining queue below BDP.
                bbr.bdp_bytes().saturating_sub(1460)
            } else {
                bbr.cwnd()
            };
            let act = bbr.on_ack(AckSample {
                acked_bytes: 1460,
                rtt_us: 20_000,
                now_us,
                delivered_bytes: delivered,
                delivered_interval_us: 146,
                is_app_limited: false,
                bytes_in_flight: in_flight,
            });
            assert!(act.pacing_rate_bps > 0);
            if bbr.state() == BbrState::Drain {
                saw_drain = true;
            }
            if bbr.state() == BbrState::ProbeBw {
                saw_probe_bw = true;
                break;
            }
        }
        assert!(bbr.filled_pipe());
        assert!(saw_drain);
        assert!(saw_probe_bw);
        assert_eq!(bbr.min_rtt_us(), 20_000);
        // Max bandwidth should be ~10_000_000 B/s.
        assert!((bbr.max_bw_bps() as i64 - 10_000_000).abs() <= 100_000);

        // Advance time by > 10 seconds with higher RTT -> must enter ProbeRTT!
        now_us += BBR_MIN_RTT_WINDOW_US + 50_000;
        delivered += 1460;
        let probe_act = bbr.on_ack(AckSample {
            acked_bytes: 1460,
            rtt_us: 25_000,
            now_us,
            delivered_bytes: delivered,
            delivered_interval_us: 146,
            is_app_limited: false,
            bytes_in_flight: 4 * 1460,
        });
        assert_eq!(bbr.state(), BbrState::ProbeRtt);
        assert_eq!(probe_act.cwnd, 4 * 1460);

        // Stay in ProbeRTT for > 200ms + 1 round -> returns to ProbeBw.
        now_us += BBR_PROBE_RTT_DURATION_US + 50_000;
        for _ in 0..10 {
            delivered += 1460;
            now_us += 5_000;
            bbr.on_ack(AckSample {
                acked_bytes: 1460,
                rtt_us: 20_000,
                now_us,
                delivered_bytes: delivered,
                delivered_interval_us: 146,
                is_app_limited: false,
                bytes_in_flight: 4 * 1460,
            });
            if bbr.state() == BbrState::ProbeBw {
                break;
            }
        }
        assert_eq!(bbr.state(), BbrState::ProbeBw);
    }

    #[test]
    fn congestion_controller_enum_defaults_to_cubic_and_switches_to_bbr() {
        let mut default_cc = CongestionController::default();
        assert_eq!(default_cc.algorithm(), CongestionAlgorithm::Cubic);
        assert_eq!(default_cc.cwnd(), INIT_CWND_SEGMENTS * DEFAULT_MSS);

        let mut bbr_cc = CongestionController::new(CongestionAlgorithm::Bbr, DEFAULT_MSS);
        assert_eq!(bbr_cc.algorithm(), CongestionAlgorithm::Bbr);
        let act = bbr_cc.on_ack(AckSample {
            acked_bytes: 1460,
            rtt_us: 10_000,
            now_us: 10_000,
            delivered_bytes: 1460,
            delivered_interval_us: 1_000,
            is_app_limited: false,
            bytes_in_flight: 1460,
        });
        assert!(act.pacing_rate_bps > 0);
        let _ = default_cc.on_timeout(20_000);
        assert_eq!(default_cc.cwnd(), DEFAULT_MSS);
    }
}
