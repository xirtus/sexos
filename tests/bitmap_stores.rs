// SPDX-License-Identifier: GPL-3.0-only
//! Actual store collection, Stage, Binding and SDK. Future rows/tasks are host
//! metadata, never actual BOOT grants, app activation or native pixels.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[cfg(not(bitmap_stores_catalog))]
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[cfg(bitmap_stores_catalog)]
pub mod app_catalog{include!(env!("BITMAP_STORES_CATALOG"));}
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"]mod bitmap_binding;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod bitmap_transaction;
#[path="../servers/sexdisplay/src/bitmap_stores.rs"]mod bitmap_stores;
use bitmap_batch::{Request,Geometry,Applied,hash};
use bitmap_binding::Binding;
use bitmap_stores::Stores;
const COUNT:usize=app_catalog::count(app_catalog::App::Browser);
type Set=Stores<COUNT>;
fn task(target:u32)->u32{match target{15=>150,30=>300,31=>310,_=>panic!("unknown host task")}}
fn bindings()->Vec<Binding>{
 app_catalog::INCARNATIONS.iter().filter(|r|r.app==app_catalog::App::Browser).map(|row|
  Binding::from_catalog(row.lifecycle_slot,app_lifecycle::CatalogInfo{target_pd:row.target,
   task_id:task(row.target),surface_id:row.surface,coordinator_pd:3}).unwrap()).collect()
}
fn configured()->Set{
 let mut stores=Set::UNBOUND;
 // Deliberately install out of catalog order; receipt traversal stays canonical.
 for b in bindings().into_iter().rev(){assert!(stores.initialize_once(b));}
 stores
}
fn g()->Geometry{Geometry{x:10,y:40,width:2,height:1,scale:2}}
fn packet(stores:&mut Set,b:Binding,request:Request)->Option<bitmap_transaction::Change>{
 let(op,a,c)=request.wire();stores.receive(b.target(),b.task(),b.surface(),Request::decode(op,a,c).unwrap())
}
fn chunks(stores:&mut Set,b:Binding,sequence:u32,pixels:&[u32]){
 for(index,values)in pixels.chunks(2).enumerate(){let mut pair=[0;2];pair[..values.len()].copy_from_slice(values);
  assert_eq!(packet(stores,b,Request::Chunk{sequence,offset:index*2,count:values.len(),pixels:pair}),None);
 }
}
fn publish(stores:&mut Set,b:Binding,sequence:u32,geometry:Geometry,pixels:&[u32]){
 assert_eq!(packet(stores,b,Request::Begin{sequence,geometry}),None);chunks(stores,b,sequence,pixels);
 assert_eq!(packet(stores,b,Request::Commit{sequence,hash:hash(pixels)}).unwrap().new,Some(geometry));
}
fn color(b:Binding)->u32{0xff010000|b.target()}
fn value(stores:&Set,b:Binding)->Option<u32>{stores.sample(b.target(),b.surface(),10,40)}
fn requests(sequence:u32)->[Request;4]{[
 Request::Begin{sequence,geometry:g()},Request::Chunk{sequence,offset:0,count:2,pixels:[1,2]},
 Request::Commit{sequence,hash:hash(&[1,2])},Request::Clear{sequence},
]}
fn receipt(owner:u32,word:u64)->Applied{
 let applied=Applied::decode(1,word).unwrap();let b=bindings().into_iter().find(|b|b.target()==owner).unwrap();
 assert_eq!(applied.surface as u64,b.surface());applied
}

