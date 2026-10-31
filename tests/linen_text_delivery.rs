// SPDX-License-Identifier: GPL-3.0-only
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../servers/sexdisplay/src/text_transaction.rs"]mod display_transaction;
#[path="../servers/silk-shell/src/linen_text_delivery.rs"] mod production;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[allow(unused_attributes)]
#[path="../crates/sex-net-config/src/lib.rs"]pub mod network_config;
#[path="../apps/kaleidoscope/src/document_reader.rs"]mod document_reader;
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
#[test]fn desktop_only_tracks_fixed_boot_surfaces_and_preserves_bounded_fairness(){
 let mut q=DesktopStates::new();for sid in DESKTOP_SURFACES.into_iter().chain([999,152]){q.observe(op(0xEC,sid),Admission::Full);}
 let mut sent=Vec::new();for _ in 0..DESKTOP_SURFACES.len(){q.poll(1,Some,|x|{sent.push(x.surface);Admission::Full});}assert_eq!(sent,DESKTOP_SURFACES);
}
#[test]fn restored_text_cancels_partial_payload_before_current_snapshot_repaint(){
 let mut q=Outbox::new(200);q.queue(b"old partial",1,metadata(1));q.poll(2,|_|Admission::Accepted);q.queue(b"hidden queued",2,metadata(2));
 q.cancel();q.queue(b"current published selection",3,metadata(3));let(ops,receipts)=flush(&mut q);assert_eq!(decode(&ops),vec![b"current published selection".to_vec()]);assert_eq!(receipts[0].metadata.selected,3);
}
#[test]fn boot_chrome_payload_survives_failed_create_even_if_original_fd_was_accepted(){
 let mut q=DesktopStates::new();q.observe(op(0xEC,201),Admission::Full);let chrome=Operation{opcode:0xFD,surface:201,packed:1,control:0x2300};q.observe(chrome,Admission::Accepted);
 q.poll(4,Some,|_|Admission::Accepted);q.poll(4,Some,|x|{assert_eq!(x,chrome);Admission::Accepted});assert!(!q.pending());
}

