//! One admitted exchange at a time. No inline reply waits or HID consumption.
use sex_pdx::spindle_history as w;
use super::History;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Phase{Idle,OpenSave,Write,Commit,CloseSave,OpenLoad,Stat,Read,CloseLoad,CloseFail(u32),Abort,Uncertain,Poison}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Outcome{Saved,Loaded,Missing,Conflict,Failed,Uncertain,Aborted}
#[derive(Clone,Copy,Debug)]pub struct Event{pub outcome:Outcome,pub handle:u32,pub bytes:usize,pub count:usize,pub hash:u64,pub error:u32}
// Tick units are kernel ticks, not wall-clock milliseconds. Cooperative turns
// bound stalls even when the platform counter does not advance.
pub const STALL_TURNS:u32=65_536;pub const TOTAL_TURNS:u32=1_048_576;pub const TICK_BUDGET:u64=10_000;pub const TIMEOUT:u32=8;
#[derive(Clone,Copy)]enum Action{Save,Load}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Status{Idle,Queued,Loading,Saving,Cancelling,NeedsRetry,Denied,RetryReady}
impl Status{pub fn name(self)->&'static str{match self{Self::Idle=>"idle",Self::Queued=>"queued",Self::Loading=>"loading",Self::Saving=>"saving",Self::Cancelling=>"cancelling",Self::NeedsRetry=>"uncertain",Self::Denied=>"denied",Self::RetryReady=>"retry_ready"}}pub fn label(self)->&'static str{match self{Self::Idle=>"History ready.",Self::Queued=>"History save queued.",Self::Loading=>"History loading; awaiting validation.",Self::Saving=>"History saving; awaiting receipts.",Self::Cancelling=>"History cancelling; awaiting quiescence.",Self::NeedsRetry=>"History uncertain; history-cancel first.",Self::Denied=>"History denied; incarnation cannot retry.",Self::RetryReady=>"History quiescent; history-retry available."}}}
pub struct Client{owner:u32,phase:Phase,seq:u32,request:Option<w::Request>,awaiting:bool,handle:u32,offset:usize,len:usize,count:usize,buf:[u8;w::MAX],revision:u64,dirty:bool,started:bool,last_tick:u64,ticks:u64,stall:u32,turns:u32,action:Option<Action>,retry_ready:bool}
impl Client{
 pub const fn new()->Self{Self{owner:0,phase:Phase::Idle,seq:0,request:None,awaiting:false,handle:0,offset:0,len:0,count:0,buf:[0;w::MAX],revision:0,dirty:false,started:false,last_tick:0,ticks:0,stall:0,turns:0,action:None,retry_ready:false}}
 pub fn init(&mut self,owner:u32){assert!(matches!(owner,12|17));self.owner=owner;}
 pub fn busy(&self)->bool{!matches!(self.phase,Phase::Idle|Phase::Uncertain|Phase::Poison)||self.dirty}
 pub fn status(&self)->Status{match self.phase{Phase::Uncertain=>Status::NeedsRetry,Phase::Poison=>Status::Denied,Phase::Abort=>Status::Cancelling,Phase::Idle if self.dirty=>Status::Queued,Phase::Idle if self.retry_ready=>Status::RetryReady,Phase::Idle=>Status::Idle,Phase::OpenLoad|Phase::Stat|Phase::Read|Phase::CloseLoad=>Status::Loading,_=>Status::Saving}}
 pub fn retry(&mut self,h:&History)->bool{if self.phase!=Phase::Idle||self.dirty||!self.retry_ready{return false}let Some(action)=self.action else{return false};self.retry_ready=false;match action{Action::Save=>self.save(),Action::Load=>self.load(h)}}
 pub fn blocked(&self)->bool{self.busy()||matches!(self.phase,Phase::Uncertain|Phase::Poison)}
 pub fn mark_dirty(&mut self){if !matches!(self.phase,Phase::Abort|Phase::Uncertain|Phase::Poison){self.retry_ready=false;self.dirty=true}}
 pub fn save(&mut self)->bool{if matches!(self.phase,Phase::Abort|Phase::Uncertain|Phase::Poison){return false}self.retry_ready=false;self.dirty=true;true}
 /// Quiesce only this incarnation's service session. ACK is not rollback.
 pub fn cancel(&mut self)->bool{if matches!(self.phase,Phase::Abort|Phase::Poison)||self.seq==w::SEQ_MAX{return false}self.retry_ready=false;self.phase=Phase::Abort;self.request=None;self.awaiting=false;self.started=false;self.dirty=false;self.buf.fill(0);true}
 pub fn load(&mut self,h:&History)->bool{if self.phase!=Phase::Idle||self.dirty{return false}self.retry_ready=false;self.action=Some(Action::Load);self.revision=h.revision;self.phase=Phase::OpenLoad;self.len=0;self.count=0;self.buf.fill(0);true}
 fn event(&self,outcome:Outcome,error:u32)->Event{Event{outcome,handle:self.handle,bytes:self.len,count:self.count,hash:w::hash(&self.buf[..self.len]),error}}
 fn finish(&mut self,outcome:Outcome,error:u32)->Event{let mut event=self.event(outcome,error);if outcome==Outcome::Aborted{event.bytes=0;event.count=0;event.hash=0;}self.retry_ready=outcome==Outcome::Aborted&&self.action.is_some();self.phase=Phase::Idle;self.started=false;self.handle=0;self.request=None;self.awaiting=false;self.buf.fill(0);event}
 fn fail(&mut self,error:u32)->Option<Event>{self.request=None;self.awaiting=false;
  if self.phase==Phase::Abort{let mut e=self.uncertain();e.error=error;return Some(e)}
  if self.handle!=0&&!matches!(self.phase,Phase::CloseSave|Phase::CloseLoad|Phase::CloseFail(_)){self.phase=Phase::CloseFail(error);None}
  else{let e=self.finish(Outcome::Failed,error);if e.handle!=0{self.phase=Phase::Poison;self.dirty=false;}Some(e)}
 }
 pub fn step(&mut self,h:&History,send:impl FnMut(w::Request)->Result<(),u64>)->Option<Event>{self.step_at(h,0,send)}
 fn uncertain(&mut self)->Event{let e=self.event(Outcome::Uncertain,TIMEOUT);self.phase=Phase::Uncertain;self.dirty=false;self.request=None;self.awaiting=false;self.buf.fill(0);self.started=false;e}
 pub fn step_at(&mut self,h:&History,now:u64,mut send:impl FnMut(w::Request)->Result<(),u64>)->Option<Event>{
  if self.phase==Phase::Idle&&self.dirty{self.dirty=false;self.action=Some(Action::Save);match encode(h,self.owner,&mut self.buf){Ok((len,count))=>{self.len=len;self.count=count;self.offset=0;self.phase=Phase::OpenSave},Err(e)=>return self.fail(e)}}
  if matches!(self.phase,Phase::Idle|Phase::Uncertain|Phase::Poison){return None}
  if !self.started{self.started=true;self.last_tick=now;self.ticks=0;self.stall=0;self.turns=0;}
  if now<self.last_tick{return Some(self.uncertain())}
  self.ticks=self.ticks.saturating_add(now-self.last_tick);self.last_tick=now;self.stall=self.stall.saturating_add(1);self.turns=self.turns.saturating_add(1);
  if self.ticks>=TICK_BUDGET||self.stall>=STALL_TURNS||self.turns>=TOTAL_TURNS{return Some(self.uncertain())}
  if self.awaiting{return None}
  if self.request.is_none(){
   if self.seq==w::SEQ_MAX{let e=self.event(Outcome::Failed,w::STALE);self.phase=Phase::Poison;self.dirty=false;return Some(e)}self.seq+=1;
   let(kind,a,b)=match self.phase{
    Phase::Abort=>(w::ABORT,0,0),Phase::OpenSave=>(w::OPEN_SAVE,0,0),Phase::OpenLoad=>(w::OPEN_LOAD,0,0),
    Phase::Write=>{let n=(self.len-self.offset).min(4);let mut b=[0;8];b[..n].copy_from_slice(&self.buf[self.offset..self.offset+n]);(w::WRITE,w::io(self.handle,self.offset,n),u64::from_le_bytes(b))},
    Phase::Commit=>(w::COMMIT,w::io(self.handle,self.len,0),0),Phase::Stat=>(w::STAT,w::io(self.handle,0,0),0),
    Phase::Read=>(w::READ,w::io(self.handle,self.offset,(self.len-self.offset).min(4)),0),
    Phase::CloseSave|Phase::CloseLoad|Phase::CloseFail(_)=>(w::CLOSE,w::io(self.handle,0,0),0),_=>return None};
   self.request=Some(w::Request{seq:self.seq,kind,a,b});
  }
  match send(self.request.unwrap()){
   Ok(())=>self.awaiting=true,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>{},Err(_)=>{let e=self.event(Outcome::Failed,w::DENIED);self.phase=Phase::Poison;self.dirty=false;return Some(e)}
  }None
 }
 pub fn receive(&mut self,value:u64,h:&mut History)->Option<Event>{
  if !self.awaiting{return None}let req=self.request?;let result=w::decode_reply(value,req.seq)?;
  self.awaiting=false;self.request=None;self.stall=0;let value=match result{Ok(v)=>v,Err(e)=>{if e==w::MISSING&&self.phase==Phase::OpenLoad{return Some(self.finish(Outcome::Missing,e))}return self.fail(e)}};
  match self.phase{
   Phase::Abort=>{if value!=0{return self.fail(w::INVALID)}return Some(self.finish(Outcome::Aborted,0))},
   Phase::OpenSave|Phase::OpenLoad=>{if value==0{return self.fail(w::INVALID)}self.handle=value;self.phase=if self.phase==Phase::OpenSave{Phase::Write}else{Phase::Stat};},
   Phase::Write=>{let n=(self.len-self.offset).min(4);if value!=n as u32{return self.fail(w::INVALID)}self.offset+=n;if self.offset==self.len{self.phase=Phase::Commit}},
   Phase::Commit=>{if value!=self.len as u32{return self.fail(w::INVALID)}self.phase=Phase::CloseSave},
   Phase::Stat=>{if(value as usize)<w::HEADER||value as usize>w::MAX{return self.fail(w::CORRUPT)}self.len=value as usize;self.offset=0;self.phase=Phase::Read},
   Phase::Read=>{let n=(self.len-self.offset).min(4);if n<4&&value>>(n*8)!=0{return self.fail(w::CORRUPT)}self.buf[self.offset..self.offset+n].copy_from_slice(&value.to_le_bytes()[..n]);self.offset+=n;
    if self.offset==self.len{match w::validate(&self.buf[..self.len],self.owner){Ok(count)=>self.count=count,Err(e)=>return self.fail(e)}self.phase=Phase::CloseLoad}},
   Phase::CloseSave=>{if value!=0{return self.fail(w::INVALID)}return Some(self.finish(Outcome::Saved,0))},
   Phase::CloseLoad=>{if value!=0{return self.fail(w::INVALID)}if h.revision!=self.revision{return Some(self.finish(Outcome::Conflict,w::STALE))}
    // Validation has completed; publish the entire replacement history now.
    h.clear();let mut off=w::HEADER;for _ in 0..self.count{let n=u16::from_le_bytes([self.buf[off],self.buf[off+1]])as usize;off+=2;h.push(&self.buf[off..off+n]);off+=n;}
    return Some(self.finish(Outcome::Loaded,0))},
   Phase::CloseFail(e)=>{if value!=0{return self.fail(w::INVALID)}return Some(self.finish(Outcome::Failed,e))},_=>return None,
  }None
 }
}
/// Keep the newest complete entries that fit; encode oldest first so restore
/// reconstructs the ring and newest-first navigation without reversing it.
pub fn encode(h:&History,owner:u32,out:&mut[u8;w::MAX])->Result<(usize,usize),u32>{
 out.fill(0);let mut count=0;let mut len=w::HEADER;
 for i in 0..(h.total as usize).min(super::MAX_HISTORY){let entry=h.get(i).ok_or(w::CORRUPT)?;if entry.is_empty()||!entry.iter().all(|b|(32..=126).contains(b)){return Err(w::CORRUPT)}if len+2+entry.len()>w::MAX{break}len+=2+entry.len();count+=1;}
 let mut off=w::HEADER;for i in(0..count).rev(){let e=h.get(i).unwrap();out[off..off+2].copy_from_slice(&(e.len()as u16).to_le_bytes());off+=2;out[off..off+e.len()].copy_from_slice(e);off+=e.len();}
 if !w::seal(out,owner,count,len){return Err(w::CORRUPT)}Ok((len,count))
}
