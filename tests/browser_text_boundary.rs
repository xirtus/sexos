// SPDX-License-Identifier: GPL-3.0-only
//! Extracted actual Browser text/import adapters. Host custody callbacks do not
//! establish native authority, transport authentication, storage, or pixels.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_selection.rs"]pub mod text_selection;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../apps/kaleidoscope/src/import_view.rs"]mod import_view;
#[derive(Clone,Copy,PartialEq,Eq)]pub enum Resource{WelcomeText,ArtworkQoi}
mod sex_tls{pub use crate::Resource;}
pub fn get_ticks()->u64{1}
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format!($($arg)*);}}}
struct Body(Vec<u8>);impl Body{fn as_bytes(&self)->&[u8]{&self.0}}
struct Exchange{body:Option<Body>,released:bool,taken:usize,resource:Option<Resource>}
impl Exchange{
 fn resource_kind(&self)->Option<Resource>{self.resource}
 fn has_document(&self)->bool{self.released&&self.body.is_some()}
 fn document(&self)->&[u8]{if self.released{self.body.as_ref().map_or(&[],|b|b.as_bytes())}else{&[]}}
 fn take_document_for_import(&mut self)->Option<Body>{if self.has_document(){self.taken+=1;self.body.take()}else{None}}
 fn ticket(&self)->u32{7}
}
mod document_import{
 #[derive(Clone,Copy,PartialEq,Eq)]pub enum View{Idle,Saved,Uncertain}
}
struct Importer{body:Option<Body>,view:document_import::View,busy:bool,starts:usize,clears:usize}
impl Importer{
 fn view(&self)->document_import::View{self.view}fn busy(&self)->bool{self.busy}
 fn take_document(&mut self)->Option<Body>{self.body.take()}
 fn document(&self)->Option<&Body>{self.body.as_ref()}
 fn clear(&mut self,_:u64){self.clears+=1;self.body=None}
 fn start(&mut self,body:Body,_:[u8;16],_:u64)->Result<(),Body>{self.starts+=1;self.body=Some(body);Ok(())}
}
struct App{
 exchange:Exchange,importer:Importer,import_source:Option<Body>,import_ticket:u32,
 import_panel:bool,import_status:import_view::Status,import_editor:import_view::Editor,
 https_panel:bool,peer_panel:bool,tls_panel:bool,clock_panel:bool,reading:bool,reading_https:bool,asset_panel:bool,
 selection:text_selection::Selection,body:Vec<u8>,resource_changes:usize,
}
impl App{
 fn new(bytes:&[u8])->Self{Self{exchange:Exchange{body:Some(Body(bytes.to_vec())),released:true,taken:0,resource:Some(Resource::WelcomeText)},
 importer:Importer{body:None,view:document_import::View::Idle,busy:false,starts:0,clears:0},import_source:None,import_ticket:0,
 import_panel:false,import_status:import_view::Status::NoDocument,import_editor:import_view::Editor::new(),https_panel:true,peer_panel:false,tls_panel:false,clock_panel:false,reading:true,reading_https:true,asset_panel:false,
 selection:text_selection::Selection::new(),body:bytes.to_vec(),resource_changes:0}}
 fn resources(&mut self,_:&str,_:bool){self.resource_changes+=1;}
 // The visibility gate exercises actual clipboard quiescence; this text-only
 // fixture models panel admission without claiming a clipboard transport.
 fn show_import_panel(&mut self){self.import_panel=true;}
 fn reader_body(&self)->&[u8]{&self.body}
}
fn public_fingerprint(body:&[u8])->u64{body.iter().fold(0xcbf29ce484222325,|h,b|(h^*b as u64).wrapping_mul(0x100000001b3))}
include!(env!("BROWSER_TEXT_BOUNDARY"));
#[test]fn exact_released_utf8_and_tab_enter_naming_then_submit_without_byte_changes(){
 let body="café\t中文\nemoji 😀\n".as_bytes();let mut app=App::new(body);app.begin_import();
 assert_eq!(app.import_status,import_view::Status::Naming);assert!(app.import_editor.active);assert_eq!(app.exchange.taken,1);
 assert_eq!(app.import_source.as_ref().unwrap().as_bytes(),body);app.submit_import();assert_eq!(app.importer.starts,1);
 assert_eq!(app.importer.body.as_ref().unwrap().as_bytes(),body);assert!(app.import_source.is_none());
}
#[test]fn unsupported_bytes_stay_with_original_source_and_never_start_naming_or_storage(){
 for body in [b"valid\xff".as_slice(),b"a\0b",b"line\r\n",b"\xc2\x80",b"\xf0\x9f",b"\x1b"]{
  let mut app=App::new(body);app.begin_import();assert_eq!(app.import_status,import_view::Status::UnsupportedText);
  assert!(!app.import_editor.active);assert!(app.https_panel&&app.reading&&app.reading_https);
  assert_eq!(app.exchange.taken,0);assert_eq!(app.exchange.document(),body);assert!(app.import_source.is_none());
  assert_eq!((app.importer.clears,app.importer.starts),(0,0));
 }
}
#[test]fn submit_rechecks_format_and_preserves_owned_document_before_any_effect(){
 let mut app=App::new(b"ok");app.import_source=Some(Body(b"bad\0".to_vec()));app.import_editor.begin();app.submit_import();
 assert_eq!(app.import_status,import_view::Status::UnsupportedText);assert!(!app.import_editor.active);
 assert_eq!(app.import_source.as_ref().unwrap().as_bytes(),b"bad\0");assert_eq!(app.importer.starts,0);
}
#[test]fn no_release_binary_busy_and_uncertainty_do_not_move_sources(){
 let mut app=App::new("é".as_bytes());app.exchange.released=false;app.begin_import();assert_eq!(app.import_status,import_view::Status::NoDocument);assert_eq!(app.exchange.taken,0);assert_eq!(app.importer.starts,0);
 let mut app=App::new(b"binary");app.exchange.resource=Some(Resource::ArtworkQoi);app.begin_import();assert!(!app.import_panel);assert_eq!(app.exchange.taken,0);
 for uncertain in [false,true]{let mut app=App::new(b"data");app.importer.busy=!uncertain;if uncertain{app.importer.view=document_import::View::Uncertain;}
 app.begin_import();assert_eq!(app.exchange.taken,0);assert!(!app.import_editor.active);assert_eq!(app.importer.starts,0);}
}
#[test]fn actual_horizontal_adapter_moves_one_scalar_and_collapses_selection_without_splitting(){
 let body="aé中😀z".as_bytes();let mut app=App::new(body);let src=text_selection::Source{generation:1,revision:0};app.selection.bind(src,body.len()).unwrap();
 for end in [1,3,6,10,11]{app.selection_horizontal(true,true);let s=app.selection.snapshot().unwrap();assert_eq!(s.caret(),end);assert!(text_encoding::is_boundary(body,end));}
 app.selection_horizontal(false,false);assert_eq!(app.selection.snapshot().unwrap().caret(),0);
 for end in [1,3,6,10,11]{app.selection_horizontal(true,false);assert_eq!(app.selection.snapshot().unwrap().caret(),end);}
 for start in [10,6,3,1,0]{app.selection_horizontal(false,false);assert_eq!(app.selection.snapshot().unwrap().caret(),start);}
 app.selection.move_to(src,10,false).unwrap();app.selection.move_to(src,1,true).unwrap();app.selection_horizontal(true,false);assert_eq!(app.selection.snapshot().unwrap().range(),(10,10));
}

#[test]fn retained_importer_body_is_validated_before_reentering_naming_even_outside_saved_view(){
 let mut app=App::new(b"ignored");app.exchange.body=None;app.importer.body=Some(Body(b"invalid\xff".to_vec()));app.begin_import();
 assert_eq!(app.import_status,import_view::Status::UnsupportedText);assert!(app.importer.body.is_some());assert!(app.import_source.is_none());assert!(!app.import_editor.active);
 assert_eq!((app.importer.clears,app.importer.starts),(0,0));
}
