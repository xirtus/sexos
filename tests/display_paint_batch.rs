//! Actual bounded output-plan module and actual damage helper. No renderer,
//! admission, authority, causal-stream, native-pixel or epoch closure claim.
#[path="../servers/sexdisplay/src/damage.rs"]mod damage;
#[path="../servers/sexdisplay/src/paint_batch.rs"]mod paint_batch;
use damage::Rect;
use paint_batch::{Content,Dirty,Plan};
const W:usize=1024;const H:usize=768;
fn rect(x:usize,y:usize,right:usize,bottom:usize)->Rect{Rect{x,y,right,bottom}}
fn request(d:&mut Dirty,kind:u8){match kind{0=>d.request_cursor(),1=>d.request_damage(rect(10,70,20,80),W,H),2=>d.request_damage(rect(30,60,40,90),W,H),3=>d.request_area(),4=>d.request_strip(),5=>d.request_full(),_=>unreachable!()}}
fn permutations(values:&mut[u8],at:usize,apply:&mut impl FnMut(&[u8])){
 if at==values.len(){apply(values);return;}
 for i in at..values.len(){values.swap(at,i);permutations(values,at+1,apply);values.swap(at,i);}
}
#[test]fn empty_plan_has_no_output_obligation_even_when_observed_geometry_changes(){
 let mut d=Dirty::new(W,H);let p=d.plan(W,H);assert!(p.is_empty());assert_eq!(p.label(),"none");assert!(!p.clock_dirty());assert!(!p.cursor_only());
 assert!(d.plan(640,480).is_empty());d.request_cursor();assert_eq!(d.plan(640,480),Plan{content:Content::Cursor,strip:false});
}
#[test]fn all_dominance_permutations_and_duplicates_have_identical_normalized_plans(){
 for(mut inputs,expected)in[(vec![0,1,2,4],Plan{content:Content::Damage(rect(10,60,40,90)),strip:true}),(vec![0,1,2,3,4],Plan{content:Content::Area,strip:true}),(vec![0,1,2,3,4,5],Plan::full())]{
  permutations(&mut inputs,0,&mut|order|{let mut d=Dirty::new(W,H);for kind in order{request(&mut d,*kind);request(&mut d,*kind);}assert_eq!(d.plan(W,H),expected,"{order:?}");});
 }
}
#[test]fn disjoint_touching_nested_and_duplicate_damage_union_is_conservative(){
 for(first,second,expected)in[(rect(10,60,20,70),rect(30,80,40,90),rect(10,60,40,90)),(rect(10,60,20,70),rect(20,70,30,80),rect(10,60,30,80)),(rect(10,60,40,90),rect(20,70,30,80),rect(10,60,40,90))]{
  for(a,b)in[(first,second),(second,first)]{let mut d=Dirty::new(W,H);d.request_damage(a,W,H);d.request_damage(b,W,H);d.request_damage(a,W,H);assert_eq!(d.plan(W,H),Plan{content:Content::Damage(expected),strip:false});}
 }
}
#[test]fn invalid_empty_offscreen_and_top_strip_damage_never_erase_cursor_work(){
 for r in[rect(4,60,4,70),rect(5,70,4,80),rect(0,70,1,60),rect(0,0,20,51),rect(W,60,W+1,70),rect(0,H,1,H+1),rect(usize::MAX-1,60,usize::MAX,70)]{
  let mut d=Dirty::new(W,H);d.request_cursor();d.request_damage(r,W,H);assert!(d.plan(W,H).cursor_only(),"{r:?}");
 }
 let mut d=Dirty::new(W,H);d.request_damage(rect(10,50,20,52),W,H);assert_eq!(d.plan(W,H).content,Content::Damage(rect(10,51,20,52)));
}
#[test]fn huge_extents_clip_without_wrap_and_exact_frame_edges_are_kept(){
 let mut d=Dirty::new(W,H);d.request_damage(rect(0,0,usize::MAX,usize::MAX),W,H);assert_eq!(d.plan(W,H).content,Content::Damage(rect(0,51,W,H)));
 let mut edge=Dirty::new(W,H);edge.request_damage(rect(W-1,H-1,usize::MAX,usize::MAX),W,H);assert_eq!(edge.plan(W,H).content,Content::Damage(rect(W-1,H-1,W,H)));
}
#[test]fn failure_retains_exact_work_new_requests_merge_and_only_success_clears(){
 let mut d=Dirty::new(W,H);d.request_damage(rect(10,60,20,70),W,H);d.request_strip();let before=d;let plan=d.plan(W,H);d.complete(false);assert_eq!(d,before);assert_eq!(d.plan(W,H),plan);
 d.request_damage(rect(30,80,40,90),W,H);assert_eq!(d.plan(W,H),Plan{content:Content::Damage(rect(10,60,40,90)),strip:true});
 d.complete(false);d.request_cursor();assert!(!d.plan(W,H).cursor_only());d.complete(true);assert!(d.plan(W,H).is_empty());
}
#[test]fn geometry_change_promotes_and_retains_full_until_success(){
 let mut d=Dirty::new(W,H);d.request_damage(rect(900,700,1000,760),W,H);d.request_strip();assert_eq!(d.plan(640,480),Plan::full());d.complete(false);assert_eq!(d.plan(W,H),Plan::full());d.complete(true);assert!(d.plan(W,H).is_empty());d.request_cursor();assert!(d.plan(W,H).cursor_only());
 let mut d=Dirty::new(W,H);d.request_cursor();d.request_damage(rect(1,60,4,70),640,480);assert!(d.plan(640,480).is_full());
 let mut strip=Dirty::new(W,H);strip.request_strip();assert!(strip.plan(640,480).is_full());
}
#[test]fn plan_flags_labels_and_atlas_promotion_preserve_independent_strip(){
 for(content,label)in[(Content::None,"none"),(Content::Cursor,"cursor"),(Content::Damage(rect(1,60,2,61)),"text"),(Content::Area,"area"),(Content::Full,"full")]{
  let p=Plan{content,strip:false};assert_eq!(p.label(),label);assert_eq!(p.clock_dirty(),matches!(content,Content::Full));assert_eq!(p.cursor_only(),matches!(content,Content::Cursor));assert_eq!(p.with_atlas(false),p);
  for strip in[false,true]{let p=Plan{content,strip};let transformed=p.with_atlas(true);assert_eq!(transformed.strip,strip);assert_eq!(transformed.content,if matches!(content,Content::Damage(_)){Content::Area}else{content});if strip{assert_eq!(p.label(),if matches!(content,Content::Full){"full"}else{"strip"});assert!(p.clock_dirty());assert!(!p.cursor_only());}}
 }
}


