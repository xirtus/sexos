//! Two fixed BOOT-owned banks. Publish only after payload + commit barriers.
use sex_pdx::{history_recovery::Grant,spindle_history as w};
pub const STORAGE:u32=7;
pub trait SectorIo{fn read(&mut self,lba:u64,out:&mut[u8;512])->Result<(),u32>;fn write(&mut self,lba:u64,data:&[u8;512])->Result<(),u32>;fn sync(&mut self)->Result<(),u32>;}
pub trait Store{fn save(&mut self,caller:u32,bytes:&[u8])->Result<(),u32>;fn recover(&mut self,caller:u32,out:&mut[u8;w::MAX])->Result<usize,u32>;}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Record{pub generation:u64,pub writer:u32,pub task:u32,pub len:usize,pub hash:u64,pub bank:usize}
pub struct Journal<I>{pub io:I,pub grant:Option<Grant>,pub last:Option<Record>}
impl<I:SectorIo> Journal<I>{
 pub fn new(io:I,grant:Option<Grant>)->Self{Self{io,grant,last:None}}
 fn authority(&self,caller:u32)->Result<Grant,u32>{let g=self.grant.ok_or(w::DENIED)?;
  if g.logical!=12||g.base!=sex_pdx::SPINDLE_HISTORY_BASE_LBA||g.sectors!=sex_pdx::SPINDLE_HISTORY_SECTORS||!g.permits(11,caller,caller){return Err(w::DENIED)}Ok(g)}
 fn header(g:Grant,r:Record)->[u8;512]{let mut h=[0;512];h[..8].copy_from_slice(b"SEXHST01");h[8..12].copy_from_slice(&g.logical.to_le_bytes());h[12..16].copy_from_slice(&1u32.to_le_bytes());h[16..24].copy_from_slice(&r.generation.to_le_bytes());h[24..28].copy_from_slice(&r.writer.to_le_bytes());h[28..32].copy_from_slice(&r.task.to_le_bytes());h[32..36].copy_from_slice(&(r.len as u32).to_le_bytes());h[40..48].copy_from_slice(&r.hash.to_le_bytes());let sum=w::hash(&h);h[48..56].copy_from_slice(&sum.to_le_bytes());h}
 fn bank(&mut self,g:Grant,index:usize,out:&mut[u8;w::MAX])->Result<(Option<Record>,bool),u32>{
  let mut h=[0;512];self.io.read(g.base+index as u64*9,&mut h)?;if h==[0;512]{return Ok((None,false))}
  let sum=u64::from_le_bytes(h[48..56].try_into().unwrap());h[48..56].fill(0);
  if &h[..8]!=b"SEXHST01"||h[8..12]!=g.logical.to_le_bytes()||h[12..16]!=1u32.to_le_bytes()||h[36..40]!=[0;4]||h[56..].iter().any(|b|*b!=0)||w::hash(&h)!=sum{return Ok((None,true))}
  let r=Record{generation:u64::from_le_bytes(h[16..24].try_into().unwrap()),writer:u32::from_le_bytes(h[24..28].try_into().unwrap()),task:u32::from_le_bytes(h[28..32].try_into().unwrap()),len:u32::from_le_bytes(h[32..36].try_into().unwrap())as usize,hash:u64::from_le_bytes(h[40..48].try_into().unwrap()),bank:index};
  if r.generation==0||r.len<w::HEADER||r.len>w::MAX||!g.permits(11,r.writer,r.task){return Ok((None,true))}
  out.fill(0);for n in 0..(r.len+511)/512{let mut sector=[0;512];self.io.read(g.base+index as u64*9+1+n as u64,&mut sector)?;let count=(r.len-n*512).min(512);out[n*512..n*512+count].copy_from_slice(&sector[..count]);if n*512+count==r.len&&sector[count..].iter().any(|b|*b!=0){return Ok((None,true))}}
  if w::hash(&out[..r.len])!=r.hash||w::validate(&out[..r.len],r.writer).is_err(){return Ok((None,true))}Ok((Some(r),false))
 }
 fn scan(&mut self,g:Grant)->Result<Option<Record>,u32>{let mut bytes=[0;w::MAX];let(a,bad_a)=self.bank(g,0,&mut bytes)?;let(b,bad_b)=self.bank(g,1,&mut bytes)?;
  match(a,b){(None,None)=>if bad_a||bad_b{Err(w::CORRUPT)}else{Ok(None)},(Some(r),None)|(None,Some(r))=>Ok(Some(r)),(Some(a),Some(b))=>{
   if a.generation==b.generation{if(a.writer,a.task,a.len,a.hash)!=(b.writer,b.task,b.len,b.hash){return Err(w::CORRUPT)}return Ok(Some(a))}Ok(Some(if a.generation>b.generation{a}else{b}))}}
 }
}
impl<I:SectorIo> Store for Journal<I>{
 fn save(&mut self,caller:u32,bytes:&[u8])->Result<(),u32>{let g=self.authority(caller)?;w::validate(bytes,caller)?;
  let previous=self.scan(g)?;let generation=previous.map_or(Some(1),|r|r.generation.checked_add(1)).ok_or(w::STALE)?;let bank=previous.map_or(0,|r|1-r.bank);
  let r=Record{generation,writer:caller,task:caller,len:bytes.len(),hash:w::hash(bytes),bank};let base=g.base+bank as u64*9;
  // The previously committed bank is never touched. No unflushed payload can
  // acquire a committed header, including when an older bank is overwritten.
  self.io.write(base,&[0;512])?;self.io.sync()?;
  for n in 0..(bytes.len()+511)/512{let mut sector=[0;512];let count=(bytes.len()-n*512).min(512);sector[..count].copy_from_slice(&bytes[n*512..n*512+count]);self.io.write(base+1+n as u64,&sector)?;}
  self.io.sync()?;self.io.write(base,&Self::header(g,r))?;self.io.sync()?;
  let mut check=[0;w::MAX];if self.bank(g,bank,&mut check)?.0!=Some(r){return Err(w::CORRUPT)}self.last=Some(r);Ok(())
 }
 fn recover(&mut self,caller:u32,out:&mut[u8;w::MAX])->Result<usize,u32>{let g=self.authority(caller)?;let r=self.scan(g)?.ok_or(w::MISSING)?;
  if self.bank(g,r.bank,out)?.0!=Some(r){return Err(w::CORRUPT)}let count=w::validate(&out[..r.len],r.writer)?;
  // Copy saved DATA under BOOT's explicit relationship. Rebind the snapshot
  // header to the current owner, never a source handle or source capability.
  if !w::seal(out,caller,count,r.len){return Err(w::CORRUPT)}w::validate(&out[..r.len],caller)?;self.last=Some(r);Ok(r.len)
 }
}
