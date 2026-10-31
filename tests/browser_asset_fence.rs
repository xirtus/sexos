// SPDX-License-Identifier: GPL-3.0-only
//! Actual extracted Browser fence/peer transition methods. Transport, TLS and
//! decoder are finite host custody callbacks; bitmap outbox is production code.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../apps/kaleidoscope/src/asset_view.rs"]mod asset_view;
#[path="../apps/kaleidoscope/src/bitmap_delivery.rs"]mod bitmap_delivery;
#[path="../apps/kaleidoscope/src/asset_presentation.rs"]mod asset_presentation;
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format!($($arg)*);}}}
mod tls_exchange{
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum View{Idle,Pending,Verified,Unavailable,Uncertain,Cancelled}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Intent{TlsAssetDns,TlsStart,Echo}
struct Asset(Vec<u8>);impl Asset{fn as_bytes(&self)->&[u8]{&self.0}}
struct Exchange{body:Option<Asset>,cancels:usize,takes:usize,accepted:bool,pending:bool,abort:bool,sends:usize}
impl Exchange{
 fn cancel(&mut self){self.cancels+=1;self.body=None;self.abort=true;if !self.accepted{self.pending=false;}}
 fn take_asset(&mut self)->Option<Asset>{self.takes+=1;self.body.take()}
 fn tx_bytes(&self)->usize{17}fn rx_bytes(&self)->usize{31}
}
#[derive(Default)]struct Provider{cancels:usize}
struct Tls{prepared:usize,cancels:usize}
impl Tls{
 fn cancel(&mut self,trust:&mut Provider,seed:&mut Provider,clock:&mut Provider)->usize{
  self.cancels+=1;trust.cancels+=1;seed.cancels+=1;clock.cancels+=1;let n=self.prepared;self.prepared=0;n
 }
}
#[derive(Default)]struct Dns{cancels:usize}impl Dns{fn cancel(&mut self){self.cancels+=1;}}
struct Image{body:Vec<u8>,clears:usize,begins:usize,reject:bool}
impl Image{
 fn clear(&mut self){self.body.clear();self.clears+=1;}
 fn begin(&mut self,body:Asset,_:u32)->Result<(),()>{self.begins+=1;if self.reject{return Err(())}self.body=body.0;Ok(())}
}
struct App{
 asset_fenced:bool,asset_status:asset_view::Status,asset_peer_view:tls_exchange::View,
 image:Image,bitmap:bitmap_delivery::Outbox,asset_presentation:asset_presentation::Presentation,
 exchange:Exchange,intent:Option<Intent>,dns:Dns,tls:Tls,trust:Provider,seed:Provider,clock:Provider,
}
impl App{
 fn new()->Self{Self{asset_fenced:false,asset_status:asset_view::Status::Ready,asset_peer_view:tls_exchange::View::Idle,
 image:Image{body:vec![1,2,3],clears:0,begins:0,reject:false},bitmap:bitmap_delivery::Outbox::new(),asset_presentation:asset_presentation::Presentation::new(),
 exchange:Exchange{body:Some(Asset(vec![7,8])),cancels:0,takes:0,accepted:true,pending:true,abort:false,sends:0},intent:Some(Intent::TlsStart),dns:Dns::default(),tls:Tls{prepared:64,cancels:0},trust:Provider::default(),seed:Provider::default(),clock:Provider::default()}}
}
fn public_fingerprint(bytes:&[u8])->u64{bytes.iter().fold(0xcbf29ce484222325,|hash,b|(hash^*b as u64).wrapping_mul(0x100000001b3))}
include!(env!("BROWSER_ASSET_FENCE"));
#[test]fn fenced_q_cancel_does_not_claim_display_clear(){
 let mut app=App::new();app.asset_peer_view=tls_exchange::View::Verified;app.fence_asset("display_rejected");
 assert_eq!(app.asset_status,asset_view::Status::Uncertain);app.exchange.cancel(); // actual Q always requests transport cancellation
 app.update_asset_peer(tls_exchange::View::Cancelled,7);
 assert_eq!(app.asset_peer_view,tls_exchange::View::Cancelled);assert_eq!(app.asset_status,asset_view::Status::Uncertain);
 assert_eq!(app.image.begins,0);assert_eq!(app.exchange.takes,0);assert!(!app.bitmap.pending());
}
#[test]fn fenced_late_verified_never_adopts_or_reallocates_image_storage(){
 let mut app=App::new();app.fence_asset("presentation_sequence");app.exchange.body=Some(Asset(vec![9,10]));
 app.update_asset_peer(tls_exchange::View::Verified,7);
 assert_eq!(app.asset_peer_view,tls_exchange::View::Verified);assert_eq!(app.asset_status,asset_view::Status::Uncertain);
 assert_eq!(app.exchange.takes,0);assert_eq!(app.image.begins,0);assert!(app.image.body.is_empty());
 for view in [tls_exchange::View::Unavailable,tls_exchange::View::Uncertain,tls_exchange::View::Cancelled,tls_exchange::View::Pending]{app.update_asset_peer(view,7);assert_eq!(app.asset_status,asset_view::Status::Uncertain);}
}
#[test]fn actual_fence_cancels_prepared_work_closes_cached_chunk_and_preserves_accepted_correlation(){
 for intent in [Some(Intent::TlsStart),Some(Intent::TlsAssetDns),Some(Intent::Echo),None]{
  let mut app=App::new();app.intent=intent;let pixels=[0xff123456,0x80010203];let geometry=bitmap_batch::Geometry{x:8,y:80,width:2,height:1,scale:1};
  app.bitmap.queue(&pixels,geometry,bitmap_delivery::Metadata{viewport:1,source:7}).unwrap();let mut calls=0;
  assert_eq!(app.bitmap.poll(&pixels,4,|_|{calls+=1;if calls==1{bitmap_delivery::Admission::Accepted}else{bitmap_delivery::Admission::Full}}),bitmap_delivery::Poll::Deferred);
  app.asset_presentation.text_applied(1,1,1);assert!(app.asset_presentation.expect(geometry,1,7,1,bitmap_batch::hash(&pixels)));
  app.fence_asset("display_rejected");assert!(app.asset_fenced);assert!(app.image.body.is_empty());assert!(!app.asset_presentation.has_layout());assert!(!app.bitmap.pending());
  assert_eq!(app.bitmap.poll(&[],4,|_|panic!("fenced cached chunk replay")),bitmap_delivery::Poll::Closed);
  assert_eq!(app.exchange.cancels,1);assert!(app.exchange.pending&&app.exchange.accepted&&app.exchange.abort);assert_eq!(app.exchange.sends,0);
  assert_eq!(app.tls.prepared,0);assert_eq!(app.tls.cancels,1);assert_eq!((app.trust.cancels,app.seed.cancels,app.clock.cancels),(1,1,1));
  let asset_intent=matches!(intent,Some(Intent::TlsAssetDns|Intent::TlsStart));assert_eq!(app.dns.cancels,asset_intent as usize);assert_eq!(app.intent,if asset_intent{None}else{intent});
 }
}
#[test]fn normal_verified_adopts_once_and_preserves_decoder_error_status(){
 let mut app=App::new();app.update_asset_peer(tls_exchange::View::Verified,7);assert_eq!(app.asset_status,asset_view::Status::Decoding);assert_eq!(app.image.body,[7,8]);
 app.update_asset_peer(tls_exchange::View::Verified,7);assert_eq!((app.exchange.takes,app.image.begins),(1,1));
 let mut app=App::new();app.image.reject=true;app.update_asset_peer(tls_exchange::View::Verified,7);assert_eq!(app.asset_status,asset_view::Status::Invalid);assert_eq!((app.exchange.takes,app.image.begins),(1,1));
}
#[test]fn normal_peer_statuses_and_pending_bitmap_cancel_behavior_are_unchanged(){
 for(view,status)in[(tls_exchange::View::Unavailable,asset_view::Status::Unavailable),(tls_exchange::View::Uncertain,asset_view::Status::Uncertain),(tls_exchange::View::Cancelled,asset_view::Status::Cleared)]{
  let mut app=App::new();app.update_asset_peer(view,7);assert_eq!(app.asset_status,status);assert_eq!(app.asset_peer_view,view);
 }
 let mut app=App::new();let g=bitmap_batch::Geometry{x:8,y:80,width:1,height:1,scale:1};app.bitmap.queue(&[1],g,bitmap_delivery::Metadata{viewport:1,source:7}).unwrap();app.asset_status=asset_view::Status::Clearing;
 app.update_asset_peer(tls_exchange::View::Cancelled,7);assert_eq!(app.asset_status,asset_view::Status::Clearing);
}
