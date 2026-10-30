// SPDX-License-Identifier: GPL-3.0-only
//! Finite register-only text transfer on existing BOOT Shell Domain routes.
//! Subjects and tickets are correlation data, never capability handles. The
//! broker requires a held lifecycle catalog, live exact sender PD/task and a
//! Shell-issued interaction before accepting any request.
use crate::{app_lifecycle::CatalogInfo,text_selection::Source};
pub const MAX_BYTES:usize=12288;
pub const SHELL:Subject=Subject{pd:3,task:3};
pub const INTENT:u64=0x280;
pub const BEGIN:u64=0x281;
pub const WRITE:u64=0x282;
pub const READ:u64=0x283;
pub const GET:u64=0x284;
pub const FINISH:u64=0x285;
pub const CANCEL:u64=0x286;
pub const REPLY:u64=0x287;
pub const DATA:u64=0x288;
pub const QUERY:u64=0x289;
pub fn request_opcode(op:u64)->bool{matches!(op,BEGIN|WRITE|PUBLISH|READ|GET|FINISH|CANCEL|QUERY)}
pub fn response_opcode(op:u64)->bool{matches!(op,INTENT|REPLY|DATA)}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Subject{pub pd:u32,pub task:u32}
impl Subject{pub const fn valid(self)->bool{self.pd!=0&&self.task!=0}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Participant{pub subject:Subject,pub surface:u64,pub route:u64,pub can_read:bool}
impl Participant{
 pub fn from_catalog(c:CatalogInfo)->Option<Self>{
  if !c.is_subject(c.target_pd,c.surface_id,3){return None}
  let(route,can_read)=match(c.target_pd,c.surface_id){(9,156)=>(41,true),(15,300)=>(43,false),(16,301)=>(46,true),(18,307)=>(56,true),_=>return None};
  Some(Self{subject:Subject{pd:c.target_pd,task:c.task_id},surface:c.surface_id,route,can_read})
 }
 pub fn allows(self,a:Action)->bool{match a{Action::Copy=>true,Action::Cut|Action::Paste=>self.can_read}}
}
#[repr(u8)]#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Action{Copy=1,Cut=2,Paste=3}
impl Action{pub fn decode(v:u64)->Option<Self>{match v{1=>Some(Self::Copy),2=>Some(Self::Cut),3=>Some(Self::Paste),_=>None}}}
#[repr(u8)]#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Phase{Begin=1,Publish=2,Read=3,Finish=4,Cancel=5,Query=6}
impl Phase{fn decode(v:u8)->Option<Self>{match v{1=>Some(Self::Begin),2=>Some(Self::Publish),3=>Some(Self::Read),4=>Some(Self::Finish),5=>Some(Self::Cancel),6=>Some(Self::Query),_=>None}}}
#[repr(u8)]#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Status{Ok=0,Busy=1,Denied=2,Invalid=3,Empty=4,Invalidated=5,Exhausted=6,Uncertain=7}
impl Status{fn decode(v:u8)->Option<Self>{match v{0=>Some(Self::Ok),1=>Some(Self::Busy),2=>Some(Self::Denied),3=>Some(Self::Invalid),4=>Some(Self::Empty),5=>Some(Self::Invalidated),6=>Some(Self::Exhausted),7=>Some(Self::Uncertain),_=>None}}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Packet{pub opcode:u64,pub args:[u64;3]}
impl Packet{pub fn erase(&mut self){unsafe{core::ptr::write_volatile(&mut self.opcode,0);for a in &mut self.args{core::ptr::write_volatile(a,0)}}}}
pub fn hash(bytes:&[u8])->u64{bytes.iter().fold(0xcbf29ce484222325,|h,&b|(h^b as u64).wrapping_mul(0x100000001b3))}
pub fn text(bytes:&[u8])->bool{bytes.iter().all(|&b|b==b'\n'||(b' '..=b'~').contains(&b))}
fn ticket(v:u64)->Option<u32>{(v!=0&&v<=u32::MAX as u64).then_some(v as u32)}
fn chunk(meta:u64,a:u64,b:u64)->Option<(u32,usize,usize,[u8;16])>{
 let id=meta as u32;let offset=((meta>>32)&0xffff)as usize;let count=((meta>>48)&255)as usize;
 let mut bytes=[0;16];bytes[..8].copy_from_slice(&a.to_le_bytes());bytes[8..].copy_from_slice(&b.to_le_bytes());
 (meta>>56==0&&id!=0&&offset<MAX_BYTES&&offset%16==0&&(1..=16).contains(&count)&&offset+count<=MAX_BYTES&&bytes[count..].iter().all(|&x|x==0)).then_some((id,offset,count,bytes))
}
fn chunk_packet(opcode:u64,id:u32,offset:usize,count:usize,bytes:[u8;16])->Packet{Packet{opcode,args:[id as u64|(offset as u64)<<32|(count as u64)<<48,u64::from_le_bytes(bytes[..8].try_into().unwrap()),u64::from_le_bytes(bytes[8..].try_into().unwrap())]}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Request{
 Begin{ticket:u32,len:usize,source:Source},Write{ticket:u32,offset:usize,count:usize,bytes:[u8;16]},
 Publish{ticket:u32,len:usize,hash:u64},Read{ticket:u32},Get{ticket:u32,offset:usize},
 Finish{ticket:u32,generation:u32,hash:u64},Cancel{ticket:u32},Query{ticket:u32},
}
// PUBLISH has its own operation; COMMIT does not alias a data chunk or read.
pub const PUBLISH:u64=0x28a;
impl Request{
 pub fn ticket(self)->u32{match self{Self::Begin{ticket,..}|Self::Write{ticket,..}|Self::Publish{ticket,..}|Self::Read{ticket}|Self::Get{ticket,..}|Self::Finish{ticket,..}|Self::Cancel{ticket}|Self::Query{ticket}=>ticket}}
 pub fn decode(op:u64,args:[u64;3])->Option<Self>{let[a,b,c]=args;match op{
  BEGIN if a>>48==0=>{let id=a as u32;let len=(a>>32)as usize;let source=Source{generation:b,revision:c};(id!=0&&(1..=MAX_BYTES).contains(&len)&&source.valid()).then_some(Self::Begin{ticket:id,len,source})},
  WRITE=>{let(ticket,offset,count,bytes)=chunk(a,b,c)?;Some(Self::Write{ticket,offset,count,bytes})},
  PUBLISH if b<=MAX_BYTES as u64&&b!=0=>Some(Self::Publish{ticket:ticket(a)?,len:b as usize,hash:c}),
  READ if b==0&&c==0=>Some(Self::Read{ticket:ticket(a)?}),
  GET if a>>48==0&&b==0&&c==0=>{let id=a as u32;let offset=(a>>32)as usize;(id!=0&&offset<MAX_BYTES&&offset%16==0).then_some(Self::Get{ticket:id,offset})},
  FINISH if b!=0&&b<=u32::MAX as u64=>Some(Self::Finish{ticket:ticket(a)?,generation:b as u32,hash:c}),
  CANCEL if b==0&&c==0=>Some(Self::Cancel{ticket:ticket(a)?}),
  QUERY if b==0&&c==0=>Some(Self::Query{ticket:ticket(a)?}),_=>None}}
 pub fn wire(self)->Packet{match self{
  Self::Begin{ticket,len,source}=>Packet{opcode:BEGIN,args:[ticket as u64|(len as u64)<<32,source.generation,source.revision]},
  Self::Write{ticket,offset,count,bytes}=>chunk_packet(WRITE,ticket,offset,count,bytes),
  Self::Publish{ticket,len,hash}=>Packet{opcode:PUBLISH,args:[ticket as u64,len as u64,hash]},
  Self::Read{ticket}=>Packet{opcode:READ,args:[ticket as u64,0,0]},
  Self::Get{ticket,offset}=>Packet{opcode:GET,args:[ticket as u64|(offset as u64)<<32,0,0]},
  Self::Finish{ticket,generation,hash}=>Packet{opcode:FINISH,args:[ticket as u64,generation as u64,hash]},
  Self::Cancel{ticket}=>Packet{opcode:CANCEL,args:[ticket as u64,0,0]},Self::Query{ticket}=>Packet{opcode:QUERY,args:[ticket as u64,0,0]},}}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Response{
 Intent{ticket:u32,action:Action},Meta{ticket:u32,phase:Phase,status:Status,len:usize,generation:u32,hash:u64},Data{ticket:u32,offset:usize,count:usize,bytes:[u8;16]},
}
impl Response{
 pub fn ticket(self)->u32{match self{Self::Intent{ticket,..}|Self::Meta{ticket,..}|Self::Data{ticket,..}=>ticket}}
 pub fn decode(op:u64,args:[u64;3])->Option<Self>{let[a,b,c]=args;match op{
  INTENT if c==0=>Some(Self::Intent{ticket:ticket(a)?,action:Action::decode(b)?}),
  REPLY if a>>48==0&&b&0xffff0000==0=>{let id=a as u32;let phase=Phase::decode((a>>32)as u8)?;let status=Status::decode((a>>40)as u8)?;let len=(b&0xffff)as usize;let generation=(b>>32)as u32;(id!=0&&len<=MAX_BYTES&&(status==Status::Ok||(len==0&&generation==0&&c==0))).then_some(Self::Meta{ticket:id,phase,status,len,generation,hash:c})},
  DATA=>{let(ticket,offset,count,bytes)=chunk(a,b,c)?;Some(Self::Data{ticket,offset,count,bytes})},_=>None}}
 pub fn wire(self)->Packet{match self{
  Self::Intent{ticket,action}=>Packet{opcode:INTENT,args:[ticket as u64,action as u64,0]},
  Self::Meta{ticket,phase,status,len,generation,hash}=>Packet{opcode:REPLY,args:[ticket as u64|(phase as u64)<<32|(status as u64)<<40,len as u64|(generation as u64)<<32,hash]},
  Self::Data{ticket,offset,count,bytes}=>chunk_packet(DATA,ticket,offset,count,bytes),}}
}
