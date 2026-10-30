// SPDX-License-Identifier: GPL-3.0-only
//! Register-only fixed-name DNS operation over an existing held network route.
//! The finite ticket is service data, never a capability or Unix descriptor.
pub use crate::network_job::{MAX_SEQ,MAX_TICKET,INVALID,DENIED,STALE,BUSY,OFFLINE,TIMEOUT,IO,EXHAUSTED};
pub const OP:u64=0x20D;
pub const BEGIN:u8=1;pub const STATUS:u8=2;pub const RESULT:u8=3;pub const CANCEL:u8=4;pub const RELEASE:u8=5;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Request{pub seq:u32,pub kind:u8,pub ticket:u32}
impl Request{
 pub fn encode(self)->[u64;3]{[self.seq as u64|((self.kind as u64)<<24),self.ticket as u64,0]}
 pub fn decode(args:[u64;3])->Result<Self,u32>{
  let r=Self{seq:(args[0]&MAX_SEQ as u64)as u32,kind:(args[0]>>24)as u8,ticket:args[1]as u32};
  if args[0]>>32!=0||args[1]>>32!=0||args[2]!=0||r.seq==0||!matches!(r.kind,BEGIN|STATUS|RESULT|CANCEL|RELEASE)
   ||(r.kind==BEGIN&&r.ticket!=0)||(r.kind!=BEGIN&&(r.ticket==0||r.ticket>MAX_TICKET)){return Err(INVALID);}Ok(r)
 }
}
pub const fn reply(seq:u32,value:u32)->u64{(0x79u64<<56)|((seq as u64&MAX_SEQ as u64)<<32)|value as u64}
pub const fn error(seq:u32,value:u32)->u64{(0xE9u64<<56)|((seq as u64&MAX_SEQ as u64)<<32)|value as u64}
pub fn decode_reply(word:u64,seq:u32)->Result<u32,u32>{
 if (word>>32)&MAX_SEQ as u64!=seq as u64{return Err(STALE);}
 match word>>56{0x79=>Ok(word as u32),0xE9=>Err(word as u32),_=>Err(INVALID)}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]#[repr(u32)]pub enum State{Idle=0,Waiting=1,Complete=2,Failed=3,Cancelled=4}
pub fn state(value:u32)->Option<State>{match value{0=>Some(State::Idle),1=>Some(State::Waiting),2=>Some(State::Complete),3=>Some(State::Failed),4=>Some(State::Cancelled),_=>None}}
