// SPDX-License-Identifier: GPL-3.0-only
//! Explicit finite seed custody over the held BOOT SexNet route. No logs,
//! hashes, fallback, automatic retry or source/consumer selector lives here.
//! Returned seed bytes transfer private custody; this is not a quality proof.
use sex_pdx::entropy_seed::{self as a,Request,State};
const RPC_TICKS:u64=2000;
const POLL_TICKS:u64=25;
const START_RESERVE:u32=12;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View {Idle,Pending,Ready,Unavailable,Cancelled,Uncertain}
#[derive(Clone,Copy,PartialEq,Eq)]
enum Step {Done,Begin,Poll,Read,Cancel,Release,Held}
pub struct Client {
    step:Step,seq:u32,ticket:u32,pending:Option<Request>,accepted:bool,
    deadline:u64,next_poll:u64,last_now:u64,clock_seen:bool,
    abandon:bool,closed:bool,view:View,terminal:View,
    seed:[u8;a::SEED_BYTES],read:usize,produced:usize,
}
// Private references only; never ABI/caller pointers. Volatile erasure prevents
// dead-store elimination, without claiming register/cache/physical erasure.
fn zero_bytes(bytes:&mut[u8]){for byte in bytes{unsafe{core::ptr::write_volatile(byte,0)}}}
fn zero_word(word:&mut u64){unsafe{core::ptr::write_volatile(word,0)}}
fn zero_value(value:&mut u32){unsafe{core::ptr::write_volatile(value,0)}}
impl Client {
    pub const fn new()->Self{
        Self{step:Step::Done,seq:1,ticket:0,pending:None,accepted:false,
            deadline:0,next_poll:0,last_now:0,clock_seen:false,
            abandon:false,closed:false,view:View::Idle,terminal:View::Unavailable,
            seed:[0;a::SEED_BYTES],read:0,produced:0}
    }
    pub fn busy(&self)->bool{!self.closed&&(self.step!=Step::Done||self.ticket!=0)}
    pub fn view(&self)->View{self.view}
    pub fn next_sequence(&self)->u32{self.seq}
    fn scrub(&mut self){zero_bytes(&mut self.seed);self.read=0;self.produced=0;}
    fn done(&mut self){
        self.step=Step::Done;self.pending=None;self.accepted=false;self.ticket=0;
        self.deadline=0;self.next_poll=0;self.clock_seen=false;self.last_now=0;
        self.view=self.terminal;
        if self.view!=View::Ready{self.scrub();}
    }
    pub fn start(&mut self)->bool{
        if self.closed||self.busy(){return false;}
        self.scrub();
        if self.seq==0||self.seq>a::MAX_SEQ-START_RESERVE{self.view=View::Unavailable;return false;}
        self.step=Step::Begin;self.pending=None;self.accepted=false;self.abandon=false;
        self.view=View::Pending;self.terminal=View::Unavailable;self.deadline=0;
        self.next_poll=0;self.clock_seen=false;self.last_now=0;true
    }
    /// Copies only after positive RELEASE. The caller now owns the returned
    /// bytes and must erase them after use; this client's private copy is wiped.
    pub fn take_seed(&mut self)->Option<[u8;a::SEED_BYTES]>{
        if self.closed||self.busy()||self.view!=View::Ready||self.read!=a::SEED_BYTES{return None;}
        let seed=self.seed;self.scrub();self.view=View::Idle;self.terminal=View::Unavailable;Some(seed)
    }
    pub fn cancel(&mut self){
        if self.closed{return;}
        self.abandon=true;self.terminal=View::Cancelled;self.scrub();
        if !self.busy(){self.view=View::Cancelled;return;}
        if self.step==Step::Begin&&!self.accepted{
            if let Some(r)=self.pending.take(){self.seq=r.seq+1;}
            self.done();return;
        }
        // A known pre-effect Full can be abandoned. Accepted correlation must
        // remain intact, including late BEGIN/READ/RELEASE after this cancel.
        if !self.accepted&&self.pending.is_some_and(|r|r.kind!=a::RELEASE){
            let r=self.pending.take().unwrap();self.seq=r.seq+1;
        }
        if self.step==Step::Held{
            if self.ticket!=0&&self.seq<=a::MAX_SEQ{self.step=Step::Release;}else{self.view=View::Uncertain;return;}
        }
        self.view=View::Cancelled;
    }
    /// Terminal-only local stop. It is not a positive cleanup receipt. Parent
    /// must task_finish and let the service retire this exact live incarnation.
    pub fn close(&mut self){
        self.scrub();self.closed=true;self.pending=None;self.accepted=false;
        self.step=Step::Done;self.view=View::Cancelled;self.terminal=View::Cancelled;
    }
    fn rejected(&mut self,kind:u8){
        self.scrub();self.abandon=true;
        if self.terminal!=View::Cancelled{self.terminal=View::Unavailable;}
        if self.ticket==0{
            if kind==a::BEGIN{self.done();}else{self.step=Step::Held;self.view=View::Uncertain;}
        }else if kind==a::RELEASE{self.step=Step::Held;self.view=View::Uncertain;}
        else{self.step=Step::Release;self.view=self.terminal;}
    }
    fn uncertain(&mut self){
        self.scrub();self.abandon=true;
        if self.terminal!=View::Cancelled{self.terminal=View::Unavailable;}
        self.view=View::Uncertain;
    }
    pub fn reply(&mut self,mut word:u64,now:u64){
        self.receive(&mut word,now);zero_word(&mut word);
    }
    fn receive(&mut self,word:&mut u64,now:u64){
        if self.closed{return;}
        let Some(r)=self.pending else{return;};
        if !self.accepted||!matches!(*word>>56,0x7A|0xEA)||((*word>>32)&a::MAX_SEQ as u64)!=r.seq as u64{return;}
        if (self.clock_seen&&now<self.last_now)||now>=self.deadline{self.uncertain();}
        self.last_now=now;self.clock_seen=true;
        self.pending=None;self.accepted=false;self.seq=r.seq+1;
        if *word>>56==0xEA{self.rejected(r.kind);return;}
        let mut value=*word as u32;self.acknowledged(r,&mut value,now);zero_value(&mut value);
    }
    fn acknowledged(&mut self,r:Request,value:&mut u32,now:u64){
        if self.abandon&&!matches!(r.kind,a::BEGIN|a::CANCEL|a::RELEASE){self.step=Step::Cancel;return;}
        match r.kind{
            a::BEGIN=>{
                if *value==0||*value>a::MAX_TICKET{self.uncertain();self.step=Step::Held;return;}
                self.ticket=*value;self.step=Step::Poll;self.next_poll=now;
            },
            a::STATUS=>match a::decode_status(*value){
                Some((State::Collecting,n))if n<32&&n%8==0&&n>=self.produced&&self.read==0=>{
                    self.produced=n;self.step=Step::Poll;self.next_poll=now.saturating_add(POLL_TICKS);
                },
                Some((State::Ready,a::SEED_BYTES))if self.read==0=>{self.produced=a::SEED_BYTES;self.step=Step::Read;},
                _=>{self.rejected(r.kind);return;},
            },
            a::READ=>{
                let n=r.length as usize;let end=self.read+n;
                if self.produced!=a::SEED_BYTES||r.offset as usize!=self.read
                    ||n==0||n>a::READ_BYTES||end>a::SEED_BYTES
                    ||(n<4&&*value>>(n*8)!=0){self.rejected(r.kind);return;}
                let mut bytes=value.to_le_bytes();self.seed[self.read..end].copy_from_slice(&bytes[..n]);zero_bytes(&mut bytes);
                self.read=end;
                if self.read==a::SEED_BYTES{self.terminal=View::Ready;self.step=Step::Release;}
            },
            a::CANCEL=>{if *value!=0{self.rejected(r.kind);return;}self.scrub();self.step=Step::Release;},
            a::RELEASE=>{
                if *value!=0{self.rejected(r.kind);return;}
                // A late successful cleanup cannot restore cancelled/uncertain
                // bytes. Ready needs every byte plus this exact positive ACK.
                if self.abandon||self.read!=a::SEED_BYTES||self.terminal!=View::Ready{
                    self.scrub();if self.terminal!=View::Cancelled{self.terminal=View::Unavailable;}
                }
                self.done();return;
            },
            _=>{self.rejected(r.kind);return;},
        }
        if self.abandon&&self.ticket!=0&&!matches!(self.step,Step::Release|Step::Held){self.step=Step::Cancel;}
    }
    pub fn advance(&mut self,now:u64){
        if !self.busy()||self.step==Step::Held{return;}
        let backwards=self.clock_seen&&now<self.last_now;self.clock_seen=true;self.last_now=now;
        if self.pending.is_some()&&(backwards||now>=self.deadline){
            if self.accepted{self.uncertain();self.deadline=u64::MAX;return;}
            let r=self.pending.take().unwrap();self.seq=r.seq+1;self.rejected(r.kind);
            if !self.busy()||self.step==Step::Held{return;}
        }else if backwards{
            self.uncertain();
            if self.ticket==0{self.terminal=View::Unavailable;self.done();return;}
        }
        if self.pending.is_none(){
            if self.abandon&&self.ticket!=0&&self.step!=Step::Release{self.step=Step::Cancel;}
            if self.step==Step::Poll&&now<self.next_poll{return;}
            if self.seq>=a::MAX_SEQ-1&&!matches!(self.step,Step::Cancel|Step::Release){
                self.scrub();self.abandon=true;self.terminal=View::Unavailable;
                if self.ticket==0{self.done();return;}
                self.step=if self.seq==a::MAX_SEQ{Step::Release}else{Step::Cancel};
            }
            if self.seq==0||self.seq>a::MAX_SEQ{self.uncertain();self.step=Step::Held;return;}
            let Some(deadline)=now.checked_add(RPC_TICKS)else{
                self.uncertain();if self.ticket==0{self.terminal=View::Unavailable;self.done();}else{self.step=Step::Held;}return;
            };
            let(kind,offset,length)=match self.step{
                Step::Begin=>(a::BEGIN,0,a::SEED_BYTES),Step::Poll=>(a::STATUS,0,0),
                Step::Read=>(a::READ,self.read,(a::SEED_BYTES-self.read).min(a::READ_BYTES)),
                Step::Cancel=>(a::CANCEL,0,0),Step::Release=>(a::RELEASE,0,0),Step::Done|Step::Held=>return,
            };
            self.pending=Some(Request{seq:self.seq,kind,offset:offset as u16,length:length as u16,ticket:if kind==a::BEGIN{0}else{self.ticket},a:0});
            self.accepted=false;self.deadline=deadline;
        }
        if !self.accepted{
            let r=self.pending.unwrap();let args=r.encode();
            let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,a::OP,args[0],args[1],args[2]);
            match status{0=>self.accepted=true,sex_pdx::ERR_SERVICE_NOT_READY=>{},_=>{self.pending=None;self.seq=r.seq+1;self.rejected(r.kind);}}
        }
    }
    #[cfg(test)]pub fn private_scrubbed(&self)->bool{self.seed.iter().all(|b|*b==0)&&self.read==0&&self.produced==0}
}

