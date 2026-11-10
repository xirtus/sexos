//! Actual data-model reflow, independent bounds/hit golden checks.
#[path="../crates/silkbar-model/src/lib.rs"] mod model;
use model::*;
#[test]
fn all_targets_fit_and_do_not_overlap_at_required_native_scales(){
 for (width,scale) in [(1024,1),(1280,1),(2560,1),(2560,2),(1024,3),(3840,3)]{
  let mut b=DEFAULT_SILK_BAR;assert!(reflow_chrome(&mut b,width,28*scale,24*scale));
  for (i,a) in b.layout.iter().enumerate().filter(|(_,a)|a.w>0){
   assert_eq!(a.y,2*scale);assert!(a.x+a.w<=width&&a.y+a.h<=28*scale);
   assert!(a.w>=24*scale&&a.h==24*scale);
   for z in b.layout.iter().skip(i+1).filter(|z|z.w>0){assert!(a.x+a.w<=z.x||z.x+z.w<=a.x,"overlap width={width} scale={scale}");}
  }
 }
}
#[test]
fn normal_bar_has_nine_workspaces_and_narrow_bar_has_real_overview(){
 let mut normal=DEFAULT_SILK_BAR;assert!(reflow_chrome(&mut normal,1280,28,24));
 assert_eq!((normal.layout[0].x,normal.layout[0].y,normal.layout[0].w),(8,2,24));
 assert_eq!((normal.layout[13].x,normal.layout[13].w),(1128,144));
 assert!(normal.layout[1..10].iter().all(|b|b.w==24));
 let mut narrow=DEFAULT_SILK_BAR;assert!(reflow_chrome(&mut narrow,1024,84,72));
 assert!(narrow.layout[1].action==Action::ToggleModule(Module::Workspaces(0)));
 assert!(narrow.layout[2..10].iter().all(|b|b.w==0));
}

#[test]
fn every_supported_width_has_disjoint_visible_boxes(){
 for scale in 1..=3 {for logical in 320..=4096 {
  let width=logical*scale;let mut b=DEFAULT_SILK_BAR;
  assert!(reflow_chrome(&mut b,width,28*scale,24*scale));
  for (i,a) in b.layout.iter().enumerate().filter(|(_,a)|a.w>0){
   assert!(a.x+a.w<=width);
   for z in b.layout.iter().skip(i+1).filter(|z|z.w>0){assert!(a.x+a.w<=z.x||z.x+z.w<=a.x,"overlap width={width}");}
  }
 }}
}
