// SPDX-License-Identifier: GPL-3.0-only
//! Bandwidth limits, the alternative-speed schedule, seeding rules and queueing.
//!
//! Pure state, no clock of its own: every call takes the time.
//! - [`TokenBucket`]: a byte-rate limiter with exact integer refill (no drift).
//! - [`RateLimiter`]: global, per-torrent and per-peer buckets for each direction.
//!   [`RateLimiter::allocate`] splits what is available fairly: max-min fair share
//!   between the peers of a torrent within the torrent's budget, then between all peers
//!   within the global budget, so one fast peer cannot starve the others.
//! - [`Schedule`] and [`SpeedPolicy`]: Transmission's "turtle mode" (alternative speed
//!   limits) switched on by hand or by weekly time windows, with [`SpeedPolicy::next_change`]
//!   so the program can sleep until the next switch.
//! - [`check_seeding`]: stop or remove a torrent at a ratio, after a seeding time, or
//!   after being idle, with per-torrent overrides of the global limits.
//! - [`plan_queue`]: which torrents may run under max active downloads/uploads/total,
//!   where forced torrents and (optionally) slow torrents do not count.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::Millis;

/// Transfer direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction { Up = 0, Down = 1 }

/// A token bucket: `rate` bytes per second, holding at most `burst` bytes. Rate 0 means
/// unlimited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenBucket {
    rate: u64,
    burst: u64,
    tokens: u64,
    /// Fractional tokens, in thousandths of a byte.
    milli: u64,
    last: Millis,
}

impl TokenBucket {
    /// A bucket with one second of burst, starting full.
    pub fn new(rate: u64, now: Millis) -> TokenBucket {
        TokenBucket { rate, burst: rate.max(1), tokens: rate.max(1), milli: 0, last: now }
    }
    pub fn rate(&self) -> u64 { self.rate }
    pub fn unlimited(&self) -> bool { self.rate == 0 }
    /// Changes the rate (and burst, one second of it); tokens are capped to the new burst.
    /// A bucket that was unlimited starts full.
    pub fn set_rate(&mut self, rate: u64) {
        let was_unlimited = self.rate == 0;
        self.rate = rate;
        self.burst = rate.max(1);
        self.tokens = if was_unlimited { self.burst } else { self.tokens.min(self.burst) };
    }
    /// Sets the burst explicitly (at least 1 byte).
    pub fn set_burst(&mut self, burst: u64) { self.burst = burst.max(1); self.tokens = self.tokens.min(self.burst); }
    fn refill(&mut self, now: Millis) {
        if now <= self.last { return; }
        let add = (self.rate as u128) * ((now - self.last) as u128) + self.milli as u128;
        let whole = add / 1000;
        self.milli = (add % 1000) as u64;
        self.tokens = (self.tokens as u128 + whole).min(self.burst as u128) as u64;
        if self.tokens == self.burst { self.milli = 0; }
        self.last = now;
    }
    /// Bytes that may be sent now (`u64::MAX` when unlimited).
    pub fn available(&mut self, now: Millis) -> u64 {
        if self.unlimited() { return u64::MAX; }
        self.refill(now);
        self.tokens
    }
    /// Takes `n` bytes (as many as there are).
    pub fn consume(&mut self, n: u64) { if !self.unlimited() { self.tokens = self.tokens.saturating_sub(n); } }
    /// Takes up to `want` bytes and returns how many were granted.
    pub fn request(&mut self, now: Millis, want: u64) -> u64 {
        let g = want.min(self.available(now));
        self.consume(g);
        g
    }
    /// Milliseconds until `n` bytes (capped at the burst) are available.
    pub fn wait_for(&mut self, now: Millis, n: u64) -> Millis {
        if self.unlimited() { return 0; }
        self.refill(now);
        let n = n.min(self.burst);
        if self.tokens >= n { return 0; }
        let need = (n - self.tokens) as u128 * 1000 - self.milli as u128;
        need.div_ceil(self.rate as u128) as Millis
    }
}

/// One peer's wish to transfer bytes, for [`RateLimiter::allocate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Demand { pub peer: u64, pub torrent: u64, pub want: u64 }

