// SPDX-License-Identifier: GPL-3.0-only
//! Read-only immutable BOOT policy query on the existing held route. A profile
//! receipt is not a grant to connect, mutate roots, or select an arbitrary name.
pub const OP:u64=0x210;
pub const MAX_SEQ:u32=0x00ff_ffff;
pub const INVALID:u32=1;pub const DENIED:u32=2;pub const STALE:u32=3;
pub const BUSY:u32=4;pub const OFFLINE:u32=5;pub const EXHAUSTED:u32=8;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Request {pub seq:u32}
impl Request {
    pub const fn valid(self)->bool {self.seq!=0&&self.seq<=MAX_SEQ}
    pub fn encode(self)->[u64;3] {assert!(self.valid(),"TRUST_POLICY_SEQUENCE");[self.seq as u64,0,0]}
    pub fn decode(args:[u64;3])->Result<Self,u32> {
        if args[0]==0||args[0]>MAX_SEQ as u64||args[1]!=0||args[2]!=0{return Err(INVALID);}
        Ok(Self {seq:args[0]as u32})
    }
}
const fn known_error(code:u32)->bool {matches!(code,INVALID|DENIED|STALE|BUSY|OFFLINE|EXHAUSTED)}
pub const fn reply(seq:u32,profile:u32)->u64 {
    assert!(seq!=0&&seq<=MAX_SEQ,"TRUST_POLICY_SEQUENCE");
    assert!(profile==crate::trust_config::PROFILE,"TRUST_POLICY_PROFILE");
    (0x7Cu64<<56)|((seq as u64)<<32)|profile as u64
}
pub const fn error(seq:u32,code:u32)->u64 {
    assert!(seq!=0&&seq<=MAX_SEQ,"TRUST_POLICY_SEQUENCE");assert!(known_error(code),"TRUST_POLICY_ERROR");
    (0xECu64<<56)|((seq as u64)<<32)|code as u64
}
pub fn decode_reply(word:u64,seq:u32)->Result<u32,u32> {
    if seq==0||seq>MAX_SEQ{return Err(INVALID);}
    let tag=word>>56;if !matches!(tag,0x7C|0xEC){return Err(INVALID);}
    let actual=((word>>32)as u32)&MAX_SEQ;
    if actual==0{return Err(INVALID);}if actual!=seq{return Err(STALE);}
    let value=word as u32;
    if tag==0xEC{return Err(if known_error(value){value}else{INVALID});}
    if value!=crate::trust_config::PROFILE{return Err(INVALID);}Ok(value)
}
