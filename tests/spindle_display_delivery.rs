#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../apps/spindle/src/viewport.rs"]mod viewport;
#[path="../servers/silk-shell/src/linen_text_delivery.rs"]mod text_delivery;
#[path="../apps/spindle/src/display_delivery.rs"]mod delivery;
use delivery::*;
use text_batch::{Applied,Request};
fn text(byte:u8)->viewport::Text{
 let v=spindle_view::Layout::new(640,375);let line=[byte;40];
 viewport::render(v,b"",20,0,|_|&line)
}
fn stamp(paint:u64)->Stamp{Stamp{sequence:1,paint}}
fn outbox(target:u32)->Box<Outbox>{let mut o=Box::new(Outbox::new());o.bind(target);o}
fn drain(o:&mut Outbox)->Vec<Operation>{let mut ops=Vec::new();for _ in 0..64{o.poll(128,|op|{ops.push(op);Ok(())}).unwrap();}ops}
fn decode(op:&Operation)->Request{Request::decode(op.opcode,op.a,op.b).unwrap()}
fn begun(op:&Operation)->u32{match decode(op){Request::Begin{sequence,..}=>sequence,other=>panic!("{other:?}")}}
fn applied(sid:u64,sequence:u32)->u64{Applied{surface:sid as u16,sequence}.encode().unwrap()}
#[test]fn create_precedes_one_atomic_terminal_batch_on_the_first_catalog_surface(){
 for(target,sid)in[(12,154),(17,302)]{
  let mut o=outbox(target);let t=text(b'x');o.create(0,0x1234,0x5678);o.text(&t,stamp(1),0xabcdef);
  let ops=drain(&mut o);
  assert_eq!(ops[0],Operation{opcode:0xEC,surface:sid,a:0x1234,b:0x5678});
  assert!(ops[1..].iter().all(|op|op.surface==sid));
  let sequence=begun(&ops[1]);assert!(matches!(decode(ops.last().unwrap()),Request::Commit{..}));
  let mut bytes=vec![0;t.len];
  for op in &ops[1..]{if let Request::Chunk{offset,count,bytes:b,..}=decode(op){bytes[offset..offset+count].copy_from_slice(&b[..count]);}}
  assert_eq!(&bytes[..],&t.bytes[..t.len]);
  assert!(o.pending());
  let r=o.receive(1,applied(sid,sequence)).unwrap();
  assert_eq!((r.surface,r.stamp,r.bytes),(sid,stamp(1),t.len));assert!(!o.pending());
 }
}
#[test]fn full_retains_the_exact_operation_and_budget_is_bounded(){
 let mut o=outbox(12);o.create(1,10,20);o.text(&text(b'x'),stamp(1),1);
 let mut blocked=None;o.poll(128,|op|{blocked=Some(op);Err(ERR_SERVICE_NOT_READY)}).unwrap();
 let mut retried=None;o.poll(1,|op|{retried=Some(op);Ok(())}).unwrap();assert_eq!(blocked,retried);
 let mut calls=0;assert_eq!(o.poll(4,|_|{calls+=1;Ok(())}).unwrap(),4);assert_eq!(calls,4);
}
#[test]fn repaint_coalescing_keeps_the_active_batch_and_only_the_newest_desire(){
 let mut o=outbox(12);let(a,b,c)=(text(b'a'),text(b'b'),text(b'c'));
 o.text(&a,stamp(1),1);o.text(&b,stamp(2),1);o.text(&c,stamp(3),1);
 let ops=drain(&mut o);let first=begun(&ops[0]);
 assert!(o.receive(1,applied(154,first)).is_some());
 let ops=drain(&mut o);let second=begun(&ops[0]);
 let r=o.receive(1,applied(154,second)).unwrap();assert_eq!(r.stamp,stamp(3));assert!(!o.pending());
}
#[test]fn lost_reply_sends_the_batch_again_and_the_late_reply_still_completes_it(){
 let mut o=outbox(12);o.text(&text(b'x'),stamp(1),1);let ops=drain(&mut o);let sequence=begun(&ops[0]);
 let mut again=Vec::new();for _ in 0..5000{o.poll(128,|op|{again.push(op);Ok(())}).unwrap();if !again.is_empty(){break}}
 assert_eq!(begun(&again[0]),sequence);
 assert!(o.receive(1,applied(154,sequence)).is_some());assert!(!o.pending());
}
#[test]fn destroy_drops_unsent_text_and_reopen_accepts_a_fresh_batch(){
 let mut o=outbox(12);o.text(&text(b'a'),stamp(1),1);o.poll(2,|_|Ok(())).unwrap();o.destroy(0);
 let ops=drain(&mut o);assert_eq!(ops.len(),1);assert_eq!(ops[0].opcode,0xEE);assert!(!o.pending());
 o.text(&text(b'b'),stamp(2),1);assert!(!o.pending());
 o.create(0,1,2);o.text(&text(b'c'),stamp(3),1);
 let ops=drain(&mut o);assert_eq!(ops[0].opcode,0xEC);begun(&ops[1]);
}
#[test]fn cancellation_fences_all_later_work(){
 let mut o=outbox(12);o.text(&text(b'x'),stamp(1),1);assert_eq!(o.cancel(),1);assert!(!o.pending());
 o.create(0,1,2);o.text(&text(b'y'),stamp(2),1);assert!(!o.pending());o.poll(4,|_|panic!()).unwrap();
}
#[test]fn rejection_is_terminal_without_a_fake_completion_or_retry(){
 let mut o=outbox(17);o.text(&text(b'x'),stamp(1),1);assert_eq!(o.poll(4,|_|Err(3)),Err(3));assert!(!o.pending());o.poll(4,|_|panic!()).unwrap();
}
#[test]fn unrecognized_subject_cannot_create_or_send_any_surface(){
 let mut o=outbox(9);o.create(0,1,2);o.text(&text(b'x'),stamp(1),1);assert!(!o.pending());o.poll(4,|_|panic!()).unwrap();
}
#[test]fn region_sends_one_fill_instead_of_chunks(){
 let mut region=vec![0u8;8192];let mut o=outbox(12);o.use_region(region.as_mut_ptr() as u64);
 let t=text(b'x');o.text(&t,stamp(1),1);let ops=drain(&mut o);
 assert_eq!(ops.len(),3);assert!(matches!(decode(&ops[1]),Request::Fill{..}));assert_eq!(&region[..t.len],&t.bytes[..t.len]);
 assert!(o.receive(1,applied(154,begun(&ops[0]))).is_some());assert!(!o.pending());
}
