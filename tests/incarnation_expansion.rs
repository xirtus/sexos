#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../servers/silk-shell/src/linen_text_delivery.rs"]mod admission;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/document.rs"]pub mod document;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/document_view.rs"]pub mod document_view;
#[path="../servers/sexfiles/src/document.rs"]mod documents;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../apps/spindle/src/viewport.rs"]mod viewport;
#[path="../apps/spindle/src/display_delivery.rs"]mod display_delivery;
#[path="../servers/linen/src/reply_delivery.rs"]mod linen;
#[path="../servers/silk-shell/src/app_custody.rs"]mod app_custody;
#[path="../servers/silk-shell/src/desktop_content.rs"]mod desktop_content;
#[path="../servers/silk-shell/src/launcher.rs"]mod launcher;
pub mod app_lifecycle{
 pub const DENIED:u64=3;
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
 pub fn activation(slot:usize,_commit:bool)->Result<ActivationInfo,u64>{unsafe{crate::INFO[crate::app_catalog::INCARNATIONS.iter().find(|row|row.activation_slot==Some(slot)&&row.app==crate::app_catalog::App::Quil).expect("unheld test edge").ordinal as usize-1]}}
}
use app_lifecycle::ActivationInfo;
const SUCCESSORS:usize=app_catalog::count(app_catalog::App::Quil)-1;
static mut APP_CUSTODY:[Option<app_custody::Entry>;SUCCESSORS]={
 let mut out=[None;SUCCESSORS];let mut i=0;let mut n=0;
 while i<app_catalog::COUNT {let row=&app_catalog::INCARNATIONS[i];if matches!(row.app,app_catalog::App::Quil)&&row.ordinal!=0{out[n]=Some(app_custody::Entry::new(row.lifecycle_slot,row.target,row.target*10,row.surface));n+=1;}i+=1;}out
};
static mut INFO:[Result<ActivationInfo,u64>;SUCCESSORS]=[Err(3);SUCCESSORS];
fn native_launcher_status(target:u32)->(bool,&'static str,&'static str){if target==100{(true,"RUNNING / FOCUS","original_incarnation_live")}else{(false,"CLOSED","no_boot_successor")}}
include!(env!("EXPANDED_SHELL_POLICY"));
#[test]fn fourth_quil_resolves_exact_edge_and_never_skips_denials(){unsafe{
 for row in app_catalog::INCARNATIONS.iter().filter(|row|row.app==app_catalog::App::Quil&&row.ordinal!=0){INFO[row.ordinal as usize-1]=Ok(ActivationInfo{successor:row.target,task:row.target*10,surface:row.surface,state:if row.target==100{1}else{3}});}
 let last=SUCCESSORS-1;
 assert_eq!(app_activation_info(app_catalog::App::Quil,false).unwrap().successor,100);assert_eq!(app_reserve_remaining(app_catalog::App::Quil),Some(1));
 INFO[last].as_mut().unwrap().state=2;assert_eq!(app_route_slot(app_catalog::App::Quil,11),213);assert!(app_launcher_status(app_catalog::App::Quil).0);
 INFO[last].as_mut().unwrap().task=100;assert_eq!(app_activation_info(app_catalog::App::Quil,true),Err(3));
 INFO[last].as_mut().unwrap().task=1000;INFO[0]=Err(2);assert_eq!(app_activation_info(app_catalog::App::Quil,false),Err(2));
}}
#[test]fn future_spindle_grid_and_browser_policy_use_exact_catalog_pairs(){
 let mut view=document_view::Delivery::new();view.queue(100,1000,320,240);let mut notice=None;
 view.poll(true,true,|n|{notice=Some(n);Ok(())});let n=notice.expect("future Quil viewport queued");
 assert!(document_view::Notice::decode(3,100,1000,n.args(),0).is_some());assert!(document_view::Notice::decode(3,100,18,n.args(),0).is_none());
 assert_eq!(app_catalog::DOCUMENT_TARGETS,[9,16,18,19,20,21,22,23,100,15,102]);
 let mut out=display_delivery::Outbox::new(101);out.create(0,0,123);let mut seen=vec![];out.poll(4,|op|{seen.push(op.surface);Ok(())},|_|{}).unwrap();assert_eq!(seen,[401]);
 assert!(linen::admit(101,0x42,Some(5)));assert!(!linen::admit(101,0x42,Some(2)));assert!(!linen::admit(102,0x42,Some(5)));
 assert_eq!(desktop_content::parent_frame(405),Some(402));assert_eq!(desktop_content::parent_frame(406),None);
 let good=app_custody::Entry::new(218,102,102,406);assert!(good.allows_operation(0x275,[102,102,0]));
 assert!(!app_custody::Entry::new(218,102,102,400).allows_operation(0x275,[102,102,0]));
 assert!(!good.allows_operation(0x274,[15,15,0]));
}
struct Backend;
impl documents::Backend for Backend{
 fn identity(&mut self,_:u8)->Result<u32,u32>{Ok(8)}
 fn load(&mut self,_:u8,_:&mut[u8;document::MAX])->Result<usize,u32>{Ok(0)}
 fn name(&mut self,_:u8)->Result<[u8;16],u32>{Ok([0;16])}
 fn create_new(&mut self,_:[u8;16])->Result<(u8,u32),u32>{Ok((3,8))}
}
#[test]fn future_document_sessions_keep_replies_and_create_only_import_isolated(){
 let mut service=documents::Service::new();let mut backend=Backend;
 let info=document::Request{seq:1,kind:document::INFO,object:3,offset:0,a:0,b:0};
 assert_eq!(service.handle(100,true,info,&mut backend),None);
 let create=document::Request{seq:1,kind:document::CREATE,object:0,offset:0,a:u64::from_le_bytes(*b"fresh\0\0\0"),b:0};
 assert_eq!(service.handle(102,true,create,&mut backend),None);
 assert_eq!(service.handle(15,true,info,&mut backend),None);
 let mut replies=vec![];service.poll(|pd,v|{if pd==102{return 1}replies.push((pd,document::decode_reply(v,1).unwrap()));0});
 assert_eq!(replies,[(100,Ok(8)),(15,Err(document::DENIED))]);
 assert!(service.retire(100,&mut backend));let mut last=vec![];service.poll(|pd,v|{last.push((pd,document::decode_reply(v,1).unwrap()));0});assert_eq!(last,[(102,Ok(3))]);
 let foreign=document::Request{seq:2,object:2,..info};service.handle(102,true,foreign,&mut backend);let mut denied=None;service.poll(|pd,v|{assert_eq!(pd,102);denied=document::decode_reply(v,2);0});assert_eq!(denied,Some(Err(document::DENIED)));
}