/// Max-min fair split of `total` between wishes `caps`: nobody gets more than they
/// want, and what one does not use is shared by the others.
pub fn fair_share(caps: &[u64], total: u64) -> Vec<u64> {
    let sum: u128 = caps.iter().map(|&c| c as u128).sum();
    if sum <= total as u128 { return caps.to_vec(); }
    let mut idx: Vec<usize> = (0..caps.len()).collect();
    idx.sort_by_key(|&i| caps[i]);
    let mut out = alloc::vec![0u64; caps.len()];
    let mut left = total;
    for (k, &i) in idx.iter().enumerate() {
        let share = left / (caps.len() - k) as u64;
        let g = caps[i].min(share);
        out[i] = g;
        left -= g;
    }
    // Hand out the division remainder one byte at a time to those still wanting.
    for &i in idx.iter().rev() { if left == 0 { break; } if out[i] < caps[i] { out[i] += 1; left -= 1; } }
    out
}

/// Global, per-torrent and per-peer token buckets for both directions. Torrents and
/// peers are named by the program's own handles; one without a limit set is unlimited
/// at that level.
#[derive(Clone, Debug)]
pub struct RateLimiter {
    global: [TokenBucket; 2],
    torrents: BTreeMap<u64, [TokenBucket; 2]>,
    peers: BTreeMap<u64, [TokenBucket; 2]>,
}

impl RateLimiter {
    /// Global limits in bytes/s (0 = unlimited).
    pub fn new(up: u64, down: u64, now: Millis) -> RateLimiter {
        RateLimiter { global: [TokenBucket::new(up, now), TokenBucket::new(down, now)], torrents: BTreeMap::new(), peers: BTreeMap::new() }
    }
    pub fn set_global(&mut self, d: Direction, rate: u64) { self.global[d as usize].set_rate(rate); }
    pub fn global_rate(&self, d: Direction) -> u64 { self.global[d as usize].rate() }
    pub fn set_torrent(&mut self, t: u64, d: Direction, rate: u64, now: Millis) {
        self.torrents.entry(t).or_insert_with(|| [TokenBucket::new(0, now), TokenBucket::new(0, now)])[d as usize].set_rate(rate);
    }
    pub fn set_peer(&mut self, p: u64, d: Direction, rate: u64, now: Millis) {
        self.peers.entry(p).or_insert_with(|| [TokenBucket::new(0, now), TokenBucket::new(0, now)])[d as usize].set_rate(rate);
    }
    pub fn remove_torrent(&mut self, t: u64) { self.torrents.remove(&t); }
    pub fn remove_peer(&mut self, p: u64) { self.peers.remove(&p); }

    /// Grants bytes to each demand (same order), fairly, and takes them from the buckets.
    pub fn allocate(&mut self, now: Millis, d: Direction, demands: &[Demand]) -> Vec<u64> {
        let di = d as usize;
        let mut caps: Vec<u64> = demands.iter().map(|x| match self.peers.get_mut(&x.peer) {
            Some(b) => x.want.min(b[di].available(now)),
            None => x.want,
        }).collect();
        // Fair share within each limited torrent.
        let mut groups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (i, x) in demands.iter().enumerate() { groups.entry(x.torrent).or_default().push(i); }
        for (t, members) in &groups {
            if let Some(b) = self.torrents.get_mut(t) {
                let avail = b[di].available(now);
                let sub: Vec<u64> = members.iter().map(|&i| caps[i]).collect();
                for (k, g) in fair_share(&sub, avail).into_iter().enumerate() { caps[members[k]] = g; }
            }
        }
        let grants = fair_share(&caps, self.global[di].available(now));
        for (i, x) in demands.iter().enumerate() {
            let g = grants[i];
            if let Some(b) = self.peers.get_mut(&x.peer) { b[di].consume(g); }
            if let Some(b) = self.torrents.get_mut(&x.torrent) { b[di].consume(g); }
            self.global[di].consume(g);
        }
        grants
    }
}

/// Upload and download limits in bytes/s (0 = unlimited).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Limits { pub up: u64, pub down: u64 }

/// Day bits (Transmission's `alt-speed-time-day`): bit 0 is Sunday.
pub const SUNDAY: u8 = 1;
pub const MONDAY: u8 = 2;
pub const TUESDAY: u8 = 4;
pub const WEDNESDAY: u8 = 8;
pub const THURSDAY: u8 = 16;
pub const FRIDAY: u8 = 32;
pub const SATURDAY: u8 = 64;
pub const WEEKDAYS: u8 = 62;
pub const WEEKEND: u8 = 65;
pub const ALL_DAYS: u8 = 127;

