//! Create-only workspace import over the existing BOOT storage route.
//! Owns plaintext, never a storage grant. Admission and receipt are distinct:
//! only explicit pre-effect Full retries; admitted mutations never replay.
use sex_pdx::document as w;

pub trait Payload { fn as_bytes(&self) -> &[u8]; }
impl Payload for sex_tls::Document { fn as_bytes(&self) -> &[u8] { self.as_bytes() } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View { Idle, Saving, Saved, Unavailable, Cancelled, Uncertain, Exists }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub view: View, pub object: u8, pub generation: u32,
    pub bytes: usize, pub hash: u64, pub error: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlPhase { Admitted, Reply }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlEvent {
    pub phase: ControlPhase, pub kind:u8, pub sequence:u32,
    pub object:u8, pub value:Option<Result<u32,u32>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Create, Info, Begin, Write, Commit, End, Abort }
#[derive(Clone, Copy)]
struct Pending { request: w::Request, accepted: bool, count: usize, deadline: u64 }
pub const TIMEOUT_TICKS: u64 = 2_000;
const TOTAL_TICKS: u64 = 60_000;

fn erase_words(r: &mut w::Request) {
    // No DMA/shared pointer: PDX copied the words before successful admission.
    // Accepted descriptors retain correlation only, not plaintext WRITE words.
    unsafe { core::ptr::write_volatile(&mut r.a, 0); core::ptr::write_volatile(&mut r.b, 0); }
}
fn erase_name(name: &mut [u8; 16]) { for b in name { unsafe { core::ptr::write_volatile(b, 0); } } }

pub struct Client<D: Payload> {
    body: Option<D>, name: [u8;16], len: usize, hash: u64,
    object: u8, generation: u32, seq: u32, phase: Phase,
    control: Option<Pending>, window: [Option<Pending>; w::WINDOW],
    issued: usize, completed: usize, busy: bool, closed: bool,
    created: bool, recovering: bool, fenced: bool, committed: bool,
    target: View, view: View, error: u32, event: Option<Event>, total_deadline: u64,
    control_event: Option<ControlEvent>,
}
impl<D: Payload> Client<D> {
    pub const fn new() -> Self {
        Self { body:None,name:[0;16],len:0,hash:0,object:0,generation:0,seq:0,
            phase:Phase::Create,control:None,window:[None;w::WINDOW],issued:0,completed:0,
            busy:false,closed:false,created:false,recovering:false,fenced:false,
            committed:false,target:View::Idle,view:View::Idle,error:0,event:None,total_deadline:0,control_event:None }
    }
    pub fn busy(&self) -> bool { self.busy }
    pub fn view(&self) -> View { self.view }
    pub fn document(&self) -> Option<&D> { self.body.as_ref() }
    pub fn has_document(&self) -> bool { self.body.is_some() }
    pub fn take_document(&mut self) -> Option<D> { if self.busy || self.fenced { None } else { self.body.take() } }
    pub fn take_event(&mut self) -> Option<Event> { self.event.take() }
    pub fn take_control(&mut self) -> Option<ControlEvent> { self.control_event.take() }
    pub fn next_sequence(&self) -> u32 { self.seq.saturating_add(1) }
    pub fn accepted(&self) -> usize {
        usize::from(self.control.is_some_and(|p|p.accepted)) + self.window.iter().flatten().filter(|p|p.accepted).count()
    }
    pub fn start(&mut self, body:D, name:[u8;16], now:u64) -> Result<(),D> {
        let len=body.as_bytes().len();
        // Reserve all data/control sequences and a fresh terminal ABORT before
        // taking custody. A rejected start cannot replace an older held body.
        let needed=(len.saturating_add(15)/16).saturating_add(7);
        let n=name.iter().position(|b|*b==0).unwrap_or(16);
        if self.busy || self.closed || self.fenced || self.body.is_some() || len>w::MAX
            || n==0 || !name[..n].iter().all(|b|b.is_ascii_alphanumeric()||matches!(*b,b'-'|b'_'|b'.'))
            || name[n..].iter().any(|b|*b!=0) || needed>w::SEQ_MAX.saturating_sub(self.seq) as usize {
            return Err(body);
        }
        self.hash=w::hash(body.as_bytes());self.body=Some(body);self.len=len;self.name=name;
        self.object=0;self.generation=0;self.issued=0;self.completed=0;self.created=false;
        self.recovering=false;self.committed=false;self.error=0;self.event=None;self.control_event=None;
        self.phase=Phase::Create;self.view=View::Saving;self.target=View::Unavailable;
        self.busy=true;self.total_deadline=now.saturating_add(TOTAL_TICKS);Ok(())
    }
    fn emit(&mut self, view:View, error:u32) {
        self.view=view;self.error=error;
        self.event=Some(Event{view,object:self.object,generation:self.generation,bytes:self.len,hash:self.hash,error});
    }
    fn wipe_pending(&mut self) {
        if let Some(p)=self.control.as_mut(){erase_words(&mut p.request);}
        for p in self.window.iter_mut().flatten(){erase_words(&mut p.request);}
    }
    fn discard_plaintext(&mut self) { self.body=None;erase_name(&mut self.name);self.wipe_pending(); }
    fn finish(&mut self, view:View, error:u32) {
        self.wipe_pending();self.control=None;self.window=[None;w::WINDOW];self.busy=false;
        // A correlated CREATE/EXISTS is a known pre-effect collision: no
        // service object/session was bound. Keep the released source so the
        // user can explicitly choose another name without fetching again.
        let retry_name=view==View::Exists && !self.created && !self.fenced;
        if view!=View::Saved && !retry_name { self.discard_plaintext(); }
        self.emit(if self.fenced { View::Uncertain } else { view },error);
    }
    fn recover(&mut self, target:View, error:u32, uncertain:bool) {
        if self.committed || self.control.is_some_and(|p|p.accepted&&p.request.kind==w::COMMIT) {
            self.fenced=true;
        }
        self.target=target;self.recovering=true;self.error=error;self.discard_plaintext();
        if self.control.is_some_and(|p|!p.accepted){self.control=None;}
        for p in &mut self.window { if p.is_some_and(|p|!p.accepted){*p=None;} }
        if uncertain || self.fenced { self.emit(View::Uncertain,error); }
        else { self.view=target; }
        self.cleanup_ready();
    }
    fn cleanup_ready(&mut self) {
        if !self.recovering || self.accepted()!=0 { return; }
        if self.created {
            self.phase=Phase::Abort;
        } else { self.finish(self.target,self.error); }
    }
    pub fn cancel(&mut self, _now:u64) -> bool {
        if self.closed { return false; }
        if !self.busy { self.clear(_now);return true; }
        if self.recovering { return true; }
        self.recover(View::Cancelled,0,false);true
    }
    pub fn clear(&mut self, now:u64) {
        if self.busy { let _=self.cancel(now);return; }
        self.discard_plaintext();self.event=None;self.control_event=None;
        if !self.fenced && !self.closed { self.view=View::Idle;self.error=0;self.len=0;self.hash=0;self.object=0;self.generation=0; }
    }
    /// Terminal lifecycle operation: the parent retires actual subject 15.
    /// Close emits no PDX and cannot be reversed by a late receipt.
    pub fn close(&mut self) {
        self.discard_plaintext();self.control=None;self.window=[None;w::WINDOW];
        self.busy=false;self.closed=true;self.event=None;self.control_event=None;self.view=View::Idle;
    }
    fn expired(&mut self, now:u64) {
        if !self.busy { return; }
        if self.recovering {
            // A missing ABORT receipt is still owned. Report uncertainty while
            // retaining its exact correlation; never resend accepted cleanup.
            if self.control.is_some_and(|p|p.accepted&&p.request.kind==w::ABORT&&now>=p.deadline)
                && self.view!=View::Uncertain { self.emit(View::Uncertain,w::TIMEOUT); }
            return;
        }
        let expired=now>=self.total_deadline || self.control.is_some_and(|p|now>=p.deadline)
            || self.window.iter().flatten().any(|p|now>=p.deadline);
        if expired { self.recover(View::Unavailable,w::TIMEOUT,self.accepted()!=0); }
    }
    fn allocate(&mut self, kind:u8,offset:u32,a:u64,b:u64,count:usize,now:u64)->Option<Pending> {
        let limit=if kind==w::ABORT {w::SEQ_MAX} else {w::SEQ_MAX-1};
        if self.seq>=limit {self.fenced=self.created;self.finish(View::Unavailable,w::STALE);return None;}
        self.seq+=1;
        Some(Pending{request:w::Request{seq:self.seq,kind,object:if matches!(kind,w::CREATE|w::ABORT){0}else{self.object},offset,a,b},accepted:false,count,deadline:now.saturating_add(TIMEOUT_TICKS)})
    }
    fn control_request(&mut self, now:u64)->Option<Pending> {
        let(kind,a,b)=match self.phase {
            Phase::Create=>(w::CREATE,u64::from_le_bytes(self.name[..8].try_into().unwrap()),u64::from_le_bytes(self.name[8..].try_into().unwrap())),
            Phase::Info=>(w::INFO,0,0),Phase::Begin=>(w::BEGIN_SAVE,self.generation as u64,self.len as u64),
            Phase::Commit=>(w::COMMIT,self.hash,0),Phase::End=>(w::END,0,0),Phase::Abort=>(w::ABORT,0,0),Phase::Write=>return None,
        };self.allocate(kind,0,a,b,0,now)
    }
    pub fn advance(&mut self, now:u64) {
        self.advance_with(now,|mut r|{
            let result=sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE,w::OP,r.header(),r.a,r.b).map(|_|());
            erase_words(&mut r);result
        });
    }
    /// At most four calls per event-loop turn, with no receive or busy wait.
    pub fn advance_with(&mut self, now:u64,mut send:impl FnMut(w::Request)->Result<(),u64>) {
        self.expired(now);if !self.busy{return;}
        if self.recovering {self.cleanup_ready();if !self.busy||self.accepted()!=0{return;}}
        if self.phase!=Phase::Write {
            if self.control.is_none(){self.control=self.control_request(now);}
            let Some(mut p)=self.control else{return;};if p.accepted{return;}
            if now>=p.deadline {
                if self.phase==Phase::Abort {self.fenced=true;self.finish(View::Uncertain,w::TIMEOUT);}
                else {self.recover(View::Unavailable,w::TIMEOUT,false);}return;
            }
            let result=send(p.request);erase_words(&mut p.request);
            match result {
                Ok(())=>{let p=self.control.as_mut().unwrap();p.accepted=true;erase_words(&mut p.request);
                    self.control_event=Some(ControlEvent{phase:ControlPhase::Admitted,kind:p.request.kind,sequence:p.request.seq,object:p.request.object,value:None});
                    if self.phase==Phase::Create{erase_name(&mut self.name);}},
                Err(sex_pdx::ERR_SERVICE_NOT_READY)=>{},
                Err(_)=>{self.control=None;if self.phase==Phase::Abort{self.fenced=true;self.finish(View::Uncertain,w::DENIED);}else{self.recover(View::Unavailable,w::DENIED,false);}},
            }return;
        }
        for _ in 0..w::WINDOW {
            let retry=self.window.iter().position(|p|p.is_some_and(|p|!p.accepted));
            let index=if let Some(i)=retry{i}else{
                if self.issued==self.len{break;}let Some(i)=self.window.iter().position(Option::is_none)else{break;};
                let count=(self.len-self.issued).min(16);let mut raw=[0u8;16];
                raw[..count].copy_from_slice(&self.body.as_ref().unwrap().as_bytes()[self.issued..self.issued+count]);
                let mut a=u64::from_le_bytes(raw[..8].try_into().unwrap());let mut b=u64::from_le_bytes(raw[8..].try_into().unwrap());erase_name(&mut raw);
                self.window[i]=self.allocate(w::WRITE,self.issued as u32,a,b,count,now);
                unsafe {core::ptr::write_volatile(&mut a,0);core::ptr::write_volatile(&mut b,0);}
                if !self.busy{return;}self.issued+=count;i
            };
            let mut p=self.window[index].unwrap();
            let result=send(p.request);erase_words(&mut p.request);
            match result {
                Ok(())=>{let p=self.window[index].as_mut().unwrap();p.accepted=true;erase_words(&mut p.request);},
                Err(sex_pdx::ERR_SERVICE_NOT_READY)=>break,
                Err(_)=>{self.window[index]=None;self.recover(View::Unavailable,w::DENIED,false);break;},
            }
        }
    }
    pub fn reply(&mut self, word:u64, now:u64) {
        self.expired(now);if !self.busy{return;}
        if let Some(p)=self.control.filter(|p|p.accepted) {
            let Some(value)=w::decode_reply(word,p.request.seq)else{return;};
            self.control_event=Some(ControlEvent{phase:ControlPhase::Reply,kind:p.request.kind,sequence:p.request.seq,object:p.request.object,value:Some(value)});
            self.control=None;
            if self.recovering && p.request.kind!=w::ABORT {
                if p.request.kind==w::CREATE {match value {
                    Ok(id)=>{self.created=true;if (3..15).contains(&id){self.object=id as u8;}else{self.fenced=true;}},
                    Err(w::EXISTS) if self.target!=View::Cancelled=>{self.target=View::Exists;self.error=w::EXISTS;},
                    _=>{},
                }}
                self.cleanup_ready();return;
            }
            let v=match value {Ok(v)=>v,Err(error)=>{
                if p.request.kind==w::ABORT{self.fenced=true;self.finish(View::Uncertain,error);}
                else if p.request.kind==w::CREATE{self.finish(if error==w::EXISTS{View::Exists}else{View::Unavailable},error);}
                // A negative COMMIT can follow durable journal publication:
                // flush/readback failure is not proof that no effect occurred.
                // The receipt ABI does not identify a pre-effect rejection.
                else if p.request.kind==w::COMMIT{self.fenced=true;self.recover(View::Unavailable,error,true);}
                else{self.recover(View::Unavailable,error,false);}return;
            }};
            match p.request.kind {
                w::CREATE=>{self.created=true;if !(3..15).contains(&v){self.fenced=true;self.recover(View::Unavailable,w::INVALID,true);return;}self.object=v as u8;self.phase=Phase::Info;},
                w::INFO=>{if v==0{self.recover(View::Unavailable,w::INVALID,false);return;}self.generation=v;self.phase=Phase::Begin;},
                w::BEGIN_SAVE=>{if v!=0{self.recover(View::Unavailable,w::INVALID,false);return;}self.phase=if self.len==0{Phase::Commit}else{Phase::Write};},
                w::COMMIT=>{if v as usize!=self.len{self.fenced=true;self.recover(View::Unavailable,w::INVALID,true);return;}self.committed=true;self.phase=Phase::End;},
                w::END=>{if v!=0{self.recover(View::Unavailable,w::INVALID,true);return;}self.created=false;self.finish(View::Saved,0);},
                w::ABORT=>{if v!=0{self.fenced=true;self.finish(View::Uncertain,w::INVALID);}else{self.created=false;self.finish(self.target,self.error);}},
                _=>{},
            }return;
        }
        let Some((i,p,value))=self.window.iter().enumerate().find_map(|(i,p)|{let p=(*p)?;if !p.accepted{return None;}w::decode_reply(word,p.request.seq).map(|v|(i,p,v))})else{return;};
        self.window[i]=None;
        if self.recovering {self.cleanup_ready();return;}
        match value {
            Ok(v)if v as usize==p.count=>{self.completed+=p.count;if self.completed==self.len{self.phase=Phase::Commit;}},
            Ok(_)=>self.recover(View::Unavailable,w::INVALID,false),Err(e)=>self.recover(View::Unavailable,e,false),
        }
    }
}
impl<D:Payload> Drop for Client<D> { fn drop(&mut self){self.discard_plaintext();} }

