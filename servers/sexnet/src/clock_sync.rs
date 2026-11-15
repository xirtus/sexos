// SPDX-License-Identifier: GPL-3.0-only
//! Network time (`sex_pdx::network_time`): once the open side is up, ask a time server
//! over SNTP and set civil time; again every hour, sooner after a failure. The kernel
//! keeps the CMOS reading and applies the step; sexnet is its one time keeper.
use crate::jobs::Identity;
use sex_net::{Address, Engine};
use sex_pdx::network_time as n;

/// sexnet's own hold on the resolver and on a UDP socket of the open side.
pub const SELF: Identity = Identity { pd: u32::MAX, task: u32::MAX };
const RESOLVE_MS: u64 = 5_000;
const REPLY_MS: u64 = 3_000;
/// After a failure: 1 min, doubling to the hourly check.
const RETRY_MS: u64 = 60_000;
/// Smaller offsets are left alone (the tick carries time well enough between checks).
const STEP_MS: i64 = 50;

#[derive(Clone, Copy)]
enum Stage { Idle, Resolving { since: u64 }, Waiting { socket: usize, query: sex_ntp::Query, sent: u64, since: u64, server: Address } }
pub struct Sync { enabled: bool, stage: Stage, next_at: u64, backoff: u64, state: n::State, step: i64, synced_at: Option<u64> }

impl Sync {
    pub const fn new() -> Self { Self { enabled: true, stage: Stage::Idle, next_at: 0, backoff: RETRY_MS, state: n::State::Waiting, step: 0, synced_at: None } }
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.state = if on { n::State::Waiting } else { n::State::Off };
        if on { self.next_at = 0; }
        sex_pdx::serial_record!("[net.time.enabled] on={}", on as u8);
    }
    pub fn idle(&self) -> bool { matches!(self.stage, Stage::Idle) }
    pub fn sync_now(&mut self) { if self.enabled { self.next_at = 0; self.backoff = RETRY_MS; } }
    pub fn status(&self, now: u64) -> u64 {
        n::status_word(self.state, self.step, self.synced_at.map_or(u64::MAX, |t| now.saturating_sub(t) / 60_000))
    }
    /// The open side went away with its stack; any question in flight with it.
    pub fn reset(&mut self) { self.stage = Stage::Idle; self.next_at = 0; }
    fn failed(&mut self, now: u64, why: &str) {
        sex_pdx::serial_record!("[net.time.failed] why={} retry_ms={}", why, self.backoff);
        self.state = n::State::Failed;
        self.next_at = now + self.backoff;
        self.backoff = (self.backoff * 2).min(n::RESYNC_MS);
        self.stage = Stage::Idle;
    }
    /// One turn. `resolving` is the open side's one resolver question; `socket` gives a
    /// UDP socket held for sexnet (and `release` lets it go).
    pub fn turn(&mut self, engine: &mut Engine<'static>, now: u64, resolving: &mut Option<(Identity, u64)>,
        socket: &mut dyn FnMut(&mut Engine<'static>) -> Option<usize>, release: &mut dyn FnMut(&mut Engine<'static>, usize)) {
        match self.stage {
            Stage::Idle => {
                if !self.enabled || now < self.next_at || !engine.open_ready() { return; }
                // Programs' questions come first: ask when the resolver is free.
                if resolving.is_some_and(|(o, at)| o != SELF && now.saturating_sub(at) < RESOLVE_MS) { return; }
                if engine.open_resolve_all(n::SERVER).is_err() { return self.failed(now, "resolve"); }
                *resolving = Some((SELF, now));
                self.state = n::State::Asking;
                self.stage = Stage::Resolving { since: now };
            }
            Stage::Resolving { since } => {
                // A program asked meanwhile and took the resolver: ask again shortly.
                if resolving.map(|(o, _)| o) != Some(SELF) { self.stage = Stage::Idle; self.next_at = now + 10_000; return; }
                let found = match engine.open_resolved_all() {
                    Ok(None) if now.saturating_sub(since) < RESOLVE_MS => return,
                    Ok(Some(found)) => found,
                    _ => { *resolving = None; return self.failed(now, "no_server"); }
                };
                *resolving = None;
                // IPv6 first: it was asked only when the machine can use it.
                let Some(server) = found.v6.map(Address::V6).or(found.v4.map(Address::V4)) else { return self.failed(now, "no_server") };
                let Some(k) = socket(engine) else { return self.failed(now, "socket") };
                if engine.open_udp_bind(k, 0).is_err() { release(engine, k); return self.failed(now, "bind"); }
                let Ok(sent) = sex_pdx::civil_time_ms() else { release(engine, k); return self.failed(now, "clock") };
                let nonce = sex_pdx::entropy_word(sex_pdx::browser_sources::PLANS[0].entropy_slot()).map_or(now as u32, |w| w.value as u32);
                let (packet, query) = sex_ntp::request(sent, nonce);
                if !matches!(engine.open_udp_send(k, server, sex_ntp::PORT, &packet), Ok(true)) { release(engine, k); return self.failed(now, "send"); }
                self.stage = Stage::Waiting { socket: k, query, sent, since: now, server };
            }
            Stage::Waiting { socket: k, query, sent, since, server } => {
                let mut reply = [0u8; 128];
                let got = engine.open_udp_receive(k, &mut reply);
                let Ok(Some((from, port, len))) = got else {
                    if got.is_err() || now.saturating_sub(since) >= REPLY_MS { release(engine, k); self.failed(now, "no_reply"); }
                    return;
                };
                // Anything not from the server we asked is dropped, and we go on waiting.
                if from != server || port != sex_ntp::PORT { return; }
                let Ok(received) = sex_pdx::civil_time_ms() else { return };
                match query.answer(&reply[..len], sent, received) {
                    Ok(sample) => {
                        release(engine, k);
                        self.stage = Stage::Idle;
                        let mut step = 0;
                        if sample.offset_ms.abs() >= STEP_MS {
                            let to = received.saturating_add_signed(sample.offset_ms);
                            if let Err(e) = sex_pdx::set_civil_time_ms(to) { return self.failed(now, if e == sex_pdx::clock_source::DENIED { "denied" } else { "set" }); }
                            step = sample.offset_ms;
                        }
                        self.step = step; self.state = n::State::Synced; self.synced_at = Some(now);
                        self.backoff = RETRY_MS; self.next_at = now + n::RESYNC_MS;
                        sex_pdx::serial_record!("[net.time.synced] offset_ms={} delay_ms={} stratum={} stepped={}", sample.offset_ms, sample.delay_ms, sample.stratum, (step != 0) as u8);
                    }
                    // A stray or spoofed reply: wait for the real one.
                    Err(sex_ntp::Refused::NotOurs) => {}
                    Err(sex_ntp::Refused::Kiss(_)) => { release(engine, k); self.backoff = n::RESYNC_MS; self.failed(now, "kiss") }
                    Err(_) => { release(engine, k); self.failed(now, "bad_reply") }
                }
            }
        }
    }
}