/// A weekly time window: on `days`, from `begin` to `end` (minutes after local
/// midnight). When `begin > end` the window runs past midnight and its tail belongs to
/// the day it started on. `begin == end` is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window { pub days: u8, pub begin: u16, pub end: u16 }

impl Window {
    /// Whether the window covers `minute` on `weekday` (0 = Sunday).
    pub fn active(&self, weekday: u8, minute: u16) -> bool {
        let on = |d: u8| self.days & (1 << (d % 7)) != 0;
        if self.begin < self.end { on(weekday) && minute >= self.begin && minute < self.end }
        else if self.begin > self.end { (on(weekday) && minute >= self.begin) || (on((weekday + 6) % 7) && minute < self.end) }
        else { false }
    }
}

/// Local weekday (0 = Sunday) and minute of day for a unix time and UTC offset.
pub fn local_time(unix: i64, utc_offset_secs: i32) -> (u8, u16) {
    let t = unix + utc_offset_secs as i64;
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // 1970-01-01 was a Thursday.
    (((days + 4).rem_euclid(7)) as u8, (secs / 60) as u16)
}

/// A set of weekly windows; active when any window is.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Schedule { pub windows: Vec<Window> }

impl Schedule {
    pub fn active(&self, unix: i64, utc_offset_secs: i32) -> bool {
        let (d, m) = local_time(unix, utc_offset_secs);
        self.windows.iter().any(|w| w.active(d, m))
    }
}

/// Normal and alternative ("turtle") limits, and when the alternative ones apply.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SpeedPolicy {
    pub normal: Limits,
    pub alt: Limits,
    /// Turtle mode switched on by hand.
    pub turtle: bool,
    /// Whether the schedule switches turtle mode.
    pub scheduled: bool,
    pub schedule: Schedule,
}

impl SpeedPolicy {
    /// Whether the alternative limits apply now.
    pub fn alt_active(&self, unix: i64, utc_offset_secs: i32) -> bool {
        self.turtle || (self.scheduled && self.schedule.active(unix, utc_offset_secs))
    }
    /// The limits in effect now.
    pub fn effective(&self, unix: i64, utc_offset_secs: i32) -> Limits {
        if self.alt_active(unix, utc_offset_secs) { self.alt } else { self.normal }
    }
    /// The next unix time (on a minute boundary) when the scheduled state changes, if
    /// within the next eight days.
    pub fn next_change(&self, unix: i64, utc_offset_secs: i32) -> Option<i64> {
        if !self.scheduled || self.turtle { return None; }
        let now_state = self.schedule.active(unix, utc_offset_secs);
        let local = unix + utc_offset_secs as i64;
        let midnight = local - local.rem_euclid(86_400) - utc_offset_secs as i64;
        let mut cands: Vec<i64> = Vec::new();
        for day in -1..=8i64 {
            for w in &self.schedule.windows {
                for m in [w.begin, w.end] { cands.push(midnight + day * 86_400 + (m.min(1440) as i64) * 60); }
            }
        }
        cands.sort_unstable();
        cands.into_iter().filter(|&t| t > unix).find(|&t| self.schedule.active(t, utc_offset_secs) != now_state)
    }
}

/// A per-torrent setting that can defer to the global one.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Limit<T> { #[default] Global, Unlimited, Value(T) }

impl<T: Copy> Limit<T> {
    fn resolve(self, global: Option<T>) -> Option<T> {
        match self { Limit::Global => global, Limit::Unlimited => None, Limit::Value(v) => Some(v) }
    }
}

/// What to do when a seeding limit is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SeedAction { #[default] Pause, Remove, RemoveWithData }

/// Which limit was reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedReason { Ratio, SeedingTime, Idle }

/// Global seeding limits.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SeedLimits {
    pub ratio: Option<f64>,
    pub seeding_minutes: Option<u32>,
    pub idle_minutes: Option<u32>,
    pub action: SeedAction,
}

/// A torrent's overrides of the global seeding limits.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SeedOverride {
    pub ratio: Limit<f64>,
    pub seeding_minutes: Limit<u32>,
    pub idle_minutes: Limit<u32>,
}

