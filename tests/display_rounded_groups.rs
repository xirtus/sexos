// SPDX-License-Identifier: GPL-3.0-only
//! Actual immutable owner selection, with independent geometric expectations.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_stack.rs"]pub mod desktop_stack;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"]mod surface_lifecycle;
#[path="../servers/sexdisplay/src/surface_groups.rs"]mod surface_groups;
use surface_lifecycle::{Surface,SURFACE_EMPTY,Catalog};
use surface_groups::FrameGroups;
fn bar_height()->usize{28}
fn surface(id:u64,x:i32,y:i32,w:u32,h:u32)->Surface{Surface{surface_id:id,owner_pd:8,x,y,w,h,active:true,..SURFACE_EMPTY}}
// Deliberately floating point and explicit global translation: no production
// radius helper, table, row spans, clipping or winner function supplies this.
fn alpha(s:&Surface,scale:u32,x:usize,y:usize)->u8{
 let lx=x as i64-s.x as i64;let ly=y as i64-s.y as i64;
 if y<28*scale as usize||lx<0||ly<0||lx>=s.w as i64||ly>=s.h as i64{return 0;}
 let r=(12*scale).min(s.w/2).min(s.h/2)as f64;
 let cx=(lx as u32).min(s.w-1-lx as u32)as f64;
 let cy=(ly as u32).min(s.h-1-ly as u32)as f64;
 if cx>=r||cy>=r||r==0.0{return 255;}
 let mut on=0u32;
 for sy in [0.125,0.375,0.625,0.875]{for sx in [0.125,0.375,0.625,0.875]{
  if (cx+sx-r).powi(2)+(cy+sy-r).powi(2)<=r*r{on+=1;}
 }}
 ((on*255+8)/16)as u8
}
#[test]fn actual_group_scalar_and_row_owners_match_independent_visible_shape(){
 assert!(core::mem::size_of::<surface_groups::RowWinners<'_, '_>>()<=64);
 assert!(core::mem::size_of::<surface_groups::RowScratch>()<=12*1024);
 for scale in 1..=3{for (x,y,w,h)in [(-20,45,120,100),(20,45,120,100),(-80,-30,120,160),(145,145,90,80),(i32::MIN,45,u32::MAX,100),(5,60,7,5),(5,60,0,0)]{
  let mut g=FrameGroups::new();g.set_scale(scale);
  let surfaces=[surface(800,x,y,w,h),surface(801,0,28,180,160)];
  for point in [(-1,100),(180,100),(20,-1),(20,180),(20,27),(20,(28*scale-1)as i32)]{
   assert!(!desktop_style::chrome::Metrics::new(scale).panel_hit((x,y,w,h),point,(180,180)),"outside workarea {point:?}");
  }
  for py in 0..180{let mut row_scratch=surface_groups::RowScratch::new();let mut row=g.row_winners(&surfaces,800,py,180,180,&mut row_scratch).unwrap();
   for px in 0..180{
    let expected=surfaces.iter().find(|s|alpha(s,scale,px,py)!=0).map(|s|s.surface_id);
    assert_eq!(g.winner(&surfaces,800,px,py,180,180).map(|s|s.surface_id),expected,"s={scale} at{px},{py}");
    assert_eq!(row.selected(px),expected,"row s={scale} at{px},{py}");
    assert_eq!(desktop_style::chrome::Metrics::new(scale).panel_hit((x,y,w,h),(px as i32,py as i32),(180,180)),expected==Some(800),"hit s={scale} at{px},{py}");
    assert_eq!(g.coverage(&surfaces[0],px,py),alpha(&surfaces[0],scale,px,py));
    let(a,b)=g.row_span(&surfaces[0],py,180,180,true);
    assert_eq!(px>=a&&px<b,alpha(&surfaces[0],scale,px,py)==255);
   }
  }
 }}
}

