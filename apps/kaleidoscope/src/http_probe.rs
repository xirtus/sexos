// SPDX-License-Identifier: GPL-3.0-only
//! Explicit controlled HTTP intent over the existing owned network-job route.
//! Sequence custody is handed off by the app after the preceding TCP job's
//! positive RELEASE. An accepted timeout never authorizes mutation replay.
use sex_pdx::network_job as w;
use crate::http_wire;
const BLANK:[u8;64]=[b' ';64];
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View { Idle, Pending, Ready, Unavailable, Cancelled, Uncertain }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Step { Done, Begin, Write, Start, Poll, Read, Cancel, Release, Held }
pub struct Client {
    step:Step,seq:u32,ticket:u32,pending:Option<w::Request>,accepted:bool,
    deadline:u64,next_poll:u64,cancel:bool,view:View,terminal:View,
    request:[u8;http_wire::MAX_REQUEST],request_len:usize,written:usize,
    response:[u8;http_wire::MAX_RESPONSE],response_len:usize,read:usize,
    preview:[u8;64],body_len:usize,
    generation:u32,
}
impl Client {
    pub const fn new()->Self {
        Self {step:Step::Done,seq:1,ticket:0,pending:None,accepted:false,
            deadline:0,next_poll:0,cancel:false,view:View::Idle,terminal:View::Unavailable,
            request:[0;http_wire::MAX_REQUEST],request_len:0,written:0,
            response:[0;http_wire::MAX_RESPONSE],response_len:0,read:0,
            preview:BLANK,body_len:0,generation:0}
    }
    pub fn next_sequence(&self)->u32 { self.seq }
    pub fn busy(&self)->bool { self.step!=Step::Done || self.ticket!=0 }
    pub fn view(&self)->View { self.view }
    pub fn preview(&self)->&[u8;64] { if self.view==View::Ready {&self.preview}else{&BLANK} }
    pub fn body_length(&self)->usize { if self.view==View::Ready {self.body_len}else{0} }
    pub fn start(&mut self,next_seq:u32)->bool {
        self.start_with_generation(next_seq,0)
    }
    pub fn wait_for_resolution(&mut self) {
        if !self.busy(){self.scrub_raw();self.clear_preview();self.view=View::Pending;self.terminal=View::Unavailable;self.generation=0;}
    }
    pub fn start_resolved(&mut self,next_seq:u32,generation:u32)->bool {
        if self.busy(){return false;}
        if generation==0 || generation>w::MAX_TICKET {
            self.scrub_raw();self.clear_preview();self.view=View::Unavailable;self.terminal=View::Unavailable;self.generation=0;return false;
        }
        self.start_with_generation(next_seq,generation)
    }
    fn start_with_generation(&mut self,next_seq:u32,generation:u32)->bool {
        if self.busy() { return false; }
        if next_seq<self.seq || next_seq>w::MAX_SEQ-8 { self.scrub_raw();self.clear_preview();self.view=View::Unavailable;self.terminal=View::Unavailable;return false; }
        self.scrub_raw();self.clear_preview();self.seq=next_seq;
        self.generation=generation;
        self.request_len=http_wire::request(&mut self.request);self.written=0;self.read=0;self.response_len=0;
        self.pending=None;self.accepted=false;self.cancel=false;
        self.terminal=View::Unavailable;self.view=View::Pending;self.step=Step::Begin;true
    }
    fn scrub_raw(&mut self) { self.request.fill(0);self.response.fill(0); }
    fn clear_preview(&mut self) { self.preview=BLANK;self.body_len=0; }
    fn finished(&mut self) {
        self.scrub_raw();self.pending=None;self.accepted=false;self.step=Step::Done;
        self.request_len=0;self.written=0;self.response_len=0;self.read=0;
        self.generation=0;
        self.view=self.terminal;
        if self.view!=View::Ready { self.clear_preview(); }
    }
    pub fn cancel(&mut self) {
        self.cancel=true;self.terminal=View::Cancelled;self.clear_preview();self.scrub_raw();
        if !self.busy() { self.finished();return; }
        if self.step==Step::Begin && !self.accepted {
            if self.pending.take().is_some() { self.seq+=1; }
            self.finished();return;
        }
        if !self.accepted && self.pending.is_some_and(|r|r.kind!=w::RELEASE) {
            let r=self.pending.take().unwrap();self.seq=r.seq+1;
        }
        if self.step==Step::Held && self.ticket!=0 { self.step=Step::Release; }
    }
    fn rejected(&mut self,kind:u8) {
        self.scrub_raw();self.clear_preview();
        self.terminal=if self.cancel {View::Cancelled}else{View::Unavailable};
        self.view=if kind==w::RELEASE && self.ticket!=0 {View::Uncertain}else{View::Unavailable};
        if self.ticket==0 { self.finished(); }
        else { self.step=if kind==w::RELEASE {Step::Held}else{Step::Release}; }
    }
    fn validate_body(&mut self) {
        let parsed=match http_wire::parse(&self.response[..self.response_len]) {
            Ok(parsed)=>parsed,Err(_)=>{self.rejected(w::READ);return;},
        };
        let body=&self.response[parsed.body];
        self.body_len=parsed.content_length;self.preview=normalize(body);
        let mut hash=0xcbf29ce484222325u64;
        for &byte in body { hash=(hash^byte as u64).wrapping_mul(0x100000001b3); }
        sex_pdx::serial_record!("[browser.http.body] ticket={} status=200 length={} hash={:#x} controlled_peer=1 tls=0",self.ticket,self.body_len,hash);
        self.terminal=View::Ready;self.step=Step::Release;
    }
    pub fn reply(&mut self,word:u64,now:u64) {
        let Some(r)=self.pending else{return;};
        if !self.accepted || !matches!(word>>56,0x78|0xE8)
            || ((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64 { return; }
        self.pending=None;self.accepted=false;self.seq=r.seq+1;
        let value=match w::decode_reply(word,r.seq) {
            Ok(value)=>value,Err(_)=>{self.rejected(r.kind);return;},
        };
        // Late data receipts are reconciled by cleanup, never used to republish
        // cancelled bytes or restart a request whose authority is uncertain.
        if self.cancel && !matches!(r.kind,w::BEGIN|w::BEGIN_RESOLVED|w::CANCEL|w::RELEASE) {
            self.step=Step::Cancel;return;
        }
        match r.kind {
            w::BEGIN|w::BEGIN_RESOLVED=>{
                if value==0 || value>w::MAX_TICKET {
                    self.scrub_raw();self.clear_preview();self.view=View::Uncertain;self.step=Step::Held;return;
                }
                self.ticket=value;self.step=Step::Write;
            }
            w::WRITE=>{
                let end=r.offset as usize+r.length as usize;
                if value as usize!=end || end>self.request_len {self.rejected(r.kind);return;}
                self.written=end;self.step=if end==self.request_len {Step::Start}else{Step::Write};
            }
            w::START=>{
                if value!=0 {self.rejected(r.kind);return;}
                self.step=Step::Poll;self.next_poll=now;
            }
            w::STATUS=>match w::decode_status(value) {
                Some((w::State::Complete,len)) if len!=0 && len<=http_wire::MAX_RESPONSE=>{
                    self.response_len=len;self.read=0;self.step=Step::Read;
                }
                Some((w::State::Staging|w::State::Connecting|w::State::Sending|w::State::Receiving,_))=>{
                    self.next_poll=now.saturating_add(25);self.step=Step::Poll;
                }
                _=>{self.rejected(r.kind);return;}
            },
            w::READ=>{
                let end=self.read+r.length as usize;
                if r.offset as usize!=self.read || end>self.response_len {self.rejected(r.kind);return;}
                self.response[self.read..end].copy_from_slice(&value.to_le_bytes()[..r.length as usize]);
                self.read=end;
                if end==self.response_len { self.validate_body(); }
            }
            w::CANCEL=>{
                if value!=0 {self.rejected(r.kind);return;}
                self.scrub_raw();self.clear_preview();self.step=Step::Release;
            }
            w::RELEASE=>{
                if value!=0 {self.rejected(r.kind);return;}
                self.ticket=0;self.finished();
                sex_pdx::serial_record!("[browser.http.released] ticket={} view={:?} private_job=0",r.ticket,self.view);
                return;
            }
            _=>{self.rejected(r.kind);return;}
        }
        if self.cancel && self.ticket!=0 && !matches!(self.step,Step::Release|Step::Held) {self.step=Step::Cancel;}
    }
    pub fn advance(&mut self,now:u64) {
        if !self.busy() || self.step==Step::Held {return;}
        if self.pending.is_some() && now>=self.deadline {
            if self.accepted {
                self.cancel=true;self.terminal=View::Cancelled;self.view=View::Uncertain;
                self.scrub_raw();self.clear_preview();self.deadline=u64::MAX;return;
            }
            let kind=self.pending.take().unwrap().kind;self.seq+=1;self.accepted=false;
            self.rejected(kind);
            if !self.busy() || self.step==Step::Held {return;}
        }
        if self.pending.is_none() {
            if self.cancel && self.ticket!=0 && self.step!=Step::Release {self.step=Step::Cancel;}
            if self.step==Step::Poll && now<self.next_poll {return;}
            if self.seq>=w::MAX_SEQ-1 && !matches!(self.step,Step::Cancel|Step::Release) {
                self.scrub_raw();self.clear_preview();self.terminal=View::Unavailable;self.view=View::Unavailable;self.step=Step::Cancel;
            }
            if self.seq>w::MAX_SEQ {self.scrub_raw();self.clear_preview();self.view=View::Uncertain;self.step=Step::Held;return;}
            let(kind,offset,length,a)=match self.step {
                Step::Begin=>if self.generation!=0{(w::BEGIN_RESOLVED,0,self.request_len,self.generation as u64)}else{(w::BEGIN,0,self.request_len,0)},
                Step::Write=>{
                    let length=(self.request_len-self.written).min(8);let mut bytes=[0;8];
                    bytes[..length].copy_from_slice(&self.request[self.written..self.written+length]);
                    (w::WRITE,self.written,length,u64::from_le_bytes(bytes))
                }
                Step::Start=>(w::START,0,0,0),Step::Poll=>(w::STATUS,0,0,0),
                Step::Read=>(w::READ,self.read,(self.response_len-self.read).min(4),0),
                Step::Cancel=>(w::CANCEL,0,0,0),Step::Release=>(w::RELEASE,0,0,0),Step::Done|Step::Held=>return,
            };
            self.pending=Some(w::Request {seq:self.seq,kind,offset:offset as u16,length:length as u16,
                ticket:if matches!(kind,w::BEGIN|w::BEGIN_RESOLVED) {0}else{self.ticket},a});
            self.deadline=now.saturating_add(2000);self.accepted=false;
        }
        if !self.accepted {
            let r=self.pending.unwrap();let args=r.encode();
            let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,w::OP,args[0],args[1],args[2]);
            match status {
                0=>self.accepted=true,sex_pdx::ERR_SERVICE_NOT_READY=>{},
                _=>{self.pending=None;self.seq=r.seq+1;self.rejected(r.kind);}
            }
        }
    }
    #[cfg(test)] pub fn raw_scrubbed(&self)->bool {
        self.request.iter().all(|b|*b==0) && self.response.iter().all(|b|*b==0)
    }
}

fn normalize(body:&[u8])->[u8;64] {
    let mut out=BLANK;let mut length=0;let mut space=false;let mut truncated=false;
    for &byte in body {
        let byte=match byte {0..=32|127=>b' ',33..=126=>byte,_=>b'?'};
        if byte==b' ' {space=length!=0;continue;}
        if space {
            if length<out.len() {out[length]=b' ';length+=1;}else{truncated=true;}
            space=false;
        }
        if length<out.len() {out[length]=byte;length+=1;}else{truncated=true;}
    }
    if truncated {out[61..].copy_from_slice(b"...");}
    out
}
