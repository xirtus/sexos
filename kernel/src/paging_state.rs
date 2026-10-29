// SPDX-License-Identifier: GPL-3.0-only
//! Allocation-free validation and execution ledger for immutable paging grants.
pub const PAGE: u64 = 4096;
pub const PAGES: usize = 8;
pub const CLIENTS: usize = 2;
pub const INVALID: u64 = 1;
pub const RETRY: u64 = 2;
pub const DENIED: u64 = 3;
pub const EXHAUSTED: u64 = 4;
pub const LIVE: u64 = 5;
pub const READ: u8 = 2;
pub const WRITE: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePoolGrant { pub pool_id: u32, pub base_va: u64, pub length: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagerMappingGrant {
    pub range_id: u32, pub pool_id: u32, pub target_pd: u32,
    pub base_va: u64, pub length: u64, pub rights: u8, pub quota: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemandRegionGrant {
    pub range_id: u32, pub base_va: u64, pub length: u64, pub rights: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultView { pub task_id: u32, pub token: u64, pub target_pd: u32, pub pager_pd: u32, pub page: u64, pub error: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub token: u64, pub task_id: u32, pub target_pd: u32, pub pager_pd: u32, pub range_id: u32,
    pub page_index: usize, pub frame_index: usize, pub rights: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitRecord { pub binding: Binding, pub finalized: bool }

pub fn validate(map: PagerMappingGrant, pool: FramePoolGrant, pager: u32,
    fault: FaultView, frame: u64, rights: u64) -> Result<Binding, u64>
{
    if map.pool_id == 0 || pool.pool_id != map.pool_id || map.range_id == 0
        || map.range_id as usize > CLIENTS || map.target_pd == 0 || pager == 0 || map.target_pd == pager
        || map.length == 0 || map.length > PAGES as u64 * PAGE || map.length % PAGE != 0
        || pool.length == 0 || pool.length > PAGES as u64 * PAGE || pool.length % PAGE != 0
        || map.base_va == 0 || map.base_va % PAGE != 0 || pool.base_va == 0 || pool.base_va % PAGE != 0
        || !map.base_va.checked_add(map.length).is_some_and(|end| end <= 1 << 47)
        || !pool.base_va.checked_add(pool.length).is_some_and(|end| end <= 1 << 47)
        || map.quota == 0 || map.quota as usize > PAGES
    { return Err(INVALID); }
    if fault.token == 0 || fault.target_pd != map.target_pd || fault.pager_pd != pager
        || fault.page % PAGE != 0 || fault.error & !6 != 0 || fault.error & 4 == 0
    { return Err(DENIED); }
    let rights = u8::try_from(rights).map_err(|_| DENIED)?;
    if rights & READ == 0 || rights & !(READ | WRITE) != 0 || rights & !map.rights != 0
        || (fault.error & 2 != 0 && rights & WRITE == 0)
    { return Err(DENIED); }
    let offset = fault.page.checked_sub(map.base_va).ok_or(DENIED)?;
    if offset >= map.length || frame >= pool.length / PAGE { return Err(DENIED); }
    Ok(Binding { token: fault.token, task_id: fault.task_id, target_pd: fault.target_pd, pager_pd: pager, range_id: map.range_id,
        page_index: (offset / PAGE) as usize, frame_index: frame as usize, rights })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupRecord { pub target_pd: u32, pub task_id: u32, pub frames: u8 }

pub struct MappingLedger {
    pub entries: [Option<CommitRecord>; PAGES],
    retired: [Option<Binding>; PAGES],
    retired_highwater: [u64; CLIENTS],
    closed: [Option<CleanupRecord>; CLIENTS],
}
impl MappingLedger {
    pub const fn new() -> Self { Self { entries: [None; PAGES], retired: [None; PAGES], retired_highwater: [0; CLIENTS], closed: [None; CLIENTS] } }
    pub fn token(&self, token: u64) -> Option<CommitRecord> {
        self.entries.iter().flatten().find(|record| record.binding.token == token).copied()
    }
    pub fn check_new(&self, binding: Binding, quota: u8) -> Result<(), u64> {
        if binding.frame_index >= PAGES || binding.page_index >= PAGES
            || binding.range_id == 0 || binding.range_id as usize > CLIENTS || binding.token == 0
            || quota == 0 || quota as usize > PAGES { return Err(INVALID); }
        if self.cleanup_record(binding.range_id).is_some() || self.is_retired(binding.range_id, binding.token) { return Err(DENIED); }
        if self.token(binding.token).is_some() || self.entries[binding.frame_index].is_some()
            || self.entries.iter().flatten().any(|r|r.binding.target_pd == binding.target_pd && r.binding.range_id == binding.range_id && r.binding.page_index == binding.page_index)
        { return Err(DENIED); }
        if self.entries.iter().flatten().filter(|r|r.binding.target_pd == binding.target_pd).count() >= quota as usize { return Err(EXHAUSTED); }
        Ok(())
    }
    pub fn install(&mut self, binding: Binding, quota: u8) -> Result<(), u64> {
        self.check_new(binding, quota)?;
        self.entries[binding.frame_index] = Some(CommitRecord { binding, finalized: false });
        Ok(())
    }
    pub fn finalize(&mut self, binding: Binding) -> Result<(), u64> {
        let record = self.entries.get_mut(binding.frame_index).and_then(|r|r.as_mut()).ok_or(INVALID)?;
        if record.binding != binding { return Err(DENIED); }
        record.finalized = true;
        Ok(())
    }
    // One registered main faulting task per immutable range is required: newly
    // pending tokens exceed every previously retired token in that range. Look
    // up still-live exact commits first, since they can predate this floor.
    pub fn is_retired(&self, range_id: u32, token: u64) -> bool {
        range_id == 0 || range_id as usize > CLIENTS || token == 0
            || token <= self.retired_highwater[range_id as usize - 1]
    }
    pub fn retired(&self, frame: usize) -> Option<Binding> {
        self.retired.get(frame).copied().flatten()
    }
    // Call only AFTER the exact leaf is cleared and its TLB invalidation has
    // completed. Until then the live entry continues to exclude all reuse.
    pub fn retire(&mut self, binding: Binding) -> Result<(), u64> {
        let record = self.entries.get(binding.frame_index).copied().flatten().ok_or(DENIED)?;
        if record.binding != binding || !record.finalized { return Err(DENIED); }
        self.retired[binding.frame_index] = Some(binding);
        let floor = &mut self.retired_highwater[binding.range_id as usize - 1];
        *floor = (*floor).max(binding.token);
        self.entries[binding.frame_index] = None;
        Ok(())
    }

    pub fn cleanup_record(&self, range_id: u32) -> Option<CleanupRecord> {
        self.closed.get(range_id.checked_sub(1)? as usize).copied().flatten()
    }
    pub fn cleanup_plan(&self, range_id: u32, target: u32, pager: u32, task: u32)
        -> Result<[Option<Binding>; PAGES], u64>
    {
        if range_id == 0 || range_id as usize > CLIENTS { return Err(INVALID); }
        if self.cleanup_record(range_id).is_some() { return Err(DENIED); }
        let mut plan = [None; PAGES];
        for (index,record) in self.entries.iter().enumerate() {
            let Some(record) = record else { continue; };
            let b = record.binding;
            if b.range_id != range_id && b.target_pd != target { continue; }
            if b.range_id != range_id || b.target_pd != target || b.pager_pd != pager
                || b.task_id != task || b.frame_index != index { return Err(DENIED); }
            plan[index] = Some(b);
        }
        Ok(plan)
    }
    /// Caller has authenticated terminal detached task ownership, retired its
    /// fault transaction, and cleared/flushed EVERY planned leaf. Unfinalized
    /// published bindings are deliberately included; ordinary reclaim retains
    /// its stronger finalized-only rule.
    pub fn close_exited_range(&mut self, range_id: u32, target: u32, pager: u32, task: u32)
        -> Result<CleanupRecord, u64>
    {
        let plan = self.cleanup_plan(range_id,target,pager,task)?;
        let mut frames = 0u8;
        for binding in plan.into_iter().flatten() {
            frames |= 1 << binding.frame_index;
            self.retired[binding.frame_index] = Some(binding);
            let floor = &mut self.retired_highwater[range_id as usize - 1];
            *floor = (*floor).max(binding.token);
            self.entries[binding.frame_index] = None;
        }
        let result = CleanupRecord { target_pd: target, task_id: task, frames };
        self.closed[range_id as usize - 1] = Some(result);
        Ok(result)
    }

}