#[test]fn focus_full_preserves_exact_current_operation_until_admitted(){
 let mut f=FocusDelivery::new();f.observe(153,Admission::Full);
 for _ in 0..3 {assert_eq!(f.poll(153,|_|true,|_|true,|op|{assert_eq!(op,Operation{opcode:0xED,surface:153,packed:0,control:0});Admission::Full}),LatestPoll::Deferred);}
 assert!(matches!(f.poll(153,|_|true,|_|true,|_|Admission::Accepted),LatestPoll::Applied(_)));
 assert_eq!(f.poll(153,|_|panic!(),|_|panic!(),|_|panic!()),LatestPoll::Idle);
}
#[test]fn superseded_dead_and_permanently_denied_focus_never_replay(){
 for current in [153,306] {let mut f=FocusDelivery::new();f.observe(153,Admission::Full);assert_eq!(f.poll(current,|_|false,|_|panic!(),|_|panic!()),LatestPoll::Idle);}
 let mut f=FocusDelivery::new();f.observe(153,Admission::Full);f.observe(306,Admission::Accepted);assert_eq!(f.poll(306,|_|panic!(),|_|panic!(),|_|panic!()),LatestPoll::Idle);
 f.observe(153,Admission::Full);assert_eq!(f.poll(153,|_|true,|_|true,|_|Admission::Rejected(3)),LatestPoll::Rejected(3));assert_eq!(f.poll(153,|_|panic!(),|_|panic!(),|_|panic!()),LatestPoll::Idle);
}
#[test]fn focus_waits_for_geometry_but_clear_is_a_real_operation(){
 let mut f=FocusDelivery::new();f.observe(153,Admission::Full);assert_eq!(f.poll(153,|_|true,|_|false,|_|panic!()),LatestPoll::Deferred);
 f.observe(0,Admission::Full);assert!(matches!(f.poll(0,|_|panic!(),|_|true,|op|{assert_eq!((op.opcode,op.surface),(0xED,0));Admission::Accepted}),LatestPoll::Applied(_)));
}
#[test]fn atomic_text_is_visible_and_metadata_published_only_after_exact_display_receipt(){
 let mut o=Outbox::atomic(200);let mut stage=display_transaction::Stage::EMPTY;assert!(o.queue(b"listed\nobjects",0x123456,metadata(7)));
 let mut visible=b"previous list".to_vec();let mut calls=0;
 while calls<5 {let result=o.poll(1,|op|{calls+=1;let r=text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap();
  if let Some((n,_,bytes))=stage.receive(3,op.surface,r){visible=bytes[..n].to_vec();}else{assert_eq!(visible,b"previous list");}Admission::Accepted});assert_eq!(result,Poll::Pending);
  if visible==b"listed\nobjects"{break;}
 }
 assert_eq!(visible,b"listed\nobjects");assert_eq!(o.poll(4,|_|panic!("awaits receipt")),Poll::Pending);
 let mut value=0;stage.poll_reply(|owner,v|{assert_eq!(owner,3);value=v;ERR_SERVICE_NOT_READY});
 assert_eq!(o.receive(11,value),None);assert_eq!(o.receive(1,value|1<<56),None);
 assert_eq!(o.receive(1,value),Some(Receipt{metadata:metadata(7),bytes:14}));assert!(!o.pending());
}
#[test]fn atomic_full_cancel_recreate_and_ack_queue_gap_never_publish_stale_list(){
 let mut o=Outbox::atomic(200);o.queue(b"old",1,metadata(1));let mut op=None;
 assert_eq!(o.poll(4,|v|{op=Some(v);Admission::Full}),Poll::Deferred);
 assert_eq!(o.poll(1,|v|{assert_eq!(Some(v),op);Admission::Accepted}),Poll::Pending);
 o.cancel();let mut stage=display_transaction::Stage::EMPTY;stage.reset();o.queue(b"current",1,metadata(2));
 let mut done=0;
 for _ in 0..4{o.poll(4,|v|{if stage.receive(3,v.surface,text_batch::Request::decode(v.opcode,v.packed,v.control).unwrap()).is_some(){done+=1;}Admission::Accepted});}
 assert_eq!(done,1);o.queue(b"superseded",1,metadata(3));stage.poll_reply(|_,v|{assert_eq!(o.receive(1,v).unwrap().metadata.selected,2);0});
 o.queue(b"latest",1,metadata(4));let mut applied=None;
 for _ in 0..4{o.poll(4,|v|{if let Some((_,_,_))=stage.receive(3,v.surface,text_batch::Request::decode(v.opcode,v.packed,v.control).unwrap()){applied=Some(v);}Admission::Accepted});}
 stage.poll_reply(|_,v|{assert_eq!(o.receive(1,v).unwrap().metadata.selected,4);0});assert!(applied.is_some());assert_eq!(o.poll(4,|_|panic!()),Poll::Idle);
}

