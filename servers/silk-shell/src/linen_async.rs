//! One outstanding Linen request, bounded staging, no receive loop or heap.
//! Rescan and OpenIntent acknowledgments cannot become snapshot metadata.
pub const CAPACITY: usize = 16;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Object { pub id:u64, pub kind:u8, pub name:[u8;24], pub len:u8, pub disk:u8 }
impl Object { const EMPTY:Self=Self{id:0,kind:0,name:[0;24],len:0,disk:0}; }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Request { pub opcode:u64, pub args:[u64;3] }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Phase { Idle, Rescan, Metadata, Name, Open(u64,u64) }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Reply { Ignored, Progress, Published, Rejected, OpenComplete(u64) }
pub struct Snapshot {
    phase:Phase, awaiting:bool, refresh:bool, rescan:bool,
    slot:u8, offset:u8, current:Object,
    pub objects:[Option<Object>;CAPACITY], pub count:usize,
    opens:[Option<(u64,u64)>;4], open_count:usize,
}
impl Snapshot {
    pub const fn new()->Self {Self{phase:Phase::Idle,awaiting:false,refresh:false,rescan:false,slot:0,offset:0,current:Object::EMPTY,objects:[None;CAPACITY],count:0,opens:[None;4],open_count:0}}
    pub fn refresh(&mut self,rescan:bool){self.refresh=true;self.rescan|=rescan;}
    pub fn queue_open(&mut self,id:u64,choice:u64)->bool {
        if id==0 || self.open_count==self.opens.len(){return false}
        self.opens[self.open_count]=Some((id,choice));self.open_count+=1;true
    }
    pub fn busy(&self)->bool{self.phase!=Phase::Idle || self.refresh}
    pub fn can_send(&self)->bool{!self.awaiting && (self.phase!=Phase::Idle || self.refresh || self.open_count!=0)}
    pub fn request(&mut self)->Option<Request>{
        if self.awaiting{return None}
        if self.phase==Phase::Idle {
            if self.open_count!=0 {
                let (id,choice)=self.opens[0].take().unwrap();
                for i in 1..self.open_count{self.opens[i-1]=self.opens[i];}
                self.open_count-=1;self.opens[self.open_count]=None;
                self.phase=Phase::Open(id,choice);
            }else if self.refresh{
                self.refresh=false;self.slot=0;self.offset=0;self.count=0;self.objects=[None;CAPACITY];
                self.phase=if self.rescan{Phase::Rescan}else{Phase::Metadata};self.rescan=false;
            }
        }
        Some(match self.phase {
            Phase::Idle=>return None,
            Phase::Rescan=>Request{opcode:0x49,args:[0;3]},
            Phase::Metadata=>Request{opcode:0x44,args:[self.slot as u64,0,0]},
            Phase::Name=>Request{opcode:0x45,args:[self.current.id,self.offset as u64,8]},
            Phase::Open(id,choice)=>Request{opcode:0x46,args:[id,choice,0]},
        })
    }
    pub fn sent(&mut self){assert!(self.phase!=Phase::Idle && !self.awaiting);self.awaiting=true;}
    fn failed(&mut self)->Reply{self.phase=Phase::Idle;Reply::Rejected}
    fn next_slot(&mut self)->Reply{
        self.slot+=1;
        if self.slot as usize==CAPACITY{self.phase=Phase::Idle;Reply::Published}else{self.phase=Phase::Metadata;Reply::Progress}
    }
    fn push_current(&mut self)->Reply{self.objects[self.count]=Some(self.current);self.count+=1;self.next_slot()}
    pub fn reply(&mut self,caller:u32,opcode:u64,value:u64)->Reply {
        if caller!=7 || opcode!=1 || !self.awaiting{return Reply::Ignored}
        self.awaiting=false;
        match self.phase {
            Phase::Idle=>Reply::Ignored,
            Phase::Open(..)=>{self.phase=Phase::Idle;Reply::OpenComplete(value)},
            Phase::Rescan=>{if (value as i64)<0{return self.failed()}self.phase=Phase::Metadata;Reply::Progress},
            Phase::Metadata=>{
                if value==0{return self.next_slot()}
                let id=value&0xffff_ffff;let kind=(value>>32)as u8;let len=(value>>40)as u8;
                if id==0 || kind>2 || len>24 || self.objects.iter().flatten().any(|o|o.id==id){return self.failed()}
                self.current=Object{id,kind,name:[0;24],len,disk:(value>>48)as u8};self.offset=0;
                if len==0{self.push_current()}else{self.phase=Phase::Name;Reply::Progress}
            }
            Phase::Name=>{
                if value==0 || (value as i64)<0{return self.failed()}
                let bytes=value.to_le_bytes();let take=core::cmp::min(8,self.current.len-self.offset);
                for i in 0..take as usize {let b=bytes[i];self.current.name[self.offset as usize+i]=if (0x20..=0x7e).contains(&b){b}else{b'?'};}
                self.offset+=take;
                if self.offset==self.current.len{self.push_current()}else{Reply::Progress}
            }
        }
    }
}
