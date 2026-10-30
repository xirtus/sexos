//! Serialized bounded diagnostic job over the existing BOOT network route.
//! No caller pointers, socket descriptors, automatic accepted-request replay or
//! online-web claim. Exact reply sequences let Close remain independently live.
use sex_pdx::network_job as w;
pub const BYTES:usize=1025;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum View{Pending,Verified,Unavailable,Cancelled,Uncertain}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Step{Begin,Write,Start,Poll,Read,Cancel,Release,Done,Held}
pub struct Probe{step:Step,seq:u32,ticket:u32,offset:usize,length:usize,pending:Option<w::Request>,accepted:bool,
 deadline:u64,next_poll:u64,hash:u64,cancel:bool,timed_out:bool,retry_after_release:bool,view:View}
pub fn byte(offset:usize)->u8{(offset.wrapping_mul(17).wrapping_add(78))as u8}
impl Probe{
 pub const fn new()->Self{Self{step:Step::Begin,seq:1,ticket:0,offset:0,length:0,pending:None,accepted:false,deadline:0,next_poll:0,hash:0xcbf29ce484222325,cancel:false,timed_out:false,retry_after_release:false,view:View::Pending}}
 pub fn busy(&self)->bool{self.step!=Step::Done||self.ticket!=0}
 pub fn next_sequence(&self)->u32{self.seq}
 /// Handoff only after positive cleanup; the shared route fence cannot regress.
 pub fn restart_at(&mut self,seq:u32)->bool{
  if self.busy(){return false;}
  if seq<self.seq||seq==0||seq>w::MAX_SEQ-4{self.view=View::Unavailable;return false;}
  *self=Self::new();self.seq=seq;true
 }
 pub fn view(&self)->View{self.view}
 pub fn retry(&mut self){
  if self.busy(){self.retry_after_release=true;self.cancel=true;return;}
  let Some(seq)=self.seq.checked_add(1).filter(|n|*n<=w::MAX_SEQ)else{return;};
  if self.ticket!=0{self.seq=seq;self.step=Step::Release;self.retry_after_release=true;self.cancel=true;return;}
  *self=Self::new();self.seq=seq;
 }
 pub fn cancel(&mut self){self.retry_after_release=false;if self.busy(){self.cancel=true;if self.step==Step::Held&&self.ticket!=0&&self.seq<=w::MAX_SEQ{self.step=Step::Release;}}}
 fn stop(&mut self){self.pending=None;self.accepted=false;
  self.step=if self.ticket==0{Step::Done}else if self.seq<=w::MAX_SEQ{Step::Release}else{Step::Held};
  self.view=if self.step==Step::Held{View::Uncertain}else{View::Unavailable};
 }
 pub fn reply(&mut self,word:u64,now:u64){
  let Some(r)=self.pending else{return;};
  if !self.accepted||!matches!(word>>56,0x78|0xE8)||((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64{return;}
  self.pending=None;self.accepted=false;
  self.seq=r.seq+1;
  let value=match w::decode_reply(word,r.seq){Ok(v)=>v,Err(_)=>{
   self.view=View::Unavailable;
   if r.kind==w::RELEASE{self.step=Step::Held;self.view=View::Uncertain;}else if self.ticket==0{self.step=Step::Done;}else{self.step=Step::Release;}return;
  }};
  match r.kind{
   w::BEGIN=>{if value==0||value>w::MAX_TICKET{self.step=Step::Held;self.view=View::Uncertain;return;}self.ticket=value;self.offset=0;self.step=Step::Write;},
   w::WRITE=>{self.offset+=r.length as usize;if value!=self.offset as u32{self.stop();return;}if self.offset==BYTES{self.step=Step::Start;}},
   w::START=>{if value!=0{self.stop();return;}self.next_poll=now;self.step=Step::Poll;},
   w::STATUS=>{let Some((state,len))=w::decode_status(value)else{self.stop();return;};match state{
    w::State::Complete=>{if len!=BYTES{self.view=View::Unavailable;self.step=Step::Release;}else{self.length=len;self.offset=0;self.hash=0xcbf29ce484222325;self.step=Step::Read;}},
    w::State::Failed|w::State::Cancelled|w::State::Idle=>{self.view=View::Unavailable;self.step=Step::Release;},
    _=>{self.next_poll=now.saturating_add(25);},
   }},
   w::READ=>{for(i,b)in value.to_le_bytes()[..r.length as usize].iter().enumerate(){if *b!=byte(self.offset+i){self.view=View::Unavailable;self.step=Step::Release;return;}self.hash=(self.hash^*b as u64).wrapping_mul(0x100000001b3);}self.offset+=r.length as usize;if self.offset==self.length{self.view=View::Verified;self.step=Step::Release;sex_pdx::serial_record!("[browser.net.job.bytes] ticket={} length={} hash={:#x} exact=1 endpoint=controlled_peer tls=0 web=0",self.ticket,self.length,self.hash);}},
   w::CANCEL=>{if value!=0{self.stop();return;}self.view=View::Cancelled;self.step=Step::Release;},
   w::RELEASE=>{
    if value!=0{self.step=Step::Held;self.view=View::Uncertain;return;}
    self.step=Step::Done;sex_pdx::serial_record!("[browser.net.job.released] ticket={} view={:?} private_job=0",self.ticket,self.view);self.ticket=0;
    if self.retry_after_release{let seq=self.seq;*self=Self::new();self.seq=seq;}
   },
   _=>self.stop(),
  }
  if self.cancel&&self.ticket!=0&&r.kind!=w::CANCEL&&r.kind!=w::RELEASE{self.step=Step::Cancel;}
 }
 pub fn advance(&mut self,now:u64){
  if !self.busy()||self.step==Step::Held{return;}
  if self.pending.is_some()&&now>=self.deadline{
   if self.accepted{
    // Accepted admission is uncertain, never permission to forget its job or
    // replay the mutation. Retain exact correlation until its late receipt;
    // reconcile via CANCEL/RELEASE before honoring a queued retry intent.
    self.timed_out=true;self.cancel=true;self.view=View::Uncertain;self.deadline=u64::MAX;return;
   }
   // Queue Full is known pre-effect. A previously acquired job still needs a
   // fresh correlated RELEASE; an unaccepted BEGIN holds no service resource.
   self.pending=None;self.accepted=false;self.view=View::Unavailable;
   if self.ticket==0{self.step=Step::Done;return;}
   let Some(seq)=self.seq.checked_add(1).filter(|n|*n<=w::MAX_SEQ)else{self.stop();return;};
   self.seq=seq;self.step=Step::Release;
  }
  if self.pending.is_none(){
   if self.seq>w::MAX_SEQ||(self.ticket==0&&self.step==Step::Begin&&self.seq>w::MAX_SEQ-4){self.stop();return;}
   if self.ticket!=0&&self.seq>=w::MAX_SEQ-1&&!matches!(self.step,Step::Cancel|Step::Release){self.cancel=true;self.step=Step::Cancel;}
   if self.cancel&&self.ticket!=0&&self.step!=Step::Release{self.step=Step::Cancel;}
   if self.step==Step::Poll&&now<self.next_poll{return;}
   let(kind,offset,length,a)=match self.step{
    Step::Begin=>(w::BEGIN,0,BYTES,0),
    Step::Write=>{let len=(BYTES-self.offset).min(8);let mut bytes=[0;8];for(i,b)in bytes[..len].iter_mut().enumerate(){*b=byte(self.offset+i);}(w::WRITE,self.offset,len,u64::from_le_bytes(bytes))},
    Step::Start=>(w::START,0,0,0),Step::Poll=>(w::STATUS,0,0,0),
    Step::Read=>(w::READ,self.offset,(self.length-self.offset).min(4),0),
    Step::Cancel=>(w::CANCEL,0,0,0),Step::Release=>(w::RELEASE,0,0,0),Step::Done|Step::Held=>return,
   };
   self.pending=Some(w::Request{seq:self.seq,kind,offset:offset as u16,length:length as u16,ticket:if kind==w::BEGIN{0}else{self.ticket},a});self.deadline=now.saturating_add(2000);self.accepted=false;
  }
  if !self.accepted{let args=self.pending.unwrap().encode();let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,w::OP,args[0],args[1],args[2]);match status{0=>self.accepted=true,sex_pdx::ERR_SERVICE_NOT_READY=>{},_=>self.stop()}}
 }
}
/// Fixed64-byte status replacement; one retained8-byte display enqueue per turn.
/// The inherited preview is visibly marked cached/sample independently.
pub struct Status{offset:usize,desired:[u8;64],active:[u8;64],cursor:usize,failed:bool}
impl Status{
 pub fn new(offset:usize)->Self{let mut s=Self{offset,desired:[b' ';64],active:[0;64],cursor:64,failed:offset+64>256};s.set(View::Pending);s}
 pub fn set(&mut self,view:View){self.desired.fill(b' ');let text:&[u8]=match view{View::Pending=>b"\n\nPeer probe: checking\nR retry  C cancel",View::Verified=>b"\n\nPeer probe: bytes verified\nR retry  C cancel",View::Unavailable=>b"\n\nPeer probe: unavailable\nR retry  C cancel",View::Cancelled=>b"\n\nPeer probe: cancelled\nR retry  C cancel",View::Uncertain=>b"\n\nPeer probe: waiting receipt\nR retry  C cancel"};self.desired[..text.len()].copy_from_slice(text);}
 pub fn pending(&self)->bool{!self.failed&&(self.cursor<64||self.active!=self.desired)}
 pub fn advance(&mut self){if !self.pending(){return;}if self.cursor==64{self.active=self.desired;self.cursor=0;}
  let bytes:&[u8;8]=self.active[self.cursor..self.cursor+8].try_into().unwrap();let args=(self.offset+self.cursor)as u64|(8u64<<8)|(0x00CDD6F4u64<<32);
  let(result,_)=sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY,0xFB,300,u64::from_le_bytes(*bytes),args);match result{0=>self.cursor+=8,sex_pdx::ERR_SERVICE_NOT_READY=>{},_=>self.failed=true}
 }
}