#[test]fn constant_static_backing_is_exactly_one_stage_per_canonical_browser(){
 static BACKING:std::sync::Mutex<Set>=std::sync::Mutex::new(Set::UNBOUND);
 let mut stores=BACKING.lock().unwrap();let address=&*stores as*const Set;
 assert_eq!(stores.initialized_count(),0);
 for b in bindings().into_iter().rev(){assert!(stores.initialize_once(b));assert!(!stores.initialize_once(b));}
 assert_eq!(&*stores as*const Set,address);assert_eq!(stores.initialized_count(),COUNT);
 #[cfg(not(bitmap_stores_catalog))]assert_eq!(COUNT,3);
 #[cfg(bitmap_stores_catalog)]assert_eq!(COUNT,3);
 assert_eq!(core::mem::size_of::<Stores<1>>(),core::mem::size_of::<bitmap_transaction::Stage>());
 assert_eq!(core::mem::size_of::<Stores<3>>(),3*core::mem::size_of::<bitmap_transaction::Stage>());
 println!("[display.bitmap.stores.storage] stores1={} stores3={} stage={} current_count={}",
  core::mem::size_of::<Stores<1>>(),core::mem::size_of::<Stores<3>>(),core::mem::size_of::<bitmap_transaction::Stage>(),COUNT);
 for b in bindings(){assert_eq!(stores.binding_for(b.target(),b.surface()),Some(b));assert!(stores.matches_surface(b.target(),b.surface()));assert_eq!(stores.scrubbed(b.target(),b.surface()),Some(true));}
}
#[test]fn absent_store_cannot_be_created_by_any_packet_or_alias(){
 let mut stores=Set::UNBOUND;
 for b in bindings(){for request in requests(7){assert_eq!(packet(&mut stores,b,request),None);}
  assert_eq!(stores.binding_for(b.target(),b.surface()),None);assert_eq!(stores.sequence(b.target(),b.surface()),None);assert_eq!(value(&stores,b),None);
 }
 stores.poll_reply(|_,_|panic!("packet cannot create a store or receipt"));assert_eq!(stores.initialized_count(),0);
 let first=bindings()[0];assert!(stores.initialize_once(first));
 for b in bindings().into_iter().skip(1){for request in requests(7){assert_eq!(packet(&mut stores,b,request),None);}assert_eq!(stores.sequence(b.target(),b.surface()),None);}
 for(owner,sid)in[(0,300),(13,300),(15,0),(15,300|(1<<32)),(u32::MAX,u64::MAX)]{
  assert_eq!(stores.binding_for(owner,sid),None);assert!(!stores.matches_surface(owner,sid));
  assert_eq!(stores.receive(owner,150,sid,Request::Clear{sequence:99}),None);
 }
 assert_eq!(stores.initialized_count(),1);assert_eq!(stores.sequence(first.target(),first.surface()),Some(0));
}
#[test]fn incorrect_collection_capacity_rejects_initialization_without_effect(){
 fn wrong<const N:usize>(){let mut stores=Stores::<N>::UNBOUND;
  for b in bindings(){assert!(!stores.initialize_once(b));assert_eq!(stores.binding_for(b.target(),b.surface()),None);
   assert_eq!(stores.receive(b.target(),b.task(),b.surface(),Request::Clear{sequence:1}),None);}
  assert_eq!(stores.initialized_count(),0);stores.poll_reply(|_,_|panic!("wrong-size collection"));
 }
 wrong::<0>();wrong::<{COUNT+1}>();
}
#[test]fn equal_sequences_private_staging_pixels_and_receipts_remain_per_incarnation(){
 let mut stores=configured();let all=bindings();
 for b in &all{packet(&mut stores,*b,Request::Begin{sequence:7,geometry:g()});chunks(&mut stores,*b,7,&[color(*b),color(*b)+1]);}
 for b in &all{assert_eq!(value(&stores,*b),None);assert_eq!(stores.sequence(b.target(),b.surface()),Some(7));}
 stores.poll_reply(|_,_|panic!("staging has no publication receipt"));
 for b in &all{
  let change=packet(&mut stores,*b,Request::Commit{sequence:7,hash:hash(&[color(*b),color(*b)+1])}).unwrap();assert_eq!(change.old,None);
  assert_eq!(value(&stores,*b),Some(color(*b)));assert_eq!(stores.sample(b.target(),b.surface(),12,40),Some(color(*b)+1));
 }
 assert!(stores.needs_publication());stores.poll_reply(|_,_|panic!("unreleased aggregate receipt"));
 assert_eq!(stores.release_after_publication(),COUNT);assert!(!stores.needs_publication());assert_eq!(stores.release_after_publication(),0);
 let mut seen=Vec::new();stores.poll_reply(|owner,word|{assert_eq!(receipt(owner,word).sequence,7);seen.push(owner);0});
 assert_eq!(seen,all.iter().map(|b|b.target()).collect::<Vec<_>>());
 stores.poll_reply(|_,_|panic!("accepted receipts must not repeat"));
}
#[test]fn identical_payload_hash_and_sequence_still_produce_distinct_bound_receipts(){
 let mut stores=configured();let all=bindings();let pixels=[0xffabcdef,0x80123456];
 for b in &all{publish(&mut stores,*b,11,g(),&pixels);}
 assert_eq!(stores.release_after_publication(),COUNT);
 let mut seen=Vec::new();stores.poll_reply(|owner,word|{assert_eq!(receipt(owner,word).sequence,11);seen.push((owner,word));u64::MAX});
 assert_eq!(seen.len(),COUNT);for(i,left)in seen.iter().enumerate(){for right in seen.iter().skip(i+1){assert_ne!(left.0,right.0);assert_ne!(left.1,right.1);}}
 stores.poll_reply(|_,_|0);let old=all[0];assert!(packet(&mut stores,old,Request::Clear{sequence:12}).is_some());
 assert_eq!(value(&stores,old),None);for b in all.into_iter().skip(1){assert_eq!(value(&stores,b),Some(pixels[0]));assert_eq!(stores.sequence(b.target(),b.surface()),Some(11));}
}
#[test]fn full_owner_keeps_exact_destination_without_starving_other_stores(){
 let mut stores=configured();let all=bindings();for b in &all{publish(&mut stores,*b,7,g(),&[color(*b),color(*b)+1]);}
 assert_eq!(stores.release_after_publication(),COUNT);
 let old=all[0];let mut first=Vec::new();stores.poll_reply(|owner,word|{first.push((owner,word));if owner==old.target(){u64::MAX}else{0}});
 assert_eq!(first.len(),COUNT);assert_eq!(first.iter().map(|v|v.0).collect::<Vec<_>>(),all.iter().map(|b|b.target()).collect::<Vec<_>>());
 for _ in 0..3{let mut seen=Vec::new();stores.poll_reply(|owner,word|{seen.push((owner,word));u64::MAX});assert_eq!(seen,vec![first[0]]);
  packet(&mut stores,old,Request::Begin{sequence:8,geometry:g()});assert_eq!(stores.sequence(old.target(),old.surface()),Some(7));assert_eq!(value(&stores,old),Some(color(old)));
 }
 for b in all.iter().skip(1){assert!(packet(&mut stores,*b,Request::Clear{sequence:8}).is_some());}
 assert_eq!(stores.release_after_publication(),COUNT-1);
 let mut next=Vec::new();stores.poll_reply(|owner,word|{next.push((owner,receipt(owner,word).sequence));if owner==old.target(){u64::MAX}else{0}});
 assert_eq!(next.len(),COUNT);assert_eq!(next[0],(old.target(),7));for item in next.iter().skip(1){assert_eq!(item.1,8);}
 stores.poll_reply(|owner,word|{assert_eq!((owner,word),first[0]);0});
}
#[test]fn wrong_stamped_tasks_and_crossed_subjects_cannot_poison_or_publish_a_stage(){
 let mut stores=configured();let all=bindings();for b in &all{publish(&mut stores,*b,1,g(),&[color(*b),color(*b)+1]);}stores.release_after_publication();stores.poll_reply(|_,_|0);
 for b in &all{packet(&mut stores,*b,Request::Begin{sequence:2,geometry:g()});chunks(&mut stores,*b,2,&[color(*b)+2,color(*b)+3]);
  let malicious=[Request::Begin{sequence:2,geometry:Geometry{width:1,..g()}},Request::Chunk{sequence:2,offset:0,count:2,pixels:[0,0]},Request::Commit{sequence:2,hash:0},Request::Clear{sequence:99}];
  for bad_task in[0,b.target(),b.task()+1,u32::MAX]{for request in malicious{assert_eq!(stores.receive(b.target(),bad_task,b.surface(),request),None);}}
  for request in malicious{assert_eq!(stores.receive(13,b.task(),b.surface(),request),None);assert_eq!(stores.receive(b.target(),b.task(),b.surface()|(1<<32),request),None);}
  assert_eq!(value(&stores,*b),Some(color(*b)));assert_eq!(stores.sequence(b.target(),b.surface()),Some(2));
 }
 for b in all{assert!(packet(&mut stores,b,Request::Commit{sequence:2,hash:hash(&[color(b)+2,color(b)+3])}).is_some());assert_eq!(value(&stores,b),Some(color(b)+2));}
}
#[test]fn retirement_erases_only_exact_old_incarnation_and_cannot_rebind_or_revive(){
 let mut stores=configured();let all=bindings();for b in &all{publish(&mut stores,*b,5,g(),&[color(*b),color(*b)+1]);}
 let old=all[0];stores.retire(old.target(),old.target());assert_eq!(value(&stores,old),Some(color(old)));
 stores.retire(old.target(),old.task());assert_eq!(stores.scrubbed(old.target(),old.surface()),Some(true));assert_eq!(value(&stores,old),None);
 assert_eq!(stores.initialized_count(),COUNT);assert_eq!(stores.binding_for(old.target(),old.surface()),Some(old));
 assert!(!stores.initialize_once(old));let replacement=Binding::from_catalog(old.slot(),app_lifecycle::CatalogInfo{target_pd:old.target(),task_id:old.task()+1,surface_id:old.surface(),coordinator_pd:3}).unwrap();assert!(!stores.initialize_once(replacement));
 stores.reset_surface(old.target(),old.surface());for request in requests(u32::MAX){assert_eq!(packet(&mut stores,old,request),None);}
 assert_eq!(stores.release_after_publication(),COUNT-1);
 let mut seen=Vec::new();stores.poll_reply(|owner,word|{assert_ne!(owner,old.target());assert_eq!(receipt(owner,word).sequence,5);seen.push(owner);0});assert_eq!(seen.len(),COUNT-1);
 for b in all.into_iter().skip(1){assert_eq!(value(&stores,b),Some(color(b)));assert_eq!(stores.scrubbed(b.target(),b.surface()),Some(false));publish(&mut stores,b,6,g(),&[3,4]);assert_eq!(value(&stores,b),Some(3));}
 assert_eq!(stores.scrubbed(old.target(),old.surface()),Some(true));assert_eq!(stores.sequence(old.target(),old.surface()),Some(5));
}
#[test]fn reset_erases_pixels_but_preserves_only_that_stores_fence_and_historical_receipt(){
 let mut stores=configured();let all=bindings();for b in &all{publish(&mut stores,*b,7,g(),&[color(*b),color(*b)+1]);}
 let old=all[0];stores.reset_surface(old.target(),old.surface());
 assert_eq!(stores.sequence(old.target(),old.surface()),Some(7));assert_eq!(value(&stores,old),None);assert_eq!(stores.pixels_scrubbed(old.target(),old.surface()),Some(true));assert_eq!(stores.scrubbed(old.target(),old.surface()),Some(false));
 for b in all.iter().skip(1){assert_eq!(value(&stores,*b),Some(color(*b)));}
 packet(&mut stores,old,Request::Begin{sequence:8,geometry:g()});assert_eq!(stores.sequence(old.target(),old.surface()),Some(7));
 assert!(stores.needs_publication());stores.poll_reply(|_,_|panic!("reset cannot release a historical receipt"));assert_eq!(stores.release_after_publication(),COUNT);
 let mut historical=None;stores.poll_reply(|owner,word|{if owner==old.target(){assert_eq!(receipt(owner,word).sequence,7);historical=Some((owner,word));u64::MAX}else{0}});
 stores.poll_reply(|owner,word|{assert_eq!(Some((owner,word)),historical);0});assert_eq!(stores.scrubbed(old.target(),old.surface()),Some(true));
 assert_eq!(packet(&mut stores,old,Request::Commit{sequence:7,hash:hash(&[color(old),color(old)+1])}),None);assert_eq!(value(&stores,old),None);
 stores.poll_reply(|owner,word|{assert_eq!(Some((owner,word)),historical);0});
 publish(&mut stores,old,8,g(),&[3,4]);assert_eq!(value(&stores,old),Some(3));
}
#[test]fn local_sampling_and_actual_viewport_clipping_never_cross_store_or_geometry(){
 use bitmap_transaction::Viewport;
 let mut stores=configured();let geometry=Geometry{x:0,y:0,width:8,height:8,scale:4};
 for b in bindings(){let pixels:Vec<_>=(0..geometry.len()).map(|i|color(b)+i as u32).collect();publish(&mut stores,b,1,geometry,&pixels);
  for view in[Viewport{x:-7,y:5,width:36,height:40,inset:4,top:12},Viewport{x:25,y:25,width:18,height:20,inset:4,top:12}]{
   let footprint=view.screen(Some(geometry),60,70);
   for y in 0..70{for x in 0..60{let sample=view.local(x,y).and_then(|(lx,ly)|stores.sample(b.target(),b.surface(),lx,ly));
    let inside=x>=footprint.0&&x-footprint.0<footprint.2&&y>=footprint.1&&y-footprint.1<footprint.3;assert_eq!(sample.is_some(),inside);
    if let Some(value)=sample{let lx=x as i64-view.x as i64;let ly=y as i64-view.y as i64;assert_eq!(value,pixels[(ly/4)as usize*8+(lx/4)as usize]);}
   }}
  }
  for(x,y)in[(-1,0),(0,-1),(32,0),(0,32),(i64::MIN,i64::MIN),(i64::MAX,i64::MAX)]{assert_eq!(stores.sample(b.target(),b.surface(),x,y),None);}
 }
}
#[cfg(bitmap_stores_ambiguous)]
#[test]fn ambiguous_catalog_associations_cannot_install_or_select_a_store(){
 let mut stores=Set::UNBOUND;
 for row in app_catalog::INCARNATIONS.iter().filter(|r|r.app==app_catalog::App::Browser&&r.ordinal>0){
  if let Some(b)=Binding::from_catalog(row.lifecycle_slot,app_lifecycle::CatalogInfo{target_pd:row.target,task_id:task(row.target),surface_id:row.surface,coordinator_pd:3}){
   assert!(!stores.initialize_once(b));assert_eq!(stores.binding_for(b.target(),b.surface()),None);
  }
 }
 assert_eq!(stores.initialized_count(),0);
}

