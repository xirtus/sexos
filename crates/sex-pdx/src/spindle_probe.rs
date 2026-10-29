//! Register-only diagnostic exchanges on the existing BOOT storage edge.
//! Object numbers are service data, never kernel capabilities.
pub const OP:u64=0x4C;
pub const STAT:u8=1;pub const GENERATION:u8=2;pub const INFO:u8=3;pub const NAME:u8=4;
pub const BEGIN:u8=5;pub const READ:u8=6;pub const WRITE:u8=7;pub const TRUNCATE:u8=8;pub const CREATE:u8=9;pub const DELETE:u8=10;pub const RENAME:u8=11;pub const SYNC:u8=12;pub const END:u8=13;
pub const ABORT:u8=14;
pub const MAX_BYTES:u32=65_536;pub const MISMATCH:u32=10;
pub const SEQ_MAX:u32=0xFF_FFFF;
pub const INVALID:u32=1;pub const DENIED:u32=2;pub const STALE:u32=3;
pub const BUSY:u32=5;pub const TIMEOUT:u32=8;pub const IO:u32=9;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Request{pub seq:u32,pub kind:u8,pub object:u8,pub offset:u32,pub a:u64,pub b:u64}
impl Request{
 pub fn header(self)->u64{self.seq as u64|((self.kind as u64)<<24)|((self.offset as u64)<<32)|((self.object as u64)<<56)}
 pub fn decode(h:u64,a:u64,b:u64)->Option<Self>{let r=Self{seq:(h&0xFF_FFFF)as u32,kind:(h>>24)as u8,offset:((h>>32)&0xFF_FFFF)as u32,object:(h>>56)as u8,a,b};if r.seq==0||!(STAT..=ABORT).contains(&r.kind){return None}Some(r)}
}
pub fn reply(seq:u32,result:Result<u32,u32>)->u64{let(tag,v)=match result{Ok(v)=>(0x54u64,v),Err(e)=>(0xE4u64,e)};(tag<<56)|((seq as u64)<<32)|v as u64}
pub fn decode_reply(v:u64,seq:u32)->Option<Result<u32,u32>>{if(v>>32)&0xFF_FFFF!=seq as u64{return None}match v>>56{0x54=>Some(Ok(v as u32)),0xE4=>Some(Err(v as u32)),_=>None}}
