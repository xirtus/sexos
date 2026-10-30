// SPDX-License-Identifier: GPL-3.0-only
//! One explicit controlled TLS exchange on the already held network route.
//! Session bytes and DNS generations are data, never endpoint authority. The
//! parent serializes this client with the route's Echo/HTTP replay fence.
use sex_pdx::network_job as w;
use sex_tls::{Resource,Session,Stats};
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View{Idle,Pending,Verified,Unavailable,Cancelled,Uncertain}
/// Diagnostic of one positively acknowledged RELEASE; not a new capability.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Released{pub ticket:u32,pub outcome:View,pub authenticated:bool,pub document:bool,pub resource:Option<Resource>,pub payload_bytes:usize}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Step{Done,Begin,InitialWrite,Start,Active,Write,Status,Read,Finish,Cancel,Release,Held}
pub struct Client{
    released:Option<Released>,session:Option<Session>,stats:Option<Stats>,document:Option<sex_tls::Document>,is_document:bool,document_received:usize,http_failure:Option<sex_tls::HttpError>,failure:Option<sex_tls::Failure>,step:Step,seq:u32,ticket:u32,last_ticket:u32,
    asset:Option<sex_tls::Asset>,resource:Option<Resource>,
    generation:u32,initial:usize,tx:usize,rx:usize,produced:usize,chunk:[u8;8],chunk_len:usize,
    pending:Option<w::Request>,accepted:bool,deadline:u64,total:Option<u64>,last:u64,next_poll:u64,
    active:bool,finish:bool,complete:bool,prefer_read:bool,abort:bool,closed:bool,
    authenticated:bool,auth_event:bool,verified:bool,verify_event:bool,view:View,terminal:View,
}
fn erase(bytes:&mut[u8]){for byte in bytes{unsafe{core::ptr::write_volatile(byte,0)}}}
impl Client{
    pub const fn new()->Self{Self{released:None,session:None,stats:None,document:None,asset:None,resource:None,is_document:false,document_received:0,http_failure:None,failure:None,step:Step::Done,seq:1,ticket:0,last_ticket:0,
        generation:0,initial:0,tx:0,rx:0,produced:0,chunk:[0;8],chunk_len:0,
        pending:None,accepted:false,deadline:0,total:None,last:0,next_poll:0,
        active:false,finish:false,complete:false,prefer_read:true,abort:false,closed:false,
        authenticated:false,auth_event:false,verified:false,verify_event:false,view:View::Idle,terminal:View::Unavailable}}
    pub fn take_released(&mut self)->Option<Released>{self.released.take()}
    pub fn busy(&self)->bool{self.step!=Step::Done||self.ticket!=0}
    pub fn view(&self)->View{self.view}
    pub fn next_sequence(&self)->u32{self.seq}
    pub fn stats(&self)->Option<Stats>{self.stats}
    pub fn allocated_payload_bytes(&self)->usize{
        self.session.as_ref().map_or(0,Session::allocated_payload_bytes)
            +if self.document.is_some(){sex_tls::MAX_DOCUMENT}else{0}
            +if self.asset.is_some(){sex_tls::MAX_DOCUMENT}else{0}
    }
    /// Nonsensitive reason from this attempt's actual TLS task error only.
    /// Transport timeout/cancellation alone never synthesizes a TLS diagnosis.
    pub fn failure(&self)->Option<sex_tls::Failure>{self.failure}
    pub fn http_failure(&self)->Option<sex_tls::HttpError>{self.http_failure}
    pub fn is_document(&self)->bool{self.is_document}
    pub fn resource_kind(&self)->Option<Resource>{self.resource}
    pub fn has_asset(&self)->bool{self.resource==Some(Resource::ArtworkQoi)&&self.view==View::Verified&&!self.busy()&&self.asset.is_some()}
    /// Asset custody stays private until the exact positive RELEASE receipt.
    pub fn asset(&self)->Option<&sex_tls::Asset>{if self.has_asset(){self.asset.as_ref()}else{None}}
    pub fn take_asset(&mut self)->Option<sex_tls::Asset>{if self.has_asset(){self.asset.take()}else{None}}
    pub fn retained_asset_len(&self)->usize{self.asset.as_ref().map_or(0,|a|a.as_bytes().len())}
    /// Diagnostic only; no retained binary prefix is published before RELEASE.
    pub fn retained_asset_hash(&self)->u64{self.asset.as_ref().map_or(0,|a|a.as_bytes().iter().fold(0xcbf29ce484222325u64,|h,b|(h^*b as u64).wrapping_mul(0x100000001b3)))}
    pub fn document_received(&self)->usize{self.document_received}
    pub fn has_document(&self)->bool{self.is_document&&self.view==View::Verified&&self.document.is_some()&&!self.busy()}
    pub fn document(&self)->&[u8]{if self.has_document(){self.document.as_ref().unwrap().as_bytes()}else{&[]}}
    /// Move already-published bytes into a separately owned workspace import.
    /// A pending lease, partial body or uncertain cleanup can never transfer.
    pub fn take_document_for_import(&mut self)->Option<sex_tls::Document>{
        if !self.has_document(){return None;}
        self.document.take()
    }
    pub fn retained_document_len(&self)->usize{self.document.as_ref().map_or(0,|d|d.as_bytes().len())}
    /// Public diagnostic fingerprint; this never publishes the retained body.
    pub fn retained_document_hash(&self)->u64{self.document.as_ref().map_or(0,|d|d.as_bytes().iter().fold(0xcbf29ce484222325u64,|h,b|(h^*b as u64).wrapping_mul(0x100000001b3)))}
    pub fn tx_bytes(&self)->usize{self.tx}
    pub fn rx_bytes(&self)->usize{self.rx}
    pub fn ticket(&self)->u32{self.last_ticket}
    #[cfg(test)]
    pub fn private_scrubbed_for_test(&self)->bool{self.session.is_none()&&self.document.is_none()&&self.asset.is_none()&&self.chunk.iter().all(|b|*b==0)&&self.chunk_len==0&&self.pending.is_none_or(|r|r.a==0)}
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
        self.stats=Some(stats);self.is_document=session.is_document();self.resource=session.resource_kind();self.session=Some(session);self.step=Step::Begin;self.view=View::Pending;true
    }
    fn scrub(&mut self){self.session=None;erase(&mut self.chunk);self.chunk_len=0;
        if let Some(r)=self.pending.as_mut(){r.a=0;}self.auth_event=false;self.verify_event=false;}
    /// Failed cleanup can retain only its authority/correlation fence, never
    /// the authenticated body waiting for a positive RELEASE.
    fn hold_uncertain(&mut self){self.document=None;self.asset=None;self.resource=None;self.scrub();self.step=Step::Held;self.view=View::Uncertain;}
    fn forget_request(&mut self){if let Some(r)=self.pending.as_mut(){r.a=0;}self.pending=None;self.accepted=false;}
    fn done(&mut self){self.scrub();self.forget_request();self.total=None;self.active=false;self.step=Step::Done;self.view=self.terminal;if self.view!=View::Verified{self.document=None;self.asset=None;self.resource=None;}}
    fn cleanup(&mut self){
        if self.pending.is_some()&&self.accepted{self.view=View::Uncertain;return;}
        if let Some(r)=self.pending{self.seq=r.seq+1;self.forget_request();}
        if self.ticket==0{self.done();}
        else if self.seq<=w::MAX_SEQ{if self.step!=Step::Release{self.step=Step::Cancel;}}
        else{self.hold_uncertain();}
    }
    fn failed(&mut self,uncertain:bool){self.document=None;self.asset=None;self.resource=None;self.abort=true;self.terminal=View::Unavailable;self.scrub();self.cleanup();if uncertain&&self.busy(){self.view=View::Uncertain;}}
    fn clear_idle_metadata(&mut self){self.document=None;self.asset=None;self.resource=None;self.is_document=false;self.document_received=0;self.http_failure=None;
        self.failure=None;self.last_ticket=0;self.stats=None;self.tx=0;self.rx=0;self.produced=0;
        self.authenticated=false;self.verified=false;self.generation=0;self.initial=0;
    }
    /// A fresh prerequisite failure precedes any duplex admission. Preserve the
    /// route fence; never reset or scrub a busy lease or terminal incarnation.
    pub fn reject_prerequisites(&mut self){
        if self.closed||self.busy(){return;}
        self.clear_idle_metadata();self.terminal=View::Unavailable;self.done();
    }
    pub fn cancel(&mut self){
        if self.closed{return;}
        // No live lease means this is a new local cancellation, not a second
        // positive cleanup receipt for the preceding released attempt.
        if !self.busy(){self.clear_idle_metadata();}
        self.document=None;self.asset=None;self.resource=None;self.abort=true;self.terminal=View::Cancelled;self.scrub();self.cleanup();
    }
    /// Parent retires the exact service incarnation. No PDX is issued here.
    pub fn close(&mut self){self.released=None;self.document=None;self.asset=None;self.resource=None;self.is_document=false;self.document_received=0;self.http_failure=None;self.failure=None;self.closed=true;self.scrub();self.forget_request();self.ticket=0;self.step=Step::Done;self.active=false;self.total=None;self.view=View::Cancelled;self.terminal=View::Cancelled;}
    fn poll_session(&mut self){
        if !self.active||self.abort||self.verified{return;}
        let Some(session)=self.session.as_mut()else{self.failed(false);return;};
        let state=session.poll();self.stats=Some(session.stats());self.document_received=self.document_received.max(session.document_received());
        match state{
            sex_tls::State::PeerAuthenticated=>{if !self.authenticated{self.authenticated=true;self.auth_event=true;}},
            sex_tls::State::ExchangeVerified=>{
                if self.is_document&&self.document.is_none(){
                    self.document=session.take_document();if self.document.is_none(){self.failed(false);return;}
                }
                if self.resource==Some(Resource::ArtworkQoi)&&self.asset.is_none(){
                    self.asset=session.take_asset();if self.asset.is_none(){self.failed(false);return;}
                }
                if !self.authenticated{self.authenticated=true;self.auth_event=true;}self.verified=true;self.verify_event=true;},
            sex_tls::State::Rejected=>{
                self.failure=session.failure();self.http_failure=session.http_failure();
                self.failed(false);
            },
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
            if cleanup{self.forget_request();self.hold_uncertain();return;}
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
        match status{0=>self.accepted=true,sex_pdx::ERR_SERVICE_NOT_READY=>{},_=>{self.forget_request();self.seq=r.seq+1;if r.kind==w::RELEASE&&self.ticket!=0{self.hold_uncertain();}else{self.failed(false);}}}
    }
    pub fn reply(&mut self,word:u64,now:u64){
        if self.closed||!self.accepted{return;}let Some(r)=self.pending else{return;};
        if !matches!(word>>56,0x78|0xE8)||((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64{return;}
        if !self.abort&&(now<self.last||now>=self.deadline||self.total.is_some_and(|end|now>=end)){self.failed(true);}
        self.last=now;let result=w::decode_reply(word,r.seq);self.forget_request();self.seq=r.seq+1;
        if r.kind==w::BEGIN_DUPLEX_RESOLVED{
            match result{Ok(t)if t!=0&&t<=w::MAX_TICKET=>{self.ticket=t;self.last_ticket=t;self.step=Step::InitialWrite;},
                Ok(_)=>{self.hold_uncertain();return;},
                Err(_)=>{self.terminal=if self.abort{self.terminal}else{View::Unavailable};self.done();return;}}
            if self.abort{self.cleanup();}return;
        }
        if self.abort&&!matches!(r.kind,w::CANCEL|w::RELEASE){self.cleanup();return;}
        let value=match result{
            Ok(v)=>v,
            Err(w::BUSY)if matches!(r.kind,w::READ|w::WRITE)&&!self.abort=>{self.prefer_read=r.kind==w::WRITE;self.next_poll=now.saturating_add(2);self.step=if self.active{Step::Active}else{Step::InitialWrite};return;},
            Err(_)=>{if r.kind==w::RELEASE{self.hold_uncertain();}else if r.kind==w::CANCEL{self.step=Step::Release;}else{self.failed(false);}return;},
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
            w::CANCEL=>{if value!=0{self.hold_uncertain();}else{self.step=Step::Release;}},
            w::RELEASE=>{if value!=0{self.hold_uncertain();}else{
                let ticket=self.ticket;self.ticket=0;self.done();
                self.released=Some(Released{ticket,outcome:self.view,authenticated:self.authenticated,document:self.is_document,resource:self.resource,payload_bytes:self.allocated_payload_bytes()});
            }},
            _=>self.failed(false),
        }
    }
}
impl Drop for Client{fn drop(&mut self){self.asset=None;self.resource=None;self.scrub();}}
