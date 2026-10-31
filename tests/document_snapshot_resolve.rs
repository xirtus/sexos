//! Actual library syscall adapter; current-domain capability tables are mocks.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/document.rs"]pub mod document;
#[path="../crates/sex-pdx/src/document_buffer.rs"]pub mod document_buffer;
#[path="../crates/sex-pdx/src/document_snapshot.rs"]pub mod document_snapshot;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
use memory_region::*;
use std::cell::RefCell;
thread_local!{static CAPS:RefCell<Vec<(u64,RegionGrant)>>=const{RefCell::new(Vec::new())};static QUERIES:RefCell<Vec<u64>>=const{RefCell::new(Vec::new())};}
fn memory_region_info(slot:u64)->Result<MemoryRegionInfo,u64>{QUERIES.with(|q|q.borrow_mut().push(slot));CAPS.with(|c|resolve_info(c.borrow().iter().find(|(s,_)|*s==slot).map(|(_,g)|g)).map_err(|_|u64::MAX))}
fn memory_region_view(slot:u64,offset:u64,length:u64,rights:u8)->Result<RegionView,u64>{QUERIES.with(|q|q.borrow_mut().push(slot));CAPS.with(|c|resolve_view(c.borrow().iter().find(|(s,_)|*s==slot).map(|(_,g)|g),ViewRequest{offset,length,rights}).map_err(|_|u64::MAX))}
include!(env!("DOCUMENT_SNAPSHOT_RESOLVER_SOURCE"));
fn setup(client:u32,writer:bool){
 let i=[9,16,18,19,20,21,22,23].iter().position(|p|*p==client).unwrap();let slot=if writer{64+i as u64}else{64};
 CAPS.with(|c|*c.borrow_mut()=vec![(slot,RegionGrant{region_id:if i<3{6+i as u32}else{8192+i as u32-3},virtual_base:0x40000000,length:16384,rights:3|if writer{4}else{0},dma_base:None,peer_pd:Some(if writer{client}else{11})})]);QUERIES.with(|q|q.borrow_mut().clear());
}
#[test]fn actual_adapter_uses_exact_current_domain_slots_and_never_upgrades_client(){
 for client in [9,16,18,19,20,21,22,23]{for writer in [false,true]{setup(client,writer);assert_eq!(document_snapshot_view(client,writer).unwrap().subject(),client);
  let slot=if writer{64+[9,16,18,19,20,21,22,23].iter().position(|p|*p==client).unwrap()as u64}else{64};QUERIES.with(|q|assert_eq!(*q.borrow(),[slot,slot]));}}
 setup(9,false);assert!(document_snapshot_view(9,true).is_err());assert!(document_snapshot_view(16,false).is_err());
 setup(9,true);assert!(document_snapshot_view(9,false).is_err());
}
#[test]fn absent_wrong_peer_wrong_kind_metadata_and_unsupported_subject_reject(){
 setup(9,false);assert!(document_snapshot_view(99,false).is_err());QUERIES.with(|q|assert!(q.borrow().is_empty()));
 CAPS.with(|c|c.borrow_mut().clear());assert!(document_snapshot_view(9,false).is_err());
 setup(9,false);CAPS.with(|c|c.borrow_mut()[0].1.peer_pd=Some(16));assert!(document_snapshot_view(9,false).is_err());
 setup(9,false);CAPS.with(|c|c.borrow_mut()[0].1.region_id=7);assert!(document_snapshot_view(9,false).is_err());
}
