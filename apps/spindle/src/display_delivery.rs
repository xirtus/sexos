//! Bounded nonblocking submission on the existing BOOT display edge.
//! Surface control per pane, and one atomic text batch for the terminal pane.
use super::viewport::Text;
use super::text_delivery;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Operation{pub opcode:u64,pub surface:u64,pub a:u64,pub b:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Stamp{pub sequence:u64,pub paint:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Receipt{pub surface:u64,pub stamp:Stamp,pub bytes:usize}
pub struct Outbox{ids:[u64;4],control:[Option<Operation>;4],hidden:bool,closed:bool,text:text_delivery::Outbox}
impl Outbox{
 pub const fn new()->Self{Self{ids:[0;4],control:[None;4],hidden:false,closed:true,text:text_delivery::Outbox::new(0)}}
 /// In place, for the static outbox. An unrecognized subject stays closed.
 pub fn bind(&mut self,target:u32){let Some(ids)=sex_pdx::app_catalog::spindle_grid(target)else{return};self.ids=ids;self.closed=false;self.text.bind_atomic(ids[0]);}
 pub fn create(&mut self,index:usize,position:u64,size:u64){if self.closed||index>=4{return}if index==0{self.hidden=false;}self.control[index]=Some(Operation{opcode:0xEC,surface:self.ids[index],a:position,b:size});}
 pub fn destroy(&mut self,index:usize){if self.closed||index>=4{return}if index==0{self.hidden=true;self.text.cancel();}self.control[index]=Some(Operation{opcode:0xEE,surface:self.ids[index],a:0,b:0});}
 pub fn text(&mut self,text:&Text,stamp:Stamp,color:u32){if self.closed||self.hidden||text.len>text.bytes.len(){return}
  self.text.queue(&text.bytes[..text.len],color,text_delivery::Metadata{count:stamp.paint as usize,first:stamp.sequence as usize,rows:text.rows,selected:0});}
 pub fn pending(&self)->bool{!self.closed&&(self.control.iter().any(Option::is_some)||self.text.pending())}
 pub fn cancel(&mut self)->usize{let dropped=self.control.iter().filter(|c|c.is_some()).count()+self.text.pending()as usize;self.control=[None;4];self.text.cancel();self.closed=true;dropped}
 pub fn receive(&mut self,caller:u32,value:u64)->Option<Receipt>{if self.closed{return None}
  let r=self.text.receive(caller,value)?;Some(Receipt{surface:self.ids[0],stamp:Stamp{sequence:r.metadata.first as u64,paint:r.metadata.count as u64},bytes:r.bytes})}
 pub fn poll(&mut self,budget:usize,mut send:impl FnMut(Operation)->Result<(),u64>)->Result<usize,u64>{
  let mut attempts=0;if self.closed{return Ok(0)}
  for i in 0..4{let Some(op)=self.control[i]else{continue};if attempts>=budget{return Ok(attempts)}attempts+=1;
   match send(op){Err(sex_pdx::ERR_SERVICE_NOT_READY)=>return Ok(attempts),Err(e)=>{self.cancel();return Err(e)},Ok(())=>self.control[i]=None}}
  if self.hidden{return Ok(attempts)}
  let left=budget-attempts;
  let result=self.text.poll(left,|op|{attempts+=1;match send(Operation{opcode:op.opcode,surface:op.surface,a:op.packed,b:op.control}){
   Ok(())=>text_delivery::Admission::Accepted,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>text_delivery::Admission::Full,Err(e)=>text_delivery::Admission::Rejected(e)}});
  if let text_delivery::Poll::Rejected(e)=result{self.cancel();return Err(e)}
  Ok(attempts)
 }
}