#[test]fn selected_atomic_full_at_every_boundary_retains_bytes_range_sequence_and_receipt(){
 let text=[b'x';512];
 for full_at in 0..67 {
  let mut o=Outbox::atomic(300);let mut stage=display_transaction::Stage::EMPTY;
  assert!(o.queue_selected(&text,0x123456,metadata(31),(17,509)));
  let mut accepted=Vec::new();let mut deferred=None;let mut commits=0;
  for _ in 0..100 {let _=o.poll(4,|op|{
   if accepted.len()==full_at && deferred.is_none(){deferred=Some(op);return Admission::Full;}
   if accepted.len()==full_at{assert_eq!(Some(op),deferred);}
   let req=text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap();
   if let Some((n,_,bytes))=stage.receive(15,300,req){assert_eq!(n,512);assert_eq!(&bytes[..n],&text);commits+=1;}
   else{assert_eq!(stage.selection(),(0,0));}
   accepted.push(op);Admission::Accepted
  });if accepted.len()==67{break;}}
  assert!(deferred.is_some());assert_eq!(commits,1);assert_eq!(stage.selection(),(17,509));
  assert_eq!(accepted[65].opcode,text_batch::SELECT);assert_eq!(accepted[66].opcode,text_batch::COMMIT);
  assert_eq!(o.poll(4,|_|panic!("await receipt")),Poll::Pending);
  let mut word=0;stage.poll_reply(|_,v|{word=v;1});assert_eq!(o.receive(1,word),Some(Receipt{metadata:metadata(31),bytes:512}));
  assert_eq!(o.receive(1,word),None);assert_eq!(o.poll(4,|_|panic!()),Poll::Idle);
 }
}
#[test]fn legacy_queue_clears_old_atomic_selection_and_styled_legacy_or_invalid_ranges_are_atomic(){
 let mut o=Outbox::atomic(300);let mut stage=display_transaction::Stage::EMPTY;
 assert!(o.queue_selected(b"words",1,metadata(1),(1,4)));
 for _ in 0..5{o.poll(4,|op|{stage.receive(15,300,text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap());Admission::Accepted});}
 assert_eq!(stage.selection(),(1,4));stage.poll_reply(|_,v|{assert_eq!(o.receive(1,v).unwrap().metadata,metadata(1));0});
 assert!(o.queue(b"next",1,metadata(2)));
 for range in[(4,2),(1,1),(0,5)]{assert!(!o.queue_selected(b"next",2,metadata(3),range));}
 let mut seq=0;let mut saw_select=false;
 for _ in 0..5{o.poll(4,|op|{saw_select|=op.opcode==text_batch::SELECT;let req=text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap();if let text_batch::Request::Begin{sequence,..}=req{seq=sequence;}stage.receive(15,300,req);Admission::Accepted});}
 assert_eq!(seq,2);assert!(!saw_select);assert_eq!(stage.selection(),(0,0));
 stage.poll_reply(|_,v|{assert_eq!(o.receive(1,v).unwrap().metadata,metadata(2));0});
 let mut legacy=Outbox::new(200);assert!(!legacy.queue_selected(b"words",1,metadata(1),(0,3)));assert!(!legacy.pending());
}
#[test]fn queued_selection_never_changes_blocked_active_style_and_latest_style_coalesces(){
 let mut o=Outbox::atomic(300);let mut stage=display_transaction::Stage::EMPTY;
 assert!(o.queue_selected(b"same words",1,metadata(1),(0,4)));
 let mut blocked=None;assert_eq!(o.poll(8,|op|{if op.opcode==text_batch::SELECT{blocked=Some(op);Admission::Full}else{stage.receive(15,300,text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap());Admission::Accepted}}),Poll::Deferred);
 o.queue_selected(b"same words",1,metadata(2),(1,5));o.queue_selected(b"same words",1,metadata(3),(5,10));
 let mut first=true;
 o.poll(4,|op|{if first{assert_eq!(Some(op),blocked);first=false;}stage.receive(15,300,text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap());Admission::Accepted});
 assert_eq!(stage.selection(),(0,4));stage.poll_reply(|_,v|{assert_eq!(o.receive(1,v).unwrap().metadata,metadata(1));0});
 for _ in 0..4{o.poll(4,|op|{stage.receive(15,300,text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap());Admission::Accepted});}
 assert_eq!(stage.selection(),(5,10));stage.poll_reply(|_,v|{assert_eq!(o.receive(1,v).unwrap().metadata,metadata(3));0});assert!(!o.pending());
}

