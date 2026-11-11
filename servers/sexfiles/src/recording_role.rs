//! One explicitly authorized recorder binding. No executable-name inference.
//! Runtime callers must supply fresh kernel peer/task snapshots before filesystem
//! admission and again before delivery of every retained positive response.
use sex_pdx::desktop_recording::{BindReply, BindRequest, BindStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Wait,
    Invalid,
    Live,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub task: u32,
    pub shell_epoch: u64,
    pub files_epoch: u32,
}
impl Scope {
    pub fn request(request: BindRequest) -> Self {
        Self {
            task: request.task,
            shell_epoch: request.shell_epoch,
            files_epoch: request.files_epoch,
        }
    }
}
#[derive(Clone, Copy)]
struct Pending {
    request: BindRequest,
    status: BindStatus,
}

pub struct Role {
    bound: Option<Scope>,
    pending: Option<Pending>,
}
impl Role {
    pub const fn new() -> Self {
        Self {
            bound: None,
            pending: None,
        }
    }
    pub fn bound(&self) -> Option<Scope> {
        self.bound
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn retire(&mut self, task: u32) {
        if self.bound.is_some_and(|scope| scope.task == task) {
            self.bound = None;
        }
        if self
            .pending
            .is_some_and(|pending| pending.request.task == task)
        {
            self.pending = None;
        }
    }

    /// Refuse concurrent replacement of an undelivered response. Exact retries
    /// keep the original result; they do not rerun directory creation.
    pub fn busy(&self, request: BindRequest) -> bool {
        self.pending.is_some_and(|p| p.request != request)
    }
    pub fn repeated(&self, request: BindRequest) -> bool {
        self.pending.is_some_and(|p| p.request == request)
    }
    pub fn retain(&mut self, request: BindRequest, status: BindStatus) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(Pending { request, status });
        true
    }
    /// Invalidating an initiating lifecycle retires its binding and positive
    /// receipt together. A transient unavailable snapshot retains custody.
    pub fn reconcile(&mut self, mut check: impl FnMut(Scope) -> Check) {
        if self
            .bound
            .is_some_and(|scope| check(scope) == Check::Invalid)
        {
            self.bound = None;
        }
        if self
            .pending
            .is_some_and(|p| check(Scope::request(p.request)) == Check::Invalid)
        {
            self.pending = None;
        }
    }
    /// The bound scope is installed only at authenticated successful delivery.
    /// `send` returns false on provider reply backpressure; the exact receipt is
    /// kept and must pass a new lifecycle check on the next attempt.
    pub fn poll(
        &mut self,
        mut check: impl FnMut(Scope) -> Check,
        mut send: impl FnMut(BindReply) -> bool,
    ) {
        self.reconcile(&mut check);
        let Some(pending) = self.pending else {
            return;
        };
        let scope = Scope::request(pending.request);
        if check(scope) != Check::Live {
            return;
        }
        if send(BindReply::new(pending.request, pending.status)) {
            if pending.status == BindStatus::Ok {
                self.bound = Some(scope);
            }
            self.pending = None;
        }
    }
}

/// Files owns a data request's response until actual delivery. A retained write
/// acknowledgement cannot be reassigned to a later task or service epoch.
pub struct ScopedReply {
    pending: Option<(Scope, u64)>,
}
impl ScopedReply {
    pub const fn new() -> Self {
        Self { pending: None }
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn retain(&mut self, scope: Scope, word: u64) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some((scope, word));
        true
    }
    /// Recheck the same Files epoch and exact current task in `recipient`,
    /// without requiring the retired Shell to remain live. A negative response
    /// releases old buffer custody and grants no role/session authority.
    pub fn poll(
        &mut self,
        check: impl FnOnce(Scope) -> Check,
        recipient: impl FnOnce(Scope) -> Check,
        cancelled: impl FnOnce(u64) -> u64,
        send: impl FnOnce(u64) -> bool,
    ) {
        let Some((scope, word)) = self.pending else {
            return;
        };
        match check(scope) {
            Check::Wait => {}
            Check::Live => {
                if send(word) {
                    self.pending = None;
                }
            }
            Check::Invalid => {
                // Observed revocation stays sticky through reply backpressure.
                let denied = cancelled(word);
                self.pending = Some((scope, denied));
                match recipient(scope) {
                    Check::Invalid => self.pending = None,
                    Check::Wait => {}
                    Check::Live => {
                        if send(denied) {
                            self.pending = None;
                        }
                    }
                }
            }
        }
    }
    /// Called before Files acknowledges app retirement/rebirth. The DAG prevents
    /// a new run from starting until this single-threaded provider has drained
    /// old I/O and removed its retained responses.
    pub fn retire(&mut self, task: u32) {
        if self.pending.is_some_and(|(scope, _)| scope.task == task) {
            self.pending = None;
        }
    }
}
