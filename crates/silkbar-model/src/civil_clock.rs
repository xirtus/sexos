//! Bounded civil snapshots, never an elapsed clock. Caller metadata is supplied
//! by the invocation boundary; a payload never chooses its own publisher.
use super::{SilkBarUpdate,UpdateKind};
pub const VERSION:u8=1;
/// Observation budgets, not civil seconds. Expiry only hides a snapshot.
pub const HEALTH_TICKS:u64=620;
pub const STAGNANT_TURNS:u32=65_536;
pub const TOTAL_TURNS:u32=1_048_576;
#[repr(u8)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum State {Unknown=0,Snapshot=1,Unavailable=2,Rollback=3,Exhausted=4}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Update {pub state:State,pub seconds:u32,pub sequence:u32}
impl Update {
    pub const fn encode(self)->SilkBarUpdate {
        SilkBarUpdate::new(UpdateKind::SetCivilClock as u32,(VERSION<<4)|self.state as u8,self.seconds,self.sequence)
    }
    fn decode(args:[u64;3],min:u32,max:u32)->Option<Self>{
        if args[0]!=UpdateKind::SetCivilClock as u64 || args[1]>>40!=0 || args[2]>u32::MAX as u64 || args[2]==0 || min>max{return None;}
        let index=(args[1]>>32)as u8;
        if index>>4!=VERSION{return None;}
        let state=match index&15 {0=>State::Unknown,1=>State::Snapshot,2=>State::Unavailable,3=>State::Rollback,4=>State::Exhausted,_=>return None};
        let seconds=args[1]as u32;
        if if state==State::Snapshot {seconds<min||seconds>max} else {seconds!=0} {return None;}
        Some(Self{state,seconds,sequence:args[2]as u32})
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Applied {New,Duplicate}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Reject {Publisher,Framing,Stale,Conflict,Terminal}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct CivilClock {
    state:State,seconds:u32,sequence:u32,high_water:u32,last:Option<Update>,stale:bool,
    admitted_tick:u64,observed_tick:u64,stagnant:u32,turns:u32,
}
impl CivilClock {
    pub const fn new()->Self {Self{state:State::Unknown,seconds:0,sequence:0,high_water:0,last:None,stale:false,admitted_tick:0,observed_tick:0,stagnant:0,turns:0}}
    pub const fn state(&self)->State {self.state}
    pub const fn seconds(&self)->u32 {self.seconds}
    pub const fn sequence(&self)->u32 {self.sequence}
    pub const fn high_water(&self)->u32 {self.high_water}
    pub const fn is_stale(&self)->bool {self.stale}
    /// Only a fresh, fully canonical update from the exact sanctioned producer
    /// can refresh observation health. Identical duplicates remain idempotent.
    pub fn receive(&mut self,caller:(u32,u32),expected:(u32,u32),args:[u64;3],min:u32,max:u32,now:u64)->Result<Applied,Reject>{
        if caller.0==0||caller.1==0||expected.0==0||expected.1==0||caller!=expected{return Err(Reject::Publisher);}
        let update=Update::decode(args,min,max).ok_or(Reject::Framing)?;
        if update.sequence==self.sequence {
            return if self.last==Some(update){Ok(Applied::Duplicate)}else{Err(Reject::Conflict)};
        }
        if update.sequence<self.sequence{return Err(Reject::Stale);}
        if matches!(self.state,State::Rollback|State::Exhausted){return Err(Reject::Terminal);}
        self.last=Some(update);self.sequence=update.sequence;self.stale=false;
        self.admitted_tick=now;self.observed_tick=now;self.stagnant=0;self.turns=0;
        self.state=update.state;self.seconds=0;
        if update.state==State::Snapshot {
            if update.seconds<self.high_water {self.state=State::Rollback;}
            else {self.high_water=update.seconds;self.seconds=update.seconds;}
        }
        // Reserve the final sequence as a terminal fence even if a producer
        // mistakenly spends it on a sample. No last numeric date lives forever.
        if update.sequence==u32::MAX && self.state!=State::Rollback {self.state=State::Exhausted;self.seconds=0;}
        Ok(Applied::New)
    }
    /// Monotonic observations are used only to expire presentation. Neither
    /// advancing ticks nor stagnant turns can manufacture a new civil second.
    /// A source high-water and its last sequence survive local staleness.
    pub fn health(&mut self,now:u64)->bool {
        if self.state!=State::Snapshot{return false;}
        self.turns=self.turns.saturating_add(1);
        let backwards=now<self.observed_tick;
        if now==self.observed_tick {self.stagnant=self.stagnant.saturating_add(1);}else{self.stagnant=0;}
        self.observed_tick=now;
        if backwards || now.saturating_sub(self.admitted_tick)>=HEALTH_TICKS || self.stagnant>=STAGNANT_TURNS || self.turns>=TOTAL_TURNS {
            self.state=State::Unknown;self.seconds=0;self.stale=true;return true;
        }
        false
    }
}