#[test]fn settle_is_nonempty_least_work_under_every_join_order(){
 for(content,label)in[(Content::Settle,"settle"),(Content::Cursor,"cursor"),(Content::Damage(rect(10,70,20,80)),"text"),(Content::Area,"area"),(Content::Full,"full")]{
  for before in[false,true]{let mut d=Dirty::new(W,H);
   if before{d.request_settle();d.request_settle();}
   match content{Content::Settle=>d.request_settle(),Content::Cursor=>d.request_cursor(),Content::Damage(r)=>d.request_damage(r,W,H),Content::Area=>d.request_area(),Content::Full=>d.request_full(),Content::None=>unreachable!()}
   d.request_settle();d.request_settle();let p=d.plan(W,H);
   assert_eq!(p,Plan{content,strip:false});assert!(!p.is_empty());assert_eq!(p.label(),label);
   assert_eq!(p.settle_only(),matches!(content,Content::Settle));
   assert_eq!(p.cursor_only(),matches!(content,Content::Cursor));
  }
 }
 // Full permutation coverage includes duplicate Settle, existing partial work
 // and independent strip. Higher priorities cannot erase or be erased by it.
 for(mut values,expected)in[(vec![0,1,4,6],Plan{content:Content::Damage(rect(10,70,20,80)),strip:true}),
                           (vec![0,1,3,4,6],Plan{content:Content::Area,strip:true}),
                           (vec![0,1,3,4,5,6],Plan::full())]{
  permutations(&mut values,0,&mut|order|{let mut d=Dirty::new(W,H);for &kind in order{if kind==6{d.request_settle();d.request_settle();}else{request(&mut d,kind);}}assert_eq!(d.plan(W,H),expected,"{order:?}");});
 }
}
#[test]fn settle_failure_retains_exact_obligation_and_later_work_upgrades_it(){
 let mut d=Dirty::new(W,H);d.request_settle();let before=d;
 d.complete(false);assert_eq!(d,before);assert!(d.plan(W,H).settle_only());
 for invalid in[rect(0,0,20,51),rect(5,60,5,70),rect(W,60,W+1,70)]{d.request_damage(invalid,W,H);assert!(d.plan(W,H).settle_only());}
 d.request_cursor();assert!(d.plan(W,H).cursor_only());d.request_settle();assert!(d.plan(W,H).cursor_only());
 d.request_damage(rect(2,60,5,70),W,H);d.complete(false);assert_eq!(d.plan(W,H).content,Content::Damage(rect(2,60,5,70)));
 d.complete(true);assert!(d.plan(W,H).is_empty());assert!(!d.plan(W,H).settle_only());
}
#[test]fn settle_strip_and_atlas_are_explicit_and_preserve_clock_priority(){
 for strip in[false,true]{for first in[false,true]{let mut d=Dirty::new(W,H);
  if strip&&first{d.request_strip();}d.request_settle();if strip&&!first{d.request_strip();}
  let p=d.plan(W,H);assert_eq!(p,Plan{content:Content::Settle,strip});
  assert_eq!(p.settle_only(),!strip);assert!(!p.cursor_only());assert_eq!(p.clock_dirty(),strip);
  assert_eq!(p.label(),if strip{"strip"}else{"settle"});assert_eq!(p.with_atlas(false),p);
  assert_eq!(p.with_atlas(true),Plan{content:Content::Area,strip});
 }}
}
#[test]fn body_coverage_requires_valid_entire_region_and_never_uses_strip_or_cursor(){
 let required=rect(10,60,30,80);
 for content in[Content::None,Content::Settle,Content::Cursor]{for strip in[false,true]{assert!(!Plan{content,strip}.covers_body(required));}}
 for strip in[false,true]{for content in[Content::Full,Content::Area,Content::Damage(required),Content::Damage(rect(9,51,31,81))]{assert!(Plan{content,strip}.covers_body(required));}}
 // Any missing edge, overlap-only, disjoint region or invalid source Damage
 // refuses. Containing the top-left point alone cannot establish coverage.
 for r in[rect(11,60,30,80),rect(10,61,30,80),rect(10,60,29,80),rect(10,60,30,79),
          rect(20,70,40,90),rect(31,60,40,80),rect(30,60,30,80),rect(40,60,30,80),rect(0,50,40,90)]{
  assert!(!Plan{content:Content::Damage(r),strip:true}.covers_body(required),"{r:?}");
 }
 for invalid in[rect(0,0,20,60),rect(0,50,20,60),rect(5,60,5,70),rect(6,60,5,70),rect(0,70,20,70),rect(0,71,20,70)]{
  for content in[Content::Full,Content::Area,Content::Damage(rect(0,51,W,H))]{assert!(!Plan{content,strip:false}.covers_body(invalid),"{invalid:?}");}
 }
 let edge=rect(usize::MAX-1,usize::MAX-1,usize::MAX,usize::MAX);
 assert!(Plan{content:Content::Damage(edge),strip:false}.covers_body(edge));
 assert!(!Plan{content:Content::Damage(rect(0,51,W,H)),strip:false}.covers_body(edge));
}
#[test]fn settle_geometry_change_including_tiny_shapes_promotes_full_and_retains_failure(){
 for(width,height)in[(640,480),(1,1),(W,50),(0,0)]{
  let mut d=Dirty::new(W,H);d.request_settle();assert_eq!(d.plan(width,height),Plan::full());
  d.complete(false);assert_eq!(d.plan(width,height),Plan::full());assert_eq!(d.plan(W,H),Plan::full());
  d.complete(true);assert!(d.plan(W,H).is_empty());
 }
 let mut tiny=Dirty::new(1,1);tiny.request_settle();assert!(tiny.plan(1,1).settle_only());
 // Geometry validation/publication remain the output owner's responsibility;
 // an empty Dirty never manufactures work just because geometry was observed.
 let mut empty=Dirty::new(W,H);assert!(empty.plan(1,1).is_empty());
}
