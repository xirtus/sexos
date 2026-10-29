//! Bounded terminal layout data over existing BOOT shell→Spindle edges.
//! Rectangles are relative to the shell frame, never pointers or authority.
pub const OP:u64=0x270;
pub const MAX_TEXT:usize=126;
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub struct Rect{pub x:u32,pub y:u32,pub w:u32,pub h:u32}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Layout{pub panes:[Rect;4],pub cols:usize,pub rows:[usize;2]}
pub const fn scale(width:u32)->u32{if width>=256{2}else{1}}
fn rows(height:u32,scale:u32)->usize{let usable=height.saturating_sub(12);if usable<7*scale{0}else{(1+(usable-7*scale)/(9*scale)).min(6)as usize}}
impl Layout{
 pub fn new(w:u32,h:u32)->Self{
  let bw=w.saturating_sub(16);let bh=h.saturating_sub(44);let cw=bw.saturating_sub(8)/2;let rw=bw.saturating_sub(cw+8);
  let max_scale=scale(cw).max(scale(rw));let split=bh>=2*(12+7*max_scale)+8;
  let top=if split{bh.saturating_sub(8)/2}else{0};let bottom=if split{bh.saturating_sub(top+8)}else{bh};let by=36+if split{top+8}else{0};
  let panes=[Rect{x:8,y:36,w:cw,h:top},Rect{x:16+cw,y:36,w:rw,h:top},Rect{x:8,y:by,w:cw,h:bottom},Rect{x:16+cw,y:by,w:rw,h:bottom}];
  // Keep one physical column free so an explicit newline cannot wrap twice.
  let cols=((cw.saturating_sub(16)/(6*scale(cw))).min(rw.saturating_sub(16)/(6*scale(rw)))).saturating_sub(1).min(20)as usize;
  let rows=if cols==0{[0,0]}else{[rows(top,max_scale),rows(bottom,max_scale)]};Self{panes,cols,rows}
 }
 pub fn columns(self)->usize{self.cols*2}
 pub fn lines(self)->usize{self.rows[0]+self.rows[1]}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Notice{pub target:u32,pub task:u32,pub w:u32,pub h:u32,pub sequence:u64}
impl Notice{
 pub fn args(self)->[u64;3]{[(self.task as u64)<<32|self.target as u64,(self.h as u64)<<32|self.w as u64,self.sequence]}
 pub fn decode(caller:u32,target:u32,task:u32,args:[u64;3],last:u64)->Option<Self>{
  let n=Self{target:args[0]as u32,task:(args[0]>>32)as u32,w:args[1]as u32,h:(args[1]>>32)as u32,sequence:args[2]};
  if caller!=3||!matches!(target,12|17)||n.target!=target||n.task!=task||n.sequence<=last||n.w==0||n.h==0||n.w>4096||n.h>4096{return None}Some(n)
 }
}
/// One desired notice per fixed BOOT subject; full queues retain exact data.
pub struct Delivery{desired:Option<Notice>,last:Option<(u32,u32)>,sequence:u64,dead:bool}
impl Delivery{
 pub const fn new()->Self{Self{desired:None,last:None,sequence:0,dead:false}}
 pub fn queue(&mut self,target:u32,task:u32,w:u32,h:u32){
  if self.dead||!matches!(target,12|17)||task==0||w==0||h==0||w>4096||h>4096{return}
  if self.desired.map(|n|(n.w,n.h)).or(self.last)==Some((w,h)){return}
  let Some(sequence)=self.sequence.checked_add(1)else{self.retire();return};self.sequence=sequence;self.desired=Some(Notice{target,task,w,h,sequence});
 }
 pub fn refresh(&mut self){if !self.dead{self.last=None;}}
 pub fn pending(&self)->bool{self.desired.is_some()}
 pub fn retire(&mut self){self.dead=true;self.desired=None;self.last=None;}
 pub fn poll(&mut self,live:bool,ready:bool,mut send:impl FnMut(Notice)->Result<(),u64>){
  let Some(n)=self.desired else{return};if !live{self.retire();return}if !ready{return}
  match send(n){Ok(())=>{self.last=Some((n.w,n.h));self.desired=None},Err(crate::ERR_SERVICE_NOT_READY)=>{},Err(_)=>self.retire()}
 }
}
