// SPDX-License-Identifier: GPL-3.0-only
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/serial_record_buffer.rs"] mod serial_record_buffer;

#[test]
fn structured_record_uses_one_transport_invocation() {
    let mut calls = Vec::new();
    serial_record_buffer::emit(format_args!("[test] token={} page={:#x}", 7, 4096), |line| calls.push(line.to_owned()));
    assert_eq!(calls, ["[test] token=7 page=0x1000\n"]);
}

#[test]
fn structured_record_overflow_emits_only_fatal_diagnostic() {
    let oversized = "x".repeat(256);
    let mut calls = Vec::new();
    serial_record_buffer::emit(format_args!("{}", oversized), |line| calls.push(line.to_owned()));
    assert_eq!(calls, ["[serial.record.overflow] panic diagnostic-too-long\n"]);
}

#[test]
fn structured_record_preserves_utf8_and_capacity_boundary() {
    let exact = "é".repeat(127) + "!";
    serial_record_buffer::emit(format_args!("{}", exact), |line| {
        assert_eq!(line.len(), 256);
        assert_eq!(line.trim_end(), exact);
    });
}
#[path="../crates/sex-pdx/src/paging.rs"] pub mod paging;
#[path="../crates/sex-pdx/src/memory_region.rs"] pub mod memory_region;
#[path="../servers/sext/src/allocator.rs"] mod allocator;
#[path="../servers/sext/src/pager.rs"] mod pager;
use paging::*;
use memory_region::{READ, WRITE};
use allocator::*;
use pager::*;
use std::collections::VecDeque;
const POOL:u64=0x400000;
const ARENA:u64=0x800000;
fn info()->(PagingInfo,PagingInfo) {
    (PagingInfo{base:POOL,length:8*4096,target_pd:0,rights:6},
     PagingInfo{base:ARENA,length:8*4096,target_pd:15,rights:6})
}
fn notice(page:usize)->FaultNotice { FaultNotice{fault_addr:ARENA+page as u64*4096+19,error_code:if page%2==0 {4}else{6},token:page as u64+1,caller_pd:15} }
fn query(n:FaultNotice)->FaultQuery { FaultQuery{page_va:n.page(),error_code:n.error_code,target_pd:n.caller_pd,allowed_rights:6} }
struct Fake {
    answer:Result<FaultQuery,u64>, queries:usize, zeros:Vec<(u64,u64)>,
    bytes:Vec<u8>, commits:Vec<(usize,u64,u8)>, rejections:Vec<u64>,
    commit_status:VecDeque<u64>, reject_status:VecDeque<u64>, reclaim_status:VecDeque<u64>, reclaimed:Vec<(u64,usize,u64)>, cleanup_status:VecDeque<Result<CleanupResult,u64>>, cleanups:Vec<u64>, routes:Vec<u64>, events:Vec<&'static str>,
}
impl Fake { fn new(n:FaultNotice)->Self { Self{answer:Ok(query(n)),queries:0,zeros:vec![],bytes:vec![0xA5;8*4096],
    commits:vec![],rejections:vec![],commit_status:VecDeque::new(),reject_status:VecDeque::new(),reclaim_status:VecDeque::new(),reclaimed:vec![],cleanup_status:VecDeque::new(),cleanups:vec![],routes:vec![],events:vec![]} } }
