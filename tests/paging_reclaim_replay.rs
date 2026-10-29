// SPDX-License-Identifier: GPL-3.0-only
//! Production sext policy and allocator; backend doubles leaf/TLB execution.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/paging.rs"] pub mod paging;
#[path="../crates/sex-pdx/src/memory_region.rs"] pub mod memory_region;
#[path="../servers/sext/src/allocator.rs"] mod allocator;
#[path="../servers/sext/src/pager.rs"] mod pager;
use paging::*;
use pager::*;
use std::collections::{BTreeMap,VecDeque};
const POOL:u64=0x400000;
const A:u64=0x800000;
const B:u64=0x900000;
struct Fake {
    queries:BTreeMap<(u64,u64),FaultQuery>,
    bytes:Vec<u8>, commits:Vec<(u64,usize,u64)>, reclaims:Vec<(u64,usize,u64)>, zeros:Vec<usize>,
    reclaim_status:VecDeque<u64>, commit_status:VecDeque<u64>,
}
impl Fake {
 fn new()->Self{Self{queries:BTreeMap::new(),bytes:vec![0xa5;8*4096],commits:vec![],reclaims:vec![],zeros:vec![],reclaim_status:VecDeque::new(),commit_status:VecDeque::new()}}
 fn notice(&mut self,target:u32,page:usize,token:u64)->FaultNotice{
  let (base,slot)=if target==15{(A,36)}else{(B,37)};
  let n=FaultNotice{fault_addr:base+page as u64*4096,error_code:6,token,caller_pd:target};
  self.queries.insert((slot,token),FaultQuery{page_va:n.page(),error_code:6,target_pd:target,allowed_rights:6});n
 }
}
impl Backend for Fake {
 fn cleanup(&mut self,_mapping:u64)->Result<CleanupResult,u64>{Err(ERR_PAGING_LIVE)}
 fn query(&mut self,map:u64,token:u64)->Result<FaultQuery,u64>{self.queries.get(&(map,token)).copied().ok_or(ERR_PAGING_INVALID)}
 fn zero(&mut self,va:u64,len:u64){let start=(va-POOL)as usize;assert_eq!(len,4096);self.zeros.push(start/4096);self.bytes[start..start+4096].fill(0);}
 fn commit(&mut self,map:u64,index:usize,token:u64,_rights:u8)->u64{self.commits.push((map,index,token));self.commit_status.pop_front().unwrap_or(PAGING_OK)}
 fn reject(&mut self,_map:u64,_token:u64)->u64{PAGING_OK}
 fn reclaim(&mut self,map:u64,index:usize,token:u64)->u64{self.reclaims.push((map,index,token));self.reclaim_status.pop_front().unwrap_or(PAGING_OK)}
}
fn pager()->Pager<8>{Pager::new_multi(PagingInfo{base:POOL,length:8*4096,target_pd:0,rights:6},[
 Some(Mapping{slot:36,info:PagingInfo{base:A,length:8*4096,target_pd:15,rights:6}}),
 Some(Mapping{slot:37,info:PagingInfo{base:B,length:8*4096,target_pd:10,rights:6}})],4).unwrap()}
