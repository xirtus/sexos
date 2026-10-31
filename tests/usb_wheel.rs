// SPDX-License-Identifier: GPL-3.0-only
//! Extracted actual decoders, packing and normalizer. Host callbacks establish
//! data preservation and bounds, not native hardware/IPC authority or pixels.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_scroll.rs"]pub mod app_scroll;
const ERR_SERVICE_NOT_READY:u64=9;const OP_USB_MOUSE_REPORT:u64=0x260;const OP_USB_KEYBOARD_REPORT:u64=0x261;const EV_KEY:u64=1;
const EV_REL:u64=2;const EV_ABS:u64=3;const EV_BTN:u64=4;const EV_WHEEL:u64=5;
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
macro_rules! serial_println{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
struct PdxMessage {caller_pd:u32,caller_task:u32}
use std::cell::RefCell;
#[derive(Default)]struct Transport { replies:std::collections::VecDeque<Result<bool,u64>>, calls:Vec<(u64,u64,u64,u64)> }
thread_local! { static TRANSPORT:RefCell<Transport>=RefCell::new(Transport::default()); }
fn send_report_once(op:u64,a:u64,b:u64,c:u64)->Result<bool,u64> {
 TRANSPORT.with(|t|{let mut t=t.borrow_mut();t.calls.push((op,a,b,c));t.replies.pop_front().unwrap_or(Ok(true))})
}
static USB_TEST_LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
include!(env!("USB_WHEEL_PRODUCTION"));
#[test]fn optional_tablet_wheel_is_taken_only_from_received_bytes(){
 let bytes=[7,0x12,0x34,0x56,0x78,0xff];
 assert!(decode_tablet_report(&bytes,4).is_none());assert!(decode_tablet_report(&bytes,7).is_none());
 let old=decode_tablet_report(&bytes,5).unwrap();assert_eq!((old.abs_x,old.abs_y,old.buttons,old.wheel),(0x3412,0x7856,7,0));
 let new=decode_tablet_report(&bytes,6).unwrap();assert_eq!(new.wheel,-1);
 for len in 0..3 {assert!(decode_boot_mouse_report(&bytes,len).is_none());}
 assert!(decode_boot_mouse_report(&bytes,7).is_none());
 assert_eq!(decode_boot_mouse_report(&bytes,3).unwrap().wheel,0);
 assert_eq!(decode_boot_mouse_report(&bytes,4).unwrap().wheel,0x56);
}
#[test]fn every_signed_wheel_survives_tablet_and_mouse_packing_without_axis_overlap(){
 for wheel in i8::MIN..=i8::MAX {
  for (x,y) in [(0,0),(1,32767),(32767,1),(12345,23456)] {
   let bytes=[5,x as u8,(x>>8)as u8,y as u8,(y>>8)as u8,wheel as u8];
   let decoded=decode_tablet_report(&bytes,6).unwrap();
   let input=decode_pointer_payload(decoded.buttons as u64,pack_tablet_axes(&decoded)).unwrap();
   assert_eq!((input.dx,input.dy,input.buttons,input.wheel,input.is_abs),(x,y,5,wheel,true));
  }
  let bytes=[3,(-127i8)as u8,126,wheel as u8];let decoded=decode_boot_mouse_report(&bytes,4).unwrap();
  let input=decode_pointer_payload(decoded.buttons as u64,pack_mouse_axes(&decoded)).unwrap();
  assert_eq!((input.dx,input.dy,input.buttons,input.wheel,input.is_abs),(-127,126,3,wheel,false));
 }
}
#[test]fn maximum_report_emits_position_three_edges_then_exact_wheel_once(){
 for absolute in [false,true] {for wheel in [i8::MIN,-1,1,i8::MAX] {
  let mut last=0;let mut events=Vec::new();
  let report=HidPointerRawReport{dx:123,dy:45,buttons:7,wheel,is_abs:absolute};
  assert_eq!(normalize_pointer_report_v1(report,&mut last,|a,b,c|events.push((a,b,c))),5);
  assert_eq!(events[0],(123,45,if absolute{EV_ABS}else{EV_REL}));
  assert_eq!(&events[1..4],&[(1,1,EV_BTN),(2,1,EV_BTN),(3,1,EV_BTN)]);
  assert_eq!(events[4],(wheel as i64 as u64,0,EV_WHEEL));assert_eq!(last,7);
  let mut empty=Vec::new();assert_eq!(normalize_pointer_report_v1(HidPointerRawReport{dx:0,dy:0,buttons:7,wheel:0,is_abs:false},&mut last,|a,b,c|empty.push((a,b,c))),0);assert!(empty.is_empty());
 }}
}
#[test]fn malformed_payloads_and_other_producer_incarnations_are_rejected(){
 for buttons in [8,256,u64::MAX] {assert!(decode_pointer_payload(buttons,0).is_none());}
 for bit in 24..64 {if bit!=32 {assert!(decode_pointer_payload(0,1u64<<bit).is_none());}}
 assert!(decode_pointer_payload(0,1<<32|32768).is_none());
 assert!(decode_pointer_payload(0,1<<32|(32768<<16)).is_none());
 for bit in 33..64 {if !(40..48).contains(&bit) {assert!(decode_pointer_payload(0,1<<32|1u64<<bit).is_none());}}
 for pd in [0,3,4,5,15] {for task in [0,3,4,5,50] {assert_eq!(usb_report_sender(&PdxMessage{caller_pd:pd,caller_task:task}),pd==5&&task==5);}}
}

