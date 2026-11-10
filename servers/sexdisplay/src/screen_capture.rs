//! One-shot screenshot custody; no scanout pointer or file authority enters here.
use crate::surface_lifecycle::{Lifecycle, Phase, CATALOG_CAPACITY};
use sex_pdx::screen_capture as c;
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Waiting,
    Ready,
}
pub struct Server {
    epoch: u64,
    bank: Option<c::Bank>,
    state: State,
    task: u32,
    sequence: u32,
    started: u64,
    pending: Option<u64>,
}
impl Server {
    pub const fn new() -> Self {
        Self {
            epoch: 0,
            bank: None,
            state: State::Idle,
            task: 0,
            sequence: 0,
            started: 0,
            pending: None,
        }
    }
    pub fn attach(&mut self, epoch: u64) {
        self.epoch = epoch;
        self.bank = sex_pdx::screen_capture_bank(true).ok();
        if let Some(bank) = self.bank.as_mut() {
            bank.scrub();
            bank.install_epoch(epoch);
        }
        if epoch == 0 {
            self.bank = None;
        }
    }
    pub fn live(lifecycle: &Lifecycle, task: u32) -> bool {
        (0..CATALOG_CAPACITY).any(|i| {
            lifecycle
                .catalog(i)
                .is_some_and(|r| r.target_pd == c::CLIENT && r.task_id == task)
                && lifecycle.phase(i) == Some(Phase::Live)
        })
    }
    /// Must run before lifecycle's cleanup ACK can release/reuse a PD.
    pub fn reconcile(&mut self, lifecycle: &Lifecycle, now: u64) {
        let live = Self::live(lifecycle, self.task);
        if self.task != 0 && !live {
            if self.state != State::Idle {
                self.scrub();
            }
            self.pending = None;
            self.task = 0;
            self.sequence = 0;
        } else if self.state != State::Idle && now.saturating_sub(self.started) >= c::LEASE_TICKS {
            self.scrub();
            self.pending = Some(c::reply(self.sequence, c::CANCELLED, c::BEGIN));
            sex_pdx::serial_record!(
                "[display.capture.cancelled] target={} task={} sequence={} scrubbed=1 live=1",
                c::CLIENT,
                self.task,
                self.sequence
            );
        }
    }
    fn scrub(&mut self) {
        if let Some(b) = self.bank.as_mut() {
            b.scrub()
        }
        self.state = State::Idle;
    }
    /// True requests a new completed full desktop publication. Replies carry only
    /// typed status; authority still comes from the held route and stamped sender.
    pub fn request(
        &mut self,
        msg: sex_pdx::PdxMessage,
        lifecycle: &Lifecycle,
        now: u64,
        w: usize,
        h: usize,
    ) -> bool {
        self.reconcile(lifecycle, now);
        let sequence = msg.arg1 as u32;
        let denied = msg.caller_pd != c::CLIENT
            || !Self::live(lifecycle, msg.caller_task)
            || sequence == 0
            || msg.arg1 != sequence as u64
            || msg.arg2 != self.epoch
            || self.bank.is_none();
        let status = if denied {
            Some(c::DENIED)
        } else if msg.arg0 == c::BEGIN {
            if self.state != State::Idle || self.pending.is_some() {
                Some(c::BUSY)
            } else if self.task == msg.caller_task && sequence <= self.sequence {
                Some(c::DENIED)
            } else if c::pixels(w, h).is_none() {
                Some(c::OVERSIZE)
            } else {
                self.task = msg.caller_task;
                self.sequence = sequence;
                self.started = now;
                self.state = State::Waiting;
                return true;
            }
        } else if msg.arg0 == c::RELEASE {
            if self.task != msg.caller_task
                || self.sequence != sequence
                || self.state == State::Idle
            {
                Some(c::DENIED)
            } else {
                self.scrub();
                self.pending = None;
                sex_pdx::serial_record!(
                    "[display.capture.released] target={} task={} sequence={} scrubbed=1",
                    c::CLIENT,
                    self.task,
                    self.sequence
                );
                Some(c::OK)
            }
        } else {
            Some(c::DENIED)
        };
        if let Some(status) = status {
            let _ = sex_pdx::pdx_reply(
                msg.caller_pd,
                c::reply(
                    sequence,
                    status,
                    if msg.arg0 == c::RELEASE {
                        c::RELEASE
                    } else {
                        c::BEGIN
                    },
                ),
            );
        }
        false
    }
    /// Called only after the Store accepted successful CPU publication, with an
    /// immediately borrowed completed front. No front pointer is retained.
    pub fn published(
        &mut self,
        generation: u64,
        w: usize,
        h: usize,
        pixels: &[u32],
        night: u8,
    ) -> bool {
        if self.state != State::Waiting {
            return false;
        }
        let header = c::Header {
            epoch: self.epoch,
            sequence: self.sequence,
            task: self.task,
            generation,
            width: w,
            height: h,
            night,
        };
        if self
            .bank
            .as_mut()
            .is_some_and(|b| b.publish(header, pixels, |p| sex_pdx::desktop_look::night(p, night)))
        {
            self.state = State::Ready;
            self.pending = Some(c::reply(self.sequence, c::OK, c::BEGIN));
            sex_pdx::serial_record!("[display.capture.ready] target={} task={} sequence={} epoch={} generation={} width={} height={} native=1 night={} bytes={}",c::CLIENT,self.task,self.sequence,self.epoch,generation,w,h,night,pixels.len()*4);
            true
        } else {
            self.scrub();
            self.pending = Some(c::reply(self.sequence, c::CANCELLED, c::BEGIN));
            false
        }
    }
    pub fn poll(&mut self) {
        if let Some(word) = self.pending {
            if sex_pdx::pdx_reply(c::CLIENT, word) == 0 {
                self.pending = None;
            }
        }
    }
}
