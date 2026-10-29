//! Main-loop owned diagnostic jobs. No nested receive or HID drain.
use sex_pdx::spindle_probe as w;
pub const STALL:u32=1_048_576;const TOTAL:u32=16_777_216;const TICKS:u64=100_000;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Job{Stat(u8),List,Create([u8;16]),Delete(u8),Rename(u8,[u8;16]),Fill(u8,u32,bool),Truncate(u8,u32),Verify(u8,bool),Cancel}
impl Job{
 pub fn valid(self)->bool{match self{Self::Stat(id)|Self::Verify(id,_)=>id<15,Self::List|Self::Cancel=>true,Self::Delete(id)=>id>=3&&id<15,Self::Fill(id,n,_)|Self::Truncate(id,n)=>id>=3&&id<15&&n<=w::MAX_BYTES,Self::Create(name)=>valid_name(&name),Self::Rename(id,name)=>id>=3&&id<15&&valid_name(&name)}}
}
fn valid_name(name:&[u8;16])->bool{let n=name.iter().position(|&b|b==0).unwrap_or(16);n>0&&name[..n].iter().all(|b|(32..=126).contains(b))&&name[n..].iter().all(|&b|b==0)}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Outcome{Done,Failed,Uncertain,Cancelled}
#[derive(Clone,Copy,Debug)]pub struct Event{pub job:Job,pub outcome:Outcome,pub value:u32,pub hash:u64,pub error:u32}
#[derive(Clone,Copy)]enum Phase{Stat,GenStart,Info,Name,GenEnd,Begin,Write,Truncate,Read,Create,Delete,Rename,Sync,End,Abort}
pub struct Client{job:Option<Job>,seq:u32,req:Option<w::Request>,awaiting:bool,blocked:bool,dead:bool,started:bool,last:u64,ticks:u64,stall:u32,total:u32,phase:Phase,slot:u8,chunk:u8,generation:u32,names:[[u8;24];15],used:[bool;15],published:bool,object:u8,offset:u32,length:u32,value:u32,hash:u64,error:u32,last_job:Option<Job>,quiescent:bool}
impl Client{
 pub const fn new()->Self{Self{job:None,seq:0,req:None,awaiting:false,blocked:false,dead:false,started:false,last:0,ticks:0,stall:0,total:0,phase:Phase::Stat,slot:0,chunk:0,generation:0,names:[[0;24];15],used:[false;15],published:false,object:0,offset:0,length:0,value:0,hash:0,error:0,last_job:None,quiescent:false}}
 pub fn busy(&self)->bool{self.job.is_some()}
 pub fn fenced(&self)->bool{self.blocked||self.dead}
 pub fn status(&self)->&'static str{if self.dead{"denied"}else if self.blocked{"uncertain"}else if matches!(self.job,Some(Job::Cancel)){"cancelling"}else if self.busy(){"running"}else if self.quiescent{"quiescent"}else{"idle"}}
 pub fn cancel(&mut self)->bool{if self.dead||self.seq==w::SEQ_MAX||self.job==Some(Job::Cancel){return false}self.job=Some(Job::Cancel);self.phase=Phase::Abort;self.req=None;self.awaiting=false;self.started=false;self.blocked=false;self.quiescent=false;self.names=[[0;24];15];self.used=[false;15];self.published=false;true}
 pub fn retry(&mut self)->bool{if !self.quiescent||self.blocked(){return false}match self.last_job{Some(job@(Job::Stat(_)|Job::List|Job::Verify(_,_)))=>self.submit(job),_=>false}}
 pub fn blocked(&self)->bool{self.busy()||self.blocked||self.dead}
 pub fn submit(&mut self,job:Job)->bool{
  if self.blocked()||!job.valid()||job==Job::Cancel{return false}self.last_job=Some(job);self.quiescent=false;self.job=Some(job);self.phase=match job{Job::Stat(_)=>Phase::Stat,Job::List=>Phase::GenStart,Job::Create(_)=>Phase::Create,Job::Delete(_)=>Phase::Delete,Job::Rename(_,_)=>Phase::Rename,Job::Cancel=>Phase::Abort,_=>Phase::Begin};
  self.object=match job{Job::Stat(id)|Job::Delete(id)|Job::Rename(id,_)|Job::Fill(id,_,_)|Job::Truncate(id,_)|Job::Verify(id,_)=>id,_=>0};
  self.length=match job{Job::Fill(_,n,_)|Job::Truncate(_,n)=>n,_=>0};self.offset=0;self.value=0;self.hash=0xcbf29ce484222325;self.error=0;
  self.slot=0;self.chunk=0;self.names=[[0;24];15];self.used=[false;15];self.published=false;self.started=false;true
 }
 pub fn entry(&self,slot:usize)->Option<&[u8]>{if !self.published||slot>=15||!self.used[slot]{return None}let b=&self.names[slot];Some(&b[..b.iter().position(|&v|v==0).unwrap_or(24)])}
 fn next_slot(&mut self){self.slot+=1;self.phase=if self.slot==15{Phase::GenEnd}else{Phase::Info};self.chunk=0;}
 pub fn close(&mut self){self.names=[[0;24];15];self.used=[false;15];self.published=false;self.job=None;self.req=None;self.awaiting=false;self.dead=true;}
 fn finish(&mut self,outcome:Outcome,value:u32,error:u32)->Event{let job=self.job.take().unwrap();if outcome==Outcome::Uncertain{sex_pdx::serial_record!("[spindle.probe.deadline] stall={} total={} ticks={} phase={}",self.stall,self.total,self.ticks,self.phase as u8);}if outcome!=Outcome::Done{self.names=[[0;24];15];self.used=[false;15];self.published=false;}self.req=None;self.awaiting=false;self.started=false;self.blocked|=outcome==Outcome::Uncertain;Event{job,outcome,value,hash:self.hash,error}}
 fn fail(&mut self,e:u32)->Option<Event>{if matches!(self.phase,Phase::Abort|Phase::End){return Some(self.finish(Outcome::Uncertain,0,e))}if matches!(self.phase,Phase::Write|Phase::Read|Phase::Truncate|Phase::Sync){self.error=e;self.phase=Phase::End;None}else{Some(self.finish(Outcome::Failed,0,e))}}
 pub fn step(&mut self,now:u64,mut send:impl FnMut(w::Request)->Result<(),u64>)->Option<Event>{
  let job=self.job?;if !self.started{self.started=true;self.last=now;self.ticks=0;self.stall=0;self.total=0;}
  if now<self.last{return Some(self.finish(Outcome::Uncertain,0,w::TIMEOUT))}
  self.ticks=self.ticks.saturating_add(now-self.last);self.last=now;self.stall+=1;self.total+=1;
  if self.ticks>=TICKS||self.stall>=STALL||self.total>=TOTAL{return Some(self.finish(Outcome::Uncertain,0,w::TIMEOUT))}
  if self.awaiting{return None}
  if self.req.is_none(){if self.seq==w::SEQ_MAX{self.dead=true;return Some(self.finish(Outcome::Failed,0,w::STALE))}self.seq+=1;
   let mut object=self.object;let(mut offset,mut a,mut b)=(0,0,0);
   let kind=match self.phase{
    Phase::Stat=>w::STAT,Phase::GenStart|Phase::GenEnd=>{object=0;w::GENERATION},Phase::Info=>{object=self.slot;w::INFO},Phase::Name=>{object=self.slot;offset=self.chunk as u32;w::NAME},Phase::Begin=>w::BEGIN,
    Phase::Create|Phase::Rename=>{let name=match job{Job::Create(n)|Job::Rename(_,n)=>n,_=>unreachable!()};a=u64::from_le_bytes(name[..8].try_into().unwrap());b=u64::from_le_bytes(name[8..].try_into().unwrap());if matches!(self.phase,Phase::Create){w::CREATE}else{w::RENAME}},
    Phase::Delete=>w::DELETE,Phase::Write=>{offset=self.offset;let mask=match job{Job::Fill(_,_,full)=>if full{255}else{127},_=>unreachable!()};let mut data=[0;16];for(i,v)in data.iter_mut().enumerate(){let n=self.offset+i as u32;if n<self.length{*v=((n.wrapping_mul(37).wrapping_add(11))&mask)as u8;}}a=u64::from_le_bytes(data[..8].try_into().unwrap());b=u64::from_le_bytes(data[8..].try_into().unwrap());w::WRITE},
    Phase::Truncate=>{a=self.length as u64;w::TRUNCATE},Phase::Read=>{offset=self.offset;a=(self.length-self.offset).min(4)as u64;w::READ},Phase::Sync=>w::SYNC,Phase::End=>w::END,Phase::Abort=>{object=0;w::ABORT}};
   self.req=Some(w::Request{seq:self.seq,kind,object,offset,a,b});}
  match send(self.req.unwrap()){Ok(())=>self.awaiting=true,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>{},Err(_)=>{self.dead=true;return Some(self.finish(Outcome::Failed,0,w::DENIED))}}None
 }
 pub fn receive(&mut self,caller:u32,value:u64)->Option<Event>{
  if caller!=11||!self.awaiting{return None}let req=self.req?;let result=w::decode_reply(value,req.seq)?;self.stall=0;self.awaiting=false;self.req=None;
  let v=match result{Ok(v)=>v,Err(e)=>return self.fail(e)};
  match self.phase{
   Phase::Abort=>{if v!=0{return self.fail(w::INVALID)}self.blocked=false;self.quiescent=true;return Some(self.finish(Outcome::Cancelled,0,0))},
   Phase::Stat=>return Some(self.finish(Outcome::Done,v,0)),
   Phase::GenStart=>{self.generation=v;self.phase=Phase::Info},
   Phase::Info=>{if v>1{return self.fail(w::INVALID)}self.used[self.slot as usize]=v==1;if v==1{self.phase=Phase::Name}else{self.next_slot()}},
   Phase::Name=>{let off=self.chunk as usize*4;self.names[self.slot as usize][off..off+4].copy_from_slice(&v.to_le_bytes());self.chunk+=1;if self.chunk==6{self.next_slot()}},
   Phase::GenEnd=>{if v!=self.generation{return self.fail(w::STALE)}for i in 0..15{if self.used[i]{let n=self.names[i].iter().position(|&b|b==0).unwrap_or(24);if n==0||!self.names[i][..n].iter().all(|b|(32..=126).contains(b))||self.names[i][n..].iter().any(|&b|b!=0){return self.fail(w::INVALID)}}}self.published=true;return Some(self.finish(Outcome::Done,self.used.iter().filter(|&&v|v).count()as u32,0))},
   Phase::Begin=>{if v>w::MAX_BYTES{self.error=w::INVALID;self.phase=Phase::End}else{self.phase=match self.job.unwrap(){Job::Fill(_,n,_)=>if n==0{Phase::Truncate}else{Phase::Write},Job::Truncate(_,_)=>Phase::Truncate,Job::Verify(_,_)=>{self.length=v;self.value=v;if v==0{Phase::End}else{Phase::Read}},_=>unreachable!()}}},
   Phase::Create=>{if !(3..15).contains(&v){self.blocked=true;return self.fail(w::INVALID)}self.object=v as u8;self.value=v;self.phase=Phase::Sync},
   Phase::Delete|Phase::Rename=>{if v!=0{self.blocked=true;return self.fail(w::INVALID)}self.phase=Phase::Sync},
   Phase::Write=>{if v!=16{return self.fail(w::INVALID)}self.offset+=16;if self.offset>=self.length{self.phase=Phase::Truncate}},
   Phase::Truncate=>{if v!=self.length{return self.fail(w::INVALID)}self.value=v;self.phase=Phase::Sync},
   Phase::Read=>{let n=(self.length-self.offset).min(4);if n<4&&v>>(n*8)!=0{return self.fail(w::INVALID)}let full=match self.job.unwrap(){Job::Verify(_,full)=>full,_=>unreachable!()};for(i,&got)in v.to_le_bytes()[..n as usize].iter().enumerate(){let idx=self.offset+i as u32;self.hash=(self.hash^got as u64).wrapping_mul(0x100000001b3);if got!=((idx.wrapping_mul(37).wrapping_add(11))&if full{255}else{127})as u8{self.error=w::MISMATCH;}}self.offset+=n;if self.offset==self.length{self.phase=Phase::End}},
   Phase::Sync=>{if v!=0{return self.fail(w::INVALID)}self.phase=Phase::End},
   Phase::End=>{if v!=0{self.blocked=true;return self.fail(w::INVALID)}return Some(self.finish(if self.error==0{Outcome::Done}else{Outcome::Failed},self.value,self.error))}
  }None
 }
}

