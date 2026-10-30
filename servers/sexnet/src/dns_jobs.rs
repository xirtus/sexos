// SPDX-License-Identifier: GPL-3.0-only
//! Fixed-name DNS custody. Parent authenticates the exact live BOOT identity.
use crate::jobs::Identity;
use sex_pdx::network_dns::{self as a,Request,State};
use sex_net::dns_wire;
// get_ticks is a raw timer counter. Calibrated LAPIC cadence is nominally 1ms;
// fallback timing is approximate. These are bounded nominal lifetimes, not a
// wall-clock or DNS-authenticity guarantee. Provenance never grants authority.
const NOMINAL_TICKS_PER_SECOND:u64=1000;
const MAX_TTL_SECONDS:u32=60;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Resolved {
 pub owner:Identity,pub generation:u32,pub address:[u8;4],
 pub observed_tick:u64,pub expires_tick:u64,
}
pub trait Wire{fn begin(&mut self,ticket:u32)->Result<(),u32>;fn receive(&mut self,out:&mut[u8;512])->Result<Option<usize>,u32>;fn abort(&mut self);}
#[derive(Clone,Copy)]struct Receipt{owner:Identity,request:Request,word:u64}
pub struct Machine{owner:Option<Identity>,ticket:u32,next_ticket:u32,state:State,deadline:u64,last:u64,
 answer:Option<dns_wire::Answer>,candidate:Option<Resolved>,records:[Option<Resolved>;6],records_clock:u64,
 failure:u32,receive:[u8;512],pending:[Option<Receipt>;6],cached:[Option<Receipt>;6],cursor:usize}
