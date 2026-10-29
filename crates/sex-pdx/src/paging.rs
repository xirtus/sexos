// SPDX-License-Identifier: GPL-3.0-only
//! Capability-backed paging ABI. Addresses describe a fault; only the kernel's
//! pending token and the invoked mapping/pool capabilities authorize a commit.

pub const OP_FAULT_NOTICE: u64 = 0x103;
pub const OP_RELEASE_PAGE: u64 = 0x104;
pub const OP_TASK_EXIT: u64 = 0x105;
pub const OP_WAIT_CLEANUP: u64 = 0x106;
pub const MAX_RELEASE_REQUEST_ID: u64 = u64::MAX >> 8;
pub const SYS_PAGING_COMMIT: u64 = 56;
pub const SYS_PAGING_QUERY: u64 = 57;
pub const SYS_PAGING_REJECT: u64 = 58;
pub const SYS_PAGING_INFO: u64 = 59;
pub const SYS_PAGING_RECLAIM: u64 = 60;
pub const SYS_PAGING_CLEANUP: u64 = 61;
pub const SYS_TASK_FINISH: u64 = 62;
pub const SLOT_FRAME_POOL: u64 = 34;
pub const SLOT_DEMAND_REGION: u64 = 35;
pub const SLOT_PAGER_MAPPING: u64 = 36;
pub const SLOT_PAGER_MAPPING_SECOND: u64 = 37;
pub const PAGE_BYTES: u64 = 4096;
pub const PAGING_OK: u64 = 0;
pub const ERR_PAGING_INVALID: u64 = 1;
/// Retry the same operation, token and selected frame. A commit may already
/// have installed its PTE; never re-zero or release the frame on this status.
pub const ERR_PAGING_RETRY: u64 = 2;
pub const ERR_PAGING_DENIED: u64 = 3;
pub const ERR_PAGING_EXHAUSTED: u64 = 4;
pub const ERR_PAGING_LIVE: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultNotice {
    pub fault_addr: u64,
    pub error_code: u64,
    pub token: u64,
    pub caller_pd: u32,
}

impl FaultNotice {
    pub fn decode(opcode: u64, caller_pd: u64, args: [u64; 3]) -> Result<Self, u64> {
        if opcode != OP_FAULT_NOTICE || caller_pd == 0 || caller_pd > u32::MAX as u64
            || args[0] == 0 || args[0] >= (1 << 47) || !matches!(args[1], 4 | 6)
            || args[2] == 0
        { return Err(ERR_PAGING_INVALID); }
        Ok(Self { fault_addr: args[0], error_code: args[1], token: args[2], caller_pd: caller_pd as u32 })
    }

    pub fn args(self) -> [u64; 3] { [self.fault_addr, self.error_code, self.token] }
    pub fn page(self) -> u64 { self.fault_addr & !(PAGE_BYTES - 1) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultQuery {
    pub page_va: u64,
    pub error_code: u64,
    pub target_pd: u32,
    pub allowed_rights: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagingInfo {
    pub base: u64,
    pub length: u64,
    /// Zero identifies a frame pool; mapping and demand capabilities name their target.
    pub target_pd: u32,
    pub rights: u8,
}

/// Requests are serialized by each client. Never reuse or wrap a request id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseRequest { pub page: u64, pub request_id: u64, pub caller_pd: u32 }
impl ReleaseRequest {
    pub fn decode(opcode: u64, caller_pd: u64, args: [u64; 3]) -> Result<Self, u64> {
        if opcode != OP_RELEASE_PAGE || caller_pd == 0 || caller_pd > u32::MAX as u64
            || args[0] == 0 || args[0] >= (1 << 47) || args[0] % PAGE_BYTES != 0
            || args[1] == 0 || args[1] > MAX_RELEASE_REQUEST_ID || args[2] != 0 { return Err(ERR_PAGING_INVALID); }
        Ok(Self { page: args[0], request_id: args[1], caller_pd: caller_pd as u32 })
    }
    pub fn args(self) -> [u64; 3] { [self.page, self.request_id, 0] }
}

/// Existing reply transport carries correlation plus a bounded paging status.
pub fn release_reply(request_id: u64, status: u64) -> u64 {
    if request_id == 0 || request_id > MAX_RELEASE_REQUEST_ID || status > u8::MAX as u64 {
        return ERR_PAGING_DENIED;
    }
    (request_id << 8) | status
}
pub fn release_reply_status(value: u64, request_id: u64) -> Result<u64, u64> {
    if request_id == 0 || request_id > MAX_RELEASE_REQUEST_ID || value >> 8 != request_id {
        return Err(ERR_PAGING_DENIED);
    }
    Ok(value & 0xff)
}

/// Kernel-origin hint only. Cleanup still checks authoritative task custody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitNotice { pub target_pd: u32, pub task_id: u64 }
impl ExitNotice {
    pub fn decode(opcode: u64, caller_pd: u64, args: [u64; 3]) -> Result<Self, u64> {
        if opcode != OP_TASK_EXIT || caller_pd != 0 || args[0] == 0 || args[0] > u32::MAX as u64
            || args[1] == 0 || args[2] != 0 { return Err(ERR_PAGING_INVALID); }
        Ok(Self { target_pd: args[0] as u32, task_id: args[1] })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupResult { pub frame_bitmap: u64, pub task_id: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitCleanupRequest { pub target_pd: u32, pub request_id: u64, pub caller_pd: u32 }
impl WaitCleanupRequest {
    pub fn decode(opcode: u64, caller_pd: u64, args: [u64; 3]) -> Result<Self, u64> {
        if opcode != OP_WAIT_CLEANUP || caller_pd == 0 || caller_pd > u32::MAX as u64
            || args[0] == 0 || args[0] > u32::MAX as u64 || args[0] == caller_pd
            || args[1] == 0 || args[1] > MAX_RELEASE_REQUEST_ID || args[2] != 0
        { return Err(ERR_PAGING_INVALID); }
        Ok(Self { target_pd: args[0] as u32, request_id: args[1], caller_pd: caller_pd as u32 })
    }
    pub fn args(self) -> [u64; 3] { [self.target_pd as u64, self.request_id, 0] }
}
