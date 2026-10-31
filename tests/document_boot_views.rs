// SPDX-License-Identifier: GPL-3.0-only
//! Actual BOOT constructors/query; allocation, mapping and registry are mocks.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export] macro_rules! serial_println { ($($t:tt)*) => {} }
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/memory_region.rs"] pub mod memory_region;
#[path="../crates/sex-pdx/src/document_buffer.rs"] pub mod document_buffer;
#[path="../kernel/src/capability_slot.rs"] pub mod capability_slot;
use memory_region::{RegionGrant, QUERY_VIEW, READ, WRITE};
use capability::{CapabilityData, ProtectionDomain};
use std::cell::RefCell;
mod capability {
    use super::*;
    #[derive(Clone, Copy)] pub struct Pci { pub bar0_view: Option<RegionGrant> }
    #[derive(Clone, Copy)] pub enum CapabilityData { Region(RegionGrant), Pci(Pci), Domain(u32) }
    pub struct Capability { pub data: CapabilityData }
    pub struct ProtectionDomain { pub id: u32, pub entries: RefCell<Vec<(u64, CapabilityData)>> }
    impl ProtectionDomain {
        pub fn find_capability(&self, slot: u32) -> Option<Capability> {
            self.entries.borrow().iter().find(|(s,_)| *s == slot as u64).map(|(_,data)| Capability {data:*data})
        }
        pub fn grant_capability(&self, slot: u64, data: CapabilityData) { self.entries.borrow_mut().push((slot,data)); }
    }
}
mod ipc { pub const MAX_DOMAINS: usize = 1024; }
struct Model { boot: bool, domains: Vec<&'static ProtectionDomain>, backings: Vec<u64>,
    maps: Vec<(u32,u64,u64,bool,bool)>, selftests: Vec<(u32,[u32;8])> }
