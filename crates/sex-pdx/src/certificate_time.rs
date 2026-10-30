// SPDX-License-Identifier: GPL-3.0-only
//! Read-only certificate-time query ABI on an existing held route. A replay is
//! the same historical query snapshot, never a fresh observation of UTC.
pub const OP:u64=0x20F;
pub const MAX_SEQ:u32=0x00ff_ffff;
pub const INVALID:u32=1;
pub const DENIED:u32=2;
pub const STALE:u32=3;
pub const BUSY:u32=4;
pub const OFFLINE:u32=5;
pub const EXHAUSTED:u32=8;
pub const ROLLBACK:u32=9;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Request {pub seq:u32}
impl Request {
    pub const fn valid(self)->bool {self.seq!=0&&self.seq<=MAX_SEQ}
    pub fn encode(self)->[u64;3] {assert!(self.valid(),"CERTIFICATE_TIME_SEQUENCE");[self.seq as u64,0,0]}
    pub fn decode(args:[u64;3])->Result<Self,u32> {
        if args[0]==0||args[0]>MAX_SEQ as u64||args[1]!=0||args[2]!=0 {return Err(INVALID);}
        Ok(Self {seq:args[0] as u32})
    }
}
const fn known_error(code:u32)->bool {matches!(code,DENIED|OFFLINE|BUSY|INVALID|STALE|ROLLBACK|EXHAUSTED)}
pub const fn reply(seq:u32,seconds:u32)->u64 {
    assert!(seq!=0&&seq<=MAX_SEQ,"CERTIFICATE_TIME_SEQUENCE");
    assert!(seconds as u64>=crate::clock_source::MIN_SECONDS&&seconds as u64<=crate::clock_source::MAX_SECONDS,"CERTIFICATE_TIME_RANGE");
    (0x7Bu64<<56)|((seq as u64)<<32)|seconds as u64
}
pub const fn error(seq:u32,code:u32)->u64 {
    assert!(seq!=0&&seq<=MAX_SEQ,"CERTIFICATE_TIME_SEQUENCE");
    assert!(known_error(code),"CERTIFICATE_TIME_ERROR");
    (0xEBu64<<56)|((seq as u64)<<32)|code as u64
}
pub fn decode_reply(word:u64,seq:u32)->Result<u32,u32> {
    if seq==0||seq>MAX_SEQ {return Err(INVALID);}
    let tag=word>>56;
    if !matches!(tag,0x7B|0xEB) {return Err(INVALID);}
    let actual=((word>>32)as u32)&MAX_SEQ;
    if actual==0 {return Err(INVALID);}
    if actual!=seq {return Err(STALE);}
    let value=word as u32;
    if tag==0xEB {return Err(if known_error(value){value}else{INVALID});}
    if (value as u64)<crate::clock_source::MIN_SECONDS||(value as u64)>crate::clock_source::MAX_SECONDS {return Err(INVALID);}
    Ok(value)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn exact_query_registers_and_no_truncating_aliases() {
        for seq in [1,MAX_SEQ] {let r=Request{seq};assert_eq!(r.encode(),[seq as u64,0,0]);assert_eq!(Request::decode(r.encode()),Ok(r));}
        for args in [[0,0,0],[MAX_SEQ as u64+1,0,0],[1+(1<<32),0,0],[1,1,0],[1,0,1<<63]] {assert_eq!(Request::decode(args),Err(INVALID));}
        for seq in [0,MAX_SEQ+1,u32::MAX] {assert!(std::panic::catch_unwind(||Request{seq}.encode()).is_err());}
    }
    #[test] fn replies_require_exact_tag_correlation_range_and_nonzero_known_error() {
        for seq in [1,MAX_SEQ] {
            for seconds in [crate::clock_source::MIN_SECONDS as u32,crate::clock_source::MAX_SECONDS as u32] {assert_eq!(decode_reply(reply(seq,seconds),seq),Ok(seconds));}
            for code in [DENIED,OFFLINE,BUSY,INVALID,STALE,ROLLBACK,EXHAUSTED] {assert_eq!(decode_reply(error(seq,code),seq),Err(code));}
        }
        for seq in [0,MAX_SEQ+1] {assert_eq!(decode_reply(reply(1,crate::clock_source::MIN_SECONDS as u32),seq),Err(INVALID));}
        assert_eq!(decode_reply(reply(1,crate::clock_source::MIN_SECONDS as u32),2),Err(STALE));
        for word in [0,(0x7Bu64<<56),(0x7Bu64<<56)|(1<<32), (0x7Bu64<<56)|(1<<32)|u32::MAX as u64,(0xEBu64<<56)|(1<<32),(0xEBu64<<56)|(1<<32)|99] {assert_eq!(decode_reply(word,1),Err(INVALID));}
        assert!(std::panic::catch_unwind(||error(1,0)).is_err());
    }
}