pub fn choose_owner(history_busy:bool,history_fenced:bool,probe_busy:bool,probe_fenced:bool)->u8{if probe_busy&&!history_fenced{2}else if history_busy&&!probe_fenced{1}else{0}}

pub struct Line{pub bytes:[u8;80],pub len:usize}
impl core::fmt::Write for Line{fn write_str(&mut self,s:&str)->core::fmt::Result{if self.len+s.len()>80{return Err(core::fmt::Error)}self.bytes[self.len..self.len+s.len()].copy_from_slice(s.as_bytes());self.len+=s.len();Ok(())}}
pub fn summary(e:&Event)->Line{
 use core::fmt::Write;let mut out=Line{bytes:[0;80],len:0};
 let _=match e.outcome{
  Outcome::Uncertain=>write!(out,"DISK UNCERTAIN / PROBE-CANCEL FIRST"),
  Outcome::Failed=>write!(out,"DISK FAILED / MAY HAVE PARTIAL WRITES"),
  Outcome::Cancelled=>write!(out,"DISK QUIESCENT / INSPECT PARTIAL WRITES"),
  Outcome::Done=>match e.job{
   Job::Stat(id)=>write!(out,"OBJECT {} / {} BYTES",id,e.value),
   Job::List=>write!(out,"{} OBJECTS / COMPLETE SNAPSHOT",e.value),
   Job::Create(_)=>write!(out,"CREATED OBJECT {}",e.value),
   Job::Delete(id)=>write!(out,"DELETED OBJECT {}",id),
   Job::Rename(id,_)=>write!(out,"RENAMED OBJECT {}",id),
   Job::Fill(id,_,_)|Job::Truncate(id,_)=>write!(out,"OBJECT {} / {} BYTES FLUSHED",id,e.value),
   Job::Verify(id,_)=>write!(out,"VERIFIED OBJECT {} / {} BYTES",id,e.value),
   Job::Cancel=>write!(out,"DISK QUIESCENT")
  }
 };out
}
