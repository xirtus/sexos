// SPDX-License-Identifier: GPL-3.0-only
//! Pure authorization for immutable, already-mapped boot region capabilities.
//!
//! The kernel must obtain `RegionGrant` from the caller's capability table.
//! A caller-supplied descriptor is data, never a substitute for that lookup.
//! Resolving a view neither allocates frames nor changes GLOBAL_VAS or grants.

pub const PAGE_BYTES: u64 = 4096;
pub const USER_VA_END: u64 = 1 << 47;
pub const PHYSICAL_END: u64 = 1 << 52;

pub const QUERY_VIEW: u8 = 1;
pub const READ: u8 = 2;
pub const WRITE: u8 = 4;
pub const DMA_ADDRESS: u8 = 8;
const KNOWN_RIGHTS: u8 = QUERY_VIEW | READ | WRITE | DMA_ADDRESS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionGrant {
    pub region_id: u32,
    pub virtual_base: u64,
    pub length: u64,
    pub rights: u8,
    /// Present only for a boot DMA-region grant; ordinary sharing exposes no PA.
    pub dma_base: Option<u64>,
    /// Peer bound by the boot graph for a paired shared region, if applicable.
    pub peer_pd: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRequest {
    pub offset: u64,
    pub length: u64,
    pub rights: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionView {
    pub region_id: u32,
    pub virtual_address: u64,
    pub length: u64,
    pub dma_address: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegionInfo {
    pub region_id: u32,
    pub length: u64,
    pub rights: u8,
    pub peer_pd: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionError {
    InvalidCapability,
    InvalidRequest,
    OutOfRange,
    PermissionDenied,
    WrongPeer,
}

fn valid_grant(grant: &RegionGrant) -> bool {
    let Some(end) = grant.virtual_base.checked_add(grant.length) else { return false; };
    if grant.region_id == 0 || grant.virtual_base == 0 || grant.length == 0
        || grant.virtual_base % PAGE_BYTES != 0 || grant.length % PAGE_BYTES != 0
        || end > USER_VA_END || grant.rights == 0 || grant.rights & !KNOWN_RIGHTS != 0
        || (grant.rights & WRITE != 0 && grant.rights & READ == 0)
        || grant.peer_pd == Some(0)
    {
        return false;
    }
    match grant.dma_base {
        Some(base) => grant.rights & DMA_ADDRESS != 0 && base != 0
            && base % PAGE_BYTES == 0
            && base.checked_add(grant.length).is_some_and(|end| end <= PHYSICAL_END),
        None => grant.rights & DMA_ADDRESS == 0,
    }
}

/// `None` represents absent or wrong-kind capability after trusted lookup.
/// Pkey state is intentionally not an input to the authority decision.
pub fn resolve_view(grant: Option<&RegionGrant>, request: ViewRequest) -> Result<RegionView, RegionError> {
    let grant = grant.filter(|grant| valid_grant(grant)).ok_or(RegionError::InvalidCapability)?;
    if request.length == 0 || request.rights & QUERY_VIEW == 0
        || request.rights & !KNOWN_RIGHTS != 0
    {
        return Err(RegionError::InvalidRequest);
    }
    if request.rights & !grant.rights != 0 {
        return Err(RegionError::PermissionDenied);
    }
    let end = request.offset.checked_add(request.length).ok_or(RegionError::OutOfRange)?;
    if end > grant.length { return Err(RegionError::OutOfRange); }
    Ok(RegionView {
        region_id: grant.region_id,
        virtual_address: grant.virtual_base + request.offset,
        length: request.length,
        dma_address: if request.rights & DMA_ADDRESS != 0 {
            Some(grant.dma_base.ok_or(RegionError::InvalidCapability)? + request.offset)
        } else {
            None
        },
    })
}

/// Preserve the register width until validation; high rights bits must not alias.
pub fn resolve_register_view(grant: Option<&RegionGrant>, offset: u64, length: u64, rights: u64) -> Result<RegionView, RegionError> {
    let rights = u8::try_from(rights).map_err(|_| RegionError::InvalidRequest)?;
    resolve_view(grant, ViewRequest { offset, length, rights })
}

/// Metadata discloses neither the virtual view nor a DMA address.
pub fn resolve_info(grant: Option<&RegionGrant>) -> Result<MemoryRegionInfo, RegionError> {
    let grant = grant.ok_or(RegionError::InvalidCapability)?;
    resolve_view(Some(grant), ViewRequest { offset: 0, length: grant.length, rights: QUERY_VIEW })?;
    Ok(MemoryRegionInfo { region_id: grant.region_id, length: grant.length,
        rights: grant.rights, peer_pd: grant.peer_pd })
}

/// Use the authenticated PDX message sender, never an ID from message payload.
pub fn resolve_peer_view(grant: Option<&RegionGrant>, authenticated_caller: u32, request: ViewRequest) -> Result<RegionView, RegionError> {
    let view = resolve_view(grant, request)?;
    if grant.and_then(|grant| grant.peer_pd) != Some(authenticated_caller)
        || authenticated_caller == 0
    {
        return Err(RegionError::WrongPeer);
    }
    Ok(view)
}
