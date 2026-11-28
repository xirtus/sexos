// SPDX-License-Identifier: GPL-3.0-only
//! Send-side bandwidth estimate from transport-cc feedback, after Google Congestion Control
//! (draft-ietf-rmcat-gcc-02): a delay-based part (trendline of one-way delay growth,
//! adaptive threshold, AIMD rate) and a loss-based part, the target being the lower.
use crate::packet::SeqUnwrapper;
use crate::rtcp::TransportCc;
use alloc::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usage {
    Normal,
    Over,
    Under,
}

fn fabs(x: f64) -> f64 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

#[derive(Clone, Copy)]
struct Sent {
    send_us: i64,
    size: usize,
    acked: bool,
}

/// Packets sent within 5 ms make one group.
#[derive(Clone, Copy)]
struct Group {
    first_send: i64,
    last_send: i64,
    last_arrival: i64,
}

const WINDOW: usize = 20;

pub struct BandwidthEstimator {
    unwrap: SeqUnwrapper,
    sent: BTreeMap<u64, Sent>,
    group: Option<Group>,
    prev_group: Option<Group>,
    // Trendline.
    first_arrival: Option<i64>,
    acc_delay: f64,
    smoothed: f64,
    samples: VecDeque<(f64, f64)>,
    num_deltas: u32,
    prev_trend: f64,
    // Overuse detector.
    threshold: f64,
    over_ms: f64,
    over_count: u32,
    last_update_ms: Option<f64>,
    pub usage: Usage,
    // AIMD.
    delay_bps: f64,
    increasing: bool,
    last_change_us: Option<i64>,
    link_bps: Option<f64>,
    // Loss.
    loss_bps: f64,
    last_loss_change_us: i64,
    pub last_loss: f64,
    // Acked bitrate over 500 ms of arrivals.
    acked: VecDeque<(i64, usize)>,
    pub min_bps: u32,
    pub max_bps: u32,
    /// Round-trip time, for the additive increase step.
    pub rtt_ms: u32,
}

impl BandwidthEstimator {
    pub fn new(start_bps: u32, min_bps: u32, max_bps: u32) -> BandwidthEstimator {
        BandwidthEstimator {
            unwrap: SeqUnwrapper::default(),
            sent: BTreeMap::new(),
            group: None,
            prev_group: None,
            first_arrival: None,
            acc_delay: 0.0,
            smoothed: 0.0,
            samples: VecDeque::new(),
            num_deltas: 0,
            prev_trend: 0.0,
            threshold: 12.5,
            over_ms: -1.0,
            over_count: 0,
            last_update_ms: None,
            usage: Usage::Normal,
            delay_bps: start_bps as f64,
            increasing: true,
            last_change_us: None,
            link_bps: None,
            loss_bps: start_bps as f64,
            last_loss_change_us: 0,
            last_loss: 0.0,
            acked: VecDeque::new(),
            min_bps,
            max_bps,
            rtt_ms: 100,
        }
    }

    /// The bitrate the encoders together should aim for.
    pub fn target_bps(&self) -> u32 {
        let t = if self.delay_bps < self.loss_bps { self.delay_bps } else { self.loss_bps };
        (t as u32).clamp(self.min_bps, self.max_bps)
    }

    /// Bitrate the far end acknowledged receiving over the last 500 ms.
    pub fn acked_bps(&self) -> Option<f64> {
        let (a, b) = (self.acked.front()?, self.acked.back()?);
        let span = (b.0 - a.0).max(1) as f64;
        if span < 100_000.0 {
            return None;
        }
        let bytes: usize = self.acked.iter().map(|x| x.1).sum();
        Some(bytes as f64 * 8.0 * 1e6 / span)
    }

    /// A packet carrying transport-wide sequence number `seq` left now.
    pub fn on_sent(&mut self, seq: u16, size: usize, now_us: i64) {
        let s = self.unwrap.unwrap(seq);
        self.sent.insert(s, Sent { send_us: now_us, size, acked: false });
        while self.sent.len() > 10_000 {
            self.sent.pop_first();
        }
    }

    pub fn on_feedback(&mut self, fb: &TransportCc, now_us: i64) {
        let (mut lost, mut total) = (0u32, 0u32);
        for (seq, arrival) in fb.arrivals() {
            let s = self.unwrap.peek(seq);
            let Some(sent) = self.sent.get_mut(&s) else { continue };
            if sent.acked {
                continue;
            }
            total += 1;
            let Some(at) = arrival else {
                lost += 1;
                continue;
            };
            sent.acked = true;
            let sent = *sent;
            self.acked.push_back((at, sent.size));
            while self.acked.front().map_or(false, |f| f.0 < at - 500_000) {
                self.acked.pop_front();
            }
            self.on_arrival(sent.send_us, at, now_us);
        }
        if total > 0 {
            self.update_loss(lost as f64 / total as f64, now_us);
        }
        let floor = self.sent.keys().next_back().map_or(0, |k| k.saturating_sub(2000));
        self.sent = self.sent.split_off(&floor);
        self.update_rate(now_us);
    }

