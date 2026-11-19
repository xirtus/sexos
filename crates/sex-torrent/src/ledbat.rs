// SPDX-License-Identifier: GPL-3.0-only
//! LEDBAT, the "Low Extra Delay Background Transport" congestion controller (RFC 6817),
//! in the form BEP 29 uses for uTP. It is what keeps torrents from swamping the line.
//!
//! The idea: every acknowledgement carries a one-way delay sample (how long our packet
//! took to reach the peer, plus an unknown clock offset). The smallest sample seen over
//! the last ten minutes is the "base delay" (the empty-queue delay plus the offset).
//! Anything above it is time our packets spent waiting in some router queue: the queuing
//! delay. LEDBAT aims to keep that queuing delay at a target of 100 ms. Below the target
//! the window grows (at most `MAX_CWND_INCREASE_PER_RTT` bytes per round trip, like TCP),
//! above it the window shrinks in proportion, so as soon as other traffic (a video call,
//! a game, a web page) starts to fill the queue, the torrent backs off.
//!
//! The window (`cwnd`) is in bytes. A loss halves it (at most once per round trip), a
//! retransmission timeout drops it to one packet. It never goes below `MIN_CWND_PACKETS`
//! packets otherwise. No I/O and no clock of its own: the caller passes `now_us`.

/// Target queuing delay (RFC 6817 TARGET, BEP 29 CCONTROL_TARGET): 100 ms.
pub const TARGET_US: u32 = 100_000;
/// The most the window grows in one round trip when the queue is empty (BEP 29
/// MAX_CWND_INCREASE_PACKETS_PER_RTT, in bytes as libutp counts it).
pub const MAX_CWND_INCREASE_PER_RTT: u32 = 3000;
/// The base-delay history: one minimum per bucket, `BASE_BUCKETS` buckets of
/// `BASE_BUCKET_US` each (10 minutes in all).
pub const BASE_BUCKET_US: u64 = 120_000_000;
pub const BASE_BUCKETS: usize = 5;
/// How many recent delay samples the "current delay" filter takes the minimum of
/// (RFC 6817 CURRENT_FILTER): one bad sample cannot shrink the window by itself.
pub const CURRENT_FILTER: usize = 4;
/// The smallest window in steady state, in packets (RFC 6817 MIN_CWND).
pub const MIN_CWND_PACKETS: u32 = 2;
/// RFC 6817 ALLOWED_INCREASE: the window may not grow past what is in flight plus this
/// many packets (no growth while the application has nothing to send).
pub const ALLOWED_INCREASE_PACKETS: u32 = 1;

/// `a` is before `b` on a wrapping 32-bit microsecond clock.
fn wrapping_lt(a: u32, b: u32) -> bool { (a.wrapping_sub(b) as i32) < 0 }

/// The minimum one-way delay seen in each of the last few 2-minute buckets.
#[derive(Clone, Debug)]
pub struct BaseDelay {
    buckets: [Option<u32>; BASE_BUCKETS],
    current: usize,
    bucket_start_us: u64,
}

impl BaseDelay {
    pub fn new(now_us: u64) -> Self { BaseDelay { buckets: [None; BASE_BUCKETS], current: 0, bucket_start_us: now_us } }

    /// Records a raw delay sample (peer's receive time minus our send time, wrapping).
    pub fn add(&mut self, sample: u32, now_us: u64) {
        // Roll over to new buckets for every bucket length that passed (at most all of them).
        let mut rolled = 0;
        while now_us.saturating_sub(self.bucket_start_us) >= BASE_BUCKET_US && rolled < BASE_BUCKETS {
            self.current = (self.current + 1) % BASE_BUCKETS;
            self.buckets[self.current] = None;
            self.bucket_start_us += BASE_BUCKET_US;
            rolled += 1;
        }
        if now_us.saturating_sub(self.bucket_start_us) >= BASE_BUCKET_US { self.bucket_start_us = now_us; }
        let b = &mut self.buckets[self.current];
        *b = Some(match *b { Some(m) if !wrapping_lt(sample, m) => m, _ => sample });
    }