impl Backend for Fake {
    fn cleanup(&mut self,mapping:u64)->Result<CleanupResult,u64> { self.cleanups.push(mapping); self.cleanup_status.pop_front().unwrap_or(Err(ERR_PAGING_LIVE)) }
    fn reclaim(&mut self,mapping:u64,index:usize,token:u64)->u64 { self.reclaimed.push((mapping,index,token));self.events.push("reclaim");self.reclaim_status.pop_front().unwrap_or(PAGING_OK) }
    fn query(&mut self,mapping:u64,_:u64)->Result<FaultQuery,u64>{self.routes.push(mapping);self.queries+=1;self.events.push("query");self.answer}
    fn zero(&mut self,va:u64,len:u64){self.zeros.push((va,len));self.events.push("zero");let offset=(va-POOL)as usize;self.bytes[offset..offset+len as usize].fill(0);}
    fn commit(&mut self,_mapping:u64,index:usize,token:u64,rights:u8)->u64{self.events.push("commit");self.commits.push((index,token,rights));self.commit_status.pop_front().unwrap_or(PAGING_OK)}
    fn reject(&mut self,_mapping:u64,token:u64)->u64{self.events.push("reject");self.rejections.push(token);self.reject_status.pop_front().unwrap_or(PAGING_OK)}
}
fn pager()->Pager<8>{let(p,m)=info();Pager::new(p,m,4).unwrap()}
#[test] fn notice_codec_rejects_foreign_width_stale_shape_and_protection_faults(){
 let n=notice(0);assert_eq!(FaultNotice::decode(OP_FAULT_NOTICE,15,n.args()),Ok(n));
 for (op,pd,args) in [(0x100,15,n.args()),(OP_FAULT_NOTICE,0,n.args()),(OP_FAULT_NOTICE,1u64<<32,n.args()),
 (OP_FAULT_NOTICE,15,[n.fault_addr,7,1]),(OP_FAULT_NOTICE,15,[n.fault_addr,20,1]),
 (OP_FAULT_NOTICE,15,[1<<47,4,1]),(OP_FAULT_NOTICE,15,[0,4,1]),(OP_FAULT_NOTICE,15,[n.fault_addr,4,0])]{assert!(FaultNotice::decode(op,pd,args).is_err());}
}
#[test] fn entire_page_is_zeroed_by_pager_before_commit(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);
 assert!(matches!(p.step(&mut b,n),Outcome::Committed{index:0,..}));
 assert_eq!(b.events,vec!["query","zero","commit"]);assert_eq!(b.zeros,vec![(POOL,4096)]);
 assert!(b.bytes[..4096].iter().all(|x|*x==0));assert!(b.bytes[4096..].iter().all(|x|*x==0xA5));assert_eq!(b.commits,vec![(0,1,READ|WRITE)]);
}
#[test] fn prepared_query_retry_preserves_notice_without_allocating(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);b.answer=Err(ERR_PAGING_RETRY);
 for _ in 0..32{assert_eq!(p.step(&mut b,n),Outcome::Retry);assert_eq!(p.used(),0);}assert!(b.zeros.is_empty());
 b.answer=Ok(query(n));assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));assert_eq!(b.zeros.len(),1);
}
#[test] fn commit_retry_keeps_exact_frame_and_never_zeroes_client_data_again(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);
 b.commit_status.extend([ERR_PAGING_RETRY,ERR_PAGING_RETRY,PAGING_OK]);assert_eq!(p.step(&mut b,n),Outcome::Retry);
 b.bytes[0]=0x77;assert_eq!(p.step(&mut b,n),Outcome::Retry);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 assert_eq!(b.zeros.len(),1);assert_eq!(b.bytes[0],0x77);assert_eq!(b.queries,1);assert_eq!(b.commits,vec![(0,1,6);3]);assert_eq!(p.used(),1);
}
#[test] fn post_retry_denial_is_uncertain_and_cannot_free_installed_backing(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);
 b.commit_status.extend([ERR_PAGING_RETRY,ERR_PAGING_DENIED]);assert_eq!(p.step(&mut b,n),Outcome::Retry);
 assert!(matches!(p.step(&mut b,n),Outcome::Fatal(_)));assert_eq!(p.used(),1);assert!(b.rejections.is_empty());
}
#[test] fn definite_prepublication_failure_releases_frame_and_retries_reject_only(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);
 b.commit_status.push_back(ERR_PAGING_DENIED);b.reject_status.extend([ERR_PAGING_RETRY,PAGING_OK]);assert_eq!(p.step(&mut b,n),Outcome::Retry);assert_eq!(p.used(),0);
 assert_eq!(p.step(&mut b,n),Outcome::Retry);assert!(matches!(p.step(&mut b,n),Outcome::Rejected{reason:RejectReason::CommitDenied,..}));assert_eq!(b.zeros.len(),1);assert_eq!(b.commits.len(),1);
}
#[test] fn unknown_commit_status_keeps_custody(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);b.commit_status.push_back(999);
 assert_eq!(p.step(&mut b,n),Outcome::Fatal(999));assert_eq!(p.used(),1);assert!(b.rejections.is_empty());
}
#[test] fn userland_quota_rejects_fifth_in_range_page_retaining_four_mappings(){let mut p=pager();let mut b=Fake::new(notice(0));
 for page in 0..4 {let n=notice(page);b.answer=Ok(query(n));assert!(matches!(p.step(&mut b,n),Outcome::Committed{index,..} if index==page));}
 let n=notice(4);b.answer=Ok(query(n));assert_eq!(p.step(&mut b,n),Outcome::Rejected{reason:RejectReason::Quota,page:n.page(),target:15,token:n.token});
 assert_eq!(p.used(),4);assert_eq!(b.zeros.len(),4);assert_eq!(b.rejections,vec![5]);
}
#[test] fn stale_token_or_foreign_notice_does_not_zero_commit_or_reject(){
 for mode in 0..5 {let mut p=pager();let n=notice(0);let mut b=Fake::new(n);
  match mode {0=>b.answer=Err(ERR_PAGING_INVALID),1=>b.answer.as_mut().unwrap().target_pd=27,
  2=>b.answer.as_mut().unwrap().page_va+=4096,3=>b.answer.as_mut().unwrap().error_code=6,_=>b.answer=Err(ERR_PAGING_DENIED)}
  assert_eq!(p.step(&mut b,n),Outcome::Ignored);assert_eq!(p.used(),0);assert!(b.zeros.is_empty()&&b.commits.is_empty()&&b.rejections.is_empty()); }
}
#[test] fn another_notice_cannot_replace_an_uncertain_commit(){let mut p=pager();let n=notice(0);let mut b=Fake::new(n);b.commit_status.push_back(ERR_PAGING_RETRY);
 assert_eq!(p.step(&mut b,n),Outcome::Retry);assert_eq!(p.step(&mut b,notice(1)),Outcome::Ignored);assert_eq!(b.queries,1);assert_eq!(p.used(),1);
 assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
}
#[test] fn write_policy_and_extent_fail_before_frame_selection(){for mode in 0..2 {let mut p=pager();let n=if mode==0 {notice(1)}else{notice(8)};let mut b=Fake::new(n);
 if mode==0 {b.answer.as_mut().unwrap().allowed_rights=READ;}
 assert!(matches!(p.step(&mut b,n),Outcome::Rejected{reason:RejectReason::Policy,..}));assert_eq!(p.used(),0);assert!(b.zeros.is_empty());}}