// Actual Reader -> shared atomic outbox -> Display Stage -> Applied chain.
// Admission callbacks model a full/closed transport, not native IPC authority.
#[test]fn utf8_reader_frame_crosses_every_full_boundary_and_only_exact_applied_publishes_metadata(){
 let body="1234567é€😀\tZ\nsecond é row".as_bytes();let original=body.to_vec();
 for(w,h)in[(1280,750),(1024,718)]{
  let mut reader=document_reader::Reader::new();reader.resize(w,h);
  let frame=reader.render_path_selected(body,b"/utf8",Some((7,16)));
  let bytes=&frame.bytes[..frame.len];let(start,end)=frame.selection;
  assert!(text_encoding::valid_text(bytes));assert_eq!(&bytes[start as usize..end as usize],"é€😀".as_bytes());
  assert_eq!(start as usize%8,7,"first UTF8 scalar must cross register boundary");
  let operations=1+(frame.len+7)/8+1+1;
  for full_at in 0..operations{
   let mut out=Outbox::atomic(300);let mut stage=display_transaction::Stage::EMPTY;
   assert!(out.queue_selected(bytes,0xcdd6f4,metadata(258),frame.selection));
   let mut accepted=Vec::new();let mut blocked=None;let mut commits=0;
   let mut visible=b"previous overview".to_vec();
   for _ in 0..operations+2{
    let result=out.poll(4,|operation|{
     if accepted.len()==full_at&&blocked.is_none(){blocked=Some(operation);return Admission::Full;}
     if accepted.len()==full_at{assert_eq!(Some(operation),blocked);}
     let request=text_batch::Request::decode(operation.opcode,operation.packed,operation.control).unwrap();
     if let Some((len,color,committed))=stage.receive(15,operation.surface,request){
      assert_eq!(color,0xffcdd6f4);visible=committed[..len].to_vec();commits+=1;
     }else{assert_eq!(visible,b"previous overview");}
     accepted.push(operation);Admission::Accepted
    });
    assert!(!matches!(result,Poll::Complete(_)),"admission is not application");
    if accepted.len()==operations{break;}
   }
   assert!(blocked.is_some());assert_eq!(commits,1);assert_eq!(&visible,bytes);
   assert_eq!(stage.selection(),frame.selection);
   assert_eq!(accepted[operations-2].opcode,text_batch::SELECT);
   assert_eq!(accepted[operations-1].opcode,text_batch::COMMIT);
   assert_eq!(out.poll(4,|_|panic!("wait for Applied")),Poll::Pending);
   let mut receipt=0;stage.poll_reply(|owner,value|{assert_eq!(owner,15);receipt=value;ERR_SERVICE_NOT_READY});
   assert_eq!(out.receive(3,receipt),None);assert_eq!(out.receive(1,receipt^(1<<24)),None);
   let done=out.receive(1,receipt).unwrap();assert_eq!(done,Receipt{metadata:metadata(258),bytes:frame.len});
   assert_eq!(out.receive(1,receipt),None);assert_eq!(out.poll(4,|_|panic!()),Poll::Idle);
  }
 }
 assert_eq!(body,&original);
}
#[test]fn invalid_utf8_or_mid_scalar_selection_cannot_consume_sequence_or_replace_latest_valid_batch(){
 let mut out=Outbox::atomic(300);let mut stage=display_transaction::Stage::EMPTY;
 assert!(out.queue_selected("active é".as_bytes(),1,metadata(1),(7,9)));
 let mut blocked=None;
 assert_eq!(out.poll(1,|op|{blocked=Some(op);Admission::Full}),Poll::Deferred);
 assert!(out.queue_selected("latest €".as_bytes(),2,metadata(2),(7,10)));
 for bytes in[&b"\xff"[..],&b"\xe2\x82"[..],&b"\0"[..],&b"\xc2\x85"[..]]{
  assert!(!out.queue(bytes,3,metadata(99)));
 }
 for range in[(1,2),(0,1),(1,3)]{assert!(!out.queue_selected("éZ".as_bytes(),3,metadata(99),range));}
 let mut seen=Vec::new();let mut texts=Vec::new();
 for expected in[1,2]{
  for _ in 0..10{out.poll(4,|operation|{
   if seen.is_empty(){assert_eq!(Some(operation),blocked);}
   let request=text_batch::Request::decode(operation.opcode,operation.packed,operation.control).unwrap();
   if let text_batch::Request::Begin{sequence,..}=request{assert_eq!(sequence,expected);}
   if let Some((len,_,bytes))=stage.receive(15,300,request){texts.push(bytes[..len].to_vec());}
   seen.push(operation);Admission::Accepted
  });}
  stage.poll_reply(|_,value|{assert_eq!(out.receive(1,value).unwrap().metadata,metadata(expected as u64));0});
 }
 assert_eq!(texts,vec!["active é".as_bytes().to_vec(),"latest €".as_bytes().to_vec()]);assert!(!out.pending());
 // Opaque non-atomic legacy text retains its existing byte contract.
 let mut legacy=Outbox::new(200);assert!(legacy.queue(b"\xff\0",1,metadata(3)));
 assert_eq!(decode(&flush(&mut legacy).0),vec![b"\xff\0".to_vec()]);
}
#[test]fn utf8_cancel_or_closed_admission_never_acknowledges_old_frame_or_replays_cancelled_latest(){
 for closed in[false,true]{
  let mut reader=document_reader::Reader::new();reader.resize(624,706);let frame=reader.render("é € 😀".as_bytes());
  let mut out=Outbox::atomic(300);let mut stage=display_transaction::Stage::EMPTY;
  assert!(out.queue(&frame.bytes[..frame.len],1,metadata(1)));
  if closed{
   assert_eq!(out.poll(4,|_|Admission::Rejected(77)),Poll::Rejected(77));assert!(!out.pending());
  }else{
   for _ in 0..100{out.poll(4,|op|{stage.receive(15,300,text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap());Admission::Accepted});}
   let mut old=0;stage.poll_reply(|_,value|{old=value;ERR_SERVICE_NOT_READY});
   assert!(out.queue("queued ü".as_bytes(),1,metadata(2)));out.cancel();
   assert_eq!(out.receive(1,old),None);assert!(!out.pending());stage.reset();
  }
  assert_eq!(out.poll(4,|_|panic!("cancelled/closed frame replayed")),Poll::Idle);
 }
}