/// What a torrent has done, for [`check_seeding`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SeedStats {
    pub uploaded: u64,
    pub downloaded: u64,
    /// Size of the wanted data (the ratio's denominator when nothing was downloaded,
    /// as when a complete torrent was added).
    pub size: u64,
    /// Unix seconds when it became complete; `None` while downloading.
    pub seeding_since: Option<i64>,
    /// Unix seconds of the last upload or download.
    pub last_activity: i64,
}

/// Share ratio: uploaded over downloaded (or over size when nothing was downloaded).
pub fn ratio(s: &SeedStats) -> f64 {
    let d = if s.downloaded > 0 { s.downloaded } else { s.size };
    if d == 0 { if s.uploaded > 0 { f64::INFINITY } else { 0.0 } } else { s.uploaded as f64 / d as f64 }
}

/// Whether a seeding torrent has reached a limit, and what to do.
pub fn check_seeding(global: &SeedLimits, ov: &SeedOverride, s: &SeedStats, now: i64) -> Option<(SeedAction, SeedReason)> {
    let since = s.seeding_since?;
    if let Some(r) = ov.ratio.resolve(global.ratio) { if ratio(s) >= r { return Some((global.action, SeedReason::Ratio)); } }
    if let Some(m) = ov.seeding_minutes.resolve(global.seeding_minutes) {
        if now - since >= m as i64 * 60 { return Some((global.action, SeedReason::SeedingTime)); }
    }
    if let Some(m) = ov.idle_minutes.resolve(global.idle_minutes) {
        if now - s.last_activity.max(since) >= m as i64 * 60 { return Some((global.action, SeedReason::Idle)); }
    }
    None
}

/// Queue limits (`None` = no limit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct QueueConfig {
    pub max_downloads: Option<u32>,
    pub max_uploads: Option<u32>,
    pub max_total: Option<u32>,
    /// Slow torrents do not count toward the limits.
    pub ignore_slow: bool,
    /// Below these rates (bytes/s) a running torrent is slow...
    pub slow_down: u64,
    pub slow_up: u64,
    /// ...once it has been running at least this long.
    pub slow_grace_secs: u32,
}

/// What a torrent wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want { Download, Seed, Paused }

/// A torrent as the queue sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    pub id: u64,
    pub position: u32,
    pub want: Want,
    /// "Start now": bypasses the queue.
    pub forced: bool,
    pub running: bool,
    pub rate_down: u64,
    pub rate_up: u64,
    /// Unix seconds since it is running.
    pub running_since: Option<i64>,
}

/// The queue's verdict for one torrent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueVerdict { pub id: u64, pub run: bool, pub counted: bool }