#[test] fn allocator_exhaustion_is_distinct_from_per_client_quota(){let mut a=FrameAllocator::<8>::new(8);
 for i in 0..8{assert_eq!(a.reserve(i+1,i as u32+1,0x1000).unwrap().index,i as usize);}
 assert_eq!(a.reserve(9,9,0x1000),Err(AllocationError::Exhausted));assert_eq!(a.used(),8);
}
#[test] fn allocator_rejects_foreign_transition_alias_and_unzeroed_commit(){let mut a=FrameAllocator::<8>::new(4);
 let r=a.reserve(1,15,0x1000).unwrap();assert!(!a.commit(r.index,1));assert!(!a.mark_zeroed(r.index,2));assert!(!a.rollback_unpublished(r.index,2));
 assert_eq!(a.reserve(1,16,0x1000),Err(AllocationError::Conflict));assert_eq!(a.reserve(2,15,0x1000),Err(AllocationError::Conflict));
 assert!(a.mark_zeroed(r.index,1));assert!(!a.reserve(1,15,0x1000).unwrap().needs_zero);assert!(a.commit(r.index,1));
 assert!(!a.rollback_unpublished(r.index,1));assert_eq!(a.reserve(1,15,0x1000),Err(AllocationError::Conflict));
}
#[test] fn malformed_boot_views_and_quota_fail_closed(){let(p,m)=info();
 for bad in [PagingInfo{base:0,..p},PagingInfo{length:4095,..p},PagingInfo{rights:READ,..p},PagingInfo{target_pd:15,..p},PagingInfo{base:u64::MAX-4095,..p}]{assert!(Pager::<8>::new(bad,m,4).is_err());}
 assert!(Pager::<8>::new(p,m,0).is_err());assert!(Pager::<8>::new(p,m,9).is_err());
}

