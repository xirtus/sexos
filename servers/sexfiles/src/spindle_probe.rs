//! Exact live BOOT subjects; service-owned diagnostic sessions are not caps.
use sex_pdx::spindle_probe as w;
pub trait Backend{
 fn invoke(&mut self,caller:u32,r:w::Request)->Result<u32,u32>;
 fn generation(&mut self,caller:u32)->Result<u32,u32>{self.invoke(caller,w::Request{seq:1,kind:w::GENERATION,object:0,offset:0,a:0,b:0})}
}
#[derive(Clone,Copy)]struct Session{last:Option<w::Request>,receipt:u64,pending:Option<u64>,dead:bool,lease:Option<(u8,u32)>}
impl Session{const EMPTY:Self=Self{last:None,receipt:0,pending:None,dead:false,lease:None};}
pub struct Service{sessions:[Session;2]}
impl Service{
 pub const fn new()->Self{Self{sessions:[Session::EMPTY;2]}}
 fn index(caller:u32)->Option<usize>{match caller{12=>Some(0),17=>Some(1),_=>None}}
 pub fn retire(&mut self,caller:u32){if let Some(i)=Self::index(caller){self.sessions[i]=Session{dead:true,..Session::EMPTY};sex_pdx::serial_record!("[files.probe.retire] target={} lease=0 pending=0 terminal=1",caller);}}
 pub fn pending(&self)->bool{self.sessions.iter().any(|s|s.pending.is_some())}
 pub fn poll(&mut self,mut send:impl FnMut(u32,u64)->u64){for(i,s)in self.sessions.iter_mut().enumerate(){if let Some(v)=s.pending{if send(if i==0{12}else{17},v)==0{s.pending=None;}}}}
 fn execute(s:&mut Session,caller:u32,r:w::Request,b:&mut impl Backend)->Result<u32,u32>{
  if r.kind==w::ABORT{if r.object!=0||r.offset!=0||r.a!=0||r.b!=0{return Err(w::INVALID)}s.lease=None;return Ok(0)}
  if r.kind==w::END{if r.offset!=0||r.a!=0||r.b!=0||s.lease.map(|v|v.0)!=Some(r.object){return Err(w::DENIED)}s.lease=None;return Ok(0)}
  if matches!(r.kind,w::READ|w::WRITE|w::TRUNCATE|w::SYNC){
   let Some((id,generation))=s.lease else{return Err(w::DENIED)};
   if r.object!=id||b.generation(caller)?!=generation{return Err(w::STALE)}
   let result=b.invoke(caller,r)?;s.lease=Some((id,b.generation(caller)?));return Ok(result)
  }
  if s.lease.is_some(){return Err(w::BUSY)}
  let result=b.invoke(caller,r)?;
  if matches!(r.kind,w::BEGIN|w::CREATE|w::DELETE|w::RENAME){
   let id=if r.kind==w::CREATE{u8::try_from(result).map_err(|_|w::INVALID)?}else{r.object};
   s.lease=Some((id,b.generation(caller)?));
   if r.kind==w::BEGIN{sex_pdx::serial_record!("[files.probe.bound] target={} object={} generation={}",caller,id,s.lease.unwrap().1);}
  }Ok(result)
 }
 pub fn handle(&mut self,caller:u32,live:bool,r:w::Request,b:&mut impl Backend)->Option<u64>{
  let Some(i)=Self::index(caller)else{return Some(w::reply(r.seq,Err(w::DENIED)))};
  let s=&mut self.sessions[i];if s.dead||!live{return Some(w::reply(r.seq,Err(w::DENIED)))}if s.pending.is_some(){if r.kind!=w::ABORT||r.object!=0||r.offset!=0||r.a!=0||r.b!=0||s.last.map_or(true,|old|r.seq<=old.seq){return None}s.pending=None;}
  let result=if let Some(old)=s.last{if r==old{s.receipt}else if r.seq<=old.seq{w::reply(r.seq,Err(w::STALE))}else{w::reply(r.seq,Self::execute(s,caller,r,b))}}else{w::reply(r.seq,Self::execute(s,caller,r,b))};
  if s.last.map_or(true,|old|r.seq>old.seq){s.last=Some(r);s.receipt=result;}s.pending=Some(result);None
 }
}
