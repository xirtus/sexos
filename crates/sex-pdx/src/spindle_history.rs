//! Bounded service protocol over existing BOOT storage edges. Handles are
//! SexFiles leases, never new kernel capabilities or inherited identities.
pub const OP:u64=0x4B;
pub const OPEN_SAVE:u8=1; pub const OPEN_LOAD:u8=2; pub const WRITE:u8=3;
pub const COMMIT:u8=4; pub const STAT:u8=5; pub const READ:u8=6; pub const CLOSE:u8=7;
pub const MAX:usize=4096; pub const HEADER:usize=24; pub const SEQ_MAX:u32=0xFF_FFFF;
pub const INVALID:u32=1; pub const DENIED:u32=2; pub const STALE:u32=3; pub const MISSING:u32=4; pub const BUSY:u32=5; pub const CORRUPT:u32=6;
const OK:u64=0x53;const ERROR:u64=0xE3;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Request{pub seq:u32,pub kind:u8,pub a:u64,pub b:u64}
impl Request{
 pub fn header(self)->u64{self.seq as u64|((self.kind as u64)<<24)}
 pub fn decode(header:u64,a:u64,b:u64)->Option<Self>{let seq=(header&0xFF_FFFF)as u32;let kind=(header>>24)as u8;
  if header>>32!=0||seq==0||!(OPEN_SAVE..=CLOSE).contains(&kind){return None}Some(Self{seq,kind,a,b})}
}
pub fn reply(seq:u32,result:Result<u32,u32>)->u64{let(tag,value)=match result{Ok(v)=>(OK,v),Err(e)=>(ERROR,e)};(tag<<56)|((seq as u64)<<32)|value as u64}
pub fn decode_reply(value:u64,seq:u32)->Option<Result<u32,u32>>{
 if ((value>>32)&0xFF_FFFF)!=seq as u64{return None}match value>>56{OK=>Some(Ok(value as u32)),ERROR=>Some(Err(value as u32)),_=>None}
}
pub fn io(handle:u32,offset:usize,len:usize)->u64{handle as u64|((offset as u64)<<32)|((len as u64)<<48)}
pub fn split_io(arg:u64)->(u32,usize,usize){(arg as u32,((arg>>32)&0xFFFF)as usize,(arg>>48)as usize)}
pub fn hash(bytes:&[u8])->u64{let mut h=0xcbf29ce484222325u64;for &b in bytes{h=(h^b as u64).wrapping_mul(0x100000001b3);}h}
pub fn seal(buf:&mut[u8],owner:u32,count:usize,total:usize)->bool{
 if total<HEADER||total>MAX||buf.len()<total||count>128{return false}
 buf[..4].copy_from_slice(b"SHI1");buf[4..8].copy_from_slice(&owner.to_le_bytes());
 buf[8..10].copy_from_slice(&(count as u16).to_le_bytes());buf[10..12].copy_from_slice(&(total as u16).to_le_bytes());buf[12..24].fill(0);
 let h=hash(&buf[..total]);buf[16..24].copy_from_slice(&h.to_le_bytes());true
}
pub fn validate(buf:&[u8],owner:u32)->Result<usize,u32>{
 if buf.len()<HEADER||buf.len()>MAX||&buf[..4]!=b"SHI1"||buf[4..8]!=owner.to_le_bytes()||buf[12..16]!=[0;4]{return Err(CORRUPT)}
 let count=u16::from_le_bytes([buf[8],buf[9]])as usize;let total=u16::from_le_bytes([buf[10],buf[11]])as usize;
 if total!=buf.len()||count>128{return Err(CORRUPT)}
 let mut h=0xcbf29ce484222325u64;for(i,&b)in buf.iter().enumerate(){h=(h^if(16..24).contains(&i){0}else{b as u64}).wrapping_mul(0x100000001b3);}
 if h!=u64::from_le_bytes(buf[16..24].try_into().unwrap()){return Err(CORRUPT)}
 let mut off=HEADER;for _ in 0..count{
  if off+2>total{return Err(CORRUPT)}let n=u16::from_le_bytes([buf[off],buf[off+1]])as usize;off+=2;
  if n==0||n>256||off+n>total||!buf[off..off+n].iter().all(|b|(32..=126).contains(b)){return Err(CORRUPT)}off+=n;
 }if off!=total{return Err(CORRUPT)}Ok(count)
}