#[test]fn row_scratch_reuse_resolves_only_the_current_slice_and_current_prefix(){
 let g=FrameGroups::new();let mut scratch=surface_groups::RowScratch::new();
 let old=[surface(800,20,50,80,80),surface(801,0,28,180,160)];
 {
  let mut row=g.row_winners(&old,800,70,180,180,&mut scratch).unwrap();
  let selected=row.source(40).unwrap();
  assert!(core::ptr::eq(selected.surface,&old[0]));
  assert!(core::ptr::eq(selected.content,&old[0]));
 }
 // Same IDs and changed slot order cannot reuse a pointer into the old slice.
 let new=[surface(801,0,28,180,160),surface(800,80,50,80,80)];
 {
  let mut row=g.row_winners(&new,800,70,180,180,&mut scratch).unwrap();
  assert!(core::ptr::eq(row.source(40).unwrap().surface,&new[0]));
  assert!(core::ptr::eq(row.source(100).unwrap().surface,&new[1]));
 }
 assert!(g.row_winners(&new,800,180,180,180,&mut scratch).is_none());
 // A shorter next row must not read old runs left beyond its initialized prefix.
 let empty:[Surface;0]=[];
 let mut row=g.row_winners(&empty,0,70,180,180,&mut scratch).unwrap();
 assert_eq!(row.run(0).0,180);assert!(row.source(40).is_none());
 drop(row);scratch.clear();
 let mut row=g.row_winners(&new,800,70,180,180,&mut scratch).unwrap();
 assert!(core::ptr::eq(row.source(100).unwrap().surface,&new[1]));
}
#[test]fn next_owner_requires_same_scene_reference_and_stops_at_last_layer(){
 let g=FrameGroups::new();let surfaces=[surface(800,20,50,80,80),surface(801,0,28,180,160)];
 let top=g.winner(&surfaces,800,30,50,180,180).unwrap();assert_eq!(top.surface_id,800);
 let lower=g.winner_below(&surfaces,800,Some(top),30,50,180,180).unwrap();assert_eq!(lower.surface_id,801);
 assert!(g.winner_below(&surfaces,800,Some(lower),30,50,180,180).is_none());
 let foreign=surfaces[0];assert!(g.winner_below(&surfaces,800,Some(&foreign),30,50,180,180).is_none());
}
#[test]fn children_share_parent_shape_and_offscreen_body_keeps_original_origin(){
 let mut g=FrameGroups::new();g.register(Catalog{slot:9,target_pd:9,task_id:9,surface_id:156,coordinator_pd:3});
 let mut surfaces=[Surface{owner_pd:3,..surface(201,-20,50,100,100)},Surface{owner_pd:9,..surface(156,0,0,200,200)}];
 g.observe(&surfaces);g.fit_children(&mut surfaces);
 assert_eq!((surfaces[1].x,surfaces[1].y,surfaces[1].w,surfaces[1].h),(-12,84,84,58));
 assert!(!g.independent(&surfaces[1]));assert_eq!(g.coverage(&surfaces[1],0,84),255);
 assert_eq!(g.content(&surfaces[0],&surfaces,0,84,180,180).surface_id,156);
 assert_eq!(g.content(&surfaces[0],&surfaces,0,83,180,180).surface_id,201);
 let old=g;g.set_scale(3);let floating=surface(800,-20,100,100,100);
 assert_eq!(old.coverage(&floating,0,100),255);assert_eq!(g.coverage(&floating,0,100),0);
}

#[test]fn shared_order_is_slot_independent_and_covers_programs_and_bound_child_focus(){
 let mut g=FrameGroups::new();g.register(Catalog{slot:9,target_pd:9,task_id:9,surface_id:156,coordinator_pd:3});
 let parent=Surface{owner_pd:3,..surface(201,20,100,100,100)};
 let linen=Surface{owner_pd:3,..surface(200,0,28,180,172)};
 let mesh=Surface{owner_pd:3,..surface(202,0,28,180,172)};
 let child=Surface{owner_pd:9,..surface(156,28,134,84,58)};
 let mut surfaces=[mesh,parent,child,linen];g.observe(&surfaces);
 // Literal overlapping three real windows: at the parent's cut corner Linen
 // is the real next owner, regardless of admission/reuse order.
 for order in [[0,1,2,3],[3,2,1,0],[2,0,3,1]]{
  let reordered=order.map(|i|surfaces[i]);
  assert_eq!(g.winner(&reordered,201,20,100,180,200).unwrap().surface_id,200);
  assert_eq!(g.winner(&reordered,156,40,140,180,200).unwrap().surface_id,201);
  let top=g.winner(&reordered,201,40,140,180,200).unwrap();
  assert_eq!(g.winner_below(&reordered,201,Some(top),40,140,180,200).unwrap().surface_id,200);
  let mut row_scratch=surface_groups::RowScratch::new();let mut row=g.row_winners(&reordered,201,100,180,200,&mut row_scratch).unwrap();assert_eq!(row.selected(20),Some(200));
 }
 // Clearing/reusing the earlier slot cannot raise Mesh over Linen.
 surfaces[0]=SURFACE_EMPTY;surfaces[0]=mesh;
 assert_eq!(g.winner(&surfaces,0,40,140,180,200).unwrap().surface_id,200);
 surfaces[3].active=false;assert_eq!(g.winner(&surfaces,0,40,140,180,200).unwrap().surface_id,201);
 let program=app_catalog::INCARNATIONS.iter().find(|row|row.app==app_catalog::App::Program).unwrap();
 assert!(desktop_stack::ORDER.contains(&program.frame()));
 surfaces[0]=Surface{owner_pd:program.target,..surface(program.frame(),0,28,180,172)};
 assert_eq!(g.winner(&surfaces,program.frame(),40,140,180,200).unwrap().surface_id,program.frame());
 // Only the trusted Shell palette has modal precedence.
 surfaces[3]=Surface{owner_pd:3,..surface(152,0,28,180,172)};
 assert_eq!(g.winner(&surfaces,program.frame(),40,140,180,200).unwrap().surface_id,152);
 surfaces[3].owner_pd=8;
 assert_eq!(g.winner(&surfaces,program.frame(),40,140,180,200).unwrap().surface_id,program.frame());
}
