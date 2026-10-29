// SPDX-License-Identifier: GPL-3.0-only
//! Production policy. Host tests replace register transport and memory writes.
use crate::allocator::{AllocationError, Binding, FrameAllocator};
use sex_pdx::memory_region::{READ, WRITE};
use sex_pdx::paging::*;
pub trait Backend {
    fn query(&mut self, mapping_slot: u64, token: u64) -> Result<FaultQuery, u64>;
    fn zero(&mut self, virtual_address: u64, length: u64);
    fn commit(&mut self, mapping_slot: u64, frame_index: usize, token: u64, rights: u8) -> u64;
    fn reject(&mut self, mapping_slot: u64, token: u64) -> u64;
    fn reclaim(&mut self, mapping_slot: u64, frame_index: usize, token: u64) -> u64;
    fn cleanup(&mut self, mapping_slot: u64) -> Result<CleanupResult, u64>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason { Quota, PoolExhausted, Policy, CommitDenied }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Retry, Ignored, Fatal(u64),
    Committed { index: usize, page: u64, target: u32, token: u64 },
    Rejected { reason: RejectReason, page: u64, target: u32, token: u64 },
}
#[derive(Clone, Copy)]
enum Operation { Commit { index: usize, rights: u8 }, Reject(RejectReason) }
#[derive(Clone, Copy)]
struct Active { notice: FaultNotice, operation: Operation, may_have_published: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping { pub slot: u64, pub info: PagingInfo }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOutcome {
    Retry, Fatal(u64),
    Done { status: u64, binding: Option<Binding>, replay: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupOutcome {
    Retry, Live, Ignored, Fatal(u64),
    Closed { result: CleanupResult, released: usize, replay: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome { Pending, Reply(u64) }
#[derive(Clone, Copy)]
struct Release { request: ReleaseRequest, binding: Option<Binding>, result: Option<u64> }
pub struct Pager<const N: usize> {
    allocator: FrameAllocator<N>, pool: PagingInfo, mappings: [Option<Mapping>; 2],
    active: [Option<Active>; 2], releases: [Option<Release>; 2], closed: [Option<CleanupResult>; 2],
}
impl<const N: usize> Pager<N> {
    pub fn new(pool: PagingInfo, mapping: PagingInfo, quota: usize) -> Result<Self, u64> {
        Self::new_multi(pool, [Some(Mapping { slot: SLOT_PAGER_MAPPING, info: mapping }), None], quota)
    }
    pub fn new_multi(pool: PagingInfo, mappings: [Option<Mapping>; 2], quota: usize) -> Result<Self, u64> {
        let valid_range = |base: u64, bytes: u64| base != 0 && base % PAGE_BYTES == 0
            && bytes != 0 && bytes % PAGE_BYTES == 0
            && base.checked_add(bytes).is_some_and(|end| end <= (1 << 47));
        if !valid_range(pool.base, pool.length) || pool.target_pd != 0
            || pool.length / PAGE_BYTES != N as u64 || pool.rights & (READ | WRITE) != (READ | WRITE)
            || quota == 0 || quota > N || mappings[0].is_none()
        { return Err(ERR_PAGING_INVALID); }
        for mapping in mappings.iter().flatten() {
            if mapping.slot == 0 || mapping.info.target_pd == 0 || mapping.info.rights & READ == 0
                || !valid_range(mapping.info.base, mapping.info.length) { return Err(ERR_PAGING_INVALID); }
        }
        if let [Some(a), Some(b)] = mappings {
            if a.slot == b.slot || a.info.target_pd == b.info.target_pd
                || (a.info.base < b.info.base + b.info.length && b.info.base < a.info.base + a.info.length)
            { return Err(ERR_PAGING_INVALID); }
        }
        Ok(Self { allocator: FrameAllocator::new(quota), pool, mappings,
            active: [None; 2], releases: [None; 2], closed: [None; 2] })
    }
    pub fn client_index(&self, target: u32) -> Option<usize> {
        self.mappings.iter().position(|m| m.is_some_and(|m| m.info.target_pd == target))
    }
    pub fn mapping_for(&self, target: u32) -> Option<Mapping> {
        self.client_index(target).and_then(|index| self.mappings[index])
    }
    pub fn used_by(&self, target: u32) -> usize { self.allocator.used_by(target) }
    pub fn bindings_for(&self, target: u32) -> [Option<Binding>; N] { self.allocator.bindings_for(target) }
    pub fn closed_result(&self, target: u32) -> Option<CleanupResult> {
        self.client_index(target).and_then(|client| self.closed[client])
    }
    pub fn cleanup(&mut self, backend: &mut impl Backend, notice: ExitNotice) -> CleanupOutcome {
        let Some(client) = self.client_index(notice.target_pd) else { return CleanupOutcome::Ignored; };
        if notice.task_id == 0 { return CleanupOutcome::Ignored; }
        if let Some(result) = self.closed[client] {
            return if result.task_id == notice.task_id {
                CleanupOutcome::Closed { result, released: 0, replay: true }
            } else { CleanupOutcome::Ignored };
        }
        let result = match backend.cleanup(self.mappings[client].unwrap().slot) {
            Ok(result) => result,
            Err(ERR_PAGING_RETRY) => return CleanupOutcome::Retry,
            Err(ERR_PAGING_LIVE) => return CleanupOutcome::Live,
            Err(error) => return CleanupOutcome::Fatal(error),
        };
        // A bitmap is evidence, not a license to free somebody else's current
        // frame. Unpublished local reservations can be absent from this mask.
        if result.task_id != notice.task_id || result.frame_bitmap & !self.allocator.target_mask(notice.target_pd) != 0 {
            return CleanupOutcome::Fatal(ERR_PAGING_DENIED);
        }
        let released = self.allocator.finish_target_cleanup(notice.target_pd);
        self.active[client] = None;
        self.releases[client] = None;
        self.closed[client] = Some(result);
        CleanupOutcome::Closed { result, released, replay: false }
    }
    pub fn wait_cleanup(&self, request: WaitCleanupRequest) -> WaitOutcome {
        if WaitCleanupRequest::decode(OP_WAIT_CLEANUP, request.caller_pd as u64, request.args()).is_err()
            || self.client_index(request.caller_pd).is_none() { return WaitOutcome::Reply(ERR_PAGING_DENIED); }
        let Some(target) = self.client_index(request.target_pd) else { return WaitOutcome::Reply(ERR_PAGING_DENIED); };
        if self.closed[target].is_some() { WaitOutcome::Reply(PAGING_OK) } else { WaitOutcome::Pending }
    }
    pub fn release(&mut self, backend: &mut impl Backend, request: ReleaseRequest) -> ReleaseOutcome {
        if ReleaseRequest::decode(OP_RELEASE_PAGE, request.caller_pd as u64, request.args()).is_err() {
            return ReleaseOutcome::Done { status: ERR_PAGING_INVALID, binding: None, replay: false };
        }
        let Some(client) = self.client_index(request.caller_pd) else {
            return ReleaseOutcome::Done { status: ERR_PAGING_DENIED, binding: None, replay: false };
        };
        if self.closed[client].is_some() {
            return ReleaseOutcome::Done { status: ERR_PAGING_DENIED, binding: None, replay: false };
        }
        if let Some(previous) = self.releases[client] {
            if request.request_id < previous.request.request_id
                || (request.request_id == previous.request.request_id && request != previous.request)
            { return ReleaseOutcome::Done { status: ERR_PAGING_DENIED, binding: None, replay: false }; }
            if request == previous.request {
                if let Some(status) = previous.result {
                    return ReleaseOutcome::Done { status, binding: previous.binding, replay: true };
                }
            } else if previous.result.is_none() { return ReleaseOutcome::Retry; }
        }
        if self.active[client].is_some() { return ReleaseOutcome::Retry; }
        if self.releases[client].is_none_or(|release| release.request != request) {
            let binding = self.allocator.begin_reclaim(request.caller_pd, request.page);
            let result = if binding.is_none() { Some(ERR_PAGING_DENIED) } else { None };
            self.releases[client] = Some(Release { request, binding, result });
        }
        let release = self.releases[client].unwrap();
        let Some(binding) = release.binding else {
            return ReleaseOutcome::Done { status: ERR_PAGING_DENIED, binding: None, replay: false };
        };
        let mapping = self.mappings[client].unwrap();
        match backend.reclaim(mapping.slot, binding.index, binding.token) {
            PAGING_OK => {
                if !self.allocator.finish_reclaim(binding) { return ReleaseOutcome::Fatal(ERR_PAGING_INVALID); }
                self.releases[client].as_mut().unwrap().result = Some(PAGING_OK);
                ReleaseOutcome::Done { status: PAGING_OK, binding: Some(binding), replay: false }
            }
            ERR_PAGING_RETRY => ReleaseOutcome::Retry,
            // No error authorizes reuse. Preserve exact identity for diagnosis.
            error => ReleaseOutcome::Fatal(error),
        }
    }
    pub fn used(&self) -> usize { self.allocator.used() }
    pub fn step(&mut self, backend: &mut impl Backend, notice: FaultNotice) -> Outcome {
        if FaultNotice::decode(OP_FAULT_NOTICE, notice.caller_pd as u64, notice.args()).is_err() { return Outcome::Ignored; }
        let Some(client) = self.client_index(notice.caller_pd) else { return Outcome::Ignored; };
        if self.closed[client].is_some() { return Outcome::Ignored; }
        let mapping = self.mappings[client].unwrap();
        if self.releases[client].is_some_and(|release| release.result.is_none()) { return Outcome::Retry; }
        if let Some(active) = self.active[client] {
            if active.notice != notice { return Outcome::Ignored; }
        } else {
            let query = match backend.query(mapping.slot, notice.token) {
                Ok(query) => query,
                Err(ERR_PAGING_RETRY) => return Outcome::Retry,
                Err(ERR_PAGING_INVALID | ERR_PAGING_DENIED) => return Outcome::Ignored,
                Err(error) => return Outcome::Fatal(error),
            };
            // Guessed tokens and forged payloads cannot complete or reject the
            // actual pending fault. Compare authenticated query fields first.
            if query.page_va != notice.page() || query.target_pd != notice.caller_pd
                || query.target_pd != mapping.info.target_pd || query.error_code != notice.error_code
            { return Outcome::Ignored; }
            let fits = query.page_va >= mapping.info.base
                && query.page_va.checked_add(PAGE_BYTES).is_some_and(|end| end <= mapping.info.base + mapping.info.length);
            let rights = query.allowed_rights & mapping.info.rights & (READ | WRITE);
            let operation = if !fits || rights & READ == 0 || (notice.error_code == 6 && rights & WRITE == 0) {
                Operation::Reject(RejectReason::Policy)
            } else {
                match self.allocator.reserve(notice.token, query.target_pd, query.page_va) {
                    Ok(allocation) => {
                        if allocation.needs_zero {
                            backend.zero(self.pool.base + allocation.index as u64 * PAGE_BYTES, PAGE_BYTES);
                            if !self.allocator.mark_zeroed(allocation.index, notice.token) { return Outcome::Fatal(ERR_PAGING_INVALID); }
                        }
                        Operation::Commit { index: allocation.index, rights }
                    }
                    Err(AllocationError::Quota) => Operation::Reject(RejectReason::Quota),
                    Err(AllocationError::Exhausted) => Operation::Reject(RejectReason::PoolExhausted),
                    Err(_) => Operation::Reject(RejectReason::Policy),
                }
            };
            self.active[client] = Some(Active { notice, operation, may_have_published: false });
        }
        let active = self.active[client].unwrap();
        match active.operation {
            Operation::Commit { index, rights } => match backend.commit(mapping.slot, index, notice.token, rights) {
                PAGING_OK => {
                    if !self.allocator.commit(index, notice.token) { return Outcome::Fatal(ERR_PAGING_INVALID); }
                    self.active[client] = None;
                    Outcome::Committed { index, page: notice.page(), target: notice.caller_pd, token: notice.token }
                }
                ERR_PAGING_RETRY => {
                    self.active[client].as_mut().unwrap().may_have_published = true;
                    Outcome::Retry // PTE may exist: never zero or release this frame.
                },
                ERR_PAGING_INVALID | ERR_PAGING_DENIED | ERR_PAGING_EXHAUSTED => {
                    if active.may_have_published { return Outcome::Fatal(ERR_PAGING_DENIED); }
                    if !self.allocator.rollback_unpublished(index, notice.token) { return Outcome::Fatal(ERR_PAGING_INVALID); }
                    self.active[client] = Some(Active { notice, operation: Operation::Reject(RejectReason::CommitDenied), may_have_published: false });
                    Outcome::Retry
                }
                error => Outcome::Fatal(error), // Unknown outcome retains custody, fail closed.
            },
            Operation::Reject(reason) => match backend.reject(mapping.slot, notice.token) {
                PAGING_OK => { self.active[client] = None; Outcome::Rejected { reason, page: notice.page(), target: notice.caller_pd, token: notice.token } }
                ERR_PAGING_RETRY => Outcome::Retry,
                ERR_PAGING_INVALID | ERR_PAGING_DENIED => { self.active[client] = None; Outcome::Ignored }
                error => Outcome::Fatal(error),
            },
        }
    }
}
