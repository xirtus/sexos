// SPDX-License-Identifier: GPL-3.0-only
//! One explicit controlled TLS exchange on the already held network route.
//! Session bytes and DNS generations are data, never endpoint authority. The
//! parent serializes this client with the route's Echo/HTTP replay fence.
use sex_pdx::network_job as w;
use sex_tls::{Session,Stats};
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View{Idle,Pending,Verified,Unavailable,Cancelled,Uncertain}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Step{Done,Begin,InitialWrite,Start,Active,Write,Status,Read,Finish,Cancel,Release,Held}
pub struct Client{
    session:Option<Session>,stats:Option<Stats>,step:Step,seq:u32,ticket:u32,last_ticket:u32,
    generation:u32,initial:usize,tx:usize,rx:usize,produced:usize,chunk:[u8;8],chunk_len:usize,
    pending:Option<w::Request>,accepted:bool,deadline:u64,total:Option<u64>,last:u64,next_poll:u64,
    active:bool,finish:bool,complete:bool,prefer_read:bool,abort:bool,closed:bool,
    authenticated:bool,auth_event:bool,verified:bool,verify_event:bool,view:View,terminal:View,
}
fn erase(bytes:&mut[u8]){for byte in bytes{unsafe{core::ptr::write_volatile(byte,0)}}}
impl Client{
    pub const fn new()->Self{Self{session:None,stats:None,step:Step::Done,seq:1,ticket:0,last_ticket:0,
        generation:0,initial:0,tx:0,rx:0,produced:0,chunk:[0;8],chunk_len:0,
        pending:None,accepted:false,deadline:0,total:None,last:0,next_poll:0,
        active:false,finish:false,complete:false,prefer_read:true,abort:false,closed:false,
        authenticated:false,auth_event:false,verified:false,verify_event:false,view:View::Idle,terminal:View::Unavailable}}
    pub fn busy(&self)->bool{self.step!=Step::Done||self.ticket!=0}
    pub fn view(&self)->View{self.view}
    pub fn next_sequence(&self)->u32{self.seq}
    pub fn stats(&self)->Option<Stats>{self.stats}
    pub fn tx_bytes(&self)->usize{self.tx}
    pub fn rx_bytes(&self)->usize{self.rx}
    pub fn ticket(&self)->u32{self.last_ticket}
    #[cfg(test)]
    pub fn private_scrubbed_for_test(&self)->bool{self.session.is_none()&&self.chunk.iter().all(|b|*b==0)&&self.chunk_len==0&&self.pending.is_none_or(|r|r.a==0)}
    pub fn peer_authenticated(&self)->bool{self.authenticated}
    pub fn take_authenticated(&mut self)->bool{let v=self.auth_event;self.auth_event=false;v}
    pub fn take_response_verified(&mut self)->bool{let v=self.verify_event;self.verify_event=false;v}
    /// Taking this session transfers private byte custody, never a transport
    /// grant. A new attempt needs a fresh held profile/seed/time and DNS receipt.
    pub fn start(&mut self,session:Session,next_seq:u32,generation:u32)->bool{
        if self.closed||self.busy(){return false;}
        let fence=self.seq;*self=Self::new();self.seq=fence;
        if next_seq==0||next_seq<self.seq||next_seq>w::MAX_SEQ-4||generation==0||generation>w::MAX_TICKET
            ||session.state()!=sex_tls::State::HelloPending||session.outgoing_len()==0||session.outgoing_len()>w::MAX_BYTES{
            self.view=View::Unavailable;return false;
        }
        let stats=session.stats();let initial=session.outgoing_len();
        *self=Self::new();self.seq=next_seq;self.generation=generation;self.initial=initial;
        self.stats=Some(stats);self.session=Some(session);self.step=Step::Begin;self.view=View::Pending;true
    }
    fn scrub(&mut self){self.session=None;erase(&mut self.chunk);self.chunk_len=0;
        if let Some(r)=self.pending.as_mut(){r.a=0;}self.auth_event=false;self.verify_event=false;}
    fn forget_request(&mut self){if let Some(r)=self.pending.as_mut(){r.a=0;}self.pending=None;self.accepted=false;}
    fn done(&mut self){self.scrub();self.forget_request();self.total=None;self.active=false;self.step=Step::Done;self.view=self.terminal;}
    fn cleanup(&mut self){
        if self.pending.is_some()&&self.accepted{self.view=View::Uncertain;return;}
        if let Some(r)=self.pending{self.seq=r.seq+1;self.forget_request();}
        if self.ticket==0{self.done();}
        else if self.seq<=w::MAX_SEQ{if self.step!=Step::Release{self.step=Step::Cancel;}}
        else{self.step=Step::Held;self.view=View::Uncertain;}
    }
    fn failed(&mut self,uncertain:bool){self.abort=true;self.terminal=View::Unavailable;self.scrub();self.cleanup();if uncertain&&self.busy(){self.view=View::Uncertain;}}
    /// A fresh prerequisite failure precedes any duplex admission. Preserve the
    /// route fence; never reset or scrub a busy lease or terminal incarnation.
    pub fn reject_prerequisites(&mut self){
        if self.closed||self.busy(){return;}
        self.terminal=View::Unavailable;self.authenticated=false;self.verified=false;self.done();
    }
    pub fn cancel(&mut self){
        if self.closed{return;}
        // No live lease means this is a new local cancellation, not a second
        // positive cleanup receipt for the preceding released attempt.
        if !self.busy(){self.last_ticket=0;self.stats=None;self.tx=0;self.rx=0;self.produced=0;self.authenticated=false;self.verified=false;self.generation=0;self.initial=0;}
        self.abort=true;self.terminal=View::Cancelled;self.scrub();self.cleanup();
    }
    /// Parent retires the exact service incarnation. No PDX is issued here.
    pub fn close(&mut self){self.closed=true;self.scrub();self.forget_request();self.ticket=0;self.step=Step::Done;self.active=false;self.total=None;self.view=View::Cancelled;self.terminal=View::Cancelled;}
    fn poll_session(&mut self){
        if !self.active||self.abort||self.verified{return;}
        let Some(session)=self.session.as_mut()else{self.failed(false);return;};
        let state=session.poll();self.stats=Some(session.stats());
        match state{
            sex_tls::State::PeerAuthenticated=>{if !self.authenticated{self.authenticated=true;self.auth_event=true;}},
            sex_tls::State::ExchangeVerified=>{if !self.authenticated{self.authenticated=true;self.auth_event=true;}self.verified=true;self.verify_event=true;},
            sex_tls::State::Rejected=>self.failed(false),
            sex_tls::State::Preparing|sex_tls::State::HelloPending=>{},
        }
    }
    fn choose_active(&mut self,now:u64){
        let Some(session)=self.session.as_ref()else{self.failed(false);return;};
        let read=self.rx<self.produced&&(self.verified||session.incoming_capacity()!=0);
        let write=self.chunk_len!=0||session.outgoing_len()!=0;
        self.step=if read&&(!write||self.prefer_read){Step::Read}else if write{Step::Write}
            else if self.verified&&!self.finish{Step::Finish}
            else if self.complete{
                if self.verified&&self.finish&&self.rx==self.produced{self.terminal=View::Verified;Step::Release}else{self.failed(false);return;}
            }else{if now<self.next_poll{return;}Step::Status};
    }
    pub fn advance(&mut self,now:u64){
        if self.closed||!self.busy()||self.step==Step::Held{return;}
        if self.total.is_none(){let Some(end)=now.checked_add(w::STREAM_TOTAL_TICKS)else{self.failed(false);return;};self.total=Some(end);self.last=now;}
        if !self.abort&&(now<self.last||self.total.is_some_and(|end|now>=end)){self.failed(self.accepted);}
        self.last=now;
        if self.pending.is_some()&&now>=self.deadline{
            if self.accepted{self.failed(true);return;}
            let cleanup=self.pending.is_some_and(|r|matches!(r.kind,w::CANCEL|w::RELEASE));
            if cleanup{self.scrub();self.forget_request();self.step=Step::Held;self.view=View::Uncertain;return;}
            self.failed(false);
        }
        if self.closed||self.step==Step::Done||self.step==Step::Held{return;}
        self.poll_session();
        if self.pending.is_some(){if !self.accepted{self.send_pending();}return;}
        if self.abort{self.cleanup();if self.step==Step::Done||self.step==Step::Held{return;}}
        if !self.abort&&self.active&&self.step==Step::Active{self.choose_active(now);}
        if self.step==Step::Active{return;}
        if self.seq>w::MAX_SEQ{self.failed(false);return;}
        if self.ticket!=0&&self.seq>=w::MAX_SEQ-1&&!matches!(self.step,Step::Cancel|Step::Release){self.failed(false);}
        if self.step==Step::Done||self.step==Step::Held{return;}
        let(kind,offset,length,a)=match self.step{
            Step::Begin=>(w::BEGIN_DUPLEX_RESOLVED,0,self.initial,self.generation as u64),
            Step::InitialWrite|Step::Write=>{
                if self.chunk_len==0{let Some(session)=self.session.as_mut()else{self.failed(false);return;};
                    self.chunk_len=session.take_outgoing(&mut self.chunk);
                    if self.chunk_len==0{self.failed(false);return;}
                }
                let Some(end)=self.tx.checked_add(self.chunk_len).filter(|end|*end<=w::MAX_DUPLEX_BYTES)else{self.failed(false);return;};
                if self.step==Step::InitialWrite&&end>self.initial{self.failed(false);return;}
                (w::WRITE,self.tx,self.chunk_len,u64::from_le_bytes(self.chunk))
            },
            Step::Start=>(w::START,0,0,0),Step::Status=>(w::STATUS,0,0,0),
            Step::Read=>{let capacity=if self.verified{4}else{self.session.as_ref().map_or(0,Session::incoming_capacity)};
                let n=(self.produced-self.rx).min(4).min(capacity);if n==0{self.step=Step::Active;return;}(w::READ,self.rx,n,0)},
            Step::Finish=>(w::FINISH,0,0,0),Step::Cancel=>(w::CANCEL,0,0,0),Step::Release=>(w::RELEASE,0,0,0),
            Step::Done|Step::Active|Step::Held=>return,
        };
        let Some(deadline)=now.checked_add(2000)else{self.failed(false);return;};
        let r=w::Request{seq:self.seq,kind,offset:offset as u16,length:length as u16,ticket:if kind==w::BEGIN_DUPLEX_RESOLVED{0}else{self.ticket},a};
        self.pending=Some(r);self.accepted=false;self.deadline=deadline;self.send_pending();
    }
    fn send_pending(&mut self){
        if self.accepted{return;}let Some(r)=self.pending else{return;};let a=r.encode();
        let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,w::OP,a[0],a[1],a[2]);
        match status{0=>self.accepted=true,sex_pdx::ERR_SERVICE_NOT_READY=>{},_=>{self.forget_request();self.seq=r.seq+1;if r.kind==w::RELEASE&&self.ticket!=0{self.scrub();self.step=Step::Held;self.view=View::Uncertain;}else{self.failed(false);}}}
    }
    pub fn reply(&mut self,word:u64,now:u64){
        if self.closed||!self.accepted{return;}let Some(r)=self.pending else{return;};
        if !matches!(word>>56,0x78|0xE8)||((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64{return;}
        if !self.abort&&(now<self.last||now>=self.deadline||self.total.is_some_and(|end|now>=end)){self.failed(true);}
        self.last=now;let result=w::decode_reply(word,r.seq);self.forget_request();self.seq=r.seq+1;
        if r.kind==w::BEGIN_DUPLEX_RESOLVED{
            match result{Ok(t)if t!=0&&t<=w::MAX_TICKET=>{self.ticket=t;self.last_ticket=t;self.step=Step::InitialWrite;},
                Ok(_)=>{self.scrub();self.step=Step::Held;self.view=View::Uncertain;return;},
                Err(_)=>{self.terminal=if self.abort{self.terminal}else{View::Unavailable};self.done();return;}}
            if self.abort{self.cleanup();}return;
        }
        if self.abort&&!matches!(r.kind,w::CANCEL|w::RELEASE){self.cleanup();return;}
        let value=match result{
            Ok(v)=>v,
            Err(w::BUSY)if matches!(r.kind,w::READ|w::WRITE)&&!self.abort=>{self.prefer_read=r.kind==w::WRITE;self.next_poll=now.saturating_add(2);self.step=if self.active{Step::Active}else{Step::InitialWrite};return;},
            Err(_)=>{if r.kind==w::RELEASE{self.scrub();self.step=Step::Held;self.view=View::Uncertain;}else if r.kind==w::CANCEL{self.step=Step::Release;}else{self.failed(false);}return;},
        };
        match r.kind{
            w::WRITE=>{let end=self.tx+r.length as usize;if r.offset as usize!=self.tx||value as usize!=end{self.failed(false);return;}self.tx=end;erase(&mut self.chunk);self.chunk_len=0;self.prefer_read=true;self.step=if self.active{Step::Active}else if end==self.initial{Step::Start}else{Step::InitialWrite};},
            w::START=>{if value!=0{self.failed(false);return;}self.active=true;self.next_poll=now;self.step=Step::Active;},
            w::STATUS=>{let Some((state,len))=w::decode_status(value)else{self.failed(false);return;};
                if len<self.produced||len<self.rx||len>w::MAX_DUPLEX_BYTES||self.complete&&state!=w::State::Complete{self.failed(false);return;}
                if !matches!(state,w::State::Connecting|w::State::Sending|w::State::Receiving|w::State::Complete)||state==w::State::Complete&&!self.finish{self.failed(false);return;}
                self.produced=len;self.complete=state==w::State::Complete;self.next_poll=now.saturating_add(2);self.step=Step::Active;
            },
            w::READ=>{if r.offset as usize!=self.rx||self.rx+r.length as usize>self.produced{self.failed(false);return;}
                // ExchangeVerified terminates the TLS task. Remaining bounded
                // control-tail ciphertext is drained, not claimed validated.
                if !self.verified&&self.session.as_mut().is_none_or(|s|s.offer_incoming(&value.to_le_bytes()[..r.length as usize]).is_err()){self.failed(false);return;}
                self.rx+=r.length as usize;self.prefer_read=false;self.step=Step::Active;
            },
            w::FINISH=>{if value!=0{self.failed(false);return;}self.finish=true;self.step=Step::Active;},
            w::CANCEL=>{if value!=0{self.step=Step::Held;self.view=View::Uncertain;}else{self.step=Step::Release;}},
            w::RELEASE=>{if value!=0{self.scrub();self.step=Step::Held;self.view=View::Uncertain;}else{self.ticket=0;self.done();}},
            _=>self.failed(false),
        }
    }
}
impl Drop for Client{fn drop(&mut self){self.scrub();}}
