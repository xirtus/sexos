//! Production asynchronous engine plus extracted actual shell publication.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../servers/silk-shell/src/linen_async.rs"]mod linen_async;
pub const SLOT_LINEN:u64=13;
macro_rules! serial_println{($($t:tt)*)=>{let _=format_args!($($t)*);}}
#[macro_export]macro_rules! serial_record{($($t:tt)*)=>{let _=format_args!($($t)*);}}
#[derive(Clone,Copy)]pub struct PdxMessage{type_id:u64,caller_pd:u32,arg0:u64}
include!(env!("LINEN_SNAPSHOT_SOURCE"));
static mut LINEN_ASYNC:linen_async::Snapshot=linen_async::Snapshot::new();
static mut LINEN_OBJECTS:[Option<LinenObject>;LINEN_MAX_OBJECTS]=[None;LINEN_MAX_OBJECTS];
static mut SELECTED_LINEN_OBJECT_ID:u64=0;static mut LINEN_REMOTE_FETCHED:bool=false;static mut LINEN_REMOTE_REAL:bool=false;
static mut PAINTS:usize=0;static mut SEND_FAIL:bool=false;static mut SENT:Vec<linen_async::Request>=Vec::new();
fn pdx_call_checked(slot:u64,opcode:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{assert_eq!(slot,SLOT_LINEN);unsafe{if SEND_FAIL{return Err(2)}SENT.push(linen_async::Request{opcode,args:[a,b,c]});}Ok(0)}
fn linen_paint_surface_fast(){unsafe{PAINTS+=1}}
fn object(id:u64,name:&[u8])->LinenObject{let mut o=LINEN_SEED_OBJECTS[0];o.object_id=id;o.name=[0;24];o.name[..name.len()].copy_from_slice(name);o.name_len=name.len()as u8;o}
unsafe fn reset(){LINEN_ASYNC=linen_async::Snapshot::new();LINEN_OBJECTS=[None;LINEN_MAX_OBJECTS];LINEN_OBJECTS[0]=Some(object(41,b"old-a"));LINEN_OBJECTS[1]=Some(object(42,b"old-b"));SELECTED_LINEN_OBJECT_ID=42;LINEN_REMOTE_FETCHED=false;LINEN_REMOTE_REAL=true;PAINTS=0;SEND_FAIL=false;SENT.clear();}
fn ids()->Vec<u64>{unsafe{LINEN_OBJECTS.iter().flatten().map(|o|o.object_id).collect()}}
fn metadata(id:u64,kind:u8,len:u8)->u64{id|((kind as u64)<<32)|((len as u64)<<40)|(3<<48)}
unsafe fn start(rescan:bool){LINEN_ASYNC.refresh(rescan);linen_async_poll();}
unsafe fn receive(value:u64){assert!(linen_async_reply(&PdxMessage{type_id:1,caller_pd:7,arg0:value}));}
unsafe fn step(value:u64){receive(value);linen_async_poll();}
unsafe fn empty_tail(slots:usize){for i in 0..slots{receive(0);if i+1<slots{assert_eq!(ids(),vec![41,42]);linen_async_poll();}}}
unsafe fn one(id:u64,name:&[u8]){step(metadata(id,0,name.len()as u8));for c in name.chunks(8){let mut b=[0;8];b[..c.len()].copy_from_slice(c);step(u64::from_le_bytes(b));}}
#[test]fn complete_view_published_once_after_all_slots(){unsafe{reset();start(false);one(42,b"fresh");assert_eq!(ids(),vec![41,42]);empty_tail(15);assert_eq!(ids(),vec![42]);assert_eq!(PAINTS,1);assert_eq!(SELECTED_LINEN_OBJECT_ID,42);assert_eq!(&LINEN_OBJECTS[0].unwrap().name[..5],b"fresh");}}
#[test]fn rescan_ack_cannot_become_object_metadata(){unsafe{reset();start(true);assert_eq!(SENT.last().unwrap().opcode,0x49);step(3);assert_eq!(SENT.last().unwrap().opcode,0x44);assert_eq!(LINEN_ASYNC.count,0);one(77,b"doc");empty_tail(15);assert_eq!(ids(),vec![77]);}}
#[test]fn foreign_reply_leaves_outstanding_request_untouched(){unsafe{reset();start(false);assert!(!linen_async_reply(&PdxMessage{type_id:1,caller_pd:8,arg0:999}));assert!(LINEN_ASYNC.request().is_none());one(77,b"doc");empty_tail(15);assert_eq!(ids(),vec![77]);}}
#[test]fn hid_selection_during_fetch_is_preserved_at_commit(){unsafe{reset();start(false);one(41,b"new-a");SELECTED_LINEN_OBJECT_ID=41;one(42,b"new-b");empty_tail(14);assert_eq!(SELECTED_LINEN_OBJECT_ID,41);}}
#[test]fn removed_selection_repairs_after_complete_publication(){unsafe{reset();start(false);one(77,b"new");empty_tail(15);assert_eq!(SELECTED_LINEN_OBJECT_ID,77);}}
#[test]fn duplicate_ids_reject_without_touching_view(){unsafe{reset();start(false);one(77,b"new");receive(metadata(77,0,3));assert_eq!(ids(),vec![41,42]);assert!(!LINEN_REMOTE_FETCHED);assert_eq!(PAINTS,0);}}
#[test]fn malformed_metadata_preserves_old_view(){unsafe{for packed in [metadata(0,0,3),metadata(77,3,3),metadata(77,0,25),u64::MAX]{reset();start(false);receive(packed);assert_eq!(ids(),vec![41,42]);assert_eq!(PAINTS,0);assert!(!LINEN_REMOTE_FETCHED);}}}
#[test]fn partial_name_failure_preserves_view_and_current_selection(){unsafe{for bad in [0,u64::MAX]{reset();start(false);step(metadata(77,0,12));step(u64::from_le_bytes(*b"partial!"));SELECTED_LINEN_OBJECT_ID=41;receive(bad);assert_eq!(ids(),vec![41,42]);assert_eq!(SELECTED_LINEN_OBJECT_ID,41);assert_eq!(PAINTS,0);}}}
#[test]fn full_length_name_chunks_are_bounded_and_sanitized(){unsafe{reset();start(false);one(77,b"abcdefgh\x01jklmnopQRSTUVWX");empty_tail(15);let obj=LINEN_OBJECTS[0].unwrap();assert_eq!(obj.name_len,24);assert_eq!(obj.name[8],b'?');let offsets:Vec<_>=SENT.iter().filter(|r|r.opcode==0x45).map(|r|r.args[1]).collect();assert_eq!(offsets,vec![0,8,16]);}}
#[test]fn empty_snapshot_commits_seed_fallback(){unsafe{reset();start(false);empty_tail(16);assert!(!LINEN_REMOTE_REAL);assert_eq!(ids(),LINEN_SEED_OBJECTS.iter().map(|o|o.object_id).collect::<Vec<_>>());}}
#[test]fn full_transport_retains_exact_request_until_accepted(){unsafe{reset();LINEN_ASYNC.refresh(true);SEND_FAIL=true;linen_async_poll();assert!(SENT.is_empty());assert!(LINEN_ASYNC.can_send());assert_eq!(LINEN_ASYNC.request().unwrap().opcode,0x49);SEND_FAIL=false;linen_async_poll();assert_eq!(SENT.len(),1);linen_async_poll();assert_eq!(SENT.len(),1);assert!(!LINEN_ASYNC.can_send());step(4);assert_eq!(SENT.last().unwrap().opcode,0x44);}}
#[test]fn open_ack_serialized_before_snapshot_and_bounded_queue(){unsafe{reset();for id in 1..=4{assert!(LINEN_ASYNC.queue_open(id,0));}assert!(!LINEN_ASYNC.queue_open(5,0));LINEN_ASYNC.refresh(false);for id in 1..=4{linen_async_poll();assert_eq!(SENT.last().unwrap().opcode,0x46);assert_eq!(SENT.last().unwrap().args[0],id);receive(0);assert_eq!(PAINTS,0);}linen_async_poll();assert_eq!(SENT.last().unwrap().opcode,0x44);one(77,b"doc");empty_tail(15);assert_eq!(ids(),vec![77]);}}
#[test]fn refresh_during_inflight_snapshot_waits_without_misrouting_reply(){unsafe{reset();start(false);LINEN_ASYNC.refresh(true);one(77,b"doc");empty_tail(15);linen_async_poll();assert_eq!(SENT.last().unwrap().opcode,0x49);assert_eq!(ids(),vec![77]);}}
#[test]fn reply_without_pending_request_is_ignored(){let mut s=linen_async::Snapshot::new();assert_eq!(s.reply(7,1,42),linen_async::Reply::Ignored);assert_eq!(s.count,0);assert!(!s.can_send());}
