#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
pub const ERR_SERVICE_NOT_READY:u64=2;
#[path="../crates/sex-pdx/src/spindle_probe.rs"]pub mod spindle_probe;
#[path="../servers/sexfiles/src/spindle_probe.rs"]mod service;
#[path="../apps/spindle/src/storage_probe.rs"]mod client;
use spindle_probe as w;use client::{Client,Job,Outcome};use service::{Backend,Service};
#[derive(Default)]struct Fake{calls:usize}
impl Backend for Fake{fn invoke(&mut self,caller:u32,r:w::Request)->Result<u32,u32>{assert!(matches!(caller,12|17));self.calls+=1;Ok(100+r.object as u32)}}
fn req(seq:u32,id:u8)->w::Request{w::Request{seq,kind:w::STAT,object:id,offset:0,a:0,b:0}}
fn rpc(s:&mut Service,b:&mut impl Backend,pd:u32,r:w::Request)->u64{if let Some(v)=s.handle(pd,true,r,b){return v}let mut result=None;s.poll(|p,v|{if p==pd{result=Some(v);0}else{1}});result.unwrap()}
#[test]fn correlated_stat_returns_without_inline_receive(){let mut c=Client::new();let mut s=Service::new();let mut b=Fake::default();assert!(c.submit(Job::Stat(3)));let mut r=None;assert!(c.step(0,|v|{r=Some(v);Ok(())}).is_none());assert!(c.busy());let v=rpc(&mut s,&mut b,12,r.unwrap());assert!(c.receive(2,v).is_none());assert!(c.receive(11,w::reply(r.unwrap().seq+1,Ok(3))).is_none());let e=c.receive(11,v).unwrap();assert_eq!(e.outcome,Outcome::Done);assert_eq!(e.value,103);assert!(!c.busy());}
#[test]fn full_retains_exact_request_and_deadline_quarantines_late_reply(){let mut c=Client::new();c.submit(Job::Stat(0));let mut old=None;c.step(0,|r|{old=Some(r);Err(ERR_SERVICE_NOT_READY)});c.step(0,|r|{assert_eq!(Some(r),old);Ok(())});let e=c.step(100_000,|_|panic!()).unwrap();assert_eq!(e.outcome,Outcome::Uncertain);assert!(!c.busy());assert!(c.blocked());assert!(c.receive(11,w::reply(old.unwrap().seq,Ok(99))).is_none());assert!(!c.submit(Job::Stat(1)));}
#[test]fn frozen_clock_is_bounded_and_close_fences_work(){let mut c=Client::new();c.submit(Job::Stat(0));let mut event=None;for _ in 0..=client::STALL{event=c.step(0,|_|Err(ERR_SERVICE_NOT_READY));if event.is_some(){break}}assert_eq!(event.unwrap().outcome,Outcome::Uncertain);let mut c=Client::new();c.submit(Job::Stat(0));c.close();assert!(c.step(0,|_|panic!()).is_none());assert!(!c.submit(Job::Stat(0)));}
#[test]fn exact_receipt_replay_pending_isolation_and_retirement(){let mut s=Service::new();let mut b=Fake::default();let r=req(1,2);s.handle(12,true,r,&mut b);s.poll(|_,_|1);assert!(s.pending());assert!(s.handle(12,true,req(2,3),&mut b).is_none());assert_eq!(b.calls,1);assert_eq!(w::decode_reply(rpc(&mut s,&mut b,17,r),1),Some(Ok(102)));s.poll(|_,_|0);let v=rpc(&mut s,&mut b,12,r);assert_eq!(w::decode_reply(v,1),Some(Ok(102)));assert_eq!(b.calls,2);s.retire(12);assert_eq!(w::decode_reply(rpc(&mut s,&mut b,12,req(3,0)),3),Some(Err(w::DENIED)));assert_eq!(b.calls,2);}
#[test]fn history_or_raw_reply_never_matches_probe_and_binary_sign_bit_is_data(){assert_eq!(w::decode_reply(w::reply(1,Ok(u32::MAX)),1),Some(Ok(u32::MAX)));assert_eq!(w::decode_reply(0x5300000100000001,1),None);assert_eq!(w::decode_reply(99,1),None);assert!(w::Request::decode(0,0,0).is_none());}

