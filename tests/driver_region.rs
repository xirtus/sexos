// SPDX-License-Identifier: GPL-3.0-only
// Actual driver adapters + actual byte allocator. No device/DMA completion claim.
#![allow(dead_code)]
macro_rules! serial_println { ($($arg:tt)*) => { let _ = format_args!($($arg)*); } }
#[macro_export]
macro_rules! serial_record { ($($arg:tt)*) => { let _ = format_args!($($arg)*); } }
#[path = "../crates/sex-pdx/src/memory_region.rs"]
mod memory_region;
#[path = "../crates/sex-pdx/src/dma_pool.rs"]
mod dma_pool;

mod sex_pdx {
    pub(crate) use crate::serial_record;
    pub(crate) use crate::{dma_pool, memory_region};
    use std::cell::{Cell, RefCell};
    pub const SLOT_DMA_REGION: u64 = 32;
    thread_local! {
        pub static VIEW: RefCell<Result<memory_region::RegionView,u64>> = const { RefCell::new(Err(1)) };
        pub static QUERIES: Cell<usize> = const { Cell::new(0) };
    }
    pub fn memory_region_full_view(slot: u64, rights: u8) -> Result<memory_region::RegionView,u64> {
        assert_eq!(slot, SLOT_DMA_REGION);
        assert_eq!(rights, memory_region::QUERY_VIEW | memory_region::READ
            | memory_region::WRITE | memory_region::DMA_ADDRESS);
        QUERIES.with(|n| n.set(n.get()+1));
        VIEW.with(|view| *view.borrow())
    }
}
mod usb { use crate::sex_pdx; include!(env!("SEXUSB_DMA_SOURCE"));
    pub fn reserve(p: &mut Option<crate::dma_pool::DmaRegionPool>, n:u64)->(u64,u64) { reserve_dma(p,n) }
}
mod net { use crate::sex_pdx; include!(env!("SEXNET_DMA_SOURCE"));
    pub fn reserve(p: &mut Option<crate::dma_pool::DmaRegionPool>, n:u64)->(u64,u64) { reserve_dma(p,n) }
}
type Reserve = fn(&mut Option<dma_pool::DmaRegionPool>,u64)->(u64,u64);
const FAILED:(u64,u64) = (u64::MAX,u64::MAX);
fn configure(view:Result<memory_region::RegionView,u64>) {
    sex_pdx::VIEW.with(|v| *v.borrow_mut()=view);
    sex_pdx::QUERIES.with(|n| n.set(0));
}
fn view()->memory_region::RegionView {
    memory_region::RegionView { region_id:7, virtual_address:0x400000,
        length:256*1024, dma_address:Some(0x800000) }
}
fn adapters()->[Reserve;2] { [usb::reserve,net::reserve] }

#[test]
fn authorization_failure_does_not_construct_pool_or_return_addresses() {
    for reserve in adapters() { configure(Err(13)); let mut pool=None;
        assert_eq!(reserve(&mut pool,4096),FAILED); assert!(pool.is_none()); }
}
#[test]
fn non_dma_and_malformed_regions_fail_closed() {
    for reserve in adapters() { for kind in 0..4 { let mut grant=view();
        match kind { 0=>grant.dma_address=None, 1=>grant.length=4095,
            2=>grant.virtual_address=0, _=>grant.dma_address=Some(u64::MAX) }
        configure(Ok(grant)); let mut pool=None;
        assert_eq!(reserve(&mut pool,4096),FAILED); assert!(pool.is_none()); } }
}
#[test]
fn reservations_return_matching_disjoint_addresses_without_remapping() {
    for reserve in adapters() { configure(Ok(view())); let mut pool=None;
        assert_eq!(reserve(&mut pool,1),(0x800000,0x400000));
        assert_eq!(reserve(&mut pool,4097),(0x801000,0x401000));
        assert_eq!(pool.unwrap().used(),12288);
        sex_pdx::QUERIES.with(|n| assert_eq!(n.get(),1)); }
}
#[test]
fn exhaustion_and_bad_sizes_never_extend_or_rewind_region() {
    for reserve in adapters() { configure(Ok(view())); let mut pool=None;
        for i in 0..64 { assert_eq!(reserve(&mut pool,4096),(0x800000+i*4096,0x400000+i*4096)); }
        for size in [1,4096,0,u64::MAX] { assert_eq!(reserve(&mut pool,size),FAILED); }
        assert_eq!(pool.unwrap().used(),256*1024);
        sex_pdx::QUERIES.with(|n| assert_eq!(n.get(),1)); }
}
#[test]
fn current_complete_usb_and_nic_initialization_budgets_fit_without_reuse() {
    // USB two slots:4 host+3 primary ctx+1 descriptor+2 primary IRQ+3 second ctx+2 second IRQ.
    // NIC proof profile:2 first RX+9 observe RX+9 permanent RX+2 first TX+2 permanent TX.
    for (reserve,pages) in [(usb::reserve as Reserve,15),(net::reserve as Reserve,24)] {
        configure(Ok(view())); let mut pool=None;
        for _ in 0..pages { assert_ne!(reserve(&mut pool,4096),FAILED); }
        assert_eq!(pool.unwrap().used(),pages*4096);
    }
}