    fn on_arrival(&mut self, send: i64, arrival: i64, _now_us: i64) {
        match self.group.as_mut() {
            Some(g) if send - g.first_send <= 5_000 => {
                g.last_send = g.last_send.max(send);
                g.last_arrival = g.last_arrival.max(arrival);
                return;
            }
            Some(g) if send < g.first_send => return,
            _ => {}
        }
        if let (Some(prev), Some(cur)) = (self.prev_group, self.group) {
            let send_d = (cur.last_send - prev.last_send) as f64 / 1000.0;
            let arr_d = (cur.last_arrival - prev.last_arrival) as f64 / 1000.0;
            self.trendline(arr_d - send_d, cur.last_arrival);
        }
        self.prev_group = self.group;
        self.group = Some(Group { first_send: send, last_send: send, last_arrival: arrival });
    }

    /// Add one inter-group delay variation (ms) and classify the trend.
    fn trendline(&mut self, delta_ms: f64, arrival_us: i64) {
        let first = *self.first_arrival.get_or_insert(arrival_us);
        let t_ms = (arrival_us - first) as f64 / 1000.0;
        self.num_deltas = (self.num_deltas + 1).min(1000);
        self.acc_delay += delta_ms;
        self.smoothed = 0.9 * self.smoothed + 0.1 * self.acc_delay;
        self.samples.push_back((t_ms, self.smoothed));
        if self.samples.len() > WINDOW {
            self.samples.pop_front();
        }
        if self.samples.len() < WINDOW {
            return;
        }
        let n = self.samples.len() as f64;
        let (mx, my) = self.samples.iter().fold((0.0, 0.0), |a, s| (a.0 + s.0, a.1 + s.1));
        let (mx, my) = (mx / n, my / n);
        let (mut num, mut den) = (0.0, 0.0);
        for &(x, y) in &self.samples {
            num += (x - mx) * (y - my);
            den += (x - mx) * (x - mx);
        }
        let slope = if den == 0.0 { 0.0 } else { num / den };
        let trend = (self.num_deltas.min(60) as f64) * slope * 4.0;
        self.detect(trend, t_ms, delta_ms);
    }

    fn detect(&mut self, trend: f64, now_ms: f64, ts_delta_ms: f64) {
        if trend > self.threshold {
            if self.over_ms < 0.0 {
                self.over_ms = 0.0;
            }
            self.over_ms += fabs(ts_delta_ms).max(5.0);
            self.over_count += 1;
            if self.over_ms > 10.0 && self.over_count > 1 && trend >= self.prev_trend {
                self.over_ms = 0.0;
                self.over_count = 0;
                self.usage = Usage::Over;
            }
        } else if trend < -self.threshold {
            self.over_ms = -1.0;
            self.over_count = 0;
            self.usage = Usage::Under;
        } else {
            self.over_ms = -1.0;
            self.over_count = 0;
            self.usage = Usage::Normal;
        }
        self.prev_trend = trend;
        // Adapt the threshold toward |trend|, quickly down, slowly up.
        let last = self.last_update_ms.unwrap_or(now_ms);
        let dt = (now_ms - last).min(100.0);
        self.last_update_ms = Some(now_ms);
        if fabs(trend) <= self.threshold + 15.0 {
            let k = if fabs(trend) < self.threshold { 0.039 } else { 0.0087 };
            self.threshold += k * (fabs(trend) - self.threshold) * dt;
            self.threshold = self.threshold.clamp(6.0, 600.0);
        }
    }

    fn update_rate(&mut self, now_us: i64) {
        let acked = self.acked_bps();
        let last = *self.last_change_us.get_or_insert(now_us);
        let dt_s = ((now_us - last) as f64 / 1e6).min(1.0);
        match self.usage {
            Usage::Over => {
                // Decrease once per round trip at most.
                if now_us - last >= (self.rtt_ms as i64 * 1000).max(100_000) || self.increasing {
                    let base = acked.unwrap_or(self.delay_bps);
                    let new = 0.85 * base;
                    if new < self.delay_bps {
                        self.delay_bps = new;
                    }
                    self.link_bps = Some(base);
                    self.increasing = false;
                    self.last_change_us = Some(now_us);
                }
            }
            Usage::Under => {
                self.last_change_us = Some(now_us);
            }
            Usage::Normal => {
                self.increasing = true;
                let near = self.link_bps.map_or(false, |l| self.delay_bps > 0.75 * l && self.delay_bps < 1.5 * l);
                let step = if near {
                    // About one packet per response time.
                    let rt = (self.rtt_ms as f64 + 100.0) / 1000.0;
                    1200.0 * 8.0 * dt_s / rt
                } else {
                    self.delay_bps * 0.08 * dt_s
                };
                self.delay_bps += step.max(0.0);
                if let Some(a) = acked {
                    self.delay_bps = self.delay_bps.min(1.5 * a + 10_000.0);
                }
                self.last_change_us = Some(now_us);
            }
        }
        self.delay_bps = self.delay_bps.clamp(self.min_bps as f64, self.max_bps as f64);
    }

    fn update_loss(&mut self, loss: f64, now_us: i64) {
        self.last_loss = loss;
        if now_us - self.last_loss_change_us < 200_000 {
            return;
        }
        self.last_loss_change_us = now_us;
        if loss < 0.02 {
            self.loss_bps = (self.loss_bps * 1.05).max(self.delay_bps.min(self.loss_bps));
        } else if loss > 0.10 {
            self.loss_bps *= 1.0 - 0.5 * loss;
        }
        self.loss_bps = self.loss_bps.clamp(self.min_bps as f64, self.max_bps as f64);
    }
}