    /// The minimum over the whole history, if any sample has been seen.
    pub fn get(&self) -> Option<u32> {
        self.buckets.iter().flatten().copied().fold(None, |acc, s| match acc {
            Some(m) if !wrapping_lt(s, m) => Some(m),
            _ => Some(s),
        })
    }
}

/// The LEDBAT controller for one connection.
#[derive(Clone, Debug)]
pub struct Ledbat {
    mss: u32,
    cwnd: u32,
    max_cwnd: u32,
    target_us: u32,
    base: BaseDelay,
    recent: [u32; CURRENT_FILTER],
    recent_len: usize,
    recent_pos: usize,
    last_cut_us: Option<u64>,
    slow_start: bool,
    ssthresh: u32,
}

impl Ledbat {
    /// A controller for packets of `mss` bytes. It starts with a window of two packets,
    /// in slow start (doubling per round trip) until the queue fills to half the target
    /// or a loss happens. `max_cwnd` caps the window (the send buffer size, say).
    pub fn new(mss: u32, max_cwnd: u32, now_us: u64) -> Self {
        let mss = mss.max(1);
        Ledbat {
            mss,
            cwnd: MIN_CWND_PACKETS * mss,
            max_cwnd: max_cwnd.max(MIN_CWND_PACKETS * mss),
            target_us: TARGET_US,
            base: BaseDelay::new(now_us),
            recent: [0; CURRENT_FILTER],
            recent_len: 0,
            recent_pos: 0,
            last_cut_us: None,
            slow_start: true,
            ssthresh: u32::MAX,
        }
    }

    /// Changes the target queuing delay (BEP 29 lets clients pick a smaller one).
    pub fn set_target(&mut self, target_us: u32) { self.target_us = target_us.clamp(1_000, 1_000_000); }

    /// The congestion window in bytes.
    pub fn window(&self) -> u32 { self.cwnd }

    /// The packet size the window is counted in.
    pub fn mss(&self) -> u32 { self.mss }

    /// Whether the controller is still in slow start.
    pub fn in_slow_start(&self) -> bool { self.slow_start }

    /// The current queuing delay estimate: the minimum of the recent samples minus the
    /// base delay. `None` before the first sample.
    pub fn queuing_delay(&self) -> Option<u32> {
        let base = self.base.get()?;
        let cur = self.recent[..self.recent_len].iter().copied().fold(None, |acc: Option<u32>, s| match acc {
            Some(m) if !wrapping_lt(s, m) => Some(m),
            _ => Some(s),
        })?;
        let q = cur.wrapping_sub(base);
        // A sample below the base (clock drift, wrap) counts as an empty queue.
        Some(if (q as i32) < 0 { 0 } else { q })
    }

    /// An acknowledgement arrived. `bytes_acked` is how much new data it acknowledged,
    /// `delay_sample` the one-way delay the peer measured for our packets (BEP 29
    /// timestamp_difference_microseconds; 0 means "none"), `flight` the bytes that were
    /// in flight before this ack.
    pub fn on_ack(&mut self, bytes_acked: u32, delay_sample: u32, flight: u32, now_us: u64) {
        if delay_sample != 0 {
            self.base.add(delay_sample, now_us);
            self.recent[self.recent_pos] = delay_sample;
            self.recent_pos = (self.recent_pos + 1) % CURRENT_FILTER;
            self.recent_len = (self.recent_len + 1).min(CURRENT_FILTER);
        }
        if bytes_acked == 0 { return; }
        let target = self.target_us as i64;
        let queuing = self.queuing_delay().unwrap_or(0) as i64;
        let off_target = target - queuing;
        let mut cwnd = self.cwnd as i64;
        if self.slow_start && queuing * 2 < target && self.cwnd < self.ssthresh {
            cwnd += bytes_acked as i64;
        } else {
            if self.slow_start { self.slow_start = false; }
            // BEP 29: gain = MAX_INCREASE * (off_target / target) * (acked / cwnd),
            // with off_target clamped so one huge sample cannot collapse the window.
            let off = off_target.clamp(-target, target);
            let denom = target * (self.cwnd.max(bytes_acked).max(1) as i64);
            cwnd += (MAX_CWND_INCREASE_PER_RTT as i64) * off * (bytes_acked as i64) / denom;
        }
        // No growth beyond what is in flight plus a little (application-limited).
        let allowed = flight as i64 + bytes_acked as i64 + (ALLOWED_INCREASE_PACKETS * self.mss) as i64;
        if cwnd > self.cwnd as i64 && cwnd > allowed { cwnd = allowed.max(self.cwnd as i64); }
        let min = (MIN_CWND_PACKETS * self.mss) as i64;
        self.cwnd = cwnd.clamp(min, self.max_cwnd as i64) as u32;
    }