#[cfg(test)]mod internal_tests{
    use super::*;
    #[test]fn exhausted_start_clears_previous_seed_and_never_wraps(){
        let mut c=Client::new();c.seq=a::MAX_SEQ-START_RESERVE+1;c.seed.fill(19);assert!(!c.start());assert!(c.private_scrubbed()&&!c.busy()&&c.view()==View::Unavailable);
        c.seq=a::MAX_SEQ+1;assert!(!c.start());assert!(c.next_sequence()==a::MAX_SEQ+1);
    }
    #[test]fn final_two_sequences_reserve_cancel_then_positive_release(){
        let mut c=Client::new();c.seq=a::MAX_SEQ-1;c.ticket=1;c.step=Step::Cancel;c.view=View::Pending;c.abandon=true;
        let r=Request{seq:c.seq,kind:a::CANCEL,offset:0,length:0,ticket:1,a:0};c.pending=Some(r);c.accepted=true;c.deadline=100;
        c.reply(a::reply(r.seq,0),1);assert!(c.step==Step::Release&&c.seq==a::MAX_SEQ);
        let r=Request{seq:c.seq,kind:a::RELEASE,..r};c.pending=Some(r);c.accepted=true;
        c.reply(a::reply(r.seq,0),2);assert!(!c.busy()&&c.private_scrubbed()&&c.next_sequence()==a::MAX_SEQ+1);assert!(!c.start());
    }
}