thread_local! { static MODEL: RefCell<Model> = RefCell::new(Model { boot:true, domains:vec![], backings:vec![], maps:vec![], selftests:vec![] }); }
struct Registry;
static DOMAIN_REGISTRY: Registry = Registry;
impl Registry { fn get(&self, id:u32) -> Option<&'static ProtectionDomain> { MODEL.with(|m| m.borrow().domains.iter().copied().find(|p|p.id==id)) } }
fn require_boot() { MODEL.with(|m|assert!(m.borrow().boot,"after seal")); }
fn backing(length:u64)->u64 {
    require_boot(); MODEL.with(|m| { let mut m=m.borrow_mut(); m.backings.push(length); 0x100000 + m.backings.len() as u64 * 0x10000 })
}
fn alias(pd:&ProtectionDomain, physical:u64, length:u64, writable:bool, mmio:bool)->u64 {
    require_boot(); MODEL.with(|m| { let mut m=m.borrow_mut(); m.maps.push((pd.id,physical,length,writable,mmio)); 0x40000000 + m.maps.len() as u64 * 0x10000 })
}
// Native selftest reads real aliases/leaves. Host model records its invocation.
fn document_selftest(files:u32, clients:[u32;8]) { MODEL.with(|m|m.borrow_mut().selftests.push((files,clients))); }
mod syscalls { include!(env!("SEXOS_REGION_QUERY_RS")); }
include!(env!("DOCUMENT_BOOT_SOURCE"));
fn setup(ids:&[u32]) {
    MODEL.with(|m| *m.borrow_mut()=Model { boot:true, domains:ids.iter().map(|id| {
        &*Box::leak(Box::new(ProtectionDomain {id:*id, entries:RefCell::new(vec![])}))
    }).collect(), backings:vec![], maps:vec![], selftests:vec![] });
}
fn query(id:u32, number:u64, slot:u64, rights:u8)->[u64;5] {
    syscalls::region_query(DOMAIN_REGISTRY.get(id).unwrap(),number,slot,0,document_buffer::BYTES,rights as u64)
}
const FAIL:[u64;5]=[u64::MAX,0,0,0,0];
#[test] fn exact_subjects_have_distinct_backing_opposite_peers_and_readonly_client_authority() {
    // Deliberately not the current native IDs: constructors use actual subjects.
    setup(&[27,41,42,43,44,45,46,47,48]); init_documents(27,[41,42,43,44,45,46,47,48]);
    for (index,client) in [41,42,43,44,45,46,47,48].into_iter().enumerate() {
        let slot=document_buffer::PROVIDER_SLOTS[index]; let id=if index<3{6+index as u64}else{8192+index as u64-3};
        assert_eq!(query(27,55,slot,0),[0,16384,7,client as u64,id]);
        assert_eq!(query(client,55,64,0),[0,16384,3,27,id]);
        let writer=query(27,54,slot,QUERY_VIEW|READ|WRITE); let reader=query(client,54,64,QUERY_VIEW|READ);
        assert_eq!([writer[0],writer[2],writer[3],writer[4]],[0,16384,0,id]);
        assert_eq!([reader[0],reader[2],reader[3],reader[4]],[0,16384,0,id]); assert_ne!(writer[1],reader[1]);
        for rights in [QUERY_VIEW|WRITE, QUERY_VIEW|memory_region::DMA_ADDRESS] { assert_eq!(query(client,54,64,rights),FAIL); }
        for wrong in [65,66,67,68,69,70,71] { assert_eq!(query(client,55,wrong,0),FAIL); }
        let cap=DOMAIN_REGISTRY.get(27).unwrap().find_capability(slot as u32).unwrap();
        let CapabilityData::Region(g)=cap.data else {panic!()};
        let request=memory_region::ViewRequest{offset:0,length:16384,rights:QUERY_VIEW|READ};
        assert!(memory_region::resolve_peer_view(Some(&g),client,request).is_ok());
        for wrong in [0,27,99] { assert_eq!(memory_region::resolve_peer_view(Some(&g),wrong,request),Err(memory_region::RegionError::WrongPeer)); }
    }
    MODEL.with(|m| { let m=m.borrow(); assert_eq!(m.backings,[16384;8]); assert_eq!(m.selftests,[(27,[41,42,43,44,45,46,47,48])]);
        for (index,pair) in m.maps.chunks_exact(2).enumerate() {
            assert_eq!(pair[0],(27,0x110000+index as u64*0x10000,16384,true,false));
            assert_eq!(pair[1],(41+index as u32,pair[0].1,16384,false,false));
        }
    });
}
#[test] fn invalid_subjects_or_occupied_slots_fail_before_any_reservation() {
    for clients in [[41,41,43,44,45,46,47,48],[41,27,43,44,45,46,47,48],[41,42,99,44,45,46,47,48]] {
        setup(&[27,41,42,43,44,45,46,47,48]); assert!(std::panic::catch_unwind(||init_documents(27,clients)).is_err());
        MODEL.with(|m|assert!(m.borrow().backings.is_empty()));
    }
    for (holder,slot) in [(27,66),(43,64)] {
        setup(&[27,41,42,43,44,45,46,47,48]); DOMAIN_REGISTRY.get(holder).unwrap().grant_capability(slot,CapabilityData::Domain(99));
        assert!(std::panic::catch_unwind(||init_documents(27,[41,42,43,44,45,46,47,48])).is_err());
        MODEL.with(|m|assert!(m.borrow().backings.is_empty()));
    }
}
#[test] fn missing_optional_subjects_skip_and_sealed_phase_never_allocates() {
    setup(&[41]); init_documents(0,[41,0,0,0,0,0,0,0]); MODEL.with(|m|assert!(m.borrow().backings.is_empty()));
    setup(&[27,41]); init_documents(27,[41,0,0,0,0,0,0,0]); MODEL.with(|m|assert_eq!(m.borrow().backings,[16384]));
    setup(&[27,41]); MODEL.with(|m|m.borrow_mut().boot=false);
    assert!(std::panic::catch_unwind(||init_documents(27,[41,0,0,0,0,0,0,0])).is_err());
    MODEL.with(|m|assert!(m.borrow().backings.is_empty()));
}
#[test] fn existing_same_slot_pair_keeps_both_ends_writable() {
    setup(&[11,2]); paired(11,2,17,4,4096,true);
    for (holder,peer) in [(11,2),(2,11)] { assert_eq!(syscalls::region_query(DOMAIN_REGISTRY.get(holder).unwrap(),55,17,0,0,0),[0,4096,7,peer,4]); }
    MODEL.with(|m|assert!(m.borrow().maps.iter().all(|r|r.3&&!r.4)));
}
