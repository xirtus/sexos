// SPDX-License-Identifier: GPL-3.0-only
//! Typed views of Display's two BOOT-reserved frame buffers.
//!
//! Layouts and syscall reply structs are descriptive data, not capabilities.
//! The existing Region capability belongs to the current PD; all tasks in that
//! PD share its authority. This module adds no task isolation or new authority.
use core::marker::PhantomData;
use crate::{MemoryRegionInfo,ERR_CAP_INVALID};
use crate::memory_region::{RegionView,QUERY_VIEW,READ,WRITE,USER_VA_END};

pub const SLOT:u64=98;
pub const REGION_ID:u32=11;
pub const MAX_WIDTH:u64=8192;
pub const MAX_HEIGHT:u64=4320;
pub const PAGE:u64=4096;

/// Checked packed-XRGB geometry and reservation sizes. No backing is acquired.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Layout{
    width:usize,
    height:usize,
    pitch:u32,
    frame_bytes:u64,
    span_bytes:u64,
    total_bytes:u64,
    buddy_bytes:u64,
}
impl Layout{
    pub fn new(width:u64,height:u64,pitch:u64)->Option<Self>{
        if width==0||height==0||width>MAX_WIDTH||height>MAX_HEIGHT
            ||pitch!=width.checked_mul(4)?{return None;}
        let frame_bytes=pitch.checked_mul(height)?;
        let span_bytes=frame_bytes.checked_add(PAGE-1)?.checked_div(PAGE)?.checked_mul(PAGE)?;
        let total_bytes=span_bytes.checked_mul(2)?;
        let buddy_bytes=(total_bytes/PAGE).checked_next_power_of_two()?.checked_mul(PAGE)?;
        Some(Self{width:usize::try_from(width).ok()?,height:usize::try_from(height).ok()?,
            pitch:u32::try_from(pitch).ok()?,frame_bytes,span_bytes,total_bytes,buddy_bytes})
    }
    pub const fn width(self)->usize{self.width}
    pub const fn height(self)->usize{self.height}
    pub const fn pitch(self)->u32{self.pitch}
    pub const fn frame_bytes(self)->u64{self.frame_bytes}
    pub const fn span_bytes(self)->u64{self.span_bytes}
    pub const fn total_bytes(self)->u64{self.total_bytes}
    pub const fn buddy_bytes(self)->u64{self.buddy_bytes}
}

/// Non-transferable descriptor of already resolved PD-owned backing. It holds
/// no framebuffer pointer, pixel reference, staging state or presentation epoch.
/// Not Copy/Clone/Send/Sync; this does not make the PD's capability exclusive.
pub struct FrameBacking{
    layout:Layout,
    base:u64,
    _local:PhantomData<*mut ()>,
}
impl FrameBacking{
    /// Validate the exact replies of the SDK's held-region acquisition wrapper.
    ///
    /// # Safety
    /// `info` and `view` must come from successful current-PD Region syscalls for
    /// SLOT, with the full view requested at offset0 and QUERY_VIEW|READ|WRITE.
    /// Fabricated or cached foreign descriptors cannot establish that lookup.
    /// The caller must preserve the mapping's lifetime and the PD's access
    /// authority while this descriptor is used. Returned integers alone provide
    /// no Rust aliasing, pixel-initialization or physical presentation guarantee.
    pub unsafe fn from_resolved(layout:Layout,info:MemoryRegionInfo,view:RegionView)->Result<Self,u64>{
        if info.region_id!=REGION_ID||info.rights!=(QUERY_VIEW|READ|WRITE)
            ||info.peer_pd.is_some()||info.length!=layout.total_bytes
            ||view.region_id!=REGION_ID||view.length!=layout.total_bytes
            ||view.dma_address.is_some()||view.virtual_address==0||view.virtual_address%PAGE!=0{
            return Err(ERR_CAP_INVALID);
        }
        let end=view.virtual_address.checked_add(layout.total_bytes).ok_or(ERR_CAP_INVALID)?;
        if end>USER_VA_END{return Err(ERR_CAP_INVALID);}
        Ok(Self{layout,base:view.virtual_address,_local:PhantomData})
    }
    pub const fn layout(&self)->Layout{self.layout}
    /// Only the two disjoint, page-aligned buffer starts are exposed. The
    /// usable packed pixels occupy frame_bytes, not padding or buddy rounding.
    pub fn address(&self,index:usize)->Option<u64>{
        if index>=2{return None;}
        self.base.checked_add(self.layout.span_bytes.checked_mul(index as u64)?)
    }
}
