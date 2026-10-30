// SPDX-License-Identifier: GPL-3.0-only
//! Explicit Spindle diagnostic intent around the shared bounded echo protocol.
//! The inner implementation's `browser.net.job.*` serial labels are historical
//! shared-protocol diagnostics, not attribution to a Browser action. Main must
//! attribute Spindle results using its held self catalog and this wrapper view.
//! No route is acquired here: BOOT must already have supplied SLOT_NET.
#[path = "../../kaleidoscope/src/network_probe.rs"]
mod shared;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View { Idle, Pending, Verified, Unavailable, Cancelled, Uncertain }

pub struct Client {
    probe: shared::Probe,
    started: bool,
    closed: bool,
    generation: u64,
}

impl Client {
    pub const fn new() -> Self {
        Self { probe: shared::Probe::new(), started: false, closed: false, generation: 0 }
    }
    pub fn busy(&self) -> bool { self.started && !self.closed && self.probe.busy() }
    pub fn next_sequence(&self) -> u32 { self.probe.next_sequence() }
    pub fn view(&self) -> View {
        if self.closed { return View::Cancelled; }
        if !self.started { return View::Idle; }
        match self.probe.view() {
            shared::View::Pending => View::Pending,
            // Byte validation precedes RELEASE in the shared machine. The
            // public success receipt is withheld until its positive cleanup.
            shared::View::Verified if self.probe.busy() => View::Pending,
            shared::View::Verified => View::Verified,
            shared::View::Unavailable => View::Unavailable,
            shared::View::Cancelled => View::Cancelled,
            shared::View::Uncertain => View::Uncertain,
        }
    }
    /// Changes only when the public (view, busy) observation changes. Main can
    /// coalesce scrollback/status publication without painting every RPC turn.
    pub fn status_generation(&self) -> u64 { self.generation }
    pub fn status(&self) -> &'static str {
        match self.view() {
            View::Idle => "idle", View::Pending => "pending", View::Verified => "verified",
            View::Unavailable => "unavailable", View::Cancelled => "cancelled",
            View::Uncertain => "waiting receipt",
        }
    }
    fn observation(&self) -> (View, bool) { (self.view(), self.busy()) }
    fn changed(&mut self, before: (View, bool)) {
        if self.observation() != before { self.generation = self.generation.saturating_add(1); }
    }
    /// Each call is fresh explicit intent. While busy it queues cleanup before
    /// the new run; accepted mutations are never replayed by this operation.
    pub fn start_or_retry(&mut self) -> bool {
        if self.closed { return false; }
        let before = self.observation();
        if self.started { self.probe.retry(); } else { self.started = true; }
        self.changed(before);
        true
    }
    /// Last explicit cancel wins over an earlier queued retry. An accepted
    /// late BEGIN remains correlated until CANCEL and positive RELEASE.
    pub fn cancel(&mut self) -> bool {
        if !self.busy() { return false; }
        let before = self.observation();
        self.probe.cancel();
        self.changed(before);
        true
    }
    pub fn reply(&mut self, word: u64, now: u64) {
        if !self.busy() { return; }
        let before = self.observation();
        self.probe.reply(word, now);
        self.changed(before);
    }
    /// At most one shared-machine enqueue per main-loop turn. Initial idle
    /// calls have no PDX effects even though the inner constructor is active.
    pub fn advance(&mut self, now: u64) {
        if !self.busy() { return; }
        let before = self.observation();
        self.probe.advance(now);
        self.changed(before);
    }
    /// Terminal-only local barrier, immediately before task_finish. Service
    /// custody still requires the exact task's normal retire-before-ACK path;
    /// this method is not a positive RELEASE and must not enable another run.
    pub fn close(&mut self) {
        let before = self.observation();
        self.closed = true;
        self.changed(before);
    }
}