#[test]fn future_frame_admission_retains_backpressure_until_ready(){
 use admission::{DesktopStates,Operation,Admission};
 let op=Operation{opcode:0xEC,surface:400,packed:1,control:0};
 assert!(DesktopStates::tracks(400));let mut delivery=DesktopStates::new();
 delivery.observe(op,Admission::Full);assert!(!delivery.ready(400));
 let mut sent=Vec::new();delivery.poll(4,Some,|packet|{sent.push(packet);Admission::Accepted});
 assert_eq!(sent,[op]);assert!(!delivery.ready(400));
 delivery.poll(4,Some,|packet|{assert_eq!(packet.surface,400);assert_eq!(packet.opcode,0xFD);Admission::Accepted});
 assert!(delivery.ready(400));
}

#[test]fn future_native_labels_and_focus_colors_follow_visual_family_only(){
 for(sid,label,color)in [(400,"Quil",0xcba6f7),(401,"Spindle",0x89b4fa),(402,"Spindle",0x89b4fa),(403,"Spindle",0x89b4fa),(404,"Spindle",0x89b4fa),(405,"Spindle",0x89b4fa),(406,"Browser",0x5ed9c3)]{
  assert_eq!(desktop_style::native_label(sid),Some(label));assert_eq!(desktop_style::native_indicator(sid),Some(color));
 }
 assert_eq!(desktop_style::native_label(999),None);assert_eq!(desktop_style::native_indicator(999),None);
 // Matching a visual family never authorizes a foreign owner to paint it.
 assert_eq!(desktop_style::native_canvas(9,400),None);
}
