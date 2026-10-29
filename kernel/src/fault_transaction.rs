// SPDX-License-Identifier: GPL-3.0-only
//! Bounded fault identity and metadata custody; no mapping authority.
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const INVALID: u64 = 1;
pub const RETRY: u64 = 2;
pub const DENIED: u64 = 3;
pub const EXHAUSTED: u64 = 4;

/// Only non-present user data accesses in the lower canonical range are
/// eligible. Protection, PKU, reserved-bit and instruction-fetch faults fail closed.
pub fn eligible_user_fault(address: u64, error: u64, cs: u64) -> bool {
    cs & 3 == 3 && address != 0 && address < (1u64 << 47) && matches!(error, 4 | 6)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultRecord {
    pub token: u64,
    pub target_pd: u32,
    pub pager_pd: u32,
    pub task_id: u32,
    pub fault_addr: u64,
    pub page_addr: u64,
    pub error_code: u64,
    pub saved_rip: u64,
    pub saved_frame: u64,
}

pub struct TokenIssuer { last: AtomicU64 }
impl TokenIssuer {
    pub const fn new() -> Self { Self { last: AtomicU64::new(0) } }
    pub fn issue(&self) -> Result<u64, u64> {
        self.last.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| old.checked_add(1))
            .map(|old| old + 1).map_err(|_| EXHAUSTED)
    }
}

#[derive(Clone, Copy)]
enum Active { Idle, Prepared(FaultRecord), Pending(FaultRecord), Claimed(FaultRecord) }
struct State { active: Active, completed: Option<FaultRecord>, last_token: u64, retired: bool }

pub struct FaultSlot { locked: AtomicBool, state: UnsafeCell<State> }
// Every metadata access holds the same lock. Records leave the lock by value.
unsafe impl Sync for FaultSlot {}

struct Unlock<'a>(&'a AtomicBool);
impl Drop for Unlock<'_> { fn drop(&mut self) { self.0.store(false, Ordering::Release); } }

impl FaultSlot {
    pub const fn new() -> Self {
        Self { locked: AtomicBool::new(false), state: UnsafeCell::new(State { active: Active::Idle, completed: None, last_token: 0, retired: false }) }
    }