use bitmap_stores::PublishedStores;
use bitmap_transaction::{Published,Stage};
#[test]fn snapshot_excludes_pending_bytes_and_preserves_previous_commit(){
 let mut stores=configured();let b=bindings()[0];let mut image=PublishedStores::<COUNT>::EMPTY;
 packet(&mut stores,b,Request::Begin{sequence:1,geometry:g()});chunks(&mut stores,b,1,&[11,12]);
 assert!(stores.snapshot_into(&mut image));assert_eq!(image.sample(b.target(),b.surface(),10,40),None);assert!(image.pixels_scrubbed());
 assert_eq!(image.binding_for(b.target(),b.surface()),Some(b));stores.poll_reply(|_,_|panic!("snapshot manufactured pending reply"));
 packet(&mut stores,b,Request::Commit{sequence:1,hash:hash(&[11,12])}).unwrap();
 assert!(stores.snapshot_into(&mut image));assert_eq!(image.sample(b.target(),b.surface(),10,40),Some(11));
 assert!(stores.needs_publication());stores.poll_reply(|_,_|panic!("snapshot cannot release completion"));assert_eq!(stores.release_after_publication(),1);
 let mut replies=Vec::new();stores.poll_reply(|o,v|{replies.push((o,v));2});stores.poll_reply(|o,v|{assert_eq!(replies[0],(o,v));0});
 packet(&mut stores,b,Request::Begin{sequence:2,geometry:g()});chunks(&mut stores,b,2,&[21,22]);
 let mut pending=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut pending));assert_eq!(pending.sample(b.target(),b.surface(),10,40),Some(11));
 // A poisoned staging transfer still leaves the previous committed scene valid.
 assert_eq!(packet(&mut stores,b,Request::Commit{sequence:2,hash:0}),None);
 assert!(stores.snapshot_into(&mut pending));assert_eq!(pending.sample(b.target(),b.surface(),10,40),Some(11));
 assert_eq!(image.sample(b.target(),b.surface(),10,40),Some(11));
}
#[test]fn snapshot_is_owned_across_later_commit_clear_and_retirement(){
 let mut stores=configured();let b=bindings()[0];publish(&mut stores,b,1,g(),&[11,12]);stores.release_after_publication();stores.poll_reply(|_,_|0);
 let mut old=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut old));
 publish(&mut stores,b,2,g(),&[21,22]);stores.release_after_publication();stores.poll_reply(|_,_|0);assert_eq!(old.sample(b.target(),b.surface(),10,40),Some(11));
 let mut current=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut current));assert_eq!(current.sample(b.target(),b.surface(),10,40),Some(21));
 packet(&mut stores,b,Request::Clear{sequence:3}).unwrap();assert!(stores.snapshot_into(&mut current));assert_eq!(current.sample(b.target(),b.surface(),10,40),None);assert!(current.pixels_scrubbed());assert_eq!(current.binding_for(b.target(),b.surface()),Some(b));
 stores.retire(b.target(),b.task());assert!(stores.snapshot_into(&mut current));assert!(current.matches_surface(b.target(),b.surface()));assert_eq!(current.sample(b.target(),b.surface(),10,40),None);
 assert_eq!(old.sample(b.target(),b.surface(),10,40),Some(11));old.scrub();assert!(old.pixels_scrubbed());assert!(!old.matches_surface(b.target(),b.surface()));
}
#[test]fn captured_clients_preserve_independent_binding_task_geometry_and_clipping(){
 println!("[snapshot.size] published={} stores={}",core::mem::size_of::<Published>(),core::mem::size_of::<PublishedStores<COUNT>>());
 let mut stores=configured();let all=bindings();assert_eq!(all.iter().map(|b|(b.target(),b.task(),b.surface())).collect::<Vec<_>>(),vec![(15,150,300),(30,300,343),(31,310,344)]);
 for b in &all{publish(&mut stores,*b,1,g(),&[color(*b),color(*b)+1]);}
 let mut image=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut image));
 for b in &all{assert_eq!(image.binding_for(b.target(),b.surface()),Some(*b));assert!(!image.binding_for(b.target(),b.surface()).unwrap().admits(b.target(),b.target(),b.surface()));
  for y in [i64::MIN,0,39,40,41,42,i64::MAX]{for x in [i64::MIN,0,9,10,11,12,13,14,i64::MAX]{assert_eq!(image.sample(b.target(),b.surface(),x,y),stores.sample(b.target(),b.surface(),x,y));}}
  for other in &all{if other.target()!=b.target(){assert_eq!(image.sample(other.target(),b.surface(),10,40),None);}}
 }
 stores.retire(all[0].target(),all[0].task());let mut newer=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut newer));
 assert_eq!(newer.sample(all[0].target(),all[0].surface(),10,40),None);for b in &all[1..]{assert_eq!(newer.sample(b.target(),b.surface(),10,40),Some(color(*b)));}
}
#[test]fn startup_partial_and_unbound_snapshots_clear_old_destination_slots(){
 let all=bindings();let mut stores=configured();for b in &all{publish(&mut stores,*b,1,g(),&[1,2]);}
 let mut image=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut image));
 let mut partial=Set::UNBOUND;assert!(partial.initialize_once(all[1]));publish(&mut partial,all[1],1,g(),&[31,32]);
 assert!(partial.snapshot_into(&mut image));assert_eq!(image.binding_for(all[0].target(),all[0].surface()),None);assert_eq!(image.sample(all[1].target(),all[1].surface(),10,40),Some(31));assert_eq!(image.binding_for(all[2].target(),all[2].surface()),None);
 assert!(Set::UNBOUND.snapshot_into(&mut image));assert!(image.pixels_scrubbed());for b in all{assert!(!image.matches_surface(b.target(),b.surface()));}
}
#[test]fn malformed_installed_task_alias_fails_whole_snapshot_and_scrubs_prior_pixels(){
 let all=bindings();let mut valid=configured();publish(&mut valid,all[0],1,g(),&[7,8]);let mut image=PublishedStores::<COUNT>::EMPTY;assert!(valid.snapshot_into(&mut image));
 let mut malformed=Set::UNBOUND;for b in &all{let bad=Binding::from_catalog(b.slot(),app_lifecycle::CatalogInfo{target_pd:b.target(),task_id:150,surface_id:b.surface(),coordinator_pd:3}).unwrap();assert!(malformed.initialize_once(bad));}
 assert!(!malformed.snapshot_into(&mut image));assert!(image.pixels_scrubbed());for b in all{assert_eq!(image.binding_for(b.target(),b.surface()),None);}
 let mut wrong=PublishedStores::<2>::EMPTY;assert!(!Stores::<2>::UNBOUND.snapshot_into(&mut wrong));assert!(wrong.pixels_scrubbed());
}
#[test]fn direct_stage_unbound_retired_and_reset_snapshots_leave_no_copied_image(){
 let b=bindings()[0];let mut stage=Stage::UNBOUND;assert!(stage.initialize_once(b));
 for request in [Request::Begin{sequence:1,geometry:g()},Request::Chunk{sequence:1,offset:0,count:2,pixels:[99,100]},Request::Commit{sequence:1,hash:hash(&[99,100])}]{stage.receive(b.target(),b.task(),b.surface(),request);}
 let mut image=Published::EMPTY;stage.snapshot_into(&mut image);assert_eq!(image.geometry(),Some(g()));assert_eq!(image.binding(),Some(b));assert_eq!(image.sample(b.target(),b.surface(),10,40),Some(99));
 Stage::UNBOUND.snapshot_into(&mut image);assert!(image.pixels_scrubbed());assert_eq!(image.binding(),None);assert_eq!(image.geometry(),None);
 stage.snapshot_into(&mut image);stage.retire(b.target(),b.task()+1);stage.snapshot_into(&mut image);assert_eq!(image.sample(b.target(),b.surface(),10,40),Some(99));
 stage.reset_surface(b.target(),b.surface());stage.snapshot_into(&mut image);assert!(image.pixels_scrubbed());assert_eq!(image.geometry(),None);assert_eq!(image.binding(),Some(b));
 stage.retire(b.target(),b.task());stage.snapshot_into(&mut image);assert!(image.pixels_scrubbed());assert_eq!(image.binding(),Some(b));assert!(!stage.initialize_once(b));
 image.scrub();assert!(image.pixels_scrubbed());assert_eq!(image.binding(),None);assert_eq!(image.geometry(),None);
}
#[test]fn maximum_committed_snapshot_copies_all_pixels_without_staging_or_pointer_aliases(){
 let mut stores=configured();let b=bindings()[2];let g=Geometry{x:65535,y:65535,width:48,height:48,scale:4};let pixels:Vec<_>=(0..2304).map(|i|0xff000000|i as u32).collect();publish(&mut stores,b,1,g,&pixels);
 let mut image=PublishedStores::<COUNT>::EMPTY;assert!(stores.snapshot_into(&mut image));
 for y in 0..48{for x in 0..48{assert_eq!(image.sample(b.target(),b.surface(),65535+x*4,65535+y*4),Some(pixels[(y*48+x)as usize]));}}
 stores.reset_surface(b.target(),b.surface());assert_eq!(image.sample(b.target(),b.surface(),65535+47*4,65535+47*4),Some(pixels[2303]));image.scrub();assert!(image.pixels_scrubbed());
}