/// Decides which torrents run, in queue order (position, then id).
pub fn plan_queue(cfg: &QueueConfig, entries: &[QueueEntry], now: i64) -> Vec<QueueVerdict> {
    let mut v: Vec<&QueueEntry> = entries.iter().collect();
    v.sort_by_key(|e| (e.position, e.id));
    let (mut dl, mut ul, mut total) = (0u32, 0u32, 0u32);
    let under = |n: u32, max: Option<u32>| max.map_or(true, |m| n < m);
    v.into_iter().map(|e| {
        let (run, counted) = match e.want {
            Want::Paused => (false, false),
            _ if e.forced => (true, false),
            w => {
                let slow = cfg.ignore_slow && e.running
                    && e.running_since.is_some_and(|s| now - s >= cfg.slow_grace_secs as i64)
                    && match w { Want::Download => e.rate_down < cfg.slow_down, _ => e.rate_up < cfg.slow_up };
                if slow { (true, false) }
                else {
                    let (n, max) = if w == Want::Download { (&mut dl, cfg.max_downloads) } else { (&mut ul, cfg.max_uploads) };
                    if under(*n, max) && under(total, cfg.max_total) { *n += 1; total += 1; (true, true) } else { (false, false) }
                }
            }
        };
        QueueVerdict { id: e.id, run, counted }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn bucket_refill_is_exact() {
        let mut b = TokenBucket::new(1000, 0);
        assert_eq!(b.request(0, 5000), 1000);
        assert_eq!(b.available(0), 0);
        // 1 byte per ms; 3 ms in thirds still adds up.
        let mut b = TokenBucket::new(333, 0);
        b.consume(333);
        for t in 1..=3000 { b.available(t); }
        assert_eq!(b.available(3000), 333); // capped at burst
        let mut b = TokenBucket::new(333, 0);
        b.consume(333);
        assert_eq!(b.available(999), 332);
        assert_eq!(b.available(1000), 333);
        assert_eq!(b.wait_for(1000, 10), 0);
        b.consume(333);
        assert_eq!(b.wait_for(1000, 100), 301);
        let mut u = TokenBucket::new(0, 0);
        assert_eq!(u.request(0, u64::MAX), u64::MAX);
        b.set_rate(10);
        assert!(b.available(1000) <= 10);
        // Time going backwards does nothing odd.
        assert!(b.available(5) <= 10);
    }

    #[test]
    fn fair_share_is_max_min() {
        assert_eq!(fair_share(&[10, 20], 100), vec![10, 20]);
        assert_eq!(fair_share(&[10, 100, 100], 100), vec![10, 45, 45]);
        assert_eq!(fair_share(&[50, 50, 50], 100).iter().sum::<u64>(), 100);
        assert_eq!(fair_share(&[], 100), Vec::<u64>::new());
        assert_eq!(fair_share(&[u64::MAX, u64::MAX], 7).iter().sum::<u64>(), 7);
    }

    #[test]
    fn limiter_levels() {
        let mut l = RateLimiter::new(0, 10_000, 0);
        l.set_torrent(1, Direction::Down, 4_000, 0);
        l.set_peer(11, Direction::Down, 1_000, 0);
        let d = [
            Demand { peer: 11, torrent: 1, want: 5_000 },
            Demand { peer: 12, torrent: 1, want: 5_000 },
            Demand { peer: 21, torrent: 2, want: 50_000 },
            Demand { peer: 22, torrent: 2, want: 500 },
        ];
        let g = l.allocate(0, Direction::Down, &d);
        // Peer 11 capped at 1000, torrent 1 total 4000, global 10000 shared fairly.
        assert_eq!(g[0], 1_000);
        assert_eq!(g[1], 3_000);
        assert_eq!(g[3], 500);
        assert_eq!(g.iter().sum::<u64>(), 10_000);
        // Everything is spent; half a second later half the global budget is back.
        assert_eq!(l.allocate(0, Direction::Down, &d).iter().sum::<u64>(), 0);
        assert_eq!(l.allocate(500, Direction::Down, &d).iter().sum::<u64>(), 5_000);
        // Upload is unlimited.
        assert_eq!(l.allocate(500, Direction::Up, &d)[2], 50_000);
        l.remove_peer(11);
        l.remove_torrent(1);
        l.set_global(Direction::Down, 0);
        assert_eq!(l.global_rate(Direction::Down), 0);
        assert_eq!(l.allocate(500, Direction::Down, &d)[0], 5_000);
    }

    #[test]
    fn weekday_and_windows() {
        // 2026-11-14 is a Saturday.
        let sat_noon = crate::rss::days_from_civil(2026, 11, 14) * 86_400 + 12 * 3600;
        assert_eq!(local_time(sat_noon, 0), (6, 720));
        assert_eq!(local_time(sat_noon, 13 * 3600), (0, 60)); // Sunday 01:00 in UTC+13
        assert_eq!(local_time(0, -3600), (3, 1380)); // Wednesday 23:00
        let night = Window { days: FRIDAY, begin: 22 * 60, end: 6 * 60 };
        assert!(night.active(5, 23 * 60));
        assert!(night.active(6, 5 * 60)); // Saturday early morning belongs to Friday night
        assert!(!night.active(6, 23 * 60));
        assert!(!night.active(5, 5 * 60));
        let office = Window { days: WEEKDAYS, begin: 9 * 60, end: 17 * 60 };
        assert!(office.active(1, 9 * 60) && !office.active(1, 17 * 60) && !office.active(0, 600));
        assert!(!Window { days: ALL_DAYS, begin: 5, end: 5 }.active(1, 5));
    }

    #[test]
    fn turtle_policy() {
        let sat_noon = crate::rss::days_from_civil(2026, 11, 14) * 86_400 + 12 * 3600;
        let mut p = SpeedPolicy {
            normal: Limits { up: 0, down: 0 }, alt: Limits { up: 50_000, down: 100_000 }, turtle: false, scheduled: true,
            schedule: Schedule { windows: vec![Window { days: WEEKEND, begin: 10 * 60, end: 14 * 60 }] },
        };
        assert!(p.alt_active(sat_noon, 0));
        assert_eq!(p.effective(sat_noon, 0), p.alt);
        assert_eq!(p.next_change(sat_noon, 0), Some(sat_noon + 2 * 3600));
        assert_eq!(p.next_change(sat_noon + 2 * 3600, 0), Some(sat_noon + 22 * 3600)); // Sunday 10:00
        assert_eq!(p.next_change(sat_noon + 2 * 3600, 3600), Some(sat_noon + 21 * 3600)); // local 10:00 at UTC+1
        p.turtle = true;
        assert!(p.alt_active(sat_noon + 6 * 3600, 0));
        assert_eq!(p.next_change(sat_noon, 0), None);
        p.turtle = false;
        p.scheduled = false;
        assert_eq!(p.effective(sat_noon, 0), p.normal);
    }

    #[test]
    fn seeding_rules() {
        let g = SeedLimits { ratio: Some(2.0), seeding_minutes: Some(60 * 24), idle_minutes: Some(30), action: SeedAction::Pause };
        let mut s = SeedStats { uploaded: 100, downloaded: 100, size: 100, seeding_since: Some(1000), last_activity: 1000 };
        assert_eq!(check_seeding(&g, &SeedOverride::default(), &s, 1000), None);
        s.uploaded = 200;
        assert_eq!(check_seeding(&g, &SeedOverride::default(), &s, 1000), Some((SeedAction::Pause, SeedReason::Ratio)));
        let ov = SeedOverride { ratio: Limit::Unlimited, ..SeedOverride::default() };
        assert_eq!(check_seeding(&g, &ov, &s, 1000), None);
        assert_eq!(check_seeding(&g, &ov, &s, 1000 + 30 * 60), Some((SeedAction::Pause, SeedReason::Idle)));
        let ov = SeedOverride { ratio: Limit::Value(5.0), idle_minutes: Limit::Unlimited, seeding_minutes: Limit::Value(10) };
        assert_eq!(check_seeding(&g, &ov, &s, 1000 + 600), Some((SeedAction::Pause, SeedReason::SeedingTime)));
        s.seeding_since = None;
        assert_eq!(check_seeding(&g, &ov, &s, 99_999), None);
        // Added complete: ratio uses size.
        assert_eq!(ratio(&SeedStats { uploaded: 50, downloaded: 0, size: 100, ..SeedStats::default() }), 0.5);
        assert!(ratio(&SeedStats { uploaded: 1, ..SeedStats::default() }).is_infinite());
        assert_eq!(ratio(&SeedStats::default()), 0.0);
    }

    #[test]
    fn queue() {
        let cfg = QueueConfig { max_downloads: Some(2), max_uploads: Some(1), max_total: Some(3), ignore_slow: true, slow_down: 2048, slow_up: 2048, slow_grace_secs: 60 };
        let e = |id, position, want, running, rate| QueueEntry { id, position, want, forced: false, running, rate_down: rate, rate_up: rate, running_since: running.then_some(0) };
        let mut list = vec![
            e(1, 0, Want::Download, true, 100),      // slow: runs, not counted
            e(2, 1, Want::Download, true, 1 << 20),
            e(3, 2, Want::Download, false, 0),
            e(4, 3, Want::Download, false, 0),       // over the download limit
            e(5, 4, Want::Seed, false, 0),
            e(6, 5, Want::Seed, false, 0),           // over the upload limit
            e(7, 6, Want::Paused, true, 0),
        ];
        list.push(QueueEntry { forced: true, ..e(8, 7, Want::Download, false, 0) });
        let v = plan_queue(&cfg, &list, 120);
        let runs: Vec<(u64, bool, bool)> = v.iter().map(|q| (q.id, q.run, q.counted)).collect();
        assert_eq!(runs, vec![(1, true, false), (2, true, true), (3, true, true), (4, false, false), (5, true, true), (6, false, false), (7, false, false), (8, true, false)]);
        // Within the grace period the slow torrent still counts.
        let v = plan_queue(&cfg, &list, 30);
        assert_eq!((v[0].counted, v[2].run), (true, false));
        let unlimited = plan_queue(&QueueConfig::default(), &list, 0);
        assert_eq!(unlimited.iter().filter(|q| q.run).count(), 7);
    }
}
