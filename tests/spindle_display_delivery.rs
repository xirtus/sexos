#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../apps/spindle/src/viewport.rs"]mod viewport;
#[path="../apps/spindle/src/display_delivery.rs"]mod delivery;
use delivery::*;
fn text(byte:u8)->viewport::Text{let mut t=viewport::Text{bytes:[byte;126],len:126,cols:20,rows:6};for row in 0..6{t.bytes[row*21+20]=b'\n';}t}
fn stamp(paint:u64)->Stamp{Stamp{sequence:1,paint}}
fn drain(o:&mut Outbox)->(Vec<Operation>,Vec<Receipt>){let mut ops=Vec::new();let mut receipts=Vec::new();while o.pending(){o.poll(4,|op|{ops.push(op);Ok(())},|r|receipts.push(r)).unwrap();}(ops,receipts)}
fn batches(ops:&[Operation])->Vec<Vec<u8>>{let mut out:Vec<Vec<u8>>=Vec::new();for op in ops{if op.opcode==0xFA{out.push(Vec::new());}if op.opcode==0xFB{let off=(op.b&255)as usize;let n=((op.b>>8)&15)as usize;let t=out.last_mut().unwrap();t.resize(t.len().max(off+n),0);t[off..off+n].copy_from_slice(&op.a.to_le_bytes()[..n]);}}out}
#[test]fn every_full_boundary_preserves_exact_create_clear_and_text_for_both_incarnations(){
 for (target,sid)in[(12,154),(17,302)]{for full in 0..18{let mut o=Outbox::new(target);o.create(0,0x1234,0x5678);o.text(0,text(b'x'),stamp(1),0xabcdef);let mut ops=Vec::new();let mut receipt=Vec::new();let mut blocked=None;
 while o.pending(){let attempts=o.poll(4,|op|{assert_eq!(op.surface,sid);if ops.len()==full&&blocked.is_none(){blocked=Some(op);Err(ERR_SERVICE_NOT_READY)}else{if ops.len()==full{assert_eq!(Some(op),blocked);}ops.push(op);Ok(())}},|r|receipt.push(r)).unwrap();assert!(attempts<=4);}
 assert_eq!(receipt.len(),1);assert_eq!(receipt[0].stamp,stamp(1));assert_eq!(ops[0],Operation{opcode:0xEC,surface:sid,a:0x1234,b:0x5678});assert_eq!(batches(&ops),vec![text(b'x').bytes.to_vec()]);}}
}
#[test]fn repaint_coalescing_never_mixes_active_chunks_with_new_input(){let mut o=Outbox::new(12);o.text(2,text(b'a'),stamp(1),1);let mut ops=Vec::new();o.poll(2,|op|{ops.push(op);Ok(())},|_|panic!()).unwrap();o.text(2,text(b'b'),stamp(2),2);o.text(2,text(b'c'),stamp(3),3);let(rest,r)=drain(&mut o);ops.extend(rest);assert_eq!(r.iter().map(|r|r.stamp.paint).collect::<Vec<_>>(),vec![1,3]);assert_eq!(batches(&ops),vec![text(b'a').bytes.to_vec(),text(b'c').bytes.to_vec()]);}
#[test]fn bounded_fair_poll_leaves_input_turns_between_display_operations(){let mut o=Outbox::new(17);for i in 0..4{o.create(i,10,20);o.text(i,text(b'x'),stamp(1),1);}let mut ops=Vec::new();assert_eq!(o.poll(4,|op|{ops.push(op);Ok(())},|_|panic!()),Ok(4));assert_eq!(ops.iter().map(|o|o.surface).collect::<Vec<_>>(),vec![302,303,304,305]);assert!(ops.iter().all(|o|o.opcode==0xEC));assert_eq!(o.poll(0,|_|panic!(),|_|panic!()),Ok(0));assert!(o.pending());}
#[test]fn cancellation_discards_work_and_permanently_fences_queued_recreation(){let mut o=Outbox::new(12);o.text(0,text(b'x'),stamp(1),1);o.poll(4,|_|Err(ERR_SERVICE_NOT_READY),|_|panic!()).unwrap();assert_eq!(o.cancel(),1);o.create(0,1,2);o.text(0,text(b'z'),stamp(2),1);o.destroy(0);assert!(!o.pending());assert_eq!(o.poll(4,|_|panic!(),|_|panic!()),Ok(0));}
#[test]fn destroy_preempts_unsent_text_and_reopen_starts_a_fresh_batch(){let mut o=Outbox::new(12);o.text(0,text(b'a'),stamp(1),1);o.poll(2,|_|Ok(()),|_|panic!()).unwrap();o.destroy(0);let(ops,r)=drain(&mut o);assert!(r.is_empty());assert_eq!(ops.iter().map(|o|o.opcode).collect::<Vec<_>>(),vec![0xEE]);o.create(0,1,2);o.text(0,text(b'b'),stamp(2),1);let(ops,r)=drain(&mut o);assert_eq!(ops[0].opcode,0xEC);assert_eq!(batches(&ops),vec![text(b'b').bytes.to_vec()]);assert_eq!(r[0].stamp.paint,2);}
#[test]fn rejection_is_terminal_without_a_fake_completion_or_retry(){let mut o=Outbox::new(17);o.text(0,text(b'x'),stamp(1),1);assert_eq!(o.poll(4,|_|Err(3),|_|panic!()),Err(3));assert!(!o.pending());o.text(0,text(b'y'),stamp(2),1);assert_eq!(o.poll(4,|_|panic!(),|_|panic!()),Ok(0));}
#[test]fn unrecognized_subject_cannot_create_or_send_any_surface(){let mut o=Outbox::new(9);o.create(0,1,2);o.text(0,text(b'x'),stamp(1),1);assert!(!o.pending());o.poll(4,|_|panic!(),|_|panic!()).unwrap();}
#[test]fn clear_only_batches_complete_only_after_admission(){let mut o=Outbox::new(12);let mut t=text(b'x');t.len=0;o.text(0,t,stamp(1),1);o.poll(4,|_|Err(ERR_SERVICE_NOT_READY),|_|panic!()).unwrap();let(ops,r)=drain(&mut o);assert_eq!(ops.len(),1);assert_eq!(ops[0].opcode,0xFA);assert_eq!(r[0].bytes,0);}
