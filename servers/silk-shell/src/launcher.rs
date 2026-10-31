//! Launcher policy over an exact immutable BOOT incarnation. This selects an
//! existing instance; neither a descriptor nor a terminal ID authorizes restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub domain:u32, pub task:u32, pub surface:u64, pub coordinator:u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence { Live, Dead { domain:u32, task:u32, required:u64, acked:u64 }, Retry, Invalid }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status { Running, Closing, Closed, Unavailable }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum LiveAction { Open, Restore, BringHere, Focus }
impl LiveAction {
    pub fn title(self,quil:bool)->&'static str {
        match(self,quil){
            (Self::Open,true)=>"Open Quil",(Self::Restore,true)=>"Restore Quil",(Self::BringHere,true)=>"Bring Here Quil",(Self::Focus,true)=>"Focus Quil",
            (Self::Open,false)=>"Open Spindle",(Self::Restore,false)=>"Restore Spindle",(Self::BringHere,false)=>"Bring Here Spindle",(Self::Focus,false)=>"Focus Spindle",
        }
    }
}
/// Presentation data only. Callers still recheck the held exact live catalog
/// before moving, restoring or focusing its existing frame.
pub fn live_action(frame:Option<(u8,bool,bool)>,active:u8)->LiveAction {
    match frame {
        None=>LiveAction::Open,
        Some((scene,_,_))if scene!=active=>LiveAction::BringHere,
        Some((_,minimized,visible))if minimized||!visible=>LiveAction::Restore,
        Some(_)=>LiveAction::Focus,
    }
}
impl Status {
    pub fn presentation(self)->(bool,&'static str,&'static str) {
        match self {
            Self::Running=>(true,"RUNNING / FOCUS","original_incarnation_live"),
            Self::Closing=>(false,"CLOSING / PLEASE WAIT","incarnation_closing"),
            Self::Closed=>(false,"CLOSED / REBOOT TO REOPEN","no_boot_successor"),
            Self::Unavailable=>(false,"UNAVAILABLE / NO AUTHORITY","incarnation_unverified"),
        }
    }
}
pub fn resolve(expected:Identity, catalog:Option<Identity>, evidence:Evidence, closing:bool, terminal:bool)->Status {
    if expected.domain==0 || expected.task==0 || expected.surface==0 || expected.coordinator!=3
        || catalog!=Some(expected) {return Status::Unavailable;}
    match evidence {
        Evidence::Live if terminal=>Status::Unavailable,
        Evidence::Live if closing=>Status::Closing,
        Evidence::Live=>Status::Running,
        Evidence::Dead{domain,task,required,acked} if domain==expected.domain && task==expected.task
            && required==31 && acked&!required==0=>Status::Closed,
        Evidence::Retry=>Status::Closing,
        _=>Status::Unavailable,
    }
}

#[derive(Clone,Copy)]
pub struct Row<'a> { pub name:&'a str, pub status:&'a str }
pub struct View { pub bytes:[u8;512], pub len:usize, pub first:usize, pub rows:usize, pub scale:u32, pub row_y:u32, pub row_h:u32 }
impl View {
    fn line(&mut self, prefix:&[u8], value:&str, cols:usize) {
        for &b in prefix.iter().chain(value.as_bytes()).take(cols.saturating_sub(1)) {
            if self.len>=511 {break;}
            self.bytes[self.len]=if (32..=126).contains(&b) {b} else {b'?'};self.len+=1;
        }
        if self.len<512 {self.bytes[self.len]=b'\n';self.len+=1;}
    }
}
pub fn text(rows:&[Row<'_>],selected:usize,w:u32,h:u32)->View {
    let scale=if w>=256 {2}else{1};let line_h=9*scale;
    let capacity=(h.saturating_sub(44)/line_h).saturating_sub(2) as usize/2;
    let visible=capacity.min(5).min(rows.len());
    let first=if visible==0 {0}else{selected.min(rows.len().saturating_sub(1)).saturating_sub(visible-1)};
    let mut v=View{bytes:[0;512],len:0,first,rows:visible,scale,row_y:36+2*line_h,row_h:2*line_h};
    // At most twelve lines fit in 512 bytes even with future long labels.
    let cols=((w.saturating_sub(16)/(6*scale)) as usize).min(40);
    if cols<3 || visible==0 {v.rows=0;return v;}
    v.line(b"","APPS / ACTIONS",cols);v.line(b"","J/K CHOOSE  ENTER OPEN  ESC CLOSE",cols);
    for (index,row) in rows.iter().enumerate().skip(first).take(visible) {
        v.line(if index==selected {b"> "}else{b"  "},row.name,cols);
        v.line(b"  ",row.status,cols);
    }
    v
}

/// Finite reserve is descriptive data, never permission to activate. Only
/// states returned through both exact BOOT activation grants reach this table.
pub fn quil_reserve(first:u64,third:u64)->Option<u8>{
    match(first,third){(0|1,0)=>Some(2),(2,0)|(3,0|1)=>Some(1),(3,2|3)=>Some(0),_=>None}
}
pub fn reserve_label<'a>(fallback:&'a str,available:bool,reason:&str,count:Option<u8>)->&'a str{
    if !available{return fallback;}
    match reason {
        "original_incarnation_live"=>match count {Some(2)=>"RUNNING / 2 REOPENS LEFT",Some(1)=>"RUNNING / 1 REOPEN LEFT",Some(0)=>"RUNNING / LAST INSTANCE",Some(3..=u8::MAX)=>"RUNNING / REOPENS AVAILABLE",_=>"RESERVE CHECK / FOCUS"},
        "boot_successor_ready"=>match count {Some(2)=>"REOPEN / 2 RESERVED LEFT",Some(1)=>"REOPEN / LAST RESERVED",Some(3..=u8::MAX)=>"REOPEN / RESERVES READY",_=>"RESERVE CHECK / WAIT"},
        _=>fallback,
    }
}