fn request(target:u32,page:usize,id:u64)->ReleaseRequest{ReleaseRequest{page:if target==15{A}else{B}+page as u64*4096,request_id:id,caller_pd:target}}
fn commit(p:&mut Pager<8>,b:&mut Fake,target:u32,page:usize,token:u64)->usize{
 let n=b.notice(target,page,token);match p.step(b,n){Outcome::Committed{index,..}=>index,other=>panic!("{other:?}")}
}
fn success(result:ReleaseOutcome){assert!(matches!(result,ReleaseOutcome::Done{status:PAGING_OK,..}),"{result:?}");}
#[test]fn exact_release_replay_after_same_page_refault_does_not_touch_new_generation(){
 let(mut p,mut b)=(pager(),Fake::new());let old=commit(&mut p,&mut b,15,0,1);let r=request(15,0,1);
 success(p.release(&mut b,r));let new=commit(&mut p,&mut b,15,0,2);assert_eq!(old,new);
 b.bytes[new*4096..(new+1)*4096].fill(0x7b);let calls=b.reclaims.len();
 assert!(matches!(p.release(&mut b,r),ReleaseOutcome::Done{status:0,replay:true,..}));
 assert_eq!(b.reclaims.len(),calls);assert_eq!(p.used_by(15),1);assert!(b.bytes[new*4096..(new+1)*4096].iter().all(|&v|v==0x7b));
}
#[test]fn older_request_and_same_id_different_page_cannot_reclaim_current_mapping(){
 let(mut p,mut b)=(pager(),Fake::new());commit(&mut p,&mut b,15,0,1);commit(&mut p,&mut b,15,1,2);
 success(p.release(&mut b,request(15,0,4)));success(p.release(&mut b,request(15,1,5)));commit(&mut p,&mut b,15,0,3);
 for r in [request(15,0,4),request(15,0,5)]{assert!(matches!(p.release(&mut b,r),ReleaseOutcome::Done{status:ERR_PAGING_DENIED,..}));}
 assert_eq!(b.reclaims.len(),2);assert_eq!(p.used_by(15),1);
}
#[test]fn reclaim_retry_pins_binding_while_other_client_progresses(){
 let(mut p,mut b)=(pager(),Fake::new());let index=commit(&mut p,&mut b,15,0,11);
 b.reclaim_status.extend([ERR_PAGING_RETRY,ERR_PAGING_RETRY,PAGING_OK]);let r=request(15,0,1);
 assert_eq!(p.release(&mut b,r),ReleaseOutcome::Retry);assert_eq!(p.used_by(15),1);
 let other=commit(&mut p,&mut b,10,0,12);assert_ne!(other,index);
 assert_eq!(p.release(&mut b,r),ReleaseOutcome::Retry);success(p.release(&mut b,r));
 assert_eq!(b.reclaims,vec![(36,index,11);3]);assert_eq!(p.used_by(10),1);assert_eq!(p.used_by(15),0);
}
#[test]fn cross_client_reuse_zeroes_entire_frame_and_old_release_cannot_free_it(){
 let(mut p,mut b)=(pager(),Fake::new());let old=commit(&mut p,&mut b,10,0,21);b.bytes[old*4096..(old+1)*4096].fill(0xde);
 let r=request(10,0,1);success(p.release(&mut b,r));let new=commit(&mut p,&mut b,15,0,22);assert_eq!(old,new);
 assert!(b.bytes[new*4096..(new+1)*4096].iter().all(|&v|v==0));assert_eq!(b.zeros,vec![old,new]);
 success(p.release(&mut b,r));assert_eq!(b.reclaims,vec![(37,old,21)]);assert_eq!(p.used_by(15),1);
}
#[test]fn foreign_caller_and_malformed_request_never_reach_kernel_reclaim(){
 let(mut p,mut b)=(pager(),Fake::new());commit(&mut p,&mut b,15,0,1);
 for r in [ReleaseRequest{page:A,request_id:1,caller_pd:10},ReleaseRequest{page:A,request_id:1,caller_pd:99},request(15,0,0),ReleaseRequest{page:A+1,request_id:2,caller_pd:15}]{
  assert!(matches!(p.release(&mut b,r),ReleaseOutcome::Done{status:ERR_PAGING_DENIED|ERR_PAGING_INVALID,..}));
 }
 assert!(b.reclaims.is_empty());assert_eq!(p.used_by(15),1);
}
#[test]fn uncertain_reclaim_never_reuses_backing_or_clears_quota(){
 let(mut p,mut b)=(pager(),Fake::new());let index=commit(&mut p,&mut b,15,0,1);
 b.reclaim_status.push_back(99);assert_eq!(p.release(&mut b,request(15,0,1)),ReleaseOutcome::Fatal(99));assert_eq!(p.used_by(15),1);
 assert_ne!(commit(&mut p,&mut b,10,0,2),index);let n=b.notice(15,1,3);assert_eq!(p.step(&mut b,n),Outcome::Retry);
}
#[test]fn independent_clients_can_release_same_request_id_without_aliasing(){
 let(mut p,mut b)=(pager(),Fake::new());let a=commit(&mut p,&mut b,15,0,1);let c=commit(&mut p,&mut b,10,0,2);
 success(p.release(&mut b,request(15,0,1)));success(p.release(&mut b,request(10,0,1)));
 assert_eq!(b.reclaims,vec![(36,a,1),(37,c,2)]);assert_eq!(p.used(),0);
}
#[test]fn fault_commit_retry_prevents_release_until_finalized(){
 let(mut p,mut b)=(pager(),Fake::new());let n=b.notice(15,0,1);b.commit_status.push_back(ERR_PAGING_RETRY);
 assert_eq!(p.step(&mut b,n),Outcome::Retry);assert_eq!(p.release(&mut b,request(15,0,1)),ReleaseOutcome::Retry);assert!(b.reclaims.is_empty());
 assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));success(p.release(&mut b,request(15,0,1)));assert_eq!(p.used(),0);
}
#[test]fn delayed_ack_is_bound_to_the_exact_outstanding_request(){
 for status in [PAGING_OK,ERR_PAGING_INVALID,ERR_PAGING_RETRY,ERR_PAGING_DENIED,ERR_PAGING_EXHAUSTED]{
  let old=release_reply(1,status);assert_eq!(release_reply_status(old,1),Ok(status));assert_eq!(release_reply_status(old,2),Err(ERR_PAGING_DENIED));
  let current=release_reply(2,status);assert_eq!(release_reply_status(current,2),Ok(status));assert_eq!(release_reply_status(current,1),Err(ERR_PAGING_DENIED));
 }
}
#[test]fn release_request_ids_never_alias_through_truncation_or_wrap(){
 let max=MAX_RELEASE_REQUEST_ID;let valid=ReleaseRequest::decode(OP_RELEASE_PAGE,15,[A,max,0]).unwrap();assert_eq!(valid.request_id,max);
 assert_eq!(release_reply_status(release_reply(max,PAGING_OK),max),Ok(PAGING_OK));
 for id in [0,max+1,u64::MAX]{assert!(ReleaseRequest::decode(OP_RELEASE_PAGE,15,[A,id,0]).is_err());assert!(release_reply_status(release_reply(id,0),id).is_err());}
}