    /// A packet was lost (3 duplicate acks or selective acks): halve the window, at most
    /// once per round trip (`rtt_us`).
    pub fn on_loss(&mut self, rtt_us: u64, now_us: u64) {
        if let Some(t) = self.last_cut_us { if now_us.saturating_sub(t) < rtt_us.max(1) { return; } }
        self.last_cut_us = Some(now_us);
        self.slow_start = false;
        self.cwnd = (self.cwnd / 2).max(MIN_CWND_PACKETS * self.mss);
        self.ssthresh = self.cwnd;
    }

    /// The retransmission timer fired: back to one packet (RFC 6817 section 2.4.2).
    pub fn on_timeout(&mut self, now_us: u64) {
        self.ssthresh = (self.cwnd / 2).max(MIN_CWND_PACKETS * self.mss);
        self.cwnd = self.mss;
        self.slow_start = false;
        self.last_cut_us = Some(now_us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MSS: u32 = 1400;
    const MS: u64 = 1000;

    #[test]
    fn base_delay_is_min_and_expires() {
        let mut b = BaseDelay::new(0);
        b.add(50_000, 0);
        b.add(30_000, 1);
        b.add(40_000, 2);
        assert_eq!(b.get(), Some(30_000));
        // After the whole history passes, old minima are gone.
        b.add(90_000, BASE_BUCKET_US * (BASE_BUCKETS as u64 + 1));
        assert_eq!(b.get(), Some(90_000));
        // A huge jump in time never loops forever.
        b.add(80_000, u64::MAX / 2);
        assert_eq!(b.get(), Some(80_000));
    }

    #[test]
    fn base_delay_handles_wrap() {
        let mut b = BaseDelay::new(0);
        b.add(u32::MAX - 10, 0);
        b.add(5, 1); // 16 later, wrapping: not smaller
        assert_eq!(b.get(), Some(u32::MAX - 10));
    }

    #[test]
    fn below_target_grows_above_target_shrinks() {
        let mut l = Ledbat::new(MSS, 1 << 24, 0);
        // Leave slow start with one queue-full sample.
        l.on_ack(MSS, 10_000, 0, 0);
        for i in 0..=CURRENT_FILTER as u64 { l.on_ack(MSS, 10_000 + 80_000, 100 * MSS, 1 + i); }
        assert!(!l.in_slow_start());
        let mut t = 10;
        // Empty queue: samples at base. Grow, roughly MAX_INCREASE per window of acks.
        for _ in 0..CURRENT_FILTER { l.on_ack(1, 10_000, 1 << 20, t); t += 1; }
        let w0 = l.window();
        for _ in 0..(w0 / MSS) { l.on_ack(MSS, 10_000, 1 << 20, t); t += 1; }
        let w1 = l.window();
        assert!(w1 > w0 && w1 <= w0 + MAX_CWND_INCREASE_PER_RTT + MSS, "{} -> {}", w0, w1);
        // Queue at 200 ms (double the target): shrink.
        for _ in 0..CURRENT_FILTER { l.on_ack(1, 210_000, 1 << 20, t); t += 1; }
        let w2 = l.window();
        for _ in 0..(w2 / MSS) { l.on_ack(MSS, 210_000, 1 << 20, t); t += 1; }
        assert!(l.window() < w2);
        // Exactly at target: no change.
        for _ in 0..CURRENT_FILTER { l.on_ack(1, 110_000, 1 << 20, t); t += 1; }
        let w3 = l.window();
        l.on_ack(MSS, 110_000, 1 << 20, t);
        assert_eq!(l.window(), w3);
    }

    #[test]
    fn never_below_minimum_and_never_above_max() {
        let mut l = Ledbat::new(MSS, 20 * MSS, 0);
        l.on_ack(MSS, 1_000, 0, 0);
        for i in 0..10_000 { l.on_ack(MSS, 1_000_000, 1 << 20, i); }
        assert_eq!(l.window(), MIN_CWND_PACKETS * MSS);
        let mut l = Ledbat::new(MSS, 20 * MSS, 0);
        for i in 0..10_000 { l.on_ack(MSS, 1_000, 1 << 20, i); }
        assert_eq!(l.window(), 20 * MSS);
    }

    #[test]
    fn slow_start_doubles_then_stops_at_half_target() {
        let mut l = Ledbat::new(MSS, 1 << 24, 0);
        let w = l.window();
        for i in 0..(w / MSS) { l.on_ack(MSS, 5_000, 1 << 20, i as u64); }
        assert_eq!(l.window(), 2 * w);
        for i in 0..CURRENT_FILTER as u64 { l.on_ack(1, 5_000 + 60_000, 1 << 20, 100 + i); }
        assert!(!l.in_slow_start());
    }

    #[test]
    fn loss_halves_once_per_rtt_and_timeout_resets() {
        let mut l = Ledbat::new(MSS, 1 << 24, 0);
        for i in 0..64 { l.on_ack(MSS, 5_000, 1 << 20, i); }
        let w = l.window();
        l.on_loss(100 * MS, 1_000 * MS);
        assert_eq!(l.window(), w / 2);
        l.on_loss(100 * MS, 1_050 * MS); // same round trip: ignored
        assert_eq!(l.window(), w / 2);
        l.on_loss(100 * MS, 1_200 * MS);
        assert_eq!(l.window(), w / 4);
        l.on_timeout(2_000 * MS);
        assert_eq!(l.window(), MSS);
    }

    #[test]
    fn application_limited_does_not_grow() {
        let mut l = Ledbat::new(MSS, 1 << 24, 0);
        for i in 0..1000 { l.on_ack(MSS, 5_000, MSS, i); }
        assert!(l.window() <= 3 * MSS + MSS);
    }

    /// A bottleneck link simulation: one LEDBAT sender, a 1 MB/s link with a 20 ms base
    /// delay and a deep (2 s) buffer. LEDBAT must keep the queue near 100 ms, never fill it.
    #[test]
    fn never_swamps_a_bottleneck() {
        let rate_bytes_per_ms = 1_000u64;
        let base_ms = 20u64;
        let mut l = Ledbat::new(MSS, 1 << 26, 0);
        let mut queue_bytes: u64 = 0;
        let mut flight: std::collections::VecDeque<(u64, u32, u32)> = Default::default(); // (ack time, bytes, delay)
        let mut in_flight = 0u64;
        let mut max_late_delay = 0u64;
        for ms in 0..60_000u64 {
            let now = ms * MS;
            // Fill the window.
            while in_flight + MSS as u64 <= l.window() as u64 {
                queue_bytes += MSS as u64;
                let qdelay = queue_bytes / rate_bytes_per_ms;
                let one_way = base_ms + qdelay;
                flight.push_back((ms + one_way + base_ms, MSS, ((one_way * MS) as u32).wrapping_add(7_777_777)));
                in_flight += MSS as u64;
            }
            queue_bytes = queue_bytes.saturating_sub(rate_bytes_per_ms);
            while let Some(&(at, b, d)) = flight.front() {
                if at > ms { break; }
                flight.pop_front();
                l.on_ack(b, d, in_flight as u32, now);
                in_flight -= b as u64;
            }
            if ms > 20_000 { max_late_delay = max_late_delay.max(queue_bytes / rate_bytes_per_ms); }
        }
        assert!(max_late_delay <= 160, "queue reached {} ms", max_late_delay);
        assert!(max_late_delay >= 40, "link underused: queue only {} ms", max_late_delay);
    }
}
