// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
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
/// Scanout buffer BOOT adds beside the frames when Display holds a virtio-gpu:
/// one packed frame, DMA-addressable, the host resource's backing.
pub const SCANOUT_SLOT:u64=26;
pub const SCANOUT_REGION_ID:u32=0x4210;
/// Intel display engine lease in Display: BAR0's display registers only
/// (pipes, transcoders, planes, DDI/AUX, PCH GMBUS/hotplug/backlight).
pub const INTEL_SLOT:u64=27;
pub const INTEL_REGISTERS:u64=0xd0000;
/// AMD display engine lease in Display: the first 256 KiB of the Radeon's register
/// BAR (BAR 5), which holds DCN's OTG, HUBP, AUX and VM aperture registers.
pub const AMD_SLOT:u64=29;
pub const AMD_REGISTER_BAR:u8=5;
pub const AMD_REGISTERS:u64=0x40000;
/// Intel scanout buffers BOOT adds beside the lease (slot 28): two frames
/// the display engine flips between, then one 256x256 ARGB cursor image.
/// BOOT also writes the GPU's own page table (GGTT) so these pages, and only
/// these, appear at INTEL_GGTT_WINDOW; Display never writes GGTT entries.
pub const INTEL_SCANOUT_SLOT:u64=28;
pub const INTEL_SCANOUT_REGION_ID:u32=0x4211;
/// GGTT address of the scanout window: above any firmware framebuffer in
/// stolen memory, inside the smallest GGTT firmware can configure (1 GiB).
pub const INTEL_GGTT_WINDOW:u64=0x1000_0000;
pub const INTEL_CURSOR_BYTES:u64=256*256*4;

/// Geometry of the Intel scanout region: rows padded to the plane's 64-byte
/// stride unit, each frame page-aligned, the cursor last.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct IntelScanout{pub stride:u64,pub span:u64,pub cursor:u64,pub total:u64}
impl IntelScanout{
    pub fn new(width:u64,height:u64)->Option<Self>{
        if width==0||height==0||width>5120||height>4096{return None;}
        let stride=width.checked_mul(4)?.checked_add(63)?/64*64;
        let span=stride.checked_mul(height)?.checked_add(PAGE-1)?/PAGE*PAGE;
        let cursor=span.checked_mul(2)?;
        Some(Self{stride,span,cursor,total:cursor.checked_add(INTEL_CURSOR_BYTES)?})
    }
    /// GGTT address of frame `index` (0 or 1) or of the cursor (2).
    pub fn ggtt(&self,index:u64)->u64{INTEL_GGTT_WINDOW+if index>=2{self.cursor}else{index*self.span}}
}
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