fn multi()->Pager<8> {let(p,m)=info();Pager::new_multi(p,[Some(Mapping{slot:36,info:m}),Some(Mapping{slot:37,info:PagingInfo{target_pd:10,base:ARENA+0x100000,..m}})],4).unwrap()}
fn bell(page:usize)->FaultNotice {FaultNotice{caller_pd:10,fault_addr:ARENA+0x100000+page as u64*4096,token:100+page as u64,..notice(page)}}
fn release(n:FaultNotice,id:u64)->ReleaseRequest {ReleaseRequest{page:n.page(),request_id:id,caller_pd:n.caller_pd}}
#[test] fn two_clients_share_pool_with_independent_quotas_and_authenticated_routes(){
 let mut p=multi();let mut b=Fake::new(notice(0));
 for page in 0..4 {for n in [notice(page),bell(page)] {b.answer=Ok(query(n));assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));}}
 assert_eq!(p.used(),8);assert_eq!(p.used_by(15),4);assert_eq!(p.used_by(10),4);assert_eq!(b.routes,vec![36,37,36,37,36,37,36,37]);
 for n in [notice(4),bell(4)] {b.answer=Ok(query(n));assert!(matches!(p.step(&mut b,n),Outcome::Rejected{reason:RejectReason::Quota,..}));}
 let foreign=FaultNotice{caller_pd:11,..notice(6)};b.answer=Ok(query(foreign));let calls=b.queries;
 assert_eq!(p.step(&mut b,foreign),Outcome::Ignored);assert_eq!(b.queries,calls);
}
#[test] fn release_detaches_before_cross_client_rezero_and_stale_ack_is_correlated(){
 let mut p=multi();let old=bell(0);let mut b=Fake::new(old);assert!(matches!(p.step(&mut b,old),Outcome::Committed{index:0,..}));
 b.bytes[..4096].fill(0x91);assert!(matches!(p.release(&mut b,release(old,1)),ReleaseOutcome::Done{status:PAGING_OK,replay:false,..}));
 assert_eq!(p.used(),0);assert_eq!(b.reclaimed,vec![(37,0,100)]);assert_eq!(b.bytes[0],0x91);
 let new=notice(0);b.answer=Ok(query(new));assert!(matches!(p.step(&mut b,new),Outcome::Committed{index:0,..}));
 assert!(b.bytes[..4096].iter().all(|x|*x==0));assert_eq!(b.events,vec!["query","zero","commit","reclaim","query","zero","commit"]);
 assert!(matches!(p.release(&mut b,release(old,1)),ReleaseOutcome::Done{status:PAGING_OK,replay:true,..}));assert_eq!(p.used(),1);assert_eq!(b.reclaimed.len(),1);
 assert_eq!(release_reply_status(release_reply(1,PAGING_OK),2),Err(ERR_PAGING_DENIED));
}
#[test] fn reclaim_retry_retains_exact_binding_and_other_client_progresses(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 b.reclaim_status.extend([ERR_PAGING_RETRY,PAGING_OK]);assert_eq!(p.release(&mut b,release(n,1)),ReleaseOutcome::Retry);assert_eq!(p.used(),1);
 let other=bell(0);b.answer=Ok(query(other));assert!(matches!(p.step(&mut b,other),Outcome::Committed{index:1,..}));
 assert!(matches!(p.release(&mut b,release(n,1)),ReleaseOutcome::Done{status:PAGING_OK,..}));assert_eq!(b.reclaimed,vec![(36,0,1);2]);assert_eq!(p.used(),1);
}
#[test] fn commit_retry_does_not_block_other_client_or_change_its_route(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);b.commit_status.push_back(ERR_PAGING_RETRY);assert_eq!(p.step(&mut b,n),Outcome::Retry);
 let other=bell(0);b.answer=Ok(query(other));assert!(matches!(p.step(&mut b,other),Outcome::Committed{index:1,..}));
 assert!(matches!(p.step(&mut b,n),Outcome::Committed{index:0,..}));assert_eq!(b.zeros.len(),2);
}
#[test] fn unknown_reclaim_result_never_releases_or_rezeros_backing(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 b.reclaim_status.push_back(99);assert_eq!(p.release(&mut b,release(n,1)),ReleaseOutcome::Fatal(99));assert_eq!(p.used(),1);assert_eq!(b.zeros.len(),1);
 assert_eq!(p.step(&mut b,notice(1)),Outcome::Retry);
}
#[test] fn release_id_tombstone_rejects_old_conflicting_foreign_and_malformed_requests(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 assert!(matches!(p.release(&mut b,release(n,2)),ReleaseOutcome::Done{status:0,..}));
 for req in [release(n,1),ReleaseRequest{page:n.page()+4096,..release(n,2)},ReleaseRequest{caller_pd:11,..release(n,3)}] {
 assert!(matches!(p.release(&mut b,req),ReleaseOutcome::Done{status:ERR_PAGING_DENIED,..}));}
 assert_eq!(b.reclaimed.len(),1);
 for args in [[n.page(),0,0],[n.page(),MAX_RELEASE_REQUEST_ID+1,0],[n.page()+1,1,0],[n.page(),1,1]] {assert!(ReleaseRequest::decode(OP_RELEASE_PAGE,15,args).is_err());}
 assert_eq!(release_reply(MAX_RELEASE_REQUEST_ID,3),u64::MAX-252);
 assert_eq!(release_reply_status(release_reply(MAX_RELEASE_REQUEST_ID,3),MAX_RELEASE_REQUEST_ID),Ok(3));
}

