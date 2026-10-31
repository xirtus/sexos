// SPDX-License-Identifier: GPL-3.0-only
//! Extracted actual Browser wheel adapter and sender checks with actual Reader.
//! Source accessors model custody; no native authority or pixel claim.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_scroll.rs"]pub mod app_scroll;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/network_config.rs"]pub mod network_config;
#[path="../apps/kaleidoscope/src/document_reader.rs"]mod document_reader;
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
pub struct PdxMessage {type_id:u64,caller_pd:u32,caller_task:u32,arg0:u64,arg1:u64,arg2:u64}
#[derive(Clone)]struct Body(Vec<u8>);impl Body{fn as_bytes(&self)->&[u8]{&self.0}}
struct Exchange(Option<Body>);impl Exchange{
 fn has_document(&self)->bool{self.0.is_some()}
 fn document(&self)->&[u8]{self.0.as_ref().map_or(&[],Body::as_bytes)}
}
struct Importer(Option<Body>);impl Importer{fn document(&self)->Option<&Body>{self.0.as_ref()}}
mod http_probe{#[derive(PartialEq)]pub enum View{Ready}}
struct Http(Body);impl Http{fn document(&self)->&[u8]{self.0.as_bytes()}fn view(&self)->http_probe::View{http_probe::View::Ready}}
struct App{
 reading:bool,reading_https:bool,asset_panel:bool,import_panel:bool,viewport_sequence:u64,
 reader:document_reader::Reader,exchange:Exchange,import_source:Option<Body>,importer:Importer,http:Http,
}
impl App{fn has_https_body(&self)->bool{self.exchange.has_document()||self.import_source.is_some()||self.importer.0.is_some()}
 fn new()->Self{
 let body=Body((0..100).map(|n|format!("{n:03} café 中\n")).collect::<String>().into_bytes());
 let mut reader=document_reader::Reader::new();reader.resize(624,706);
 Self{reading:true,reading_https:false,asset_panel:false,import_panel:false,viewport_sequence:7,reader,exchange:Exchange(Some(body.clone())),import_source:Some(body.clone()),importer:Importer(Some(body.clone())),http:Http(body)}
}}
include!(env!("BROWSER_WHEEL_PRODUCTION"));
fn message()->PdxMessage{PdxMessage{type_id:0x202,caller_pd:3,caller_task:3,arg0:(-1i64)as u64,arg1:0,arg2:5}}
#[test]fn wrong_sender_task_opcode_and_noncanonical_payloads_do_not_move_reader(){
 for pd in [0,3,4,15] {for task in [0,3,4,30] {
  let mut app=App::new();
  let mut req=message();req.caller_pd=pd;req.caller_task=task;
  app.receive_wheel(&req);
  assert_eq!(app.reader.render(app.http.document()).first,if pd==3&&task==3{3}else{0});
 }}
 for field in 0..6 {let mut app=App::new();let mut req=message();match field{
  0=>req.type_id=0x201,1=>req.arg0=255,2=>req.arg0=0,3=>req.arg0=128,4=>req.arg1=1,_=>req.arg2=1};
  app.receive_wheel(&req);assert_eq!(app.reader.source_anchor(),0);
 }
}
#[test]fn every_current_retained_source_scrolls_three_rows_and_keeps_exact_source_bytes(){
 for source in 0..4 {let mut app=App::new();app.reading_https=source!=0;
  if source>=2{app.exchange.0=None}if source>=3{app.import_source=None}
  let before=app.http.document().to_vec();app.receive_wheel(&message());
  assert_eq!(app.reader.render(&before).first,3);assert!(core::str::from_utf8(&before).unwrap().is_char_boundary(app.reader.source_anchor()));
  assert_eq!(app.http.document(),before);assert_eq!(app.viewport_sequence,7);
  let mut up=message();up.arg0=1;app.receive_wheel(&up);assert_eq!(app.reader.source_anchor(),0);
 }
}
#[test]fn hidden_reader_and_other_panels_leave_anchor_unchanged(){
 for panel in 0..3 {let mut app=App::new();match panel{0=>app.reading=false,1=>app.asset_panel=true,_=>app.import_panel=true};
  app.receive_wheel(&message());assert_eq!(app.reader.source_anchor(),0);
 }
}
