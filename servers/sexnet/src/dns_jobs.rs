// SPDX-License-Identifier: GPL-3.0-only
//! Fixed-name DNS custody. Parent authenticates the exact live BOOT identity.
use crate::jobs::Identity;
use sex_pdx::network_dns::{self as a,Request,State};
use sex_net::dns_wire;
pub trait Wire{fn begin(&mut self,ticket:u32)->Result<(),u32>;fn receive(&mut self,out:&mut[u8;512])->Result<Option<usize>,u32>;fn abort(&mut self);}
#[derive(Clone,Copy)]struct Receipt{owner:Identity,request:Request,word:u64}
pub struct Machine{owner:Option<Identity>,ticket:u32,next_ticket:u32,state:State,deadline:u64,last:u64,
 answer:Option<dns_wire::Answer>,failure:u32,receive:[u8;512],pending:[Option<Receipt>;6],cached:[Option<Receipt>;6],cursor:usize}
impl Machine{
 pub const fn new()->Self{Self{owner:None,ticket:0,next_ticket:1,state:State::Idle,deadline:0,last:0,answer:None,failure:0,receive:[0;512],pending:[None;6],cached:[None;6],cursor:0}}
 pub fn owner(&self)->Option<Identity>{self.owner}
 pub fn state(&self)->State{self.state}
 pub fn ticket(&self)->u32{self.ticket}
 pub fn can_transmit(&self,ticket:u32)->bool{self.owner.is_some()&&self.ticket==ticket&&self.state==State::Waiting}
 fn slot(&self)->Option<usize>{(0..6).map(|n|(self.cursor+n)%6).find(|i|self.pending[*i].is_some())}
 pub fn pending_reply(&self)->Option<(Identity,u64)>{self.slot().and_then(|i|self.pending[i]).map(|r|(r.owner,r.word))}
 pub fn reply_accepted(&mut self){if let Some(i)=self.slot(){self.pending[i]=None;self.cursor=(i+1)%6;}}
 pub fn reply_deferred(&mut self){if let Some(i)=self.slot(){self.cursor=(i+1)%6;}}
 fn scrub(&mut self,wire:&mut impl Wire){self.receive.fill(0);self.answer=None;self.deadline=0;self.last=0;wire.abort();}
 fn fail(&mut self,why:u32,wire:&mut impl Wire){self.scrub(wire);self.failure=why;self.state=State::Failed;}
 pub fn retire(&mut self,owner:Identity,wire:&mut impl Wire)->bool{
  let held=self.owner==Some(owner);if held{self.scrub(wire);self.owner=None;self.ticket=0;self.failure=0;self.state=State::Idle;}
  for i in 0..6{if self.cached[i].map(|r|r.owner)==Some(owner){self.cached[i]=None;self.pending[i]=None;}}held
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
   a::RELEASE=>{self.scrub(wire);self.owner=None;self.ticket=0;self.failure=0;self.state=State::Idle;Ok(0)},
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
  match decoded{Ok(Some(answer))=>{self.answer=Some(answer);self.state=State::Complete;self.deadline=0;wire.abort();},Ok(None)=>{},Err(e)=>self.fail(e,wire)}
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
}