impl Machine{
 pub const fn new()->Self{Self{owner:None,ticket:0,next_ticket:1,state:State::Idle,deadline:0,last:0,answer:None,candidate:None,records:[None;6],records_clock:0,failure:0,receive:[0;512],pending:[None;6],cached:[None;6],cursor:0}}
 pub fn owner(&self)->Option<Identity>{self.owner}
 pub fn state(&self)->State{self.state}
 pub fn ticket(&self)->u32{self.ticket}
 /// Only successfully released, exact-owner provenance is visible. The parent
 /// attests current BOOT liveness and applies endpoint policy independently.
 pub fn resolved(&self,owner:Identity,generation:u32,now:u64)->Option<Resolved>{
  self.records.iter().flatten().copied().find(|record|record.owner==owner
   &&record.generation==generation&&self.valid_time(record,now))
 }
 /// Count persistent released records; the active candidate is not eligible.
 pub fn owned_records(&self,owner:Identity)->usize{
  self.records.iter().flatten().filter(|record|record.owner==owner).count()
 }
 fn valid_time(&self,record:&Resolved,now:u64)->bool{
  now>=self.records_clock&&now>=record.observed_tick&&now<record.expires_tick
 }
 /// At most six liveness queries; no replay slot eviction. A regression revokes
 /// all retained provenance and cannot resurrect it when the clock recovers.
 /// The parent invokes this on service turns, including while no job is held.
 pub fn maintain_records(&mut self,now:u64,mut live:impl FnMut(Identity)->bool){
  if now<self.records_clock{self.records.fill(None);self.candidate=None;return;}
  self.records_clock=now;
  if self.candidate.is_some_and(|r|now<r.observed_tick||now>=r.expires_tick){self.candidate=None;}
  for i in 0..6{
   if self.records[i].is_some_and(|r|now<r.observed_tick||now>=r.expires_tick){self.records[i]=None;}
   // A fresh BEGIN cleared this owner's released record. Candidate and released
   // record therefore consume at most one liveness query per replay-fence slot.
   let candidate_here=self.candidate.is_some_and(|r|self.cached[i].map(|c|c.owner)==Some(r.owner));
   let owner=self.records[i].map(|r|r.owner).or_else(||if candidate_here{self.candidate.map(|r|r.owner)}else{None});
   if owner.is_some_and(|owner|!live(owner)){
    self.records[i]=None;if candidate_here{self.candidate=None;}
   }
  }
 }
 fn observed(&self,answer:dns_wire::Answer,now:u64)->Option<Resolved>{
  if answer.ttl==0||now<self.records_clock{return None;}
  let lifetime=u64::from(answer.ttl.min(MAX_TTL_SECONDS)).checked_mul(NOMINAL_TICKS_PER_SECOND)?;
  Some(Resolved{owner:self.owner?,generation:self.ticket,address:answer.address,
   observed_tick:now,expires_tick:now.checked_add(lifetime)?})
 }
 pub fn can_transmit(&self,ticket:u32)->bool{self.owner.is_some()&&self.ticket==ticket&&self.state==State::Waiting}
 fn slot(&self)->Option<usize>{(0..6).map(|n|(self.cursor+n)%6).find(|i|self.pending[*i].is_some())}
 pub fn pending_reply(&self)->Option<(Identity,u64)>{self.slot().and_then(|i|self.pending[i]).map(|r|(r.owner,r.word))}
 pub fn reply_accepted(&mut self){if let Some(i)=self.slot(){self.pending[i]=None;self.cursor=(i+1)%6;}}
 pub fn reply_deferred(&mut self){if let Some(i)=self.slot(){self.cursor=(i+1)%6;}}
 fn scrub(&mut self,wire:&mut impl Wire){self.receive.fill(0);self.answer=None;self.candidate=None;self.deadline=0;self.last=0;wire.abort();}
 fn fail(&mut self,why:u32,wire:&mut impl Wire){self.scrub(wire);self.failure=why;self.state=State::Failed;}
 pub fn retire(&mut self,owner:Identity,wire:&mut impl Wire)->bool{
  let held=self.owner==Some(owner);if held{self.scrub(wire);self.owner=None;self.ticket=0;self.failure=0;self.state=State::Idle;}
  if self.candidate.is_some_and(|r|r.owner==owner){self.candidate=None;}
  for i in 0..6{
   if self.records[i].is_some_and(|r|r.owner==owner){self.records[i]=None;}
   if self.cached[i].map(|r|r.owner)==Some(owner){self.cached[i]=None;self.pending[i]=None;}
  }held
 }
 pub fn handle(&mut self,owner:Identity,r:Request,now:u64,other_held:bool,wire:&mut impl Wire)->bool{
  let Some(i)=self.cached.iter().position(|c|c.map(|c|c.owner)==Some(owner)).or_else(||self.cached.iter().position(Option::is_none))else{return false;};
  if let Some(p)=self.pending[i]{return p.request==r;}
  if let Some(c)=self.cached[i]{if c.request==r{self.pending[i]=Some(c);return true;}if r.seq<=c.request.seq{return self.respond(i,owner,r,Err(a::STALE));}}
  let result=self.apply(owner,r,now,other_held,wire);self.respond(i,owner,r,result)
 }
 fn respond(&mut self,i:usize,owner:Identity,r:Request,result:Result<u32,u32>)->bool{
  let receipt=Receipt{owner,request:r,word:match result{Ok(v)=>a::reply(r.seq,v),Err(e)=>a::error(r.seq,e)}};
  self.pending[i]=Some(receipt);if self.cached[i].is_none_or(|c|r.seq>c.request.seq){self.cached[i]=Some(receipt);}true
 }
 fn apply(&mut self,owner:Identity,r:Request,now:u64,other_held:bool,wire:&mut impl Wire)->Result<u32,u32>{
  if owner.pd==0||owner.task==0||Request::decode(r.encode())!=Ok(r){return Err(a::INVALID);}
  if r.kind==a::BEGIN{
   if self.owner.is_some()||other_held{return Err(a::BUSY);}
   if self.next_ticket>a::MAX_TICKET{return Err(a::EXHAUSTED);}
   let deadline=now.checked_add(5000).ok_or(a::TIMEOUT)?;
   // Only a fresh admitted BEGIN invalidates its owner's preceding record.
   // Pending/duplicate/stale admission already returned above without effects.
   for i in 0..6{if self.cached[i].map(|c|c.owner)==Some(owner){self.records[i]=None;}}
   self.candidate=None;
   self.ticket=self.next_ticket;self.next_ticket+=1;self.owner=Some(owner);self.state=State::Waiting;
   self.deadline=deadline;self.last=now;self.answer=None;self.failure=0;self.receive.fill(0);
   // BEGIN always acknowledges its allocated ticket, even if transport fails;
   // STATUS/RESULT expose failure and the caller can positively RELEASE it.
   if let Err(e)=wire.begin(self.ticket){self.fail(e,wire);}return Ok(self.ticket);
  }
  if self.owner!=Some(owner){return Err(a::DENIED);}if self.ticket!=r.ticket{return Err(a::STALE);}
  match r.kind{
   a::STATUS=>Ok(self.state as u32),
   a::RESULT=>match self.answer{Some(answer)if self.state==State::Complete=>Ok(u32::from_be_bytes(answer.address)),_=>Err(if self.state==State::Failed{self.failure}else{a::BUSY})},
   a::CANCEL=>{self.scrub(wire);self.failure=0;self.state=State::Cancelled;Ok(0)},
   a::RELEASE=>{
    let promoted=self.candidate.filter(|r|self.state==State::Complete&&r.owner==owner
     &&r.generation==self.ticket&&self.valid_time(r,now));
    if let Some(i)=self.cached.iter().position(|c|c.map(|c|c.owner)==Some(owner)){self.records[i]=promoted;}
    self.records_clock=self.records_clock.max(now);
    self.scrub(wire);self.owner=None;self.ticket=0;self.failure=0;self.state=State::Idle;Ok(0)
   },
   _=>Err(a::INVALID),
  }
 }
 /// One bounded datagram per turn; only a fully validated response commits data.
 pub fn advance(&mut self,now:u64,wire:&mut impl Wire){
  if self.state!=State::Waiting{return;}
  if now<self.last||now>=self.deadline{self.fail(a::TIMEOUT,wire);return;}self.last=now;
  self.receive.fill(0);let result=wire.receive(&mut self.receive);
  let decoded=match result{Ok(Some(n))if n<=512=>dns_wire::parse(self.ticket as u16,&self.receive[..n]).map_err(|_|a::IO),Ok(None)=>Ok(None),Ok(Some(_))=>Err(a::IO),Err(e)=>Err(e)};
  self.receive.fill(0);
  match decoded{Ok(Some(answer))=>{self.candidate=self.observed(answer,now);self.records_clock=self.records_clock.max(now);self.answer=Some(answer);self.state=State::Complete;self.deadline=0;wire.abort();},Ok(None)=>{},Err(e)=>self.fail(e,wire)}
 }
}