fn batch(report:HidPointerRawReport,last:&mut u8)->Vec<(u64,u64,u64)> {
 let mut events=Vec::new();normalize_pointer_report_v1(report,last,|a,b,c|events.push((a,b,c)));events
}
#[test]fn input_full_keeps_accepted_prefix_and_old_wheel_before_newer_position(){
 let mut last=0;let mut q=PointerOutbox::new();
 let old=batch(HidPointerRawReport{dx:100,dy:200,buttons:7,wheel:-1,is_abs:true},&mut last);
 let new=batch(HidPointerRawReport{dx:300,dy:400,buttons:7,wheel:1,is_abs:true},&mut last);
 assert!(q.push(&old));assert!(q.push(&new));let mut admitted=Vec::new();let mut calls=Vec::new();
 assert!(!q.poll(|a,b,c|{calls.push((a,b,c));if c==EV_BTN {Err(9)}else{admitted.push((a,b,c));Ok(true)}}));
 assert_eq!(admitted,&old[..1]);assert_eq!(q.batches[q.head].next,1);
 let before=q.batches[q.head].words;
 assert!(!q.poll(|a,b,c|{assert_eq!((a,b,c),old[1]);Err(9)}));assert_eq!(q.batches[q.head].words,before);
 while q.len>0 {assert!(!q.poll(|a,b,c|{admitted.push((a,b,c));Ok(true)}));}
 assert_eq!(admitted,[old,new].concat());
 assert_eq!(admitted.iter().filter(|e|**e==(100,200,EV_ABS)).count(),1);
 assert!(q.batches.iter().all(|b|b.words==[(0,0,0);5]&&b.len==0));
}
#[test]fn input_capacity_rejects_whole_batch_and_wrap_preserves_fifo(){
 let mut q=PointerOutbox::new();for i in 0..32 {assert!(q.push(&[(i,0,EV_ABS),(1,0,EV_WHEEL)]));}
 assert_eq!(q.available(),0);let before=q.batches[0].words;assert!(!q.push(&[(999,0,EV_WHEEL)]));assert_eq!(q.batches[0].words,before);
 let mut admitted=Vec::new();q.poll(|a,b,c|{admitted.push((a,b,c));Ok(true)});assert_eq!(q.available(),2);
 assert!(q.push(&[(32,0,EV_ABS),(1,0,EV_WHEEL)]));assert!(q.push(&[(33,0,EV_ABS),(1,0,EV_WHEEL)]));
 while q.len>0 {q.poll(|a,b,c|{admitted.push((a,b,c));Ok(true)});}
 assert_eq!(admitted,(0..34).flat_map(|i|[(i,0,EV_ABS),(1,0,EV_WHEEL)]).collect::<Vec<_>>());
}
#[test]fn input_stall_or_denial_erases_whole_suffix_never_orphaning_wheel(){
 for denied in [false,true] {
  let mut q=PointerOutbox::new();assert!(q.push(&[(10,20,EV_ABS),(1,1,EV_BTN),(1,0,EV_WHEEL)]));assert!(q.push(&[(30,40,EV_ABS),((-1i64)as u64,0,EV_WHEEL)]));
  let mut sent=Vec::new();q.poll(|a,b,c|{if c==EV_WHEEL {Err(9)}else{sent.push((a,b,c));Ok(true)}});
  assert_eq!(q.sent_buttons,1);assert_eq!(sent.len(),2);
  if denied {assert!(q.poll(|_,_,_|Err(7)));}else{
   for _ in 1..PointerOutbox::STALL_LIMIT-1 {assert!(!q.poll(|_,_,_|Ok(false)));}
   assert!(q.poll(|_,_,_|Err(9)));
  }
  assert_eq!(q.len,0);assert!(q.batches.iter().all(|b|b.words==[(0,0,0);5]&&b.len==0&&b.next==0));
  q.poll(|_,_,_|panic!("expired input must not replay"));assert_eq!(q.sent_buttons,1);
 }
}
#[test]fn usb_raw_full_retains_exact_report_once_and_bounds_attempts(){
 let mut q=UsbPointerOutbox::new();let first=(0,1,1<<32|100|(200<<16)|(255u64<<40));let second=(0,0,1<<32|300|(400<<16)|(1<<40));
 assert!(q.push(first));assert!(q.push(second));let mut calls=Vec::new();q.poll(|a,b,c|{calls.push((a,b,c));Err(9)});
 assert_eq!(calls,vec![first]);assert_eq!(q.words[q.head],first);assert_eq!(q.len,2);
 let mut admitted=Vec::new();q.poll(|a,b,c|{admitted.push((a,b,c));Ok(true)});assert_eq!(admitted,vec![first,second]);
 q.poll(|_,_,_|panic!("accepted USB report replay"));assert!(q.words.iter().all(|p|*p==(0,0,0)));
 for i in 0..32 {assert!(q.push((0,0,i)));}assert!(!q.push((0,0,99)));let mut n=0;q.poll(|_,_,_|{n+=1;Ok(true)});assert_eq!(n,4);
}
#[test]fn usb_expiry_never_sends_partial_raw_report_or_retains_expired_payload(){
 let mut q=UsbPointerOutbox::new();assert!(q.push((0,0,1<<32|(255u64<<40))));
 for _ in 0..UsbPointerOutbox::STALL_LIMIT-1 {q.poll(|_,_,_|Err(9));assert_eq!(q.len,1);}
 q.poll(|_,_,_|Err(9));assert_eq!(q.len,0);assert!(q.words.iter().all(|p|*p==(0,0,0)));
 q.poll(|_,_,_|panic!("expired USB replay"));
}
#[test]fn actual_usb_sender_wrapper_preserves_old_pointer_before_later_keyboard(){
 let _lock=USB_TEST_LOCK.lock().unwrap();unsafe{POINTER_OUTBOX=UsbPointerOutbox::new();KEYBOARD_OUTBOX=UsbPointerOutbox::new();INPUT_ARRIVAL=0;}
 TRANSPORT.with(|t|{*t.borrow_mut()=Transport{replies:[Err(9),Ok(true),Ok(true),Ok(true)].into(),calls:vec![]};});
 let first=1<<32|100|(200<<16)|(255u64<<40);let second=1<<32|300|(400<<16)|(1<<40);
 assert_eq!(send_report_to_sexinput(OP_USB_MOUSE_REPORT,0,1,first),Ok(false));
 assert_eq!(send_report_to_sexinput(OP_USB_MOUSE_REPORT,0,0,second),Ok(false));
 assert_eq!(send_report_to_sexinput(0x261,0,0,4),Ok(false));poll_pointer_reports();poll_pointer_reports();
 TRANSPORT.with(|t|assert_eq!(t.borrow().calls,vec![(OP_USB_MOUSE_REPORT,0,1,first),(OP_USB_MOUSE_REPORT,0,1,first),(OP_USB_MOUSE_REPORT,0,0,second),(0x261,0,0,4)]));
 unsafe{assert_eq!((*core::ptr::addr_of!(POINTER_OUTBOX)).len,0);}
 record_device_wheel(2u32,6u32,true,-1,first);
}

