// SPDX-License-Identifier: GPL-3.0-only
//! Actual extracted App transitions, Reader, selection and clipboard client.
//! Finite source/transport callbacks attest host policy, not native authority.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
extern crate self as sex_tls;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../crates/sex-pdx/src/clipboard.rs"]pub mod clipboard;
#[path="../crates/sex-pdx/src/clipboard_client.rs"]pub mod clipboard_client;
#[path="../crates/sex-pdx/src/text_selection.rs"]pub mod text_selection;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/app_key.rs"]pub mod app_key;
#[path="../crates/sex-pdx/src/app_scroll.rs"]pub mod app_scroll;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/network_config.rs"]pub mod network_config;
#[path="../apps/kaleidoscope/src/document_reader.rs"]mod document_reader;
#[path="../apps/kaleidoscope/src/import_view.rs"]mod import_view;
#[derive(Clone,Copy,PartialEq)]pub enum Resource{WelcomeText,ArtworkQoi}
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
thread_local! {static SENT:std::cell::RefCell<Vec<clipboard::Request>>=const{std::cell::RefCell::new(Vec::new())};static FULL:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
pub const SLOT_SHELL:u64=6;pub const ERR_SERVICE_NOT_READY:u64=9;
pub fn get_ticks()->u64{10}
pub fn pdx_call_checked(_:u64,op:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{
 let request=clipboard::Request::decode(op,[a,b,c]).unwrap();
 if FULL.with(|f|f.get()){return Err(9)}
 SENT.with(|s|s.borrow_mut().push(request));Ok(0)
}
static mut CLIPBOARD:clipboard_client::Client=clipboard_client::Client::new();
#[derive(Clone)]struct Body(Vec<u8>);impl Body{fn as_bytes(&self)->&[u8]{&self.0}}
mod http_probe{#[derive(Clone,Copy,PartialEq)]pub enum View{Ready,Pending,Unavailable}}
mod document_import{#[derive(Clone,Copy,PartialEq)]pub enum View{Idle,Saved,Exists,Uncertain}}
struct Http{body:Body,view:http_probe::View}impl Http{fn document(&self)->&[u8]{if self.view==http_probe::View::Ready{self.body.as_bytes()}else{&[]}}fn view(&self)->http_probe::View{self.view}}
struct Exchange{body:Option<Body>,resource:Option<Resource>}impl Exchange{
 fn has_document(&self)->bool{self.body.is_some()}
 fn document(&self)->&[u8]{self.body.as_ref().map_or(&[],Body::as_bytes)}
 fn resource_kind(&self)->Option<Resource>{self.resource}
 fn take_document_for_import(&mut self)->Option<Body>{self.body.take()}
 fn ticket(&self)->u32{7}
}
struct Importer{body:Option<Body>,view:document_import::View,busy:bool}
impl Importer{
 fn document(&self)->Option<&Body>{self.body.as_ref()}fn has_document(&self)->bool{self.body.is_some()}
 fn view(&self)->document_import::View{self.view}fn busy(&self)->bool{self.busy}
 fn take_document(&mut self)->Option<Body>{if self.busy||self.view==document_import::View::Uncertain{None}else{self.body.take()}}
 fn clear(&mut self,_:u64){self.body=None;self.view=document_import::View::Idle;}
}
struct App{
 reading:bool,reading_https:bool,asset_panel:bool,import_panel:bool,https_panel:bool,peer_panel:bool,tls_panel:bool,clock_panel:bool,
 reader:document_reader::Reader,selection:text_selection::Selection,selection_generation:u64,selection_body:Option<(bool,usize,u64)>,
 clipboard_view:clipboard_client::View,viewport_sequence:u64,http:Http,exchange:Exchange,importer:Importer,import_source:Option<Body>,import_editor:import_view::Editor,
 import_status:import_view::Status,import_ticket:u32,resources_called:usize,
}
impl App{
 fn new()->Self{
  unsafe{CLIPBOARD=clipboard_client::Client::new();assert!(CLIPBOARD.bind(clipboard::Participant{subject:clipboard::Subject{pd:15,task:15},surface:300,route:43,can_read:false}));}
  SENT.with(|s|s.borrow_mut().clear());FULL.with(|f|f.set(false));
  let mut reader=document_reader::Reader::new();reader.resize(624,706);
  Self{reading:true,reading_https:false,asset_panel:false,import_panel:false,https_panel:false,peer_panel:false,tls_panel:false,clock_panel:false,
   reader,selection:text_selection::Selection::new(),selection_generation:0,selection_body:None,clipboard_view:clipboard_client::View::Idle,viewport_sequence:1,
   http:Http{body:Body((0..100).map(|n|format!("{n} café 中\n")).collect::<String>().into_bytes()),view:http_probe::View::Ready},
   exchange:Exchange{body:None,resource:None},importer:Importer{body:None,view:document_import::View::Idle,busy:false},import_source:None,import_editor:import_view::Editor::new(),import_status:import_view::Status::NoDocument,import_ticket:0,resources_called:0}
 }
 fn resources(&mut self,_:&str,_:bool){self.resources_called+=1;}
 // Only the command dispatcher is a host adapter; its selected transition
 // helpers and all selection/navigation/copy logic are actual source methods.
 fn key(&mut self,raw:u64){let code=raw as u8;if code==0x17{self.begin_import();}else if !self.dismiss_import_key(code){self.navigate_reader(code);}}
}
fn public_fingerprint(bytes:&[u8])->u64{clipboard::hash(bytes)}
include!(env!("BROWSER_READER_VISIBILITY"));
fn key(code:u8,shift:bool,control:bool)->app_key::Key{app_key::Key{code,shift,control}}
fn response(value:clipboard::Response){let p=value.wire();assert!(unsafe{CLIPBOARD.receive(clipboard::SHELL,p.opcode,p.args,get_ticks())});}
fn intent(ticket:u32){response(clipboard::Response::Intent{ticket,action:clipboard::Action::Copy});}
fn requests()->Vec<clipboard::Request>{SENT.with(|s|s.borrow().clone())}
fn select(app:&mut App){app.key_input(key(0x1e,false,true));}
#[test]fn hidden_import_overlay_refuses_new_export_and_all_selection_navigation(){
 let mut app=App::new();app.reader.scroll_wheel(app.http.document(),-1);app.sync_selection();select(&mut app);
 let selected=app.selection.snapshot().unwrap();let anchor=app.reader.source_anchor();let body=app.http.document().to_vec();
 app.begin_import();assert_eq!(app.import_status,import_view::Status::NoDocument);assert!(app.import_panel);
 for k in [key(0x69,true,false),key(0x6c,true,false),key(0x1e,false,true),key(0x6d,false,false)]{app.key_input(k);}
 app.scroll_input(app_scroll::Scroll::new(-1).unwrap());assert_eq!(app.reader.source_anchor(),anchor);assert_eq!(app.selection.snapshot().unwrap(),selected);
 intent(1);app.clipboard_events();assert!(requests().iter().all(|r|matches!(r,clipboard::Request::Cancel{ticket:1})));assert!(unsafe{CLIPBOARD.storage_erased()});assert_eq!(app.http.document(),body);
}
#[test]fn retained_http_v_and_escape_return_same_bytes_anchor_and_selection_without_refetch(){
 for code in [0x2f,0x01]{let mut app=App::new();app.reader.scroll_wheel(app.http.document(),-2);select(&mut app);let before=app.selection.snapshot().unwrap();let anchor=app.reader.source_anchor();let bytes=app.http.document().to_vec();
  app.begin_import();assert!(app.dismiss_import_key(code));assert!(app.visible_reader());assert!(!app.reading_https);assert_eq!(app.reader.source_anchor(),anchor);assert_eq!(app.selection.snapshot().unwrap(),before);assert_eq!(app.reader_body(),bytes);assert!(requests().is_empty());
 }
}
#[test]fn no_source_v_stays_put_but_escape_returns_overview_and_clears_stale_selection(){
 let mut app=App::new();select(&mut app);app.http.view=http_probe::View::Unavailable;app.begin_import();assert!(app.import_panel);
 assert!(app.dismiss_import_key(0x2f));assert!(app.import_panel);assert!(!app.visible_reader());assert!(app.selection.snapshot().is_some());
 assert!(app.dismiss_import_key(0x01));assert!(!app.import_panel);assert!(!app.reading);assert!(!app.reading_https);assert!(app.selection.snapshot().is_none());assert!(requests().is_empty());
}
#[test]fn busy_and_uncertain_import_never_dismiss_or_release_mutation_fences(){
 for uncertain in [false,true]{let mut app=App::new();app.import_panel=true;app.importer.busy=!uncertain;app.importer.view=if uncertain{document_import::View::Uncertain}else{document_import::View::Idle};
  for code in [0x2f,0x01]{assert!(app.dismiss_import_key(code));assert!(app.import_panel);assert!(!app.visible_reader());}assert!(requests().is_empty());
 }
}
#[test]fn renderer_source_mismatch_and_asset_overlay_cannot_export_http_fallback(){
 for asset in [false,true]{let mut app=App::new();select(&mut app);app.asset_panel=asset;app.reading_https=!asset;
  assert!(!app.visible_reader());let anchor=app.reader.source_anchor();app.navigate_reader(0x6d);assert_eq!(app.reader.source_anchor(),anchor);
  intent(1);app.clipboard_events();assert!(requests().iter().all(|r|matches!(r,clipboard::Request::Cancel{..})));
 }
}
#[test]fn released_https_name_cancel_and_unsupported_overlay_restore_same_source(){
 let mut app=App::new();app.reading_https=true;app.https_panel=true;app.exchange.body=Some(Body(b"released caf\xc3\xa9\n".to_vec()));app.exchange.resource=Some(Resource::WelcomeText);select(&mut app);let before=app.selection.snapshot().unwrap();
 app.begin_import();assert!(app.import_editor.active);assert!(app.exchange.body.is_none());assert_eq!(app.import_editor.key(0x01),import_view::Key::Cancel);assert!(app.dismiss_import(true));assert!(app.visible_reader());assert!(app.reading_https);assert_eq!(app.selection.snapshot().unwrap(),before);
 let source=app.import_source.as_mut().unwrap();source.0.push(0xff);app.begin_import();assert_eq!(app.import_status,import_view::Status::UnsupportedText);assert!(app.dismiss_import(false));assert!(app.visible_reader());assert_eq!(app.reader_body().last(),Some(&0xff));
}
#[test]fn opening_overlay_quiesces_accepted_publish_without_replay_or_rollback_claim(){
 let mut app=App::new();app.http.body=Body(b"immutable old selection".to_vec());select(&mut app);intent(1);app.clipboard_events();assert!(matches!(requests().as_slice(),[clipboard::Request::Begin{..}]));
 response(clipboard::Response::Meta{ticket:1,phase:clipboard::Phase::Begin,status:clipboard::Status::Ok,len:app.http.document().len(),generation:0,hash:0});app.clipboard_events();
 let published=requests().iter().filter(|r|matches!(r,clipboard::Request::Publish{..})).count();assert_eq!(published,1);assert!(!unsafe{CLIPBOARD.storage_erased()});
 app.begin_import();assert!(unsafe{CLIPBOARD.storage_erased()});FULL.with(|f|f.set(true));app.clipboard_events();assert_eq!(requests().iter().filter(|r|matches!(r,clipboard::Request::Publish{..})).count(),1);
 // A late positive publication cannot cancel the new quiescence receipt or
 // manufacture a second publication. The broker's prior effect may remain.
 let p=clipboard::Response::Meta{ticket:1,phase:clipboard::Phase::Publish,status:clipboard::Status::Ok,len:app.http.document().len(),generation:1,hash:clipboard::hash(app.http.document())}.wire();
 assert!(!unsafe{CLIPBOARD.receive(clipboard::SHELL,p.opcode,p.args,get_ticks())});FULL.with(|f|f.set(false));app.clipboard_events();assert!(matches!(requests().last(),Some(clipboard::Request::Cancel{ticket:1})));
 response(clipboard::Response::Meta{ticket:1,phase:clipboard::Phase::Cancel,status:clipboard::Status::Ok,len:0,generation:0,hash:0});app.clipboard_events();assert!(!unsafe{CLIPBOARD.pending()});assert!(unsafe{CLIPBOARD.storage_erased()});
}

#[test]fn hidden_overlay_exports_no_new_plaintext_request(){
 let mut app=App::new();select(&mut app);app.begin_import();intent(1);app.clipboard_events();
 assert!(requests().iter().all(|r|matches!(r,clipboard::Request::Cancel{ticket:1})));assert!(unsafe{CLIPBOARD.storage_erased()});
}
