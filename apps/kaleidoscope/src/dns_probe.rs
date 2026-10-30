// SPDX-License-Identifier: GPL-3.0-only
//! Explicit-user DNS intent on the held BOOT route. Accepted requests retain
//! their correlation until a matching receipt; a timer never authorizes replay.
use sex_pdx::network_dns as w;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View { Idle, Pending, Answer([u8;4]), Unavailable, Cancelled, Uncertain }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Step { Done, Begin, Poll, Result, Cancel, Release, Held }
pub struct Client {
    step:Step, seq:u32, ticket:u32, pending:Option<w::Request>, accepted:bool,
    deadline:u64, next_poll:u64, cancel:bool, retry:bool, view:View,
}
impl Client {
    pub const fn new()->Self {
        Self { step:Step::Done,seq:1,ticket:0,pending:None,accepted:false,
            deadline:0,next_poll:0,cancel:false,retry:false,view:View::Idle }
    }
    pub fn busy(&self)->bool { self.step!=Step::Done || self.ticket!=0 }
    pub fn view(&self)->View { self.view }
    pub fn start(&mut self) {
        if self.busy() {
            self.retry=true;self.cancel=true;
            if self.step==Step::Held && self.ticket!=0 { self.step=Step::Release; }
            return;
        }
        self.begin_intent();
    }
    fn begin_intent(&mut self) {
        // Leave room for positive ticket cleanup even at sequence exhaustion.
        if self.seq>w::MAX_SEQ-4 { self.view=View::Unavailable;return; }
        self.step=Step::Begin;self.cancel=false;self.retry=false;
        self.pending=None;self.accepted=false;self.view=View::Pending;
    }
    pub fn cancel(&mut self) {
        self.retry=false;
        if !self.busy() { self.view=View::Cancelled;return; }
        self.cancel=true;
        if self.ticket==0 && !self.accepted && self.step==Step::Begin {
            if self.pending.take().is_some() { self.seq+=1; }
            self.view=View::Cancelled;self.finished();return;
        }
        if self.step==Step::Held && self.ticket!=0 { self.step=Step::Release; }
    }
    fn finished(&mut self) {
        self.step=Step::Done;self.pending=None;self.accepted=false;self.cancel=false;
        if self.retry { self.begin_intent(); }
    }
    fn rejected(&mut self,kind:u8) {
        self.view=if kind==w::RELEASE && self.ticket!=0 { View::Uncertain } else { View::Unavailable };
        if self.ticket==0 { self.finished(); }
        else { self.step=if kind==w::RELEASE { Step::Held } else { Step::Release }; }
    }
    pub fn reply(&mut self,word:u64,now:u64) {
        let Some(r)=self.pending else{return;};
        if !self.accepted || !matches!(word>>56,0x79|0xE9)
            || ((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64 { return; }
        self.pending=None;self.accepted=false;
        // The internal exhausted sentinel MAX_SEQ+1 is never sent on the wire.
        self.seq=r.seq+1;
        let value=match w::decode_reply(word,r.seq) {
            Ok(v)=>v,Err(_)=>{self.rejected(r.kind);return;},
        };
        match r.kind {
            w::BEGIN=>{
                if value==0 || value>w::MAX_TICKET {
                    self.view=View::Uncertain;self.step=Step::Held;return;
                }
                self.ticket=value;self.step=Step::Poll;self.next_poll=now;
            }
            w::STATUS=>match w::state(value) {
                Some(w::State::Waiting)=>{self.step=Step::Poll;self.next_poll=now.saturating_add(25);}
                Some(w::State::Complete)=>self.step=Step::Result,
                Some(w::State::Failed|w::State::Idle|w::State::Cancelled)|None=>{
                    self.view=View::Unavailable;self.step=Step::Release;
                }
            },
            w::RESULT=>{
                if !self.cancel {
                    let address=value.to_be_bytes();self.view=View::Answer(address);
                    sex_pdx::serial_record!("[browser.dns.answer] ticket={} address={}.{}.{}.{} controlled_peer=1 tls=0 web=0",self.ticket,address[0],address[1],address[2],address[3]);
                }
                self.step=Step::Release;
            }
            w::CANCEL=>{
                if value!=0 { self.rejected(r.kind);return; }
                self.view=View::Cancelled;self.step=Step::Release;
            }
            w::RELEASE=>{
                if value!=0 { self.rejected(r.kind);return; }
                if self.cancel { self.view=View::Cancelled; }
                sex_pdx::serial_record!("[browser.dns.released] ticket={} view={:?} private_job=0",self.ticket,self.view);
                self.ticket=0;self.finished();return;
            }
            _=>{self.rejected(r.kind);return;}
        }
        if self.cancel && self.ticket!=0 && !matches!(self.step,Step::Release|Step::Held) {
            self.step=Step::Cancel;
        }
    }
    pub fn advance(&mut self,now:u64) {
        if !self.busy() || self.step==Step::Held { return; }
        if self.pending.is_some() && now>=self.deadline {
            if self.accepted {
                self.cancel=true;self.view=View::Uncertain;self.deadline=u64::MAX;return;
            }
            // Full is pre-effect. Abandon only unaccepted admission, and retain
            // the held ticket until a separately correlated RELEASE succeeds.
            let kind=self.pending.take().unwrap().kind;self.seq+=1;self.accepted=false;
            self.rejected(kind);
            if !self.busy() || self.step==Step::Held { return; }
        }
        if self.pending.is_none() {
            if self.cancel && self.ticket!=0 && self.step!=Step::Release { self.step=Step::Cancel; }
            if self.step==Step::Poll && now<self.next_poll { return; }
            if self.seq>=w::MAX_SEQ-1 && matches!(self.step,Step::Poll|Step::Result) {
                self.cancel=true;self.view=View::Unavailable;self.step=Step::Cancel;
            }
            if self.seq>w::MAX_SEQ { self.view=View::Uncertain;self.step=Step::Held;return; }
            let kind=match self.step {
                Step::Begin=>w::BEGIN,Step::Poll=>w::STATUS,Step::Result=>w::RESULT,
                Step::Cancel=>w::CANCEL,Step::Release=>w::RELEASE,Step::Done|Step::Held=>return,
            };
            self.pending=Some(w::Request { seq:self.seq,kind,ticket:if kind==w::BEGIN {0}else{self.ticket} });
            self.deadline=now.saturating_add(2000);self.accepted=false;
        }
        if !self.accepted {
            let r=self.pending.unwrap();let args=r.encode();
            let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,w::OP,args[0],args[1],args[2]);
            match status {
                0=>self.accepted=true,
                sex_pdx::ERR_SERVICE_NOT_READY=>{},
                _=>{self.pending=None;self.seq=r.seq+1;self.rejected(r.kind);}
            }
        }
    }
}

/// Whole64-byte replacement, one retained eight-byte display enqueue per turn.
/// Updates coalesce to the latest view after the current immutable paint drains.
pub struct Status { offset:usize,desired:[u8;64],active:[u8;64],cursor:usize,failed:bool }
impl Status {
    pub fn new(offset:usize)->Self {
        let mut s=Self {offset,desired:[b' ';64],active:[0;64],cursor:64,
            failed:offset.checked_add(64).is_none_or(|end|end>256)};
        s.set(View::Idle);s
    }
    pub fn set(&mut self,view:View) {
        self.desired.fill(b' ');let mut at=0;
        append(&mut self.desired,&mut at,b"\n\nControlled DNS: ");
        match view {
            View::Idle=>append(&mut self.desired,&mut at,b"idle"),
            View::Pending=>append(&mut self.desired,&mut at,b"checking"),
            View::Answer(address)=>for(index,byte)in address.iter().enumerate() {
                if index!=0 { append(&mut self.desired,&mut at,b"."); }
                decimal(&mut self.desired,&mut at,*byte);
            },
            View::Unavailable=>append(&mut self.desired,&mut at,b"unavailable"),
            View::Cancelled=>append(&mut self.desired,&mut at,b"cancelled"),
            View::Uncertain=>append(&mut self.desired,&mut at,b"waiting receipt"),
        }
        append(&mut self.desired,&mut at,b"\nD query  X cancel");
    }
    pub fn pending(&self)->bool { !self.failed && (self.cursor<64 || self.active!=self.desired) }
    pub fn advance(&mut self) {
        if !self.pending() { return; }
        if self.cursor==64 { self.active=self.desired;self.cursor=0; }
        let bytes:&[u8;8]=self.active[self.cursor..self.cursor+8].try_into().unwrap();
        let args=(self.offset+self.cursor)as u64 | (8u64<<8) | (0x00CDD6F4u64<<32);
        let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY,0xFB,300,u64::from_le_bytes(*bytes),args);
        match status {0=>self.cursor+=8,sex_pdx::ERR_SERVICE_NOT_READY=>{},_=>self.failed=true}
    }
}
fn append(out:&mut[u8;64],at:&mut usize,bytes:&[u8]) {
    out[*at..*at+bytes.len()].copy_from_slice(bytes);*at+=bytes.len();
}
fn decimal(out:&mut[u8;64],at:&mut usize,value:u8) {
    if value>=100 { out[*at]=b'0'+value/100;*at+=1; }
    if value>=10 { out[*at]=b'0'+(value/10)%10;*at+=1; }
    out[*at]=b'0'+value%10;*at+=1;
}