#[test]fn contended_shell_ring_admits_close_key_edges_before_any_pointer(){
 let mut keys=KeyboardOutbox::new();let mut pointer=PointerOutbox::new();let mut arrival=InputArrival::new();
 for raw in [0x1d,0x10,0x90,0x9d] {assert!(arrival.ps2(&mut keys,raw));}
 let mut admitted=Vec::new();let mut attempts=Vec::new();
 for turn in 0..4 {
  assert!(arrival.pointer(&mut pointer,&[(turn,100,EV_ABS),(1,0,EV_WHEEL)]));let mut capacity=1;
  assert!(!poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{
   attempts.push((a,b,c));if capacity==0 {Err(9)}else{capacity-=1;admitted.push((a,b,c));Ok(true)}
  }));
  assert_eq!(admitted.len(),turn as usize+1);assert!(admitted.iter().all(|e|e.2==EV_KEY));
 }
 assert_eq!(admitted,vec![(0x1d,1,EV_KEY),(0x10,1,EV_KEY),(0x10,0,EV_KEY),(0x1d,0,EV_KEY)]);
 assert_eq!(keys.len,0);assert_eq!(pointer.len,4);
 let mut count=0;poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{count+=1;admitted.push((a,b,c));Ok(true)});assert_eq!(count,4);
 assert_eq!(admitted.iter().filter(|e|**e==(0x10,1,EV_KEY)).count(),1);
}
#[test]fn blocked_key_front_never_expires_or_spends_pointer_capacity(){
 let mut keys=KeyboardOutbox::new();let mut pointer=PointerOutbox::new();let mut arrival=InputArrival::new();assert!(arrival.ps2(&mut keys,0x10));assert!(arrival.pointer(&mut pointer,&[(10,20,EV_ABS),(1,0,EV_WHEEL)]));
 for _ in 0..5000 {let mut count=0;poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{count+=1;assert_eq!((a,b,c),(0x10,1,EV_KEY));Err(9)});assert_eq!(count,1);}
 assert_eq!(keys.len,1);assert_eq!(pointer.batches[pointer.head].next,0);
 let mut sent=Vec::new();poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{sent.push((a,b,c));Ok(true)});assert_eq!(sent,vec![(0x10,1,EV_KEY),(10,20,EV_ABS),(1,0,EV_WHEEL)]);
 assert!(keys.edges.iter().all(|e|e.code==0&&e.down==0&&e.modifiers==0&&e.source==0));
}
#[test]fn usb_modifier_and_key_snapshot_is_atomic_and_last_state_only_follows_retention(){
 let mut keys=KeyboardOutbox::new();for _ in 0..119 {assert!(keys.ps2(1));}
 let(mut key,mut mods)=(0x04,0x55);assert!(!keys.usb(0x05,0xaa,&mut key,&mut mods));assert_eq!((key,mods),(0x04,0x55));assert_eq!(keys.len,119);
 let mut budget=1;keys.poll(&mut budget,|_,_,_|Ok(true));assert!(keys.usb(0x05,0xaa,&mut key,&mut mods));assert_eq!((key,mods),(0x05,0xaa));assert_eq!(keys.len,128);
 let mut sent=Vec::new();while keys.len>0 {let mut budget=4;keys.poll(&mut budget,|a,b,c|{sent.push((a,b,c));Ok(true)});}
 let tail=&sent[118..];assert_eq!(tail.len(),10);assert_eq!(tail[0],(0x1e,0,EV_KEY));assert_eq!(tail[9],(0x30,1,EV_KEY));
 assert_eq!(&tail[1..5],&[(0x1d,0,EV_KEY),(0x38,0,EV_KEY),(0x61,0,EV_KEY),(0x64,0,EV_KEY)]);
 assert_eq!(&tail[5..9],&[(0x2a,1,EV_KEY),(0x5b,1,EV_KEY),(0x36,1,EV_KEY),(0x5c,1,EV_KEY)]);
}
#[test]fn key_fifo_wrap_and_shared_four_call_budget_preserve_exact_prefix(){
 let mut keys=KeyboardOutbox::new();let mut arrival=InputArrival::new();for raw in 0..128 {assert!(arrival.ps2(&mut keys,raw));}assert!(!arrival.ps2(&mut keys,1));assert!(!arrival.ps2(&mut keys,256));
 let mut pointer=PointerOutbox::new();let mut sent=Vec::new();
 for _ in 0..16 {let mut calls=0;poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{calls+=1;sent.push((a,b,c));Ok(true)});assert_eq!(calls,4);}
 for raw in 128..192 {assert!(arrival.ps2(&mut keys,raw));}
 while keys.len>0 {poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{sent.push((a,b,c));Ok(true)});}
 assert_eq!(&sent[..192],&(0..192).map(|raw|((raw&127)as u64,(raw<128)as u64,EV_KEY)).collect::<Vec<_>>());assert!(sent.iter().all(|e|e.2==EV_KEY));
}
#[test]fn upstream_usb_old_click_barrier_survives_full_before_later_key(){
 let _lock=USB_TEST_LOCK.lock().unwrap();unsafe{POINTER_OUTBOX=UsbPointerOutbox::new();KEYBOARD_OUTBOX=UsbPointerOutbox::new();INPUT_ARRIVAL=0;}
 TRANSPORT.with(|t|*t.borrow_mut()=Transport{replies:[Err(9)].into(),calls:vec![]});
 let pointer=1<<32|(255u64<<40);let later=1<<32|100|(200<<16);
 assert_eq!(send_report_to_sexinput(OP_USB_MOUSE_REPORT,0,1,pointer),Ok(false));assert_eq!(send_report_to_sexinput(OP_USB_KEYBOARD_REPORT,0,1,0x04),Ok(false));
 assert_eq!(send_report_to_sexinput(OP_USB_MOUSE_REPORT,0,0,later),Ok(false));
 TRANSPORT.with(|t|t.borrow_mut().replies.extend(std::iter::repeat_n(Err(9),5000)));
 for _ in 0..5000 {poll_pointer_reports();}
 unsafe{assert_eq!((*core::ptr::addr_of!(KEYBOARD_OUTBOX)).len,1);assert_eq!((*core::ptr::addr_of!(POINTER_OUTBOX)).len,2);}
 TRANSPORT.with(|t|{let t=t.borrow();assert_eq!(t.calls.len(),5001);assert!(t.calls.iter().all(|c|*c==(OP_USB_MOUSE_REPORT,0,1,pointer)));});
 poll_pointer_reports();TRANSPORT.with(|t|{let t=t.borrow();assert_eq!(&t.calls[t.calls.len()-3..],&[(OP_USB_MOUSE_REPORT,0,1,pointer),(OP_USB_KEYBOARD_REPORT,0,1,0x04),(OP_USB_MOUSE_REPORT,0,0,later)]);});
}
#[test]fn partial_old_click_full_then_key_then_later_motion_preserves_causal_focus_order(){
 let mut keys=KeyboardOutbox::new();let mut pointer=PointerOutbox::new();let mut arrival=InputArrival::new();
 let click=[(100,200,EV_ABS),(1,1,EV_BTN),(1,0,EV_BTN)];assert!(arrival.pointer(&mut pointer,&click));assert!(arrival.ps2(&mut keys,0x10));
 assert!(arrival.pointer(&mut pointer,&[(300,400,EV_ABS),(1,0,EV_WHEEL)]));let mut sent=Vec::new();
 poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{if c==EV_BTN {Err(9)}else{sent.push((a,b,c));Ok(true)}});assert_eq!(sent,vec![click[0]]);
 for _ in 0..5000 {poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{assert_eq!((a,b,c),click[1]);Err(9)});}
 assert_eq!(keys.len,1);assert_eq!(pointer.batches[pointer.head].next,1);
 while keys.len>0||pointer.len>0 {poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{sent.push((a,b,c));Ok(true)});}
 assert_eq!(sent,[click.as_slice(),&[(0x10,1,EV_KEY),(300,400,EV_ABS),(1,0,EV_WHEEL)]].concat());
}
#[test]fn failed_old_click_erases_dependent_key_and_later_pointer_without_leak(){
 let mut keys=KeyboardOutbox::new();let mut pointer=PointerOutbox::new();let mut arrival=InputArrival::new();
 assert!(arrival.pointer(&mut pointer,&[(100,200,EV_ABS),(1,1,EV_BTN)]));assert!(arrival.ps2(&mut keys,0x10));assert!(arrival.pointer(&mut pointer,&[(300,400,EV_ABS)]));
 let mut sent=Vec::new();assert!(poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{if c==EV_BTN {Err(7)}else{sent.push((a,b,c));Ok(true)}}));
 assert_eq!(sent,vec![(100,200,EV_ABS)]);assert_eq!((keys.len,pointer.len),(0,0));poll_input_delivery(&mut keys,&mut pointer,|_,_,_|panic!("dependent input leaked"));
 assert!(keys.edges.iter().all(|e|e.code==0));
}
#[test]fn finite_arrival_exhaustion_refuses_new_work_without_changing_retained_order_or_key_state(){
 let mut keys=KeyboardOutbox::new();let mut pointer=PointerOutbox::new();let mut arrival=InputArrival::new();
 assert!(arrival.pointer(&mut pointer,&[(100,200,EV_ABS)]));assert!(arrival.ps2(&mut keys,0x10));arrival.last=u64::MAX;
 let(mut key,mut modifiers)=(0,0);assert!(!arrival.usb(&mut keys,4,1,&mut key,&mut modifiers));assert_eq!((key,modifiers),(0,0));
 assert!(!arrival.ps2(&mut keys,0x90));assert!(!arrival.pointer(&mut pointer,&[(300,400,EV_ABS)]));
 let mut sent=Vec::new();poll_input_delivery(&mut keys,&mut pointer,|a,b,c|{sent.push((a,b,c));Ok(true)});assert_eq!(sent,vec![(100,200,EV_ABS),(0x10,1,EV_KEY)]);assert_eq!(arrival.last,u64::MAX);
}
#[test]fn upstream_hard_failure_and_counter_exhaustion_never_leak_dependent_key(){
 let _lock=USB_TEST_LOCK.lock().unwrap();unsafe{POINTER_OUTBOX=UsbPointerOutbox::new();KEYBOARD_OUTBOX=UsbPointerOutbox::new();INPUT_ARRIVAL=0;}
 TRANSPORT.with(|t|*t.borrow_mut()=Transport{replies:[Err(9),Err(7)].into(),calls:vec![]});
 assert_eq!(send_report_to_sexinput(OP_USB_MOUSE_REPORT,0,1,1<<32),Ok(false));assert_eq!(send_report_to_sexinput(OP_USB_KEYBOARD_REPORT,0,1,4),Ok(false));poll_pointer_reports();poll_pointer_reports();
 TRANSPORT.with(|t|assert_eq!(t.borrow().calls,vec![(OP_USB_MOUSE_REPORT,0,1,1<<32),(OP_USB_MOUSE_REPORT,0,1,1<<32)]));
 unsafe{INPUT_ARRIVAL=u64::MAX;}
 TRANSPORT.with(|t|t.borrow_mut().replies.push_back(Err(9)));assert_eq!(send_report_to_sexinput(OP_USB_MOUSE_REPORT,0,0,0),Err(9));
 unsafe{assert_eq!((*core::ptr::addr_of!(KEYBOARD_OUTBOX)).len,0);assert_eq!((*core::ptr::addr_of!(POINTER_OUTBOX)).len,0);assert_eq!(*core::ptr::addr_of!(INPUT_ARRIVAL),u64::MAX);}
}