#[cfg(test)]mod tests{
 use super::*;use std::vec::Vec;
 const A:Identity=Identity{pd:15,task:15};const B:Identity=Identity{pd:9,task:9};
 struct Peer{begins:Vec<u32>,aborts:usize,response:Option<Vec<u8>>,error:Option<u32>}
 impl Peer{fn new()->Self{Self{begins:Vec::new(),aborts:0,response:None,error:None}}}
 impl Wire for Peer{
  fn begin(&mut self,t:u32)->Result<(),u32>{self.begins.push(t);self.error.map_or(Ok(()),Err)}
  fn receive(&mut self,out:&mut[u8;512])->Result<Option<usize>,u32>{if let Some(e)=self.error{return Err(e);}if let Some(v)=self.response.take(){let n=v.len();out[..n.min(512)].copy_from_slice(&v[..n.min(512)]);Ok(Some(n))}else{Ok(None)}}
  fn abort(&mut self){self.aborts+=1;self.response=None;}
 }
 fn r(seq:u32,kind:u8,ticket:u32)->Request{Request{seq,kind,ticket}}
 fn call(m:&mut Machine,w:&mut Peer,o:Identity,r:Request)->Result<u32,u32>{assert!(m.handle(o,r,10,false,w));let(o2,v)=m.pending_reply().unwrap();assert_eq!(o,o2);m.reply_accepted();a::decode_reply(v,r.seq)}
 fn response(id:u16)->Vec<u8>{let mut b=[0;512];let n=dns_wire::query(id,&mut b).unwrap();let mut v=b[..n].to_vec();v[2]=0x81;v[3]=0x80;v[7]=1;v.extend_from_slice(&[0xc0,12,0,1,0,1,0,0,0,60,0,4,10,0,2,2]);v}
 fn clean(m:&Machine){assert!(m.receive.iter().all(|b|*b==0));assert!(m.answer.is_none());}
 #[test]fn canonical_registers_and_reply_correlation(){for seq in[1,a::MAX_SEQ]{for(kind,ticket)in[(a::BEGIN,0),(a::STATUS,a::MAX_TICKET),(a::RESULT,1),(a::CANCEL,1),(a::RELEASE,1)]{let q=r(seq,kind,ticket);assert_eq!(Request::decode(q.encode()),Ok(q));assert_eq!(a::decode_reply(a::reply(seq,123),seq),Ok(123));}}for args in[[0,0,0],[1|(a::BEGIN as u64)<<24,1,0],[1|(a::STATUS as u64)<<24,0,0],[1|(a::BEGIN as u64)<<24,0,1],[1|(a::BEGIN as u64)<<24|(1<<40),0,0]]{assert!(Request::decode(args).is_err());}assert_eq!(a::decode_reply(a::reply(2,0),1),Err(a::STALE));}
 #[test]fn complete_exact_query_result_and_positive_release(){let mut m=Machine::new();let mut p=Peer::new();assert_eq!(call(&mut m,&mut p,A,r(1,a::BEGIN,0)),Ok(1));p.response=Some(response(1));m.advance(11,&mut p);assert_eq!(m.state(),State::Complete);assert_eq!(call(&mut m,&mut p,A,r(2,a::RESULT,1)),Ok(0x0a000202));assert!(m.receive.iter().all(|b|*b==0));assert!(!m.can_transmit(1));assert_eq!(call(&mut m,&mut p,A,r(3,a::RELEASE,1)),Ok(0));clean(&m);assert_eq!(m.owner(),None);}
 #[test]fn duplicate_does_not_send_and_old_sequence_cannot_regress_fence(){let mut m=Machine::new();let mut p=Peer::new();let begin=r(10,a::BEGIN,0);assert_eq!(call(&mut m,&mut p,A,begin),Ok(1));assert_eq!(call(&mut m,&mut p,A,begin),Ok(1));assert_eq!(p.begins.len(),1);assert_eq!(call(&mut m,&mut p,A,r(11,a::RELEASE,1)),Ok(0));assert_eq!(call(&mut m,&mut p,A,r(1,a::BEGIN,0)),Err(a::STALE));assert_eq!(call(&mut m,&mut p,B,r(1,a::BEGIN,0)),Ok(2));assert_eq!(call(&mut m,&mut p,B,r(2,a::RELEASE,2)),Ok(0));assert_eq!(call(&mut m,&mut p,A,begin),Err(a::STALE));assert_eq!(p.begins.len(),2);}
 #[test]fn foreign_owner_task_and_ticket_have_no_effect(){let mut m=Machine::new();let mut p=Peer::new();call(&mut m,&mut p,A,r(1,a::BEGIN,0)).unwrap();for o in[B,Identity{pd:15,task:99}]{assert_eq!(call(&mut m,&mut p,o,r(1,a::CANCEL,1)),Err(a::DENIED));}assert_eq!(call(&mut m,&mut p,A,r(2,a::CANCEL,2)),Err(a::STALE));assert_eq!(m.owner(),Some(A));assert_eq!(p.aborts,0);}
 #[test]fn timeout_backward_clock_malformed_and_transport_fail_scrub(){for which in 0..4{let mut m=Machine::new();let mut p=Peer::new();call(&mut m,&mut p,A,r(1,a::BEGIN,0)).unwrap();let now=match which{0=>5010,1=>9,2=>{p.response=Some(vec![0;4]);11},_=>{p.error=Some(a::IO);11}};m.advance(now,&mut p);assert_eq!(m.state(),State::Failed);clean(&m);assert!(!m.can_transmit(1));assert_eq!(call(&mut m,&mut p,A,r(2,a::RESULT,1)),Err(if which<2{a::TIMEOUT}else{a::IO}));}}
 #[test]fn unrelated_response_ignored_then_matching_answer_accepted(){let mut m=Machine::new();let mut p=Peer::new();call(&mut m,&mut p,A,r(1,a::BEGIN,0)).unwrap();p.response=Some(response(2));m.advance(11,&mut p);assert_eq!(m.state(),State::Waiting);p.response=Some(response(1));m.advance(12,&mut p);assert_eq!(m.state(),State::Complete);}
 #[test]fn cancellation_retirement_and_late_old_response_cannot_resurrect(){let mut m=Machine::new();let mut p=Peer::new();call(&mut m,&mut p,A,r(1,a::BEGIN,0)).unwrap();call(&mut m,&mut p,A,r(2,a::CANCEL,1)).unwrap();p.response=Some(response(1));m.advance(11,&mut p);assert_eq!(m.state(),State::Cancelled);clean(&m);call(&mut m,&mut p,A,r(3,a::RELEASE,1)).unwrap();assert_eq!(call(&mut m,&mut p,A,r(4,a::BEGIN,0)),Ok(2));p.response=Some(response(1));m.advance(11,&mut p);assert_eq!(m.state(),State::Waiting);assert!(!m.retire(Identity{pd:15,task:99},&mut p));assert!(m.retire(A,&mut p));assert_eq!(m.pending_reply(),None);assert!(m.cached.iter().all(Option::is_none));clean(&m);}
 #[test]fn full_receipt_retained_and_other_owner_served_fairly(){let mut m=Machine::new();let mut p=Peer::new();assert!(m.handle(A,r(1,a::BEGIN,0),10,false,&mut p));let receipt=m.pending_reply();assert!(!m.handle(A,r(2,a::CANCEL,1),10,false,&mut p));assert!(m.handle(B,r(1,a::BEGIN,0),10,false,&mut p));assert_eq!(m.pending_reply(),receipt);m.reply_deferred();assert_eq!(m.pending_reply(),Some((B,a::error(1,a::BUSY))));m.reply_accepted();assert_eq!(m.pending_reply(),receipt);assert_eq!(p.begins.len(),1);}
 #[test]fn other_protocol_hold_and_overflow_reject_before_effects(){let mut m=Machine::new();let mut p=Peer::new();assert!(m.handle(A,r(1,a::BEGIN,0),10,true,&mut p));assert_eq!(m.pending_reply(),Some((A,a::error(1,a::BUSY))));m.reply_accepted();assert!(m.handle(A,r(2,a::BEGIN,0),u64::MAX,false,&mut p));assert_eq!(m.pending_reply(),Some((A,a::error(2,a::TIMEOUT))));assert!(p.begins.is_empty());assert_eq!(m.owner(),None);}
 #[test]fn failed_begin_still_yields_ticket_for_release(){let mut m=Machine::new();let mut p=Peer::new();p.error=Some(a::OFFLINE);assert_eq!(call(&mut m,&mut p,A,r(1,a::BEGIN,0)),Ok(1));assert_eq!(m.state(),State::Failed);assert_eq!(m.owner(),Some(A));assert_eq!(call(&mut m,&mut p,A,r(2,a::RESULT,1)),Err(a::OFFLINE));assert_eq!(call(&mut m,&mut p,A,r(3,a::RELEASE,1)),Ok(0));clean(&m);}
 #[test]fn finite_tickets_and_replay_slots_never_evict(){let mut m=Machine::new();let mut p=Peer::new();m.next_ticket=a::MAX_TICKET;assert_eq!(call(&mut m,&mut p,A,r(1,a::BEGIN,0)),Ok(a::MAX_TICKET));call(&mut m,&mut p,A,r(2,a::RELEASE,a::MAX_TICKET)).unwrap();assert_eq!(call(&mut m,&mut p,A,r(3,a::BEGIN,0)),Err(a::EXHAUSTED));for n in 1..6{let o=Identity{pd:100+n,task:100+n};assert!(m.handle(o,r(1,a::BEGIN,0),10,true,&mut p));m.reply_accepted();}let before=p.begins.len();assert!(!m.handle(B,r(1,a::BEGIN,0),10,false,&mut p));assert_eq!(p.begins.len(),before);assert!(!m.retire(Identity{pd:999,task:999},&mut p));}
 fn call_at(m:&mut Machine,w:&mut Peer,o:Identity,r:Request,now:u64)->Result<u32,u32>{
  assert!(m.handle(o,r,now,false,w));let(o2,word)=m.pending_reply().unwrap();assert_eq!(o2,o);
  m.reply_accepted();a::decode_reply(word,r.seq)
 }
 fn answer_ttl(id:u16,ttl:u32)->Vec<u8>{let mut bytes=response(id);bytes[40..44].copy_from_slice(&ttl.to_be_bytes());bytes}
 fn publish(m:&mut Machine,w:&mut Peer,o:Identity,seq:u32,now:u64,ttl:u32)->u32{
  let ticket=call_at(m,w,o,r(seq,a::BEGIN,0),now).unwrap();
  w.response=Some(answer_ttl(ticket as u16,ttl));m.advance(now+1,w);assert_eq!(m.state(),State::Complete);
  assert_eq!(call_at(m,w,o,r(seq+1,a::RELEASE,ticket),now+2),Ok(0));ticket
 }
 #[test]fn provenance_is_copied_exact_owner_data_visible_only_after_release_processing(){
  let mut m=Machine::new();let mut p=Peer::new();let ticket=call_at(&mut m,&mut p,A,r(1,a::BEGIN,0),100).unwrap();
  p.response=Some(answer_ttl(ticket as u16,60));m.advance(101,&mut p);
  assert!(m.candidate.is_some());assert_eq!(m.owned_records(A),0);assert_eq!(m.resolved(A,ticket,101),None);
  assert!(m.handle(A,r(2,a::RELEASE,ticket),102,false,&mut p));
  let expected=Resolved{owner:A,generation:ticket,address:[10,0,2,2],observed_tick:101,expires_tick:60_101};
  assert_eq!(m.resolved(A,ticket,102),Some(expected));assert_eq!(m.owned_records(A),1);assert_eq!(m.owner(),None);
  assert_eq!(m.resolved(B,ticket,102),None);assert_eq!(m.resolved(Identity{pd:A.pd,task:A.task+1},ticket,102),None);
  assert_eq!(m.resolved(A,ticket+1,102),None);assert_eq!(m.resolved(A,0,102),None);
  assert_eq!(m.resolved(A,ticket,100),None);assert_eq!(m.resolved(A,ticket,101),None);assert!(m.candidate.is_none());clean(&m);
 }
 #[test]fn ttl_is_reception_based_capped_and_expiry_has_no_release_replay_resurrection(){
  let mut m=Machine::new();let mut p=Peer::new();let ticket=publish(&mut m,&mut p,A,1,100,1);
  assert_eq!(m.resolved(A,ticket,1100).unwrap().expires_tick,1101);assert_eq!(m.resolved(A,ticket,1101),None);
  m.maintain_records(1101,|_|true);assert_eq!(m.owned_records(A),0);
  assert_eq!(call_at(&mut m,&mut p,A,r(2,a::RELEASE,ticket),1102),Ok(0));assert_eq!(m.owned_records(A),0);
  let mut m=Machine::new();let mut p=Peer::new();let ticket=publish(&mut m,&mut p,A,1,100,0x7fff_ffff);
  assert_eq!(m.resolved(A,ticket,102).unwrap().expires_tick,60_101);
  let mut m=Machine::new();let mut p=Peer::new();let ticket=call_at(&mut m,&mut p,A,r(1,a::BEGIN,0),100).unwrap();
  p.response=Some(answer_ttl(ticket as u16,1));m.advance(101,&mut p);
  assert_eq!(call_at(&mut m,&mut p,A,r(2,a::RELEASE,ticket),1101),Ok(0));assert_eq!(m.owner(),None);assert_eq!(m.owned_records(A),0);
 }
 #[test]fn zero_highbit_ttl_and_checked_expiry_overflow_keep_diagnostic_answer_without_provenance(){
  for(now,ttl)in[(100,0),(100,0x8000_0000),(100,u32::MAX),(u64::MAX-10_000,0x7fff_ffff)]{
   let mut m=Machine::new();let mut p=Peer::new();let ticket=call_at(&mut m,&mut p,A,r(1,a::BEGIN,0),now).unwrap();
   p.response=Some(answer_ttl(ticket as u16,ttl));m.advance(now+1,&mut p);
   assert_eq!(m.state(),State::Complete);assert!(m.candidate.is_none());
   assert_eq!(call_at(&mut m,&mut p,A,r(2,a::RESULT,ticket),now+2),Ok(0x0a000202));
   assert_eq!(call_at(&mut m,&mut p,A,r(3,a::RELEASE,ticket),now+3),Ok(0));
   assert_eq!(m.owner(),None);assert_eq!(m.owned_records(A),0);assert_eq!(m.resolved(A,ticket,now+3),None);
  }
 }
 #[test]fn only_fresh_admitted_begin_invalidates_preceding_own_record(){
  let mut m=Machine::new();let mut p=Peer::new();let ticket=publish(&mut m,&mut p,A,1,100,60);
  let original=m.resolved(A,ticket,102).unwrap();
  assert!(m.handle(A,r(3,a::BEGIN,0),103,true,&mut p));assert_eq!(m.pending_reply(),Some((A,a::error(3,a::BUSY))));m.reply_accepted();
  assert_eq!(m.resolved(A,ticket,103),Some(original));
  assert_eq!(call_at(&mut m,&mut p,A,r(3,a::BEGIN,0),104),Err(a::BUSY));assert_eq!(p.begins.len(),1);
  assert_eq!(call_at(&mut m,&mut p,A,r(1,a::BEGIN,0),104),Err(a::STALE));
  assert_eq!(call_at(&mut m,&mut p,A,r(4,a::BEGIN,1),104),Err(a::INVALID));
  m.next_ticket=a::MAX_TICKET+1;assert_eq!(call_at(&mut m,&mut p,A,r(5,a::BEGIN,0),104),Err(a::EXHAUSTED));m.next_ticket=2;
  assert_eq!(call_at(&mut m,&mut p,A,r(6,a::BEGIN,0),u64::MAX),Err(a::TIMEOUT));
  assert_eq!(m.resolved(A,ticket,104),Some(original));
  assert!(m.handle(A,r(7,a::STATUS,ticket),105,false,&mut p));let pending=m.pending_reply();
  assert!(!m.handle(A,r(8,a::BEGIN,0),105,false,&mut p));assert_eq!(m.pending_reply(),pending);assert_eq!(m.resolved(A,ticket,105),Some(original));m.reply_accepted();
  assert_eq!(call_at(&mut m,&mut p,A,r(8,a::BEGIN,0),106),Ok(2));assert_eq!(m.owned_records(A),0);assert_eq!(m.resolved(A,ticket,106),None);
 }
 #[test]fn exact_retirement_clears_released_records_without_active_owner_and_preserves_foreign_records(){
  let mut m=Machine::new();let mut p=Peer::new();let first=publish(&mut m,&mut p,A,1,100,60);let second=publish(&mut m,&mut p,B,1,110,60);
  assert_eq!(m.owner(),None);assert_eq!(m.owned_records(A),1);assert_eq!(m.owned_records(B),1);
  assert!(!m.retire(Identity{pd:A.pd,task:A.task+1},&mut p));assert!(m.resolved(A,first,112).is_some());
  let aborts=p.aborts;assert!(!m.retire(A,&mut p));assert_eq!(p.aborts,aborts);assert_eq!(m.owned_records(A),0);
  assert!(m.resolved(B,second,112).is_some());assert!(!m.cached.iter().flatten().any(|r|r.owner==A));
  assert!(m.cached.iter().flatten().any(|r|r.owner==B));
 }
 #[test]fn record_maintenance_is_bounded_preserves_fences_and_does_not_evict_dead_owner_slots(){
  let mut m=Machine::new();let mut p=Peer::new();
  for n in 0..6{let owner=Identity{pd:100+n,task:200+n};publish(&mut m,&mut p,owner,1,100+u64::from(n)*10,60);}
  let mut calls=0;m.maintain_records(200,|_|{calls+=1;true});assert_eq!(calls,6);
  let dead=Identity{pd:100,task:200};m.maintain_records(201,|owner|owner!=dead);
  assert_eq!(m.owned_records(dead),0);assert_eq!(m.records.iter().flatten().count(),5);assert_eq!(m.cached.iter().flatten().count(),6);
  let begins=p.begins.len();assert!(!m.handle(A,r(1,a::BEGIN,0),202,false,&mut p));assert_eq!(p.begins.len(),begins);
  m.maintain_records(70_000,|_|true);assert!(m.records.iter().all(Option::is_none));assert_eq!(m.cached.iter().flatten().count(),6);
 }
 #[test]fn maintenance_clock_rollback_revokes_records_and_candidates_without_replay_resurrection(){
  let mut m=Machine::new();let mut p=Peer::new();let ticket=publish(&mut m,&mut p,A,1,100,60);
  m.maintain_records(200,|_|true);assert!(m.resolved(A,ticket,200).is_some());assert_eq!(m.resolved(A,ticket,199),None);
  m.maintain_records(199,|_|true);assert_eq!(m.owned_records(A),0);m.maintain_records(201,|_|true);
  assert_eq!(call_at(&mut m,&mut p,A,r(2,a::RELEASE,ticket),202),Ok(0));assert_eq!(m.owned_records(A),0);
  assert_eq!(call_at(&mut m,&mut p,A,r(3,a::BEGIN,0),203),Ok(2));p.response=Some(response(2));m.advance(204,&mut p);assert!(m.candidate.is_some());
  m.maintain_records(205,|_|true);m.maintain_records(204,|_|true);assert!(m.candidate.is_none());
  assert_eq!(call_at(&mut m,&mut p,A,r(4,a::RELEASE,2),206),Ok(0));assert_eq!(m.owned_records(A),0);
 }
 #[test]fn cancellation_failure_and_malformed_later_bytes_never_publish_candidate(){
  let mut m=Machine::new();let mut p=Peer::new();assert_eq!(call_at(&mut m,&mut p,A,r(1,a::BEGIN,0),100),Ok(1));
  p.response=Some(response(1));m.advance(101,&mut p);assert!(m.candidate.is_some());
  assert_eq!(call_at(&mut m,&mut p,A,r(2,a::CANCEL,1),102),Ok(0));assert!(m.candidate.is_none());
  assert_eq!(call_at(&mut m,&mut p,A,r(3,a::RELEASE,1),103),Ok(0));assert_eq!(m.owned_records(A),0);
  assert_eq!(call_at(&mut m,&mut p,A,r(4,a::BEGIN,0),104),Ok(2));let mut malformed=response(2);malformed.push(0);p.response=Some(malformed);m.advance(105,&mut p);
  assert_eq!(m.state(),State::Failed);assert!(m.candidate.is_none());assert_eq!(call_at(&mut m,&mut p,A,r(5,a::RELEASE,2),106),Ok(0));assert_eq!(m.owned_records(A),0);
  p.error=Some(a::OFFLINE);assert_eq!(call_at(&mut m,&mut p,A,r(6,a::BEGIN,0),107),Ok(3));assert!(m.candidate.is_none());assert_eq!(m.owned_records(A),0);
 }

 #[test]fn candidate_liveness_uses_its_existing_slot_and_cannot_promote_after_death(){
  let mut m=Machine::new();let mut p=Peer::new();publish(&mut m,&mut p,B,1,100,60);
  assert_eq!(call_at(&mut m,&mut p,A,r(1,a::BEGIN,0),110),Ok(2));p.response=Some(response(2));m.advance(111,&mut p);
  assert!(m.candidate.is_some());let mut calls=0;
  m.maintain_records(112,|owner|{calls+=1;owner!=A});assert_eq!(calls,2);assert!(m.candidate.is_none());assert_eq!(m.owned_records(B),1);
  assert_eq!(call_at(&mut m,&mut p,A,r(2,a::RELEASE,2),113),Ok(0));assert_eq!(m.owned_records(A),0);
 }

}
