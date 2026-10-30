#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/document.rs"]pub mod document;
#[path="../crates/sex-pdx/src/document_buffer.rs"]pub mod document_buffer;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../crates/sex-pdx/src/document_snapshot.rs"]pub mod document_snapshot;
use document_snapshot::{Identity,Region};
use memory_region::*;
use std::sync::{Arc,atomic::{AtomicU64,Ordering::SeqCst}};
#[repr(align(4096))]struct Storage([AtomicU64;2048]);
impl Storage{fn new()->Self{Self([const{AtomicU64::new(0)};2048])}}
fn region(s:&Storage,subject:u32,writer:bool)->Region{
 let index=match subject{9=>0,16=>1,18=>2,_=>panic!()};
 let g=RegionGrant{region_id:6+index,virtual_base:s.0.as_ptr() as u64,length:16384,rights:QUERY_VIEW|READ|if writer{WRITE}else{0},dma_base:None,peer_pd:Some(if writer{subject}else{11})};
 unsafe{Region::from_resolved(resolve_info(Some(&g)).unwrap(),resolve_view(Some(&g),ViewRequest{offset:0,length:16384,rights:g.rights}).unwrap(),subject,writer).unwrap()}
}
fn id(subject:u32,sequence:u32)->Identity{Identity{subject,sequence,object:3,generation:0x40001}}
#[test]fn empty_tail_and_maximum_snapshots_copy_exactly_then_full_scrub(){
 for subject in [9,16,18]{for length in [0,1,7,8,9,12287,12288]{
  let s=Box::new(Storage::new());let mut writer=region(&s,subject,true);let reader=region(&s,subject,false);
  let bytes=(0..length).map(|i|(i%93+33)as u8).collect::<Vec<_>>();let name=*b"snapshot\0\0\0\0\0\0\0\0";
  writer.publish(id(subject,7),name,&bytes).unwrap();let mut out=[255;12288];let mut n=[255;16];reader.copy(id(subject,7),length,&mut out,&mut n).unwrap();
  assert_eq!(&out[..length],bytes);assert!(out[length..].iter().all(|&v|v==0));assert_eq!(n,name);
  writer.clear().unwrap();assert!(s.0.iter().all(|v|v.load(SeqCst)==0));
  assert!(reader.copy(id(subject,7),length,&mut out,&mut n).is_err());assert_eq!(out,[0;12288]);assert_eq!(n,[0;16]);
 }}
}
#[test]fn wrong_identity_header_hash_or_publication_never_leaves_partial_private_bytes(){
 let s=Box::new(Storage::new());let mut writer=region(&s,9,true);let reader=region(&s,9,false);
 for (word,value) in [(0,1),(0,0),(1,0),(2,16),(3,0),(4,8),(5,0),(64,0)]{
  writer.clear().unwrap();
  writer.publish(id(9,7),[1;16],b"payload").unwrap();s.0[word].store(value,SeqCst);
  let mut out=[255;12288];let mut name=[255;16];assert!(reader.copy(id(9,7),7,&mut out,&mut name).is_err());assert_eq!(out,[0;12288]);assert_eq!(name,[0;16]);
 }
 writer.clear().unwrap();writer.publish(id(9,7),[1;16],b"payload").unwrap();
 for bad in [id(16,7),id(9,8),Identity{object:4,..id(9,7)},Identity{generation:0x40002,..id(9,7)}]{
  assert!(reader.copy(bad,7,&mut[0;12288],&mut[0;16]).is_err());
 }
 assert!(reader.copy(id(9,7),6,&mut[0;12288],&mut[0;16]).is_err());
}
#[test]fn roles_and_resolved_metadata_cannot_upgrade_client_or_cross_pair(){
 let s=Box::new(Storage::new());let mut reader=region(&s,9,false);let mut writer=region(&s,9,true);
 assert_eq!(reader.clear(),Err(document::DENIED));assert_eq!(reader.publish(id(9,1),[0;16],b"x"),Err(document::DENIED));
 assert_eq!(writer.copy(id(9,1),1,&mut[0;12288],&mut[0;16]),Err(document::DENIED));
 for bad in [id(16,1),id(9,0),id(9,document::SEQ_MAX+1),Identity{object:15,..id(9,1)}]{assert!(writer.publish(bad,[0;16],b"x").is_err());}
 let info=MemoryRegionInfo{region_id:6,length:16384,rights:QUERY_VIEW|READ,peer_pd:Some(11)};
 let view=RegionView{region_id:6,virtual_address:s.0.as_ptr()as u64,length:16384,dma_address:None};
 for bad in [MemoryRegionInfo{rights:7,..info},MemoryRegionInfo{peer_pd:Some(16),..info},MemoryRegionInfo{region_id:7,..info},MemoryRegionInfo{length:8192,..info}]{assert!(unsafe{Region::from_resolved(bad,view,9,false)}.is_err());}
 for bad in [RegionView{virtual_address:view.virtual_address+1,..view},RegionView{length:8192,..view},RegionView{dma_address:Some(4096),..view}]{assert!(unsafe{Region::from_resolved(info,bad,9,false)}.is_err());}
 assert!(s.0.iter().all(|w|w.load(SeqCst)==0));
}
#[test]fn racing_atomic_publication_or_scrub_cannot_deliver_hybrid_payload(){
 let s=Arc::new(Storage::new());let mut writer=region(&s,9,true);let reader=region(&s,9,false);
 writer.publish(id(9,7),[1;16],&[b'A';12288]).unwrap();
 let shared=s.clone();let task=std::thread::spawn(move||{
  assert_eq!(writer.publish(id(9,7),[1;16],b"repeat"),Err(document::STALE));
  for n in 0..80{let byte=if n%2==0{b'A'}else{b'B'};writer.publish(id(9,8+n),[byte;16],&[byte;12288]).unwrap();writer.clear().unwrap();}
  writer.publish(id(9,88),[b'Z';16],&[b'Z';12288]).unwrap();drop(shared);
 });
 for _ in 0..160{let mut out=[0;12288];let mut name=[0;16];if reader.copy(id(9,7),12288,&mut out,&mut name).is_ok(){assert!(out.iter().all(|b|*b==b'A')||out.iter().all(|b|*b==b'B'));}}
 task.join().unwrap();let mut out=[0;12288];let mut name=[0;16];reader.copy(id(9,88),12288,&mut out,&mut name).unwrap();assert_eq!(out,[b'Z';12288]);assert_eq!(name,[b'Z';16]);
}
