//! Two fixed BOOT subjects, private names and owner-checked RamFS leases.
//! Staging is unpublished until complete snapshot validation + atomic swap.
use sex_pdx::spindle_history::*;
use crate::history_journal::Store;
use crate::{backends::{FsBackend,ramfs::RamFs},messages};
#[derive(Clone,Copy,PartialEq,Eq)]enum Mode{Idle,Save,Load}
struct Session{handle:u32,mode:Mode,bytes:[u8;MAX],len:usize,last:Option<Request>,receipt:u64,pending:Option<u64>,dead:bool}
impl Session{const fn new()->Self{Self{handle:0,mode:Mode::Idle,bytes:[0;MAX],len:0,last:None,receipt:0,pending:None,dead:false}}}
pub struct Service{sessions:[Session;2]}
impl Service{
 pub const fn new()->Self{Self{sessions:[Session::new(),Session::new()]}}
 fn index(caller:u32)->Option<usize>{match caller{12=>Some(0),17=>Some(1),_=>None}}
 pub fn retire(&mut self,caller:u32){if let Some(i)=Self::index(caller){let s=&mut self.sessions[i];s.dead=true;s.pending=None;s.last=None;s.handle=0;s.mode=Mode::Idle;s.len=0;s.bytes.fill(0);sex_pdx::serial_record!("[files.history.retire] target={} staged=0 pending=0 terminal=1",caller);}}
 pub fn pending(&self)->bool{self.sessions.iter().any(|s|s.pending.is_some())}
 pub fn poll(&mut self,mut send:impl FnMut(u32,u64)->u64){for(i,s)in self.sessions.iter_mut().enumerate(){if let Some(value)=s.pending{if send(if i==0{12}else{17},value)==0{s.pending=None;}}}}
 pub fn handle(&mut self,fs:&RamFs,caller:u32,live:bool,header:u64,a:u64,b:u64,store:&mut impl Store)->Option<u64>{
  let req=Request::decode(header,a,b)?;let Some(i)=Self::index(caller)else{return Some(reply(req.seq,Err(DENIED)))};
  let s=&mut self.sessions[i];if s.dead||!live{return Some(reply(req.seq,Err(DENIED)))}
  // Never overwrite an undelivered receipt. Legitimate clients have only one
  // admitted request; exact retries keep the original effect and receipt.
  if s.pending.is_some(){
   // A fresh own-session abort may supersede an undelivered receipt. It has
   // no caller-selected lease and cannot address another subject's state.
   if req.kind!=ABORT||req.a!=0||req.b!=0||s.last.map_or(true,|old|req.seq<=old.seq){return None}
   s.pending=None;
  }
  let value=if let Some(old)=s.last{
   if req==old{s.receipt}else if req.seq<=old.seq{reply(req.seq,Err(STALE))}else{Self::execute(fs,caller,s,req,store)}
  }else{Self::execute(fs,caller,s,req,store)};
  if s.last.map_or(true,|old|req.seq>old.seq){s.last=Some(req);s.receipt=value;}
  s.pending=Some(value);None
 }
 fn execute(fs:&RamFs,caller:u32,s:&mut Session,r:Request,store:&mut impl Store)->u64{reply(r.seq,Self::operation(fs,caller,s,r,store))}
 fn operation(fs:&RamFs,caller:u32,s:&mut Session,r:Request,store:&mut impl Store)->Result<u32,u32>{
  let (handle,offset,n)=split_io(r.a);
  match r.kind{
   ABORT=>{if r.a!=0||r.b!=0{return Err(INVALID)}
    if s.handle!=0{fs.close(s.handle as u64,caller).map_err(|_|DENIED)?;}
    s.handle=0;s.mode=Mode::Idle;s.len=0;s.bytes.fill(0);
    sex_pdx::serial_record!("[files.history.aborted] target={} seq={} lease=0 staged=0 rollback=0",caller,r.seq);Ok(0)
   },
   OPEN_SAVE|OPEN_LOAD=>{
    if r.a!=0||r.b!=0{return Err(INVALID)}if s.handle!=0{return Err(BUSY)}
    let name:&[u8]=if caller==12{b"spindle-history-12"}else{b"spindle-history-17"};
    let raw=match fs.open(name,if r.kind==OPEN_SAVE{messages::RAMFS_O_CREATE}else{0},0,caller){
     Ok(h)=>h,Err(messages::ERR_NOT_FOUND) if r.kind==OPEN_LOAD=>{
      let len=store.recover(caller,&mut s.bytes)?;
      if len<HEADER||len>MAX{return Err(CORRUPT)}
      validate(&s.bytes[..len],caller)?;
      let h=fs.open(name,messages::RAMFS_O_CREATE,0,caller).map_err(|_|DENIED)?;
      if fs.replace_owned(h,&s.bytes[..len],caller).is_err(){let _=fs.close(h,caller);return Err(DENIED)}h
     },Err(e)=>return Err(if e==messages::ERR_NOT_FOUND{MISSING}else{DENIED})};
    if raw==0||raw>u32::MAX as u64{let _=fs.close(raw,caller);return Err(INVALID)}
    s.handle=raw as u32;s.mode=if r.kind==OPEN_SAVE{Mode::Save}else{Mode::Load};s.len=0;s.bytes.fill(0);sex_pdx::serial_record!("[files.history.open] target={} handle={} mode={} seq={}",caller,s.handle,if s.mode==Mode::Save{"save"}else{"load"},r.seq);Ok(s.handle)
   },
   _=>{
    if handle==0||handle!=s.handle{return Err(DENIED)}
    match r.kind{
     WRITE=>{if s.mode!=Mode::Save||offset!=s.len||n==0||n>4||offset+n>MAX||r.b>>(n*8)!=0{return Err(INVALID)}
      s.bytes[offset..offset+n].copy_from_slice(&r.b.to_le_bytes()[..n]);s.len+=n;Ok(n as u32)},
     COMMIT=>{if s.mode!=Mode::Save||offset!=s.len||n!=0||r.b!=0{return Err(INVALID)}validate(&s.bytes[..s.len],caller)?;
      let next=fs.prepare_owned(handle as u64,&s.bytes[..s.len],caller).map_err(|_|DENIED)?;
      store.save(caller,&s.bytes[..s.len])?;
      fs.publish_owned(handle as u64,next,caller).map_err(|_|DENIED)?;sex_pdx::serial_record!("[files.history.commit] target={} handle={} seq={} bytes={} hash={:#x}",caller,handle,r.seq,s.len,hash(&s.bytes[..s.len]));Ok(s.len as u32)},
     STAT=>{if s.mode!=Mode::Load||offset!=0||n!=0||r.b!=0{return Err(INVALID)}let len=fs.stat(handle as u64,caller).map_err(|_|DENIED)?.0;
      if len as usize>MAX{return Err(CORRUPT)}s.len=len as usize;Ok(len as u32)},
     READ=>{if s.mode!=Mode::Load||n==0||n>4||offset+n>s.len||r.b!=0{return Err(INVALID)}let mut bytes=[0u8;4];
      if fs.read(handle as u64,offset as u64,&mut bytes[..n],caller).map_err(|_|DENIED)?!=n as u64{return Err(CORRUPT)}Ok(u32::from_le_bytes(bytes))},
     CLOSE=>{if offset!=0||n!=0||r.b!=0{return Err(INVALID)}fs.close(handle as u64,caller).map_err(|_|DENIED)?;
      s.handle=0;s.mode=Mode::Idle;s.len=0;s.bytes.fill(0);sex_pdx::serial_record!("[files.history.close] target={} handle={} seq={} ok=1",caller,handle,r.seq);Ok(0)},_=>Err(INVALID)
    }
   }
  }
 }
}
/// Liveness derives from the exact BOOT lifecycle subject, never a key or name.
pub fn live(caller:u32)->bool{
 let(slot,surface)=match caller{12=>(39,154),17=>(50,302),_=>return false};
 matches!(sex_pdx::app_lifecycle::catalog(slot),Ok(c) if c.is_subject(caller,surface,3))
  && sex_pdx::app_lifecycle::info(slot)==Err(sex_pdx::app_lifecycle::LIVE)
}