struct Listing{generation:u32,changed:bool}
impl Backend for Listing{fn invoke(&mut self,_:u32,r:w::Request)->Result<u32,u32>{Ok(match r.kind{w::GENERATION=>{let g=self.generation;if self.changed{self.generation+=1;}g},w::INFO=>(r.object==3)as u32,w::NAME=>{let mut b=[0;24];b[..6].copy_from_slice(b"report");u32::from_le_bytes(b[r.offset as usize*4..r.offset as usize*4+4].try_into().unwrap())},_=>0})}}
fn drive(c:&mut Client,s:&mut Service,b:&mut impl Backend)->client::Event{for _ in 0..500{let mut r=None;if let Some(e)=c.step(0,|v|{r=Some(v);Ok(())}){return e}if let Some(r)=r{let v=rpc(s,b,12,r);if let Some(e)=c.receive(11,v){return e}}}panic!("not complete")}
#[test]fn coherent_listing_publishes_only_after_generation_recheck(){let mut c=Client::new();c.submit(Job::List);assert_eq!(c.entry(3),None);let mut s=Service::new();let e=drive(&mut c,&mut s,&mut Listing{generation:4,changed:false});assert_eq!(e.outcome,Outcome::Done);assert_eq!(e.value,1);assert_eq!(c.entry(3),Some(&b"report"[..]));assert_eq!(c.entry(4),None);}
#[test]fn mixed_generation_listing_discards_all_staged_names(){let mut c=Client::new();c.submit(Job::List);let e=drive(&mut c,&mut Service::new(),&mut Listing{generation:4,changed:true});assert_eq!(e.outcome,Outcome::Failed);assert_eq!(e.error,w::STALE);assert_eq!(c.entry(3),None);}
#[derive(Default)]struct Memory{bytes:Vec<u8>,generation:u32,sync_fail:bool,writes:usize}
impl Backend for Memory{fn invoke(&mut self,_:u32,r:w::Request)->Result<u32,u32>{Ok(match r.kind{
 w::GENERATION=>self.generation,w::BEGIN|w::STAT=>self.bytes.len()as u32,w::CREATE=>{self.generation+=1;3},w::DELETE|w::RENAME=>{self.generation+=1;0},
 w::WRITE=>{self.writes+=1;let off=r.offset as usize;self.bytes.resize(self.bytes.len().max(off+16),0);self.bytes[off..off+8].copy_from_slice(&r.a.to_le_bytes());self.bytes[off+8..off+16].copy_from_slice(&r.b.to_le_bytes());self.generation+=1;16},
 w::TRUNCATE=>{self.bytes.resize(r.a as usize,0);self.generation+=1;r.a as u32},
 w::READ=>{let mut b=[0;4];let off=r.offset as usize;let n=r.a as usize;b[..n].copy_from_slice(&self.bytes[off..off+n]);u32::from_le_bytes(b)},
 w::SYNC=>if self.sync_fail{return Err(w::IO)}else{0},_=>return Err(w::INVALID)})}}
