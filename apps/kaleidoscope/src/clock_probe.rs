// SPDX-License-Identifier: GPL-3.0-only
//! Explicit read-only UTC queries on the held route. Ready is one historical
//! query snapshot, not a continuously trusted clock. No retries after admission.
use sex_pdx::certificate_time as w;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View {Idle,Pending,Ready(u32),Unavailable,Cancelled,Uncertain,Rollback}
pub struct Client {
    seq:u32,pending:Option<w::Request>,accepted:bool,deadline:Option<u64>,last:u64,
    cancelled:bool,expired:bool,closed:bool,rollback:bool,view:View,sample:Option<u32>,
}
impl Client {
    pub const fn new()->Self {Self {seq:1,pending:None,accepted:false,deadline:None,last:0,
        cancelled:false,expired:false,closed:false,rollback:false,view:View::Idle,sample:None}}
    pub fn busy(&self)->bool {self.pending.is_some()}
    pub fn view(&self)->View {self.view}
    pub fn next_sequence(&self)->u32 {self.seq}
    /// Exactly once for a correlated on-time successful query. Starting any
    /// subsequent query discards an unconsumed preceding snapshot.
    pub fn take_snapshot(&mut self)->Option<u32> {self.sample.take()}
    pub fn start(&mut self)->bool {
        if self.closed||self.rollback||self.busy() {return false;}
        self.sample=None;
        if self.seq>w::MAX_SEQ {self.view=View::Unavailable;return false;}
        self.pending=Some(w::Request {seq:self.seq});self.seq+=1;
        self.accepted=false;self.deadline=None;self.cancelled=false;self.expired=false;self.view=View::Pending;true
    }
    fn finish(&mut self) {self.pending=None;self.accepted=false;self.deadline=None;}
    pub fn cancel(&mut self) {
        if self.closed||self.rollback {return;}
        self.sample=None;self.cancelled=true;
        if self.accepted {self.view=View::Uncertain;}
        else {self.finish();self.view=View::Cancelled;}
    }
    /// Parent performs actual terminal service retirement. Locally no further
    /// request, retry, reply, or sample publication is possible after close.
    pub fn close(&mut self) {
        self.closed=true;self.sample=None;self.finish();
        if !self.rollback {self.view=View::Cancelled;}
    }
    fn time(&mut self,now:u64)->bool {
        let Some(deadline)=self.deadline else {return true;};
        if now<self.last||now>=deadline {self.expired=true;self.sample=None;return false;}
        self.last=now;true
    }
    pub fn advance(&mut self,now:u64) {
        if self.closed||!self.busy() {return;}
        if self.deadline.is_none() {self.deadline=Some(now.saturating_add(2000));self.last=now;}
        if !self.time(now) {
            if self.accepted {self.view=View::Uncertain;}else {self.finish();self.view=View::Unavailable;}
            return;
        }
        if self.accepted {return;}
        let r=self.pending.unwrap();let args=r.encode();
        let (status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,w::OP,args[0],args[1],args[2]);
        match status {
            0=>self.accepted=true,
            sex_pdx::ERR_SERVICE_NOT_READY=>{}, // Known pre-effect Full only.
            _=>{self.finish();self.view=View::Unavailable;}
        }
    }
    pub fn reply(&mut self,word:u64,now:u64) {
        if self.closed||!self.accepted {return;}
        let Some(r)=self.pending else {return;};
        if !matches!(word>>56,0x7B|0xEB)||((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64 {return;}
        let on_time=self.time(now)&&!self.expired;
        let result=w::decode_reply(word,r.seq);
        self.finish();self.sample=None;
        // A correlated rollback report is permanent even when cancellation or
        // timeout means a successful late sample would have been discarded.
        if result==Err(w::ROLLBACK) {self.rollback=true;self.view=View::Rollback;return;}
        if self.cancelled {self.view=View::Cancelled;return;}
        if !on_time {self.view=View::Uncertain;return;}
        match result {Ok(seconds)=>{self.sample=Some(seconds);self.view=View::Ready(seconds);},Err(_)=>self.view=View::Unavailable}
    }
    #[cfg(test)]
    pub fn set_next_sequence_for_test(&mut self,seq:u32) {assert!(!self.busy());self.seq=seq;}
}
