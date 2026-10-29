// SPDX-License-Identifier: GPL-3.0-only
#[path="../servers/silk-shell/src/linen_text_delivery.rs"] mod production;
use production::*;
fn metadata(id:u64)->Metadata{Metadata{count:5,first:id as usize,rows:3,selected:id}}
fn flush(out:&mut Outbox)->(Vec<Operation>,Vec<Receipt>){let mut ops=Vec::new();let mut receipts=Vec::new();while out.pending(){if let Poll::Complete(r)=out.poll(4,|op|{ops.push(op);Admission::Accepted}){receipts.push(r);}}(ops,receipts)}
fn decode(ops:&[Operation])->Vec<Vec<u8>>{let mut batches:Vec<Vec<u8>>=Vec::new();for op in ops{assert_eq!(op.surface,200);if op.opcode==0xFA{assert_eq!((op.packed,op.control),(0,0));batches.push(Vec::new());}else{assert_eq!(op.opcode,0xFB);let off=(op.control&255)|(((op.control>>12)&15)<<8);let len=(op.control>>8)&15;assert!((1..=8).contains(&len));let b=batches.last_mut().unwrap();assert_eq!(off as usize,b.len());for i in 0..len{b.push((op.packed>>(i*8))as u8);}}}batches}
#[test]fn full_at_every_clear_or_chunk_boundary_retains_exact_operation(){
 let text:Vec<u8>=(0..512).map(|i|(i%251)as u8).collect();
 for full_at in 0..65{let mut o=Outbox::new(200);assert!(o.queue(&text,0x123456,metadata(1)));let mut accepted=Vec::new();let mut rejected=None;
 loop{let result=o.poll(4,|op|{if accepted.len()==full_at&&rejected.is_none(){rejected=Some(op);Admission::Full}else{if accepted.len()==full_at{assert_eq!(Some(op),rejected);}accepted.push(op);Admission::Accepted}});
 if let Poll::Complete(r)=result{assert_eq!(r,Receipt{metadata:metadata(1),bytes:512});break;}}
 assert_eq!(decode(&accepted),vec![text.clone()]);assert!(accepted[1..].iter().all(|o|o.control>>32==0x123456));assert_eq!(o.poll(4,|_|panic!()),Poll::Idle);
 }
}
#[test]fn many_repaints_coalesce_latest_without_mixing_active_text(){
 let mut o=Outbox::new(200);o.queue(b"original long text",1,metadata(1));let mut ops=Vec::new();assert_eq!(o.poll(2,|op|{ops.push(op);Admission::Accepted}),Poll::Pending);
 o.queue(b"superseded",2,metadata(2));o.queue(b"latest complete repaint",3,metadata(3));let(rest,receipts)=flush(&mut o);ops.extend(rest);
 assert_eq!(decode(&ops),vec![b"original long text".to_vec(),b"latest complete repaint".to_vec()]);assert_eq!(receipts.iter().map(|r|r.metadata.selected).collect::<Vec<_>>(),vec![1,3]);
}
#[test]fn bounded_poll_yields_control_without_losing_progress(){
 let mut o=Outbox::new(200);o.queue(&[b'x';512],1,metadata(1));assert_eq!(o.poll(0,|_|panic!()),Poll::Pending);
 let mut calls=0;assert_eq!(o.poll(4,|_|{calls+=1;Admission::Accepted}),Poll::Pending);assert_eq!(calls,4);
 calls=0;assert_eq!(o.poll(4,|_|{calls+=1;Admission::Full}),Poll::Deferred);assert_eq!(calls,1);
}
#[test]fn full_never_reports_present_even_after_clear(){
 let mut o=Outbox::new(200);o.queue(b"x",1,metadata(1));assert_eq!(o.poll(1,|_|Admission::Accepted),Poll::Pending);
 for _ in 0..20{assert_eq!(o.poll(4,|_|Admission::Full),Poll::Deferred);assert!(o.pending());}
 assert_eq!(o.poll(4,|op|{assert_eq!(op.opcode,0xFB);Admission::Accepted}),Poll::Complete(Receipt{metadata:metadata(1),bytes:1}));
}
#[test]fn unexpected_error_abandons_active_and_latest_or_next_repaint_recovers(){
 for queued in[false,true]{let mut o=Outbox::new(200);o.queue(b"failed",1,metadata(1));o.poll(1,|_|Admission::Accepted);
 if queued{o.queue(b"recovery",2,metadata(2));}assert_eq!(o.poll(4,|_|Admission::Rejected(99)),Poll::Rejected(99));
 if !queued{assert!(!o.pending());o.queue(b"recovery",2,metadata(2));}let(ops,rs)=flush(&mut o);assert_eq!(decode(&ops),vec![b"recovery".to_vec()]);assert_eq!(rs.len(),1);assert_eq!(rs[0].metadata.selected,2);}
}
#[test]fn empty_text_is_a_retained_clear_only_and_invalid_input_is_atomic(){
 let mut o=Outbox::new(200);o.queue(b"",1,metadata(1));assert!(!o.queue(&[0;513],1,metadata(2)));assert_eq!(o.poll(4,|_|Admission::Full),Poll::Deferred);
 let(ops,rs)=flush(&mut o);assert_eq!(decode(&ops),vec![Vec::<u8>::new()]);assert_eq!(rs,vec![Receipt{metadata:metadata(1),bytes:0}]);
}
fn op(opcode:u64,surface:u64)->Operation{Operation{opcode,surface,packed:11,control:22}}
#[test]fn palette_full_hide_then_reopen_never_replays_stale_hide(){
 let mut q=LatestOperation::new();q.set(op(0xEE,152));assert_eq!(q.poll(|_|Admission::Full),LatestPoll::Deferred);
 q.set(op(0xEC,152));assert_eq!(q.poll(|x|{assert_eq!(x,op(0xEC,152));Admission::Accepted}),LatestPoll::Applied(op(0xEC,152)));assert_eq!(q.poll(|_|panic!()),LatestPoll::Idle);
}
#[test]fn palette_unsent_show_can_be_closed_and_errors_do_not_wedge(){
 let mut q=LatestOperation::new();q.set(op(0xEC,152));q.poll(|_|Admission::Full);q.set(op(0xEE,152));assert_eq!(q.poll(|_|Admission::Rejected(7)),LatestPoll::Rejected(7));assert!(!q.pending());q.set(op(0xEC,152));assert!(matches!(q.poll(|_|Admission::Accepted),LatestPoll::Applied(_)));
}
#[test]fn desktop_state_keeps_create_before_chrome_and_retries_each_phase(){
 let mut q=DesktopStates::new();q.observe(op(0xEC,200),Admission::Full);q.observe(op(0xFD,200),Admission::Accepted);
 let mut sent=Vec::new();q.poll(4,Some,|x|{sent.push(x.opcode);Admission::Accepted});assert_eq!(sent,vec![0xEC]);assert!(q.pending_surface(200));
 q.poll(4,Some,|x|{assert_eq!(x.opcode,0xFD);Admission::Full});assert!(q.pending());
 q.poll(4,Some,|x|{sent.push(x.opcode);Admission::Accepted});assert_eq!(sent,vec![0xEC,0xFD]);assert!(!q.pending());
}
#[test]fn desktop_latest_hide_replaces_create_and_reopen_replaces_hide(){
 let mut q=DesktopStates::new();q.observe(op(0xEC,201),Admission::Full);q.observe(op(0xEE,201),Admission::Full);
 q.poll(4,Some,|x|{assert_eq!(x.opcode,0xEE);Admission::Full});q.observe(op(0xEC,201),Admission::Accepted);
 q.poll(4,Some,|x|{assert_eq!(x.opcode,0xFD);Admission::Accepted});assert!(!q.pending());
}
#[test]fn dead_or_minimized_create_is_discarded_while_hide_is_delivered(){
 let mut q=DesktopStates::new();q.observe(op(0xEC,201),Admission::Full);q.observe(op(0xEE,300),Admission::Full);
 let mut sent=Vec::new();q.poll(4,|x|if x.opcode==0xEE{Some(x)}else{None},|x|{sent.push(x);Admission::Accepted});assert_eq!(sent,vec![op(0xEE,300)]);assert!(!q.pending());
}
#[test]fn desktop_only_tracks_four_boot_surfaces_and_preserves_bounded_fairness(){
 let mut q=DesktopStates::new();for sid in[200,201,153,300,999,152]{q.observe(op(0xEC,sid),Admission::Full);}
 let mut sent=Vec::new();for _ in 0..4{q.poll(1,Some,|x|{sent.push(x.surface);Admission::Full});}assert_eq!(sent,vec![200,201,153,300]);
}
#[test]fn restored_text_cancels_partial_payload_before_current_snapshot_repaint(){
 let mut q=Outbox::new(200);q.queue(b"old partial",1,metadata(1));q.poll(2,|_|Admission::Accepted);q.queue(b"hidden queued",2,metadata(2));
 q.cancel();q.queue(b"current published selection",3,metadata(3));let(ops,receipts)=flush(&mut q);assert_eq!(decode(&ops),vec![b"current published selection".to_vec()]);assert_eq!(receipts[0].metadata.selected,3);
}
#[test]fn boot_chrome_payload_survives_failed_create_even_if_original_fd_was_accepted(){
 let mut q=DesktopStates::new();q.observe(op(0xEC,201),Admission::Full);let chrome=Operation{opcode:0xFD,surface:201,packed:1,control:0x2300};q.observe(chrome,Admission::Accepted);
 q.poll(4,Some,|_|Admission::Accepted);q.poll(4,Some,|x|{assert_eq!(x,chrome);Admission::Accepted});assert!(!q.pending());
}
