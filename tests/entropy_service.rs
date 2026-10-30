// SPDX-License-Identifier: GPL-3.0-only
//! Actual bounded broker + both register ABIs, with explicit counted providers.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../crates/sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../crates/sex-pdx/src/entropy_seed.rs"]pub mod entropy_seed;
#[path="../servers/sexnet/src/entropy.rs"]mod entropy;
use entropy_seed::{self as a,Identity,Request,State};
use entropy_source::Info;
const OWNER:Identity=Identity{pd:15,task:150};
const INFO:Option<Info>=Some(Info{consumer:OWNER,available:true});
fn begin(seq:u32)->Request{Request{seq,kind:a::BEGIN,offset:0,length:32,ticket:0,a:0}}
fn held(seq:u32,kind:u8,ticket:u32)->Request{Request{seq,kind,offset:0,length:0,ticket,a:0}}
fn read(seq:u32,ticket:u32,offset:u16,length:u16)->Request{Request{seq,kind:a::READ,ticket,offset,length,a:0}}
fn reply(m:&mut entropy::Machine,seq:u32)->Result<u32,u32>{let(owner,word)=m.pending_reply().unwrap();assert!(owner==OWNER);let value=a::decode_reply(word,seq);m.reply_accepted();value}
fn started()->(entropy::Machine,u32){let mut m=entropy::Machine::new();assert!(m.handle(OWNER,begin(1),1,INFO));let ticket=reply(&mut m,1).unwrap();(m,ticket)}
fn collect(m:&mut entropy::Machine){for n in 1..=4{m.advance(n+1,||Ok(0x0102030405060700+n));}}
fn forbidden()->Result<u64,u64>{panic!("forbidden provider call")}

