// SPDX-License-Identifier: GPL-3.0-only
//! Current-generation presentation joins against actual Browser outbox and
//! Display Stage. Host transport callbacks establish no native pixel proof.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../apps/kaleidoscope/src/asset_view.rs"]mod asset_view;
#[path="../apps/kaleidoscope/src/bitmap_delivery.rs"]mod bitmap_delivery;
#[path="../apps/kaleidoscope/src/asset_presentation.rs"]mod asset_presentation;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod transaction;
use asset_view::Mode;
use asset_presentation::Presentation;
use bitmap_batch::{hash,Geometry,Request};
use bitmap_delivery::{Admission,Metadata,Operation,Outbox,Poll,Receipt};
use transaction::Stage;
fn geometry()->Geometry{Geometry{x:8,y:80,width:2,height:1,scale:1}}
fn metadata(viewport:u64,ticket:u32)->Metadata{Metadata{viewport,source:ticket as u64}}
fn wire(op:Operation)->Request{assert_eq!(op.surface,300);Request::decode(op.opcode,op.packed,op.control).unwrap()}
fn submit(stage:&mut Stage,out:&mut Outbox,pixels:&[u32])->Vec<Request>{
 let mut sent=Vec::new();
 for _ in 0..16{
  let before=sent.len();assert_eq!(out.poll(pixels,4,|op|{let request=wire(op);sent.push(request);stage.receive(15,300,request);Admission::Accepted}),Poll::Pending);
  assert!(sent.len()-before<=4);if before==sent.len(){return sent}
 }
 panic!("bounded bitmap did not await its real receipt")
}
fn receipt(stage:&mut Stage,out:&mut Outbox)->Receipt{
 let mut got=None;stage.poll_reply(|owner,word|{assert_eq!(owner,15);got=out.receive(1,word);0});got.expect("actual exact outbox Applied")
}
fn queue(p:&mut Presentation,out:&mut Outbox,pixels:&[u32],viewport:u64,ticket:u32)->u32{
 let sequence=out.queue(pixels,geometry(),metadata(viewport,ticket)).unwrap();assert!(p.expect(geometry(),viewport,ticket,sequence,hash(pixels)));sequence
}
#[test]fn exact_current_text_then_bitmap_receipt_marks_ready_once_and_retains_layout(){
 let mut p=Presentation::new();assert_eq!((p.mode(),p.generation()),(Mode::Fit,1));assert!(!p.can_queue(4));
 for(gen,vp,current)in[(0,4,4),(2,4,4),(1,3,4),(1,0,0)]{assert!(!p.text_applied(gen,vp,current));}
 assert!(p.text_applied(1,4,4));assert!(!p.text_applied(1,4,4));assert!(p.can_queue(4));assert!(!p.can_queue(5));
 let pixels=[0xff123456,0x80010203];let mut out=Outbox::new();let mut stage=Stage::EMPTY;queue(&mut p,&mut out,&pixels,4,7);
 assert!(p.has_layout());assert!(!p.can_queue(4));assert!(!p.ready());let saved=p.layout().unwrap();
 let requests=submit(&mut stage,&mut out,&pixels);assert_eq!(requests.iter().filter(|r|matches!(r,Request::Commit{..})).count(),1);
 assert_eq!(stage.sample(15,300,8,80),Some(pixels[0]));let r=receipt(&mut stage,&mut out);
 assert!(p.applied(&r,4,7,hash(&pixels)));assert!(p.ready());assert!(!p.applied(&r,4,7,hash(&pixels)));assert_eq!(p.layout(),Some(saved));
 assert_eq!(p.select(Mode::Fit),Ok(false));assert_eq!(p.generation(),1);assert!(p.ready());assert!(!p.can_queue(4));
}
#[test]fn stale_actual_fit_same_geometry_ack_cannot_be_rebound_to_new_generation(){
 let pixels=[1,2];let mut p=Presentation::new();let mut out=Outbox::new();let mut stage=Stage::EMPTY;p.text_applied(1,1,1);
 let old=queue(&mut p,&mut out,&pixels,1,7);let initial=submit(&mut stage,&mut out,&pixels);
 assert_eq!(p.select(Mode::Actual),Ok(true));assert_eq!(p.select(Mode::Fit),Ok(true));assert_eq!(p.generation(),3);
 assert!(!p.text_applied(2,1,1));assert!(p.text_applied(3,1,1));
 // Actual/Fit may have identical geometry. The old accepted COMMIT stays in
 // Outbox; queue coalescing cannot turn that sequence into current readiness.
 assert_eq!(out.queue(&pixels,geometry(),metadata(1,7)),Ok(old));assert!(!p.expect(geometry(),1,7,old,hash(&pixels)));assert!(!p.has_layout());
 for _ in 0..4{assert_eq!(out.poll(&pixels,4,|_|panic!("accepted COMMIT replay")),Poll::Pending);}
 let historical=receipt(&mut stage,&mut out);assert!(!p.applied(&historical,1,7,hash(&pixels)));assert!(!p.ready());assert!(!out.pending());
 let new=queue(&mut p,&mut out,&pixels,1,7);assert!(new>old);let fresh=submit(&mut stage,&mut out,&pixels);
 assert_eq!(initial.iter().filter(|r|matches!(r,Request::Commit{sequence,..}if *sequence==old)).count(),1);
 assert_eq!(fresh.iter().filter(|r|matches!(r,Request::Commit{sequence,..}if *sequence==new)).count(),1);
 assert!(!p.applied(&historical,1,7,hash(&pixels)));let current=receipt(&mut stage,&mut out);assert!(p.applied(&current,1,7,hash(&pixels)));
}
#[test]fn every_wrong_receipt_or_current_source_join_is_nonmutating_until_exact(){
 let pixels=[1,2];let mut p=Presentation::new();let mut out=Outbox::new();let mut stage=Stage::EMPTY;p.text_applied(1,2,2);queue(&mut p,&mut out,&pixels,2,8);submit(&mut stage,&mut out,&pixels);let r=receipt(&mut stage,&mut out);let layout=p.layout();
 for cause in 0..8{let mut bad=r;match cause{0=>bad.sequence+=1,1=>bad.geometry=None,2=>bad.geometry.as_mut().unwrap().x+=1,3=>bad.hash^=1,4=>bad.metadata.viewport+=1,5=>bad.metadata.source+=1,6=>bad.metadata.source|=1u64<<32,_=>bad.geometry.as_mut().unwrap().scale+=1};assert!(!p.applied(&bad,2,8,r.hash));assert_eq!(p.layout(),layout);assert!(!p.ready());}
 for(vp,ticket,digest)in[(3,8,r.hash),(2,9,r.hash),(2,8,r.hash^1)]{assert!(!p.applied(&r,vp,ticket,digest));assert!(!p.ready());}
 assert!(p.applied(&r,2,8,r.hash));p.invalidate().unwrap();assert!(!p.has_layout());assert!(!p.ready());assert!(!p.applied(&r,2,8,r.hash));assert!(!p.text_applied(1,2,2));assert!(p.text_applied(2,3,3));assert!(p.can_queue(3));
}
#[test]fn full_before_each_effect_retains_exact_request_and_display_ack_full_never_replays_commit(){
 let pixels=[3,4];let mut p=Presentation::new();let mut out=Outbox::new();let mut stage=Stage::EMPTY;p.text_applied(1,1,1);queue(&mut p,&mut out,&pixels,1,7);
 let mut blocked=None;let mut sent=Vec::new();
 for _ in 0..12{let result=out.poll(&pixels,4,|op|{
  if blocked.is_none(){blocked=Some(op);return Admission::Full}
  assert_eq!(Some(op),blocked);blocked=None;let r=wire(op);sent.push(r);stage.receive(15,300,r);Admission::Accepted
 });assert!(matches!(result,Poll::Deferred|Poll::Pending));if sent.iter().any(|r|matches!(r,Request::Commit{..})){break}}
 assert_eq!(sent.len(),3);assert_eq!(sent.iter().filter(|r|matches!(r,Request::Commit{..})).count(),1);
 let mut held=None;for _ in 0..3{stage.poll_reply(|_,word|{if let Some(previous)=held{assert_eq!(word,previous)}held=Some(word);9});assert_eq!(out.poll(&pixels,4,|_|panic!("accepted mutation replay")),Poll::Pending);assert!(!p.ready());}
 let r=receipt(&mut stage,&mut out);assert!(p.applied(&r,1,7,r.hash));
}
#[test]fn no_fit_clear_waits_behind_historical_commit_and_full_then_completes_once(){
 let pixels=[3,4];let mut p=Presentation::new();let mut out=Outbox::new();let mut stage=Stage::EMPTY;p.text_applied(1,1,1);queue(&mut p,&mut out,&pixels,1,7);submit(&mut stage,&mut out,&pixels);
 p.invalidate().unwrap();assert!(!p.has_layout());assert!(!p.expect_clear(2,2));assert!(p.text_applied(2,2,2));let clear=out.clear(metadata(2,0)).unwrap();assert!(p.expect_clear(2,clear));assert!(p.has_layout());assert!(p.layout().is_none());assert!(!p.can_queue(2));
 assert_eq!(out.poll(&[],4,|_|panic!("old accepted COMMIT must drain")),Poll::Pending);let old=receipt(&mut stage,&mut out);assert!(!p.applied(&old,2,7,old.hash));assert!(!p.cleared(&old,2));
 let mut blocked=None;assert_eq!(out.poll(&[],4,|op|{blocked=Some(op);Admission::Full}),Poll::Deferred);assert_eq!(stage.sample(15,300,8,80),Some(pixels[0]));
 assert_eq!(out.poll(&[],4,|op|{assert_eq!(Some(op),blocked);assert_eq!(wire(op),Request::Clear{sequence:clear});stage.receive(15,300,wire(op));Admission::Accepted}),Poll::Pending);assert_eq!(stage.sample(15,300,8,80),None);
 let r=receipt(&mut stage,&mut out);assert!(p.cleared(&r,2));assert!(!p.cleared(&r,2));assert!(!p.applied(&r,2,0,0));assert!(!p.ready());assert!(!p.can_queue(2));
 assert_eq!(out.poll(&[],4,|_|panic!("completed nofit Clear replay")),Poll::Idle);
 assert_eq!(p.select(Mode::Actual),Ok(true));assert!(!p.can_queue(2));assert!(!p.text_applied(2,2,2));assert!(p.text_applied(3,2,2));assert!(p.can_queue(2));assert!(!p.cleared(&r,2));
}
#[test]fn invalid_expect_and_clear_receipts_leave_existing_text_and_layout_unchanged(){
 let mut p=Presentation::new();assert!(p.text_applied(1,4,4));let mut invalid=geometry();invalid.scale=0;
 assert!(!p.expect(invalid,4,7,1,0));assert!(!p.expect(geometry(),3,7,1,0));assert!(!p.expect(geometry(),4,0,1,0));assert!(!p.expect(geometry(),4,7,0,0));assert!(p.can_queue(4));
 assert!(p.expect_clear(4,1));assert!(!p.expect(geometry(),4,7,2,0));assert!(!p.expect_clear(4,2));
 let r=Receipt{sequence:1,geometry:None,hash:0,metadata:metadata(4,0)};
 for cause in 0..5{let mut bad=r;match cause{0=>bad.sequence=2,1=>bad.geometry=Some(geometry()),2=>bad.hash=1,3=>bad.metadata.source=1,_=>bad.metadata.viewport=5};assert!(!p.cleared(&bad,4));assert!(!p.ready());assert!(p.has_layout());}
 assert!(!p.cleared(&r,5));assert!(p.cleared(&r,4));p.invalidate().unwrap();assert!(!p.cleared(&r,4));assert!(!p.has_layout());
}
#[test]fn viewport_change_during_accepted_clear_ignores_historical_ack_then_queues_fresh_image(){
 let pixels=[3,4];let mut p=Presentation::new();let mut out=Outbox::new();let mut stage=Stage::EMPTY;p.text_applied(1,1,1);queue(&mut p,&mut out,&pixels,1,7);submit(&mut stage,&mut out,&pixels);let image=receipt(&mut stage,&mut out);assert!(p.applied(&image,1,7,image.hash));
 p.invalidate().unwrap();p.text_applied(2,2,2);let clear=out.clear(metadata(2,0)).unwrap();assert!(p.expect_clear(2,clear));
 assert_eq!(out.poll(&[],4,|op|{stage.receive(15,300,wire(op));Admission::Accepted}),Poll::Pending);
 p.invalidate().unwrap();assert!(p.text_applied(3,3,3));assert!(p.can_queue(3));assert_eq!(out.poll(&[],4,|_|panic!("accepted Clear replay")),Poll::Pending);
 let historical=receipt(&mut stage,&mut out);assert!(!p.cleared(&historical,3));assert!(!p.applied(&historical,3,7,hash(&pixels)));assert!(!p.ready());
 let sequence=queue(&mut p,&mut out,&pixels,3,8);assert!(sequence>clear);submit(&mut stage,&mut out,&pixels);let current=receipt(&mut stage,&mut out);assert!(p.applied(&current,3,8,current.hash));assert!(p.ready());assert!(!p.cleared(&historical,3));
}
