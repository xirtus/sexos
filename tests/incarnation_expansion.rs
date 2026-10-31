#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/document.rs"]pub mod document;
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
 pub fn activation(slot:usize,_commit:bool)->Result<ActivationInfo,u64>{unsafe{crate::INFO[match slot{44=>0,55=>1,112=>2,_=>panic!("unheld test edge")}]}}
}
use app_lifecycle::ActivationInfo;
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[Some(app_custody::Entry::new(45,16,160,301)),Some(app_custody::Entry::new(54,18,180,307)),Some(app_custody::Entry::new(110,100,1000,400))];
static mut INFO:[Result<ActivationInfo,u64>;3]=[Err(3);3];
fn native_launcher_status(target:u32)->(bool,&'static str,&'static str){if target==100{(true,"RUNNING / FOCUS","original_incarnation_live")}else{(false,"CLOSED","no_boot_successor")}}
include!(env!("EXPANDED_SHELL_POLICY"));
#[test]fn fourth_quil_resolves_exact_edge_and_never_skips_denials(){unsafe{
 INFO=[Ok(ActivationInfo{successor:16,task:160,surface:301,state:3}),Ok(ActivationInfo{successor:18,task:180,surface:307,state:3}),Ok(ActivationInfo{successor:100,task:1000,surface:400,state:1})];
 assert_eq!(app_activation_info(app_catalog::App::Quil,false).unwrap().successor,100);assert_eq!(app_reserve_remaining(app_catalog::App::Quil),Some(1));
 INFO[2].as_mut().unwrap().state=2;assert_eq!(app_route_slot(app_catalog::App::Quil,11),113);assert!(app_launcher_status(app_catalog::App::Quil).0);
 INFO[2].as_mut().unwrap().task=100;assert_eq!(app_activation_info(app_catalog::App::Quil,true),Err(3));
 INFO[2].as_mut().unwrap().task=1000;INFO[0]=Err(2);assert_eq!(app_activation_info(app_catalog::App::Quil,false),Err(2));
}}
#[test]fn future_spindle_grid_and_browser_policy_use_exact_catalog_pairs(){
 assert_eq!(app_catalog::DOCUMENT_TARGETS,[9,16,18,100,15,102]);
 let mut out=display_delivery::Outbox::new(101);out.create(0,0,123);let mut seen=vec![];out.poll(4,|op|{seen.push(op.surface);Ok(())},|_|{}).unwrap();assert_eq!(seen,[401]);
 assert!(linen::admit(101,0x42,Some(5)));assert!(!linen::admit(101,0x42,Some(2)));assert!(!linen::admit(102,0x42,Some(5)));
 assert_eq!(desktop_content::parent_frame(405),Some(402));assert_eq!(desktop_content::parent_frame(406),None);
 let good=app_custody::Entry::new(118,102,102,406);assert!(good.allows_operation(0x275,[102,102,0]));
 assert!(!app_custody::Entry::new(118,102,102,400).allows_operation(0x275,[102,102,0]));
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
