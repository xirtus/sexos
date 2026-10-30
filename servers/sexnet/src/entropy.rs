// SPDX-License-Identifier: GPL-3.0-only
//! Finite private seed custody, not authority or a statistical certification.
//! Parent supplies an exact live BOOT identity at every operation/advance and
//! held-source Info. No caller address, source selector, fallback or resampling.
use sex_pdx::entropy_seed::{self as a, Identity, Request, State};
use sex_pdx::entropy_source::{self as source, Info};

pub const TOTAL_TICKS:u64=2000; // Raw nominal ticks; no wall-clock guarantee.
const OWNERS:usize=6;
struct Receipt { owner:Identity, request:Request, word:u64 }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Snapshot {
    pub owner:Option<Identity>,pub ticket:u32,pub state:Option<State>,
    pub produced:usize,pub consumed:usize,pub failure:u32,
}
pub struct Machine {
    owner:Option<Identity>,ticket:u32,next_ticket:u32,state:Option<State>,
    seed:[u8;a::SEED_BYTES],produced:usize,consumed:usize,
    last_word:u64,last_word_valid:bool,deadline:u64,last_tick:u64,failure:u32,
    pending:[Option<Receipt>;OWNERS],cached:[Option<Receipt>;OWNERS],cursor:usize,
}
// Volatile stores prevent dead-store elimination of private secret erasure.
// These pointers derive solely from internal exclusive references, never ABI
// words/caller addresses. This is not a register/CPU-cache or DMA erasure claim.
fn zero_word(word:&mut u64) { unsafe{core::ptr::write_volatile(word,0)} }
fn zero_bytes(bytes:&mut[u8]) {for byte in bytes {unsafe{core::ptr::write_volatile(byte,0)}}}
fn clear(slot:&mut Option<Receipt>) {
    if let Some(receipt)=slot.as_mut(){zero_word(&mut receipt.word);}
    *slot=None;
}
impl Machine {
    pub const fn new()->Self {
        Self{owner:None,ticket:0,next_ticket:1,state:None,seed:[0;a::SEED_BYTES],
            produced:0,consumed:0,last_word:0,last_word_valid:false,deadline:0,last_tick:0,failure:0,
            pending:[const{None};OWNERS],cached:[const{None};OWNERS],cursor:0}
    }
    pub fn owner(&self)->Option<Identity>{self.owner}
    pub fn busy(&self)->bool{self.owner.is_some()}
    pub fn snapshot(&self)->Snapshot {
        Snapshot{owner:self.owner,ticket:self.ticket,state:self.state,produced:self.produced,consumed:self.consumed,failure:self.failure}
    }
    fn selected(&self)->Option<usize>{(0..OWNERS).map(|n|(self.cursor+n)%OWNERS).find(|i|self.pending[*i].is_some())}
    pub fn pending_reply(&self)->Option<(Identity,u64)>{self.selected().and_then(|i|self.pending[i].as_ref()).map(|r|(r.owner,r.word))}
    pub fn reply_accepted(&mut self){if let Some(i)=self.selected(){clear(&mut self.pending[i]);self.cursor=(i+1)%OWNERS;}}
    pub fn reply_deferred(&mut self){if let Some(i)=self.selected(){self.cursor=(i+1)%OWNERS;}}
    fn redact_reads(&mut self,owner:Identity){
        for slots in [&mut self.pending,&mut self.cached] {
            for slot in slots {
                if let Some(receipt)=slot.as_mut(){
                    if receipt.owner==owner&&receipt.request.kind==a::READ {
                        zero_word(&mut receipt.word);
                        // Preserve the replay fence/correlation, with no secret
                        // value after failure or terminal cleanup.
                        receipt.word=a::error(receipt.request.seq,a::INVALID);
                    }
                }
            }
        }
    }
    fn scrub(&mut self){
        zero_bytes(&mut self.seed);zero_word(&mut self.last_word);self.last_word_valid=false;
        self.produced=0;self.consumed=0;self.deadline=0;self.last_tick=0;
        if let Some(owner)=self.owner{self.redact_reads(owner);}
    }
    fn fail(&mut self,error:u32){self.scrub();self.failure=error;self.state=Some(State::Failed);}
    fn clock(&mut self,now:u64)->bool {
        if !matches!(self.state,Some(State::Collecting|State::Ready)){return false;}
        if now<self.last_tick||now>=self.deadline{self.fail(a::TIMEOUT);return false;}
        self.last_tick=now;true
    }
    fn duplicate_clock(&mut self,owner:Identity,r:Request,now:u64){
        if self.owner==Some(owner)&&r.ticket==self.ticket&&r.kind==a::READ{self.clock(now);}
    }
    fn respond(&mut self,i:usize,owner:Identity,r:Request,result:Result<u32,u32>)->bool {
        let mut word=match result{Ok(value)=>a::reply(r.seq,value),Err(error)=>a::error(r.seq,error)};
        clear(&mut self.pending[i]);self.pending[i]=Some(Receipt{owner,request:r,word});
        if self.cached[i].as_ref().is_none_or(|old|r.seq>old.request.seq){clear(&mut self.cached[i]);self.cached[i]=Some(Receipt{owner,request:r,word});}
        zero_word(&mut word);true
    }
    /// False has no effect: no slot, malformed correlation or same-owner Full.
    /// All six replay slots are retained until exact retirement. The held owner
    /// always retains its own cleanup slot even when other owners fill theirs.
    pub fn handle(&mut self,owner:Identity,r:Request,now:u64,info:Option<Info>)->bool {
        if !owner.valid()||r.seq==0||r.seq>a::MAX_SEQ{return false;}
        let Some(i)=self.cached.iter().position(|c|c.as_ref().is_some_and(|c|c.owner==owner))
            .or_else(||self.cached.iter().position(Option::is_none))else{return false;};
        if let Some(p)=self.pending[i].as_ref(){
            if p.request!=r{return false;}
            self.duplicate_clock(owner,r,now);return true;
        }
        if let Some(c)=self.cached[i].as_ref(){
            if c.request==r {
                self.duplicate_clock(owner,r,now);
                let mut word=self.cached[i].as_ref().unwrap().word;
                self.pending[i]=Some(Receipt{owner,request:r,word});zero_word(&mut word);return true;
            }
            if r.seq<=c.request.seq{return self.respond(i,owner,r,Err(a::STALE));}
        }
        let result=self.apply(owner,r,now,info);self.respond(i,owner,r,result)
    }
    fn apply(&mut self,owner:Identity,r:Request,now:u64,info:Option<Info>)->Result<u32,u32>{
        if !r.valid(){return Err(a::INVALID);}
        if r.kind==a::BEGIN {
            let info=info.ok_or(a::DENIED)?;
            if !info.consumer.valid()||info.consumer!=owner{return Err(a::DENIED);}
            if !info.available{return Err(a::OFFLINE);}
            if self.owner.is_some(){return Err(a::BUSY);}
            if self.next_ticket>a::MAX_TICKET{return Err(a::EXHAUSTED);}
            let deadline=now.checked_add(TOTAL_TICKS).ok_or(a::TIMEOUT)?;
            self.scrub();self.owner=Some(owner);self.ticket=self.next_ticket;self.next_ticket+=1;
            self.state=Some(State::Collecting);self.deadline=deadline;self.last_tick=now;self.failure=0;
            return Ok(self.ticket);
        }
        if self.owner!=Some(owner){return Err(a::DENIED);}
        if self.ticket!=r.ticket{return Err(a::STALE);}
        // Cleanup remains possible if source Info is unavailable/unsupported.
        if r.kind==a::CANCEL{self.scrub();self.failure=0;self.state=Some(State::Cancelled);return Ok(0);}
        if r.kind==a::RELEASE{self.scrub();self.owner=None;self.ticket=0;self.state=None;self.failure=0;return Ok(0);}
        let info=info.ok_or(a::DENIED)?;
        if !info.consumer.valid()||info.consumer!=owner{return Err(a::DENIED);}
        if !info.available{return Err(a::OFFLINE);}
        if r.kind==a::READ&&r.offset as usize!=self.consumed{return Err(a::INVALID);}
        if matches!(self.state,Some(State::Collecting|State::Ready)){self.clock(now);}
        match r.kind {
            a::STATUS=>Ok(a::status(self.state.ok_or(a::INVALID)?,self.produced)),
            a::READ=>{
                if self.state!=Some(State::Ready){return Err(if self.state==Some(State::Failed){self.failure}else{a::BUSY});}
                let end=self.consumed+r.length as usize;
                if end>self.produced{return Err(a::BUSY);}
                let mut bytes=[0;4];bytes[..r.length as usize].copy_from_slice(&self.seed[self.consumed..end]);
                let value=u32::from_le_bytes(bytes);zero_bytes(&mut bytes);
                zero_bytes(&mut self.seed[self.consumed..end]);self.consumed=end;Ok(value)
            },
            _=>Err(a::INVALID),
        }
    }
    /// One provider invocation at most. Any provider error/repeated consecutive
    /// word ends this job; no automatic resampling, alternate source or fallback.
    /// Parent must attest the current live owner before calling advance.
    pub fn advance(&mut self,now:u64,mut provider:impl FnMut()->Result<u64,u64>){
        if !self.clock(now)||self.state!=Some(State::Collecting){return;}
        let mut word=match provider(){Ok(word)=>word,Err(error)=>{
            self.fail(match error{source::DENIED=>a::DENIED,source::UNSUPPORTED=>a::OFFLINE,source::BUSY=>a::BUSY,_=>a::IO});return;
        }};
        if self.last_word_valid&&word==self.last_word{zero_word(&mut word);self.fail(a::IO);return;}
        zero_word(&mut self.last_word);self.last_word=word;self.last_word_valid=true;
        let mut bytes=word.to_le_bytes();self.seed[self.produced..self.produced+8].copy_from_slice(&bytes);self.produced+=8;
        zero_bytes(&mut bytes);zero_word(&mut word);
        if self.produced==a::SEED_BYTES{self.state=Some(State::Ready);zero_word(&mut self.last_word);self.last_word_valid=false;}
    }
    pub fn retire(&mut self,owner:Identity)->bool {
        let held=self.owner==Some(owner);
        if held{self.scrub();self.owner=None;self.ticket=0;self.state=None;self.failure=0;}
        for i in 0..OWNERS {
            if self.cached[i].as_ref().is_some_and(|r|r.owner==owner){clear(&mut self.cached[i]);}
            if self.pending[i].as_ref().is_some_and(|r|r.owner==owner){clear(&mut self.pending[i]);}
        }
        held
    }
    #[cfg(test)]
    pub fn private_scrubbed(&self)->bool {
        self.seed.iter().all(|b|*b==0)&&self.last_word==0&&!self.last_word_valid
            &&self.pending.iter().chain(self.cached.iter()).all(|slot|slot.as_ref().is_none_or(|r|r.request.kind!=a::READ||r.word>>56!=0x7A))
    }
}

#[cfg(test)]mod internal_tests {
    use super::*;
    #[test]fn ticket_exhaustion_never_reuses_and_admission_overflow_has_no_effect(){
        let owner=Identity{pd:15,task:150};let info=Some(Info{consumer:owner,available:true});let mut m=Machine::new();
        let mut r=Request{seq:1,kind:a::BEGIN,offset:0,length:32,ticket:0,a:0};m.next_ticket=a::MAX_TICKET;
        assert!(m.handle(owner,r,u64::MAX,info));assert!(m.owner().is_none());assert!(m.next_ticket==a::MAX_TICKET);m.reply_accepted();
        r.seq+=1;assert!(m.handle(owner,r,0,info));assert!(m.ticket==a::MAX_TICKET);m.reply_accepted();
        let release=Request{seq:3,kind:a::RELEASE,offset:0,length:0,ticket:a::MAX_TICKET,a:0};assert!(m.handle(owner,release,1,None));m.reply_accepted();
        r.seq=4;assert!(m.handle(owner,r,1,info));let word=m.pending_reply().unwrap().1;assert!(a::decode_reply(word,4)==Err(a::EXHAUSTED));assert!(m.owner().is_none()&&m.private_scrubbed());
    }
}
