//! Bounded nonblocking submission on the existing BOOT display edge.
//! One immutable active text batch and one latest desired batch per pane.
use super::viewport::Text;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Operation{pub opcode:u64,pub surface:u64,pub a:u64,pub b:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Stamp{pub sequence:u64,pub paint:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Receipt{pub surface:u64,pub stamp:Stamp,pub bytes:usize}
#[derive(Clone,Copy)]struct Batch{text:Text,stamp:Stamp,color:u32,cleared:bool,chunks:usize}
impl Batch{fn new(text:Text,stamp:Stamp,color:u32)->Self{Self{text,stamp,color,cleared:false,chunks:(text.len+7)/8}}}
#[derive(Clone,Copy)]struct Pane{control:Option<Operation>,active:Option<Batch>,latest:Option<Batch>,hidden:bool}
impl Pane{
 const EMPTY:Self=Self{control:None,active:None,latest:None,hidden:false};
 fn discard(&mut self){if let Some(b)=&mut self.active{b.text.bytes.fill(0)}if let Some(b)=&mut self.latest{b.text.bytes.fill(0)}self.active=None;self.latest=None;}
 fn pending(&self)->bool{self.control.is_some()||self.active.is_some()||self.latest.is_some()}
}
pub struct Outbox{ids:[u64;4],panes:[Pane;4],cursor:usize,closed:bool}
impl Outbox{
 pub const fn new(target:u32)->Self{let ids=match target{12=>[154,160,161,162],17=>[302,303,304,305],_=>[0;4]};Self{ids,panes:[Pane::EMPTY;4],cursor:0,closed:!matches!(target,12|17)}}
 pub fn create(&mut self,index:usize,position:u64,size:u64){if self.closed||index>=4{return}let p=&mut self.panes[index];p.hidden=false;p.control=Some(Operation{opcode:0xEC,surface:self.ids[index],a:position,b:size});}
 pub fn destroy(&mut self,index:usize){if self.closed||index>=4{return}let p=&mut self.panes[index];p.discard();p.hidden=true;p.control=Some(Operation{opcode:0xEE,surface:self.ids[index],a:0,b:0});}
 pub fn text(&mut self,index:usize,text:Text,stamp:Stamp,color:u32){if self.closed||index>=4||text.len>text.bytes.len(){return}let p=&mut self.panes[index];if p.hidden{return}let batch=Batch::new(text,stamp,color);if p.active.is_none(){p.active=Some(batch)}else{p.latest=Some(batch)}}
 pub fn pending(&self)->bool{!self.closed&&self.panes.iter().any(Pane::pending)}
 pub fn cancel(&mut self)->usize{let mut dropped=0;for p in &mut self.panes{dropped+=p.pending()as usize;p.discard();p.control=None;p.hidden=true;}self.closed=true;dropped}
 pub fn poll(&mut self,budget:usize,mut send:impl FnMut(Operation)->Result<(),u64>,mut complete:impl FnMut(Receipt))->Result<usize,u64>{
  let mut attempts=0;if self.closed{return Ok(0)}
  while attempts<budget{
   let mut selected=None;for _ in 0..4{let i=self.cursor;self.cursor=(self.cursor+1)%4;if self.panes[i].pending(){selected=Some(i);break}}
   let Some(index)=selected else{break};let p=&mut self.panes[index];if p.active.is_none(){p.active=p.latest.take()}
   let op=if let Some(op)=p.control{op}else{let b=p.active.as_ref().unwrap();if !b.cleared{Operation{opcode:0xFA,surface:self.ids[index],a:0,b:0}}else{
    let off=(b.chunks-1)*8;let n=(b.text.len-off).min(8);let mut word=[0;8];word[..n].copy_from_slice(&b.text.bytes[off..off+n]);Operation{opcode:0xFB,surface:self.ids[index],a:u64::from_le_bytes(word),b:off as u64|((n as u64)<<8)|((b.color as u64)<<32)}
   }};attempts+=1;
   match send(op){Err(sex_pdx::ERR_SERVICE_NOT_READY)=>return Ok(attempts),Err(e)=>{self.cancel();return Err(e)},Ok(())=>{}}
   let p=&mut self.panes[index];if p.control.is_some(){p.control=None;continue}
   let b=p.active.as_mut().unwrap();if !b.cleared{b.cleared=true}else{b.chunks-=1}
   if b.chunks==0{let receipt=Receipt{surface:self.ids[index],stamp:b.stamp,bytes:b.text.len};b.text.bytes.fill(0);p.active=None;complete(receipt);}
  }Ok(attempts)
 }
}