#[test]fn bounded_binary_fill_verify_and_exact_short_tail(){for n in[0,1,15,16,17,512]{let mut b=Memory::default();let mut s=Service::new();let mut c=Client::new();assert!(c.submit(Job::Fill(3,n,true)));assert_eq!(drive(&mut c,&mut s,&mut b).outcome,Outcome::Done);assert_eq!(b.bytes.len(),n as usize);for(i,&v)in b.bytes.iter().enumerate(){assert_eq!(v,((i*37+11)&255)as u8);}c.submit(Job::Verify(3,true));let e=drive(&mut c,&mut s,&mut b);assert_eq!(e.outcome,Outcome::Done);assert_eq!(e.value,n);}}
#[test]fn mismatch_and_sync_failure_never_report_completion_and_release_session(){let mut b=Memory{bytes:vec![255;17],..Memory::default()};let mut s=Service::new();let mut c=Client::new();c.submit(Job::Verify(3,true));assert_eq!(drive(&mut c,&mut s,&mut b).error,w::MISMATCH);b.sync_fail=true;c.submit(Job::Fill(3,1,true));let e=drive(&mut c,&mut s,&mut b);assert_eq!(e.outcome,Outcome::Failed);assert_eq!(e.error,w::IO);b.sync_fail=false;c.submit(Job::Fill(3,1,true));assert_eq!(drive(&mut c,&mut s,&mut b).outcome,Outcome::Done);}
#[test]fn generation_change_fences_selected_object_before_a_write(){let mut b=Memory::default();let mut s=Service::new();let begin=w::Request{kind:w::BEGIN,object:3,..req(1,3)};assert_eq!(w::decode_reply(rpc(&mut s,&mut b,12,begin),1),Some(Ok(0)));b.generation+=1;let write=w::Request{seq:2,kind:w::WRITE,a:1,..begin};assert_eq!(w::decode_reply(rpc(&mut s,&mut b,12,write),2),Some(Err(w::STALE)));assert_eq!(b.writes,0);}
#[test]fn duplicate_mutation_is_not_executed_again_and_bounds_reject_system_writes(){let mut b=Memory::default();let mut s=Service::new();let begin=w::Request{kind:w::BEGIN,object:3,..req(1,3)};rpc(&mut s,&mut b,12,begin);let write=w::Request{seq:2,kind:w::WRITE,a:u64::MAX,..begin};let v=rpc(&mut s,&mut b,12,write);assert_eq!(rpc(&mut s,&mut b,12,write),v);assert_eq!(b.writes,1);let mut c=Client::new();assert!(!c.submit(Job::Fill(0,16,true)));assert!(!c.submit(Job::Fill(3,65_537,true)));assert!(!c.submit(Job::Create([0;16])));}
#[test]fn queued_probe_gets_next_job_boundary_and_uncertainty_fences_other_work(){assert_eq!(client::choose_owner(true,false,true,false),2);assert_eq!(client::choose_owner(true,false,false,false),1);assert_eq!(client::choose_owner(false,true,true,false),0);assert_eq!(client::choose_owner(true,false,false,true),0);}
#[test]fn cancellation_supersedes_pending_begin_and_quarantines_its_late_reply(){
 let mut c=Client::new();let mut s=Service::new();let mut b=Memory::default();c.submit(Job::Verify(3,true));let mut begin=None;c.step(0,|r|{begin=Some(r);Ok(())});s.handle(12,true,begin.unwrap(),&mut b);let mut old=0;s.poll(|_,v|{old=v;1});
 assert_eq!(c.step(100_000,|_|panic!()).unwrap().outcome,Outcome::Uncertain);assert!(!c.retry());assert!(c.cancel());let mut abort=None;c.step(0,|r|{abort=Some(r);Ok(())});assert!(c.receive(11,old).is_none());let v=rpc(&mut s,&mut b,12,abort.unwrap());assert_eq!(c.receive(11,v).unwrap().outcome,Outcome::Cancelled);assert!(c.retry());assert_eq!(drive(&mut c,&mut s,&mut b).outcome,Outcome::Done);
}
#[test]fn stale_abort_cannot_retire_new_binding_or_the_other_subject(){
 let mut s=Service::new();let mut b=Memory{bytes:vec![11],..Memory::default()};let begin=w::Request{kind:w::BEGIN,object:3,..req(1,3)};rpc(&mut s,&mut b,12,begin);rpc(&mut s,&mut b,17,w::Request{object:4,..begin});let abort=w::Request{seq:2,kind:w::ABORT,object:0,..begin};rpc(&mut s,&mut b,12,abort);rpc(&mut s,&mut b,12,w::Request{seq:3,..begin});assert_eq!(w::decode_reply(rpc(&mut s,&mut b,12,abort),2),Some(Err(w::STALE)));let read=w::Request{seq:4,kind:w::READ,a:1,..begin};assert_eq!(w::decode_reply(rpc(&mut s,&mut b,12,read),4),Some(Ok(11)));assert_eq!(w::decode_reply(rpc(&mut s,&mut b,17,w::Request{seq:2,object:4,..read}),2),Some(Ok(11)));
}
#[test]fn mutation_is_never_automatically_replayed_after_quiescence(){
 let mut c=Client::new();let mut s=Service::new();let mut b=Memory::default();c.submit(Job::Fill(3,16,true));let mut r=None;c.step(0,|v|{r=Some(v);Ok(())});let old=rpc(&mut s,&mut b,12,r.unwrap());c.cancel();assert!(c.receive(11,old).is_none());assert_eq!(drive(&mut c,&mut s,&mut b).outcome,Outcome::Cancelled);assert!(!c.retry());assert_eq!(b.writes,0);assert!(c.submit(Job::Stat(3)));
}
#[test]fn denied_abort_stays_uncertain_and_cannot_unlock_new_work(){
 let mut c=Client::new();c.cancel();let mut r=None;c.step(0,|v|{r=Some(v);Ok(())});let e=c.receive(11,w::reply(r.unwrap().seq,Err(w::DENIED))).unwrap();assert_eq!(e.outcome,Outcome::Uncertain);assert!(c.fenced());assert!(!c.submit(Job::Stat(0)));assert!(!c.retry());
}
mod storage_probe{pub use crate::client::Job;}
include!(env!("SPINDLE_PROBE_PARSE_SOURCE"));
#[test]fn parser_rejects_overflow_extra_words_system_mutations_and_oversized_jobs(){
 assert_eq!(parse_probe_job(b"filldocx",b"3 65536"),Some(Job::Fill(3,65536,true)));
 for (cmd,args)in [(b"filldocx".as_slice(),b"3 999999999999999999999999999999999".as_slice()),(b"filldoc",b"0 16"),(b"filldoc",b"3 65537"),(b"truncdoc",b"3 1 extra"),(b"mkdoc",b"12345678901234567"),(b"rmdoc",b"300")]{assert_eq!(parse_probe_job(cmd,args),None);}
}
#[test]fn summaries_show_exact_objects_and_never_call_uncertain_writes_flushed(){
 let mut e=client::Event{job:Job::Fill(3,17,true),outcome:Outcome::Done,value:17,error:0,hash:0};let line=client::summary(&e);assert_eq!(&line.bytes[..line.len],b"OBJECT 3 / 17 BYTES FLUSHED");
 for outcome in [Outcome::Failed,Outcome::Uncertain,Outcome::Cancelled]{e.outcome=outcome;let line=client::summary(&e);assert!(line.len<=40);assert!(!line.bytes[..line.len].windows(7).any(|w|w==b"FLUSHED"));}
}
#[test]fn stale_and_duplicate_receipts_survive_full_without_overwriting_fresh_replay(){
 let mut s=Service::new();let mut b=Fake::default();let fresh=req(3,3);let v=rpc(&mut s,&mut b,12,fresh);
 assert!(s.handle(12,true,req(1,0),&mut b).is_none());s.poll(|_,got|{assert_eq!(w::decode_reply(got,1),Some(Err(w::STALE)));1});assert!(s.pending());s.poll(|_,_|0);
 assert!(s.handle(12,true,fresh,&mut b).is_none());s.poll(|_,got|{assert_eq!(got,v);1});assert!(s.pending());s.poll(|_,got|{assert_eq!(got,v);0});assert_eq!(b.calls,1);
}
