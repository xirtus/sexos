#![allow(dead_code)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/memory_region.rs"] mod memory_region;
mod ipc { pub const MAX_DOMAINS: usize = 1024; }
#[path = "../kernel/src/capability_slot.rs"] mod capability_slot;
mod capability {
    use super::memory_region::RegionGrant;
    #[derive(Clone, Copy)] pub struct Pci { pub bar0_view: Option<RegionGrant> }
    #[derive(Clone, Copy)] pub enum CapabilityData { Region(RegionGrant), Pci(Pci), Domain(u32) }
    pub struct Capability { pub data: CapabilityData }
    pub struct ProtectionDomain { pub entries: Vec<(u32, CapabilityData)>, pub lookups: core::cell::Cell<usize> }
    impl ProtectionDomain {
        pub fn find_capability(&self, slot: u32) -> Option<Capability> {
            self.lookups.set(self.lookups.get()+1);
            self.entries.iter().find(|(id,_)| *id==slot).map(|(_,data)| Capability {data:*data})
        }
    }
}
include!(env!("SEXOS_REGION_QUERY_RS"));
use capability::*;
use memory_region::*;
fn domain() -> ProtectionDomain {
    let grant = RegionGrant { region_id: 1, virtual_base: 0x4000_0000_0000,
        length: 4096, rights: QUERY_VIEW | READ | WRITE | DMA_ADDRESS,
        dma_base: Some(0x1000_0000), peer_pd: None };
    ProtectionDomain { entries: vec![(32, CapabilityData::Region(grant)), (6, CapabilityData::Domain(3)),
        (8, CapabilityData::Pci(Pci {bar0_view: Some(RegionGrant {rights: QUERY_VIEW | READ | WRITE, dma_base: None, ..grant})}))], lookups: core::cell::Cell::new(0) }
}
const FAIL: [u64;5] = [u64::MAX,0,0,0,0];
#[test]
fn malformed_slot_never_reaches_capability_table() {
    let pd=domain();
    for number in [54,55] { for slot in [0,1025,0x1_0000_0020,u64::MAX] {
        assert_eq!(region_query(&pd,number,slot,0,4096,QUERY_VIEW as u64), FAIL);
    }}
    assert_eq!(pd.lookups.get(),0);
}
#[test]
fn missing_wrong_kind_range_rights_fail_with_all_outputs_zero() {
    let pd=domain();
    for number in [54,55] { for slot in [6,31] {
        assert_eq!(region_query(&pd,number,slot,0,4096,QUERY_VIEW as u64), FAIL);
    }}
    for (offset,len,rights) in [(0,0,1),(4096,1,1),(u64::MAX,2,1),(0,4096,0x81),(0,4096,0x1_0000_0001)] {
        assert_eq!(region_query(&pd,54,32,offset,len,rights),FAIL);
    }
}
#[test]
fn repeated_query_preserves_region_identity_and_requires_dma_right() {
    let pd=domain();
    let info=region_query(&pd,55,32,0,0,0);
    assert_eq!(info,[0,4096,15,0,1]);
    for _ in 0..1000 {
        assert_eq!(region_query(&pd,54,32,0,4096,1),[0,0x4000_0000_0000,4096,0,1]);
        assert_eq!(region_query(&pd,54,32,16,32,9),[0,0x4000_0000_0010,32,0x1000_0010,1]);
    }
    assert_eq!(region_query(&pd,55,32,0,0,0),info);
    assert_eq!(region_query(&pd,54,8,0,4096,9),FAIL);
    assert_eq!(region_query(&pd,54,8,0,4096,7),[0,0x4000_0000_0000,4096,0,1]);
}