fn exited(target:u32)->ExitNotice {ExitNotice{target_pd:target,task_id:target as u64+20}}
#[test] fn terminal_cleanup_releases_only_dead_target_once_even_after_reuse(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{index:0,..}));
 let other=bell(0);b.answer=Ok(query(other));assert!(matches!(p.step(&mut b,other),Outcome::Committed{index:1,..}));
 let done=CleanupResult{frame_bitmap:1,task_id:35};b.cleanup_status.push_back(Ok(done));
 assert_eq!(p.cleanup(&mut b,exited(15)),CleanupOutcome::Closed{result:done,released:1,replay:false});assert_eq!(p.used_by(15),0);assert_eq!(p.used_by(10),1);
 let next=bell(1);b.answer=Ok(query(next));assert!(matches!(p.step(&mut b,next),Outcome::Committed{index:0,..}));
 assert_eq!(p.cleanup(&mut b,exited(15)),CleanupOutcome::Closed{result:done,released:0,replay:true});assert_eq!(p.used_by(10),2);assert_eq!(b.cleanups,vec![36]);
 assert_eq!(p.step(&mut b,notice(1)),Outcome::Ignored);
 assert!(matches!(p.release(&mut b,release(n,1)),ReleaseOutcome::Done{status:ERR_PAGING_DENIED,..}));
}
#[test] fn cleanup_live_retry_and_unknown_status_keep_all_local_custody(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 for (status,outcome) in [(ERR_PAGING_LIVE,CleanupOutcome::Live),(ERR_PAGING_RETRY,CleanupOutcome::Retry),(77,CleanupOutcome::Fatal(77))] {
 b.cleanup_status.push_back(Err(status));assert_eq!(p.cleanup(&mut b,exited(15)),outcome);assert_eq!(p.used_by(15),1);}
 let other=bell(0);b.answer=Ok(query(other));assert!(matches!(p.step(&mut b,other),Outcome::Committed{index:1,..}));
}
#[test] fn cleanup_resolves_published_uncertain_and_unpublished_reservations(){
 for bitmap in [0,1] {let mut p=multi();let n=notice(0);let mut b=Fake::new(n);b.commit_status.push_back(ERR_PAGING_RETRY);assert_eq!(p.step(&mut b,n),Outcome::Retry);
 b.cleanup_status.push_back(Ok(CleanupResult{frame_bitmap:bitmap,task_id:35}));
 assert!(matches!(p.cleanup(&mut b,exited(15)),CleanupOutcome::Closed{released:1,..}));assert_eq!(p.used(),0);
 let other=bell(0);b.answer=Ok(query(other));assert!(matches!(p.step(&mut b,other),Outcome::Committed{index:0,..}));assert_eq!(b.zeros.len(),2);}
}
#[test] fn cleanup_can_finish_a_reclaiming_binding_and_preserve_other_client(){
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 b.reclaim_status.push_back(ERR_PAGING_RETRY);assert_eq!(p.release(&mut b,release(n,1)),ReleaseOutcome::Retry);
 b.cleanup_status.push_back(Ok(CleanupResult{frame_bitmap:1,task_id:35}));assert!(matches!(p.cleanup(&mut b,exited(15)),CleanupOutcome::Closed{released:1,..}));assert_eq!(p.used(),0);
}
#[test] fn cleanup_refuses_foreign_bitmap_task_identity_and_unknown_target(){
 for result in [CleanupResult{frame_bitmap:2,task_id:35},CleanupResult{frame_bitmap:1,task_id:99},CleanupResult{frame_bitmap:1<<63,task_id:35}] {
 let mut p=multi();let n=notice(0);let mut b=Fake::new(n);assert!(matches!(p.step(&mut b,n),Outcome::Committed{..}));
 b.cleanup_status.push_back(Ok(result));assert_eq!(p.cleanup(&mut b,exited(15)),CleanupOutcome::Fatal(ERR_PAGING_DENIED));assert_eq!(p.used(),1);}
 let mut p=multi();let mut b=Fake::new(notice(0));assert_eq!(p.cleanup(&mut b,exited(11)),CleanupOutcome::Ignored);assert!(b.cleanups.is_empty());
}
#[test] fn death_hint_and_wait_cleanup_decode_authentication_and_widths(){
 assert_eq!(ExitNotice::decode(OP_TASK_EXIT,0,[15,35,0]),Ok(exited(15)));
 for (caller,args) in [(15,[15,35,0]),(0,[1<<32,35,0]),(0,[15,0,0]),(0,[15,35,1])] {assert!(ExitNotice::decode(OP_TASK_EXIT,caller,args).is_err());}
 for (caller,args) in [(0,[15,1,0]),(1<<32,[15,1,0]),(10,[1<<32,1,0]),(10,[10,1,0]),(10,[15,0,0]),(10,[15,MAX_RELEASE_REQUEST_ID+1,0]),(10,[15,1,1])] {
 assert!(WaitCleanupRequest::decode(OP_WAIT_CLEANUP,caller,args).is_err());}
 let mut p=multi();let mut b=Fake::new(notice(0));let request=WaitCleanupRequest{target_pd:15,caller_pd:10,request_id:4};
 assert_eq!(p.wait_cleanup(request),WaitOutcome::Pending);
 assert_eq!(p.wait_cleanup(WaitCleanupRequest{caller_pd:11,..request}),WaitOutcome::Reply(ERR_PAGING_DENIED));
 assert_eq!(p.wait_cleanup(WaitCleanupRequest{target_pd:11,..request}),WaitOutcome::Reply(ERR_PAGING_DENIED));
 b.cleanup_status.push_back(Ok(CleanupResult{frame_bitmap:0,task_id:35}));assert!(matches!(p.cleanup(&mut b,exited(15)),CleanupOutcome::Closed{released:0,..}));
 assert_eq!(p.wait_cleanup(request),WaitOutcome::Reply(PAGING_OK));
}
