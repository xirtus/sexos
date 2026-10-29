//! Presentation data on existing BOOT app/shell routes; grants no authority.
pub const OP:u64=0x271;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]#[repr(u64)]pub enum State{Ready=1,Loading=2,Saving=3,Queued=4,Cancelling=5,Uncertain=6,Denied=7,RetryReady=8}
impl State{fn decode(v:u64)->Option<Self>{Some(match v{1=>Self::Ready,2=>Self::Loading,3=>Self::Saving,4=>Self::Queued,5=>Self::Cancelling,6=>Self::Uncertain,7=>Self::Denied,8=>Self::RetryReady,_=>return None})}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Notice{pub target:u32,pub task:u32,pub sequence:u64,pub state:State}
impl Notice{pub fn args(self)->[u64;3]{[self.target as u64|((self.task as u64)<<32),self.sequence,self.state as u64]}}
pub struct Publisher{target:u32,task:u32,sequence:u64,last:Option<State>,pending:Option<Notice>,closed:bool}
impl Publisher{
 pub const fn new(target:u32,task:u32)->Self{Self{target,task,sequence:0,last:None,pending:None,closed:!(matches!(target,12|17)&&target==task)}}
 pub fn update(&mut self,state:State){if self.closed||self.last==Some(state){return}if self.sequence==u64::MAX{self.cancel();return}self.sequence+=1;self.last=Some(state);self.pending=Some(Notice{target:self.target,task:self.task,sequence:self.sequence,state});}
 // A verified shell viewport proves the receiver has bound this incarnation;
 // refresh a status that might have preceded its custody registration.
 pub fn refresh(&mut self){self.last=None;}
 pub fn pending(&self)->bool{self.pending.is_some()&&!self.closed}
 pub fn cancel(&mut self){self.pending=None;self.closed=true;}
 pub fn poll(&mut self,mut send:impl FnMut(Notice)->Result<(),u64>){if self.closed{return}if let Some(n)=self.pending{match send(n){Ok(())=>self.pending=None,Err(crate::ERR_SERVICE_NOT_READY)=>{},Err(_)=>self.cancel()}}}
}
pub struct Receiver{target:u32,task:u32,sequence:u64,state:Option<State>,dead:bool}
impl Receiver{
 pub const fn new(target:u32,task:u32)->Self{Self{target,task,sequence:0,state:None,dead:!(matches!(target,12|17)&&target==task)}}
 pub fn state(&self)->Option<State>{self.state}
 pub fn retire(&mut self){self.dead=true;self.state=None;}
 /// `live` must come from the exact immutable kernel catalog and LIVE query,
 /// never from a caller's notice or a chosen surface identifier.
 pub fn accept(&mut self,caller:u32,live:bool,args:[u64;3])->bool{if self.dead||!live||caller!=self.target||args[0]!=(self.target as u64|((self.task as u64)<<32))||args[1]<=self.sequence{return false}let Some(state)=State::decode(args[2])else{return false};self.sequence=args[1];self.state=Some(state);true}
}