    fn with<T>(&self, operation: impl FnOnce(&mut State) -> Result<T, u64>) -> Result<T, u64> {
        self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).map_err(|_| RETRY)?;
        let _unlock = Unlock(&self.locked);
        operation(unsafe { &mut *self.state.get() })
    }

    fn matches(record: FaultRecord, pager: u32, token: u64) -> Result<FaultRecord, u64> {
        if record.token != token || token == 0 { return Err(INVALID); }
        if record.pager_pd != pager || pager == 0 { return Err(DENIED); }
        Ok(record)
    }

    pub fn prepare(&self, record: FaultRecord) -> Result<(), u64> {
        if !eligible_user_fault(record.fault_addr, record.error_code, 3)
            || record.token == 0 || record.target_pd == 0 || record.pager_pd == 0
            || record.target_pd == record.pager_pd || record.page_addr != (record.fault_addr & !4095)
            || record.saved_rip == 0 || record.saved_frame == 0 { return Err(INVALID); }
        self.with(|state| {
            if state.retired { return Err(DENIED); }
            if !matches!(state.active, Active::Idle) { return Err(DENIED); }
            if record.token <= state.last_token { return Err(INVALID); }
            state.last_token = record.token;
            state.active = Active::Prepared(record);
            Ok(())
        })
    }

    pub fn publish(&self, token: u64) -> Result<(), u64> {
        self.with(|state| match state.active {
            Active::Prepared(record) if record.token == token => { state.active = Active::Pending(record); Ok(()) }
            _ => Err(INVALID),
        })
    }

    pub fn cancel_prepared(&self, token: u64) -> Result<(), u64> {
        self.with(|state| match state.active {
            Active::Prepared(record) if record.token == token => { state.active = Active::Idle; Ok(()) }
            _ => Err(INVALID),
        })
    }

    pub fn query(&self, pager: u32, token: u64) -> Result<FaultRecord, u64> {
        self.with(|state| match state.active {
            Active::Pending(record) => Self::matches(record, pager, token),
            Active::Prepared(record) | Active::Claimed(record) => { Self::matches(record, pager, token)?; Err(RETRY) }
            Active::Idle => Err(INVALID),
        })
    }

    pub fn claim(&self, pager: u32, token: u64) -> Result<FaultRecord, u64> {
        self.with(|state| match state.active {
            Active::Pending(record) => {
                let record = Self::matches(record, pager, token)?;
                state.active = Active::Claimed(record);
                Ok(record)
            }
            Active::Prepared(record) | Active::Claimed(record) => { Self::matches(record, pager, token)?; Err(RETRY) }
            Active::Idle => Err(INVALID),
        })
    }

    /// Only after proving no mapping effect may a claimed request be retried.
    pub fn rollback(&self, pager: u32, token: u64) -> Result<(), u64> {
        self.with(|state| match state.active {
            Active::Claimed(record) => {
                let record = Self::matches(record, pager, token)?;
                state.active = Active::Pending(record); Ok(())
            }
            _ => Err(INVALID),
        })
    }

    /// Returns Some once, None for the exact latest completion retry. Caller
    /// must retire this metadata before exposing the saved task as runnable.
    pub fn complete(&self, pager: u32, token: u64) -> Result<Option<FaultRecord>, u64> {
        self.with(|state| {
            if let Some(old) = state.completed {
                if old.token == token { Self::matches(old, pager, token)?; return Ok(None); }
            }
            match state.active {
                Active::Claimed(record) => {
                    let record = Self::matches(record, pager, token)?;
                    state.completed = Some(record);
                    state.active = Active::Idle;
                    Ok(Some(record))
                }
                _ => Err(INVALID),
            }
        })
    }

    /// # Safety
    /// The registered main task is terminal Dead with no CPU/stack owner and
    /// every other task sharing its demand region is inactive. The pager has
    /// validated all leaf identities before beginning irreversible cleanup.
    pub unsafe fn retire_exited(&self, target: u32, pager: u32, task: u32) -> Result<(), u64> {
        self.with(|state| {
            let valid = |record: FaultRecord| record.target_pd == target
                && record.pager_pd == pager && record.task_id == task;
            match state.active {
                Active::Prepared(record) | Active::Pending(record) | Active::Claimed(record)
                    if !valid(record) => return Err(DENIED),
                _ => {}
            }
            if state.completed.is_some_and(|record| !valid(record)) { return Err(DENIED); }
            state.active = Active::Idle;
            state.completed = None;
            state.retired = true;
            Ok(())
        })
    }

    pub fn reject(&self, pager: u32, token: u64) -> Result<FaultRecord, u64> {
        self.with(|state| match state.active {
            Active::Pending(record) => {
                let record = Self::matches(record, pager, token)?;
                state.active = Active::Idle; Ok(record)
            }
            Active::Prepared(record) | Active::Claimed(record) => { Self::matches(record, pager, token)?; Err(RETRY) }
            Active::Idle => Err(INVALID),
        })
    }
}

#[cfg(test)]
#[test]
fn token_exhaustion_is_terminal_and_never_wraps() {
    let issuer = TokenIssuer { last: AtomicU64::new(u64::MAX - 1) };
    assert_eq!(issuer.issue(), Ok(u64::MAX));
    assert_eq!(issuer.issue(), Err(EXHAUSTED));
    assert_eq!(issuer.issue(), Err(EXHAUSTED));
}

#[cfg(test)]
#[test]
fn prepare_lock_contention_is_retry_not_active_request_collision() {
    let slot=FaultSlot::new();
    let record=FaultRecord{token:1,target_pd:15,pager_pd:14,task_id:0x8015,fault_addr:0x4000,page_addr:0x4000,error_code:6,saved_rip:0x1000,saved_frame:0x2000};
    slot.locked.store(true,Ordering::Relaxed);
    assert_eq!(slot.prepare(record),Err(RETRY));
    slot.locked.store(false,Ordering::Release);
    slot.prepare(record).unwrap();
    assert_eq!(slot.prepare(record),Err(DENIED));
}