#[test]fn exact_consumer_source_info_and_canonical_request_deny_without_allocation(){
 for info in [None,Some(Info{consumer:Identity{pd:12,task:120},available:true}),Some(Info{consumer:Identity{pd:15,task:151},available:true}),Some(Info{consumer:Identity{pd:0,task:150},available:true}),Some(Info{consumer:OWNER,available:false})]{
  let mut m=entropy::Machine::new();assert!(m.handle(OWNER,begin(1),1,info));assert!(reply(&mut m,1).is_err());m.advance(2,forbidden);assert!(!m.busy()&&m.private_scrubbed());
 }
 let mut m=entropy::Machine::new();for(seq,r)in[(1,Request{length:31,..begin(1)}),(2,Request{a:1,..begin(2)})]{assert!(m.handle(OWNER,r,1,INFO));assert!(reply(&mut m,seq)==Err(a::INVALID));}assert!(!m.handle(OWNER,begin(0),1,INFO));assert!(!m.handle(OWNER,begin(a::MAX_SEQ+1),1,INFO));m.advance(2,forbidden);
}
#[test]fn one_word_per_turn_four_words_ready_and_zero_is_valid_first_word(){
 let(mut m,_)=started();let mut calls=0;for n in 0..4{m.advance(n+2,||{calls+=1;Ok(n)});assert!(calls==n+1);assert!(m.snapshot().produced==(n as usize+1)*8);}
 assert!(m.snapshot().state==Some(State::Ready));m.advance(7,forbidden);assert!(m.snapshot().produced==32);
}
#[test]fn full_begin_reply_retained_and_duplicate_begin_never_restarts_or_resamples(){
 let mut m=entropy::Machine::new();let r=begin(1);assert!(m.handle(OWNER,r,1,INFO));let old=m.pending_reply();assert!(m.handle(OWNER,r,2,INFO));assert!(!m.handle(OWNER,held(2,a::CANCEL,1),2,INFO));assert!(m.pending_reply()==old);
 let mut calls=0;m.advance(2,||{calls+=1;Ok(7)});assert!(calls==1&&m.snapshot().produced==8);assert!(m.handle(OWNER,r,3,INFO));assert!(m.snapshot().produced==8);m.reply_accepted();assert!(m.handle(OWNER,r,3,INFO));assert!(m.snapshot().produced==8);m.reply_accepted();
}
#[test]fn read_consumes_exact_cursor_once_and_full_duplicate_replays_without_provider(){
 let(mut m,t)=started();collect(&mut m);let r=read(2,t,0,4);assert!(m.handle(OWNER,r,7,INFO));let first=m.pending_reply().unwrap();assert!(m.snapshot().consumed==4);assert!(m.handle(OWNER,r,8,INFO));assert!(m.pending_reply()==Some(first));assert!(!m.handle(OWNER,read(3,t,4,4),8,INFO));assert!(m.snapshot().consumed==4);m.reply_accepted();assert!(m.handle(OWNER,r,9,INFO));assert!(m.pending_reply()==Some(first));assert!(m.snapshot().consumed==4);m.reply_accepted();m.advance(10,forbidden);
}
#[test]fn all_thirty_two_bytes_drain_in_bounded_reads_and_whole_array_is_erased(){
 let(mut m,t)=started();collect(&mut m);for n in 0..8u16{let seq=n as u32+2;assert!(m.handle(OWNER,read(seq,t,n*4,4),10+n as u64,INFO));assert!(reply(&mut m,seq).is_ok());assert!(m.snapshot().consumed==(n as usize+1)*4);}
 // The final exported receipt still needs explicit cleanup despite zero seed cells.
 assert!(!m.private_scrubbed());assert!(m.handle(OWNER,held(10,a::RELEASE,t),20,None));assert!(reply(&mut m,10)==Ok(0));assert!(!m.busy()&&m.private_scrubbed());
}
#[test]fn foreign_task_ticket_stale_and_gapped_cursor_have_no_seed_effect(){
 let(mut m,t)=started();collect(&mut m);let before=m.snapshot();let foreign=Identity{pd:15,task:151};assert!(m.handle(foreign,read(1,t,0,4),u64::MAX,INFO));let word=m.pending_reply().unwrap().1;assert!(a::decode_reply(word,1)==Err(a::DENIED));m.reply_accepted();assert!(m.snapshot()==before);
 for(seq,r,error)in[(2,read(2,t+1,0,4),a::STALE),(3,read(3,t,4,4),a::INVALID),(4,read(4,t,0,0),a::INVALID)]{assert!(m.handle(OWNER,r,0,INFO));assert!(reply(&mut m,seq)==Err(error));assert!(m.snapshot()==before);}
 assert!(m.handle(OWNER,read(3,t,0,4),0,INFO));assert!(reply(&mut m,3)==Err(a::STALE));assert!(m.snapshot()==before);
}
#[test]fn source_error_is_terminal_partial_scrubbed_and_never_automatically_retried(){
 for(error,expected)in[(entropy_source::DENIED,a::DENIED),(entropy_source::UNSUPPORTED,a::OFFLINE),(entropy_source::BUSY,a::BUSY),(entropy_source::IO,a::IO),(123,a::IO)]{
  let(mut m,t)=started();m.advance(2,||Ok(9));let mut calls=0;m.advance(3,||{calls+=1;Err(error)});assert!(calls==1&&m.snapshot().state==Some(State::Failed)&&m.snapshot().failure==expected&&m.private_scrubbed());m.advance(4,forbidden);
  assert!(m.handle(OWNER,read(2,t,0,4),4,INFO));assert!(reply(&mut m,2)==Err(expected));assert!(m.handle(OWNER,held(3,a::RELEASE,t),5,None));assert!(reply(&mut m,3)==Ok(0));
 }
}
#[test]fn consecutive_repeat_including_zero_fails_conservatively_without_resampling(){
 for word in[0,u64::MAX,17]{let(mut m,_)=started();m.advance(2,||Ok(word));let mut calls=0;m.advance(3,||{calls+=1;Ok(word)});assert!(calls==1&&m.snapshot().failure==a::IO&&m.private_scrubbed());m.advance(4,forbidden);}
 let(mut m,_)=started();for(n,word)in[0,1,0,1].into_iter().enumerate(){m.advance(n as u64+2,||Ok(word));}assert!(m.snapshot().state==Some(State::Ready));
}
#[test]fn deadline_and_backward_clock_fail_before_provider_and_scrub_ready_exports(){
 for now in[0,2001,u64::MAX]{let(mut m,_)=started();m.advance(2,||Ok(7));m.advance(now,forbidden);assert!(m.snapshot().failure==a::TIMEOUT&&m.private_scrubbed());}
 let(mut m,t)=started();collect(&mut m);let r=read(2,t,0,4);assert!(m.handle(OWNER,r,7,INFO));assert!(m.pending_reply().unwrap().1>>56==0x7A);m.advance(2001,forbidden);assert!(m.private_scrubbed());assert!(m.handle(OWNER,r,2002,INFO));assert!(a::decode_reply(m.pending_reply().unwrap().1,2)==Err(a::INVALID));m.reply_accepted();
}
#[test]fn expired_or_rollback_duplicate_read_is_redacted_not_secret_replay(){
 for now in[0,2001]{let(mut m,t)=started();collect(&mut m);let r=read(2,t,0,4);assert!(m.handle(OWNER,r,8,INFO));m.reply_accepted();assert!(m.handle(OWNER,r,now,INFO));assert!(a::decode_reply(m.pending_reply().unwrap().1,2)==Err(a::INVALID));assert!(m.private_scrubbed());}
}
#[test]fn cancel_and_release_zero_seed_scratch_and_all_cached_exports(){
 for kind in[a::CANCEL,a::RELEASE]{let(mut m,t)=started();collect(&mut m);let r=read(2,t,0,4);assert!(m.handle(OWNER,r,7,INFO));m.reply_accepted();assert!(m.handle(OWNER,held(3,kind,t),8,None));assert!(m.private_scrubbed());assert!(reply(&mut m,3)==Ok(0));assert!(m.handle(OWNER,r,9,INFO));assert!(reply(&mut m,2)==Err(a::STALE));assert!(m.private_scrubbed());m.advance(10,forbidden);}
 let(mut m,t)=started();m.advance(2,||Ok(8));assert!(m.handle(OWNER,held(2,a::CANCEL,t),3,None));assert!(m.private_scrubbed());
}
#[test]fn exact_retire_zeroes_pending_secret_and_preserves_foreign_receipts(){
 let(mut m,t)=started();collect(&mut m);assert!(m.handle(OWNER,read(2,t,0,4),8,INFO));let foreign=Identity{pd:12,task:120};assert!(m.handle(foreign,begin(1),8,INFO));assert!(!m.retire(Identity{pd:15,task:151}));assert!(m.busy());assert!(m.retire(OWNER));assert!(!m.busy()&&m.private_scrubbed());assert!(m.pending_reply().unwrap().0==foreign);m.reply_accepted();assert!(m.pending_reply().is_none());
}
#[test]fn six_owner_full_cache_does_not_block_held_cleanup_or_erase_owner_fence(){
 let(mut m,t)=started();collect(&mut m);assert!(m.handle(OWNER,read(2,t,0,4),7,INFO));m.reply_accepted();
 for n in 1..=5{let foreign=Identity{pd:30+n,task:300+n};assert!(m.handle(foreign,begin(1),8,INFO));m.reply_accepted();}
 let before=m.snapshot();assert!(!m.handle(Identity{pd:99,task:990},begin(1),8,INFO));assert!(m.snapshot()==before);
 assert!(m.handle(OWNER,held(3,a::CANCEL,t),9,None));assert!(reply(&mut m,3)==Ok(0));assert!(m.private_scrubbed());assert!(m.handle(OWNER,held(4,a::RELEASE,t),10,None));assert!(reply(&mut m,4)==Ok(0));
 assert!(m.handle(OWNER,begin(1),11,INFO));assert!(reply(&mut m,1)==Err(a::STALE));assert!(!m.busy());
}
#[test]fn retained_reply_round_robin_gives_other_owner_a_turn_without_overwrite(){
 let mut m=entropy::Machine::new();assert!(m.handle(OWNER,begin(1),1,INFO));let first=m.pending_reply();let foreign=Identity{pd:12,task:120};assert!(m.handle(foreign,begin(1),1,INFO));m.reply_deferred();assert!(m.pending_reply().unwrap().0==foreign);m.reply_accepted();assert!(m.pending_reply()==first);m.reply_accepted();
}