#[cfg(test)]
mod sequence_tests {
    use super::*;
    struct Empty;
    impl Payload for Empty {fn as_bytes(&self)->&[u8]{&[]}}
    #[test]
    fn exhaustion_reserves_cleanup_and_never_wraps_or_resets() {
        let mut c=Client::new();let mut name=[0;16];name[0]=b'x';
        c.seq=w::SEQ_MAX-6;assert!(c.start(Empty,name,0).is_err());
        c.seq=w::SEQ_MAX-7;assert!(c.start(Empty,name,0).is_ok());
        for (now,kind,value) in [(0,w::CREATE,3),(1,w::INFO,1),(2,w::BEGIN_SAVE,0),(3,w::COMMIT,0),(4,w::END,0)] {
            let mut request=None;c.advance_with(now,|r|{request=Some(r);Ok(())});let r=request.unwrap();assert_eq!(r.kind,kind);
            let retained=c.control.unwrap();assert_eq!((retained.request.a,retained.request.b),(0,0));
            c.reply(w::reply(r.seq,Ok(value)),now);
        }
        assert_eq!(c.view(),View::Saved);let seq=c.seq;c.clear(5);assert_eq!(c.seq,seq);
        assert!(c.start(Empty,name,6).is_err());assert_eq!(c.seq,seq);
        c.advance_with(7,|_|panic!("no sequence wrap"));
    }
}
