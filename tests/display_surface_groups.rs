// SPDX-License-Identifier: GPL-3.0-only
//! Actual compositor body extracted by gate. Glass shaders are deterministic
//! sentinels; surface selection, clipping, fill, font and presentation bits are real.
#![allow(dead_code,unused_variables,unused_unsafe,static_mut_refs)]
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"] mod surface_lifecycle;
#[path="../servers/sexdisplay/src/surface_groups.rs"] mod surface_groups;
use surface_lifecycle::*;
use surface_groups::FrameGroups;
macro_rules! serial_println { ($($a:tt)*) => {} }
static mut SURFACES:[Surface;MAX_SURFACES]=[SURFACE_EMPTY;MAX_SURFACES];
static mut FRAME_GROUPS:FrameGroups=FrameGroups::new();
static mut APP_PRESENT_BITS:u8=0;
static RENDER_FRAME_COUNTER:core::sync::atomic::AtomicU64=core::sync::atomic::AtomicU64::new(0);
struct Tokens{frame_top_bar_color:u32,active_tab_color:u32,inactive_tab_color:u32,close_light_color:u32,minimize_light_color:u32,zoom_light_color:u32,frame_rim_color:u32}
static DISPLAY_TOKENS:Tokens=Tokens{frame_top_bar_color:0x111111,active_tab_color:0x222222,inactive_tab_color:0x333333,close_light_color:0x444444,minimize_light_color:0x555555,zoom_light_color:0x666666,frame_rim_color:0x777777};
fn hot_log()->bool{false}
fn glass_over_bg(c:u32,_x:usize,_y:usize,_a:u8)->u32{c}
fn alpha_blend_xrgb_over_xrgb(c:u32,_b:u32,_a:u8)->u32{c}
fn pulse_alpha(base:u8,_a:u8,_f:u64,_p:u64)->u8{base}
include!(env!("DISPLAY_COMPOSITION_SOURCE"));
fn cat(pd:u32)->Catalog{Catalog{slot:pd as usize,target_pd:pd,task_id:pd,surface_id:if pd==9{156}else{154},coordinator_pd:3}}
fn surf(pd:u32,sid:u64)->Surface{Surface{owner_pd:pd,surface_id:sid,x:100,y:100,w:400,h:300,color:0x123456,active:true,..SURFACE_EMPTY}}
fn frame()->Surface{Surface{chrome_flags:1,color:0x445566,..surf(3,201)}}
#[test]fn secondary_ids_reserved_only_by_exact_boot_catalog(){
 let mut l=Lifecycle::new();assert!(l.register(cat(12)));let s=[SURFACE_EMPTY;8];
 for sid in[154,160,161,162]{assert_eq!(l.admit(&s,12,sid),Admission::Vacant(0,12));assert_eq!(l.admit(&s,3,sid),Admission::Vacant(0,12));assert_eq!(l.admit(&s,9,sid),Admission::Denied);assert!(l.coordinator_controls(3,sid));}
 let mut fake=Lifecycle::new();assert!(fake.register(Catalog{surface_id:999,..cat(12)}));assert_eq!(fake.admit(&s,9,160),Admission::Vacant(0,9));
 assert!(!l.register(Catalog{surface_id:160,target_pd:20,task_id:20,..cat(12)}));
}
#[test]fn parent_seen_hides_minimized_children_without_erasing_text_and_restore_reuses_it(){
 let mut g=FrameGroups::new();g.register(cat(9));let mut s=[surf(9,156),SURFACE_EMPTY];s[0].text_len=1;s[0].text_buf[0]=b'X';
 assert!(g.winner(&s,156,110,110,1000,800).is_some());s[1]=frame();g.observe(&s);g.fit_children(&mut s);
 assert!(!g.independent(&s[0]));s[1]=SURFACE_EMPTY;assert!(g.winner(&s,156,120,150,1000,800).is_none());assert_eq!(s[0].text_buf[0],b'X');
 s[1]=frame();g.observe(&s);assert_eq!(g.winner(&s,156,120,150,1000,800).unwrap().surface_id,201);
}
#[test]fn late_native_geometry_is_fitted_and_grid_has_exact_gap_and_bounds(){
 let mut g=FrameGroups::new();g.register(cat(12));let mut s=[surf(3,153),surf(12,154),surf(12,160),surf(12,161),surf(12,162)];g.observe(&s);
 for c in &mut s[1..]{c.x=-900;c.y=-900;c.w=10000;c.h=10000;}g.fit_children(&mut s);
 assert_eq!((s[1].x,s[1].y,s[1].w,s[1].h),(108,136,188,124));assert_eq!((s[2].x,s[2].y),(304,136));assert_eq!((s[3].x,s[3].y),(108,268));assert_eq!((s[4].x,s[4].y),(304,268));
 for c in &s[1..]{assert!(c.x>=108&&c.y>=136);assert!(c.x as u32+c.w<=492);assert!(c.y as u32+c.h<=392);}
 s[0].w=1;s[0].h=1;g.fit_children(&mut s);assert!(s[1..].iter().all(|c|c.w==0&&c.h==0));
}
#[test]fn foreign_owner_never_binds_and_child_cannot_cover_parent_chrome(){
 let mut g=FrameGroups::new();g.register(cat(9));let mut s=[frame(),surf(10,156),surf(9,156)];g.observe(&s);
 assert!(g.independent(&s[1]));assert_eq!(g.content(&s[0],&s,110,110,1000,800).surface_id,201);
 assert_eq!(g.content(&s[0],&s,120,150,1000,800).owner_pd,9);
 s[2].x=900;assert_eq!(g.content(&s[0],&s,120,150,1000,800).surface_id,201);
}
#[test]fn retirement_scrubs_every_secondary_and_prevents_all_queued_recreation(){
 let mut l=Lifecycle::new();l.register(cat(12));let mut s=[surf(3,153),surf(12,154),surf(12,160),surf(12,161),surf(12,162)];let mut refs=SceneRefs::default();
 assert_eq!(l.prepare(0,12,12,&mut s,&mut refs),Some(5));for sid in[153,154,160,161,162]{assert!(!l.sid_live(sid));assert_eq!(l.admit(&s,3,sid),Admission::Denied);assert_eq!(l.admit(&s,12,sid),Admission::Denied);}
 assert_eq!(l.admit(&s,9,999),Admission::Vacant(0,9));
}
#[test]fn actual_compositor_keeps_child_color_text_parent_chrome_and_exact_pixel_proof(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(9));
 SURFACES[0]=surf(9,156);SURFACES[1]=frame();FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
 SURFACES[0].text_len=1;SURFACES[0].text_buf[0]=b'A';SURFACES[0].text_color=0xfedcba;recompute_text_lines(&mut SURFACES[0]);
 for focus in[0,201,156]{APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0,focus),0x123456);assert_eq!(APP_PRESENT_BITS,1);
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(120,127,1000,800,0,focus),FRAME_TOP_BAR_DIVIDER_COLOR);assert_eq!(APP_PRESENT_BITS,0);
 // A top-row glyph foreground uses actual scaled font coordinates.
 assert_eq!(composite_pixel(118,148,1000,800,0,focus),0xfedcba);}
 SURFACES[1]=SURFACE_EMPTY;APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0xabcdef,156),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);
 SURFACES[1]=frame();FRAME_GROUPS.observe(&SURFACES);assert_eq!(composite_pixel(150,180,1000,800,0,201),0x123456);
 // Another focused window occludes children: no false presented bit survives.
 SURFACES[2]=surf(8,888);APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0,888),0x123456);assert_eq!(APP_PRESENT_BITS,0);
 // Modal launcher beats the focused app without producing native-app bits.
 SURFACES[3]=Surface{color:0xaabbcc,..surf(3,152)};
 SURFACES[3].text_len=1;SURFACES[3].text_buf[0]=b'A';SURFACES[3].text_color=0xf0f8ff;recompute_text_lines(&mut SURFACES[3]);
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0,888),0xaabbcc);assert_eq!(APP_PRESENT_BITS,0);
 assert_eq!(composite_pixel(110,112,1000,800,0,888),0xf0f8ff);
}}
#[test]fn launcher_overlay_requires_shell_owner_and_hide_restores_app_winner(){
 let g=FrameGroups::new();let mut s=[surf(8,888),surf(3,152)];
 assert_eq!(g.winner(&s,888,150,180,1000,800).unwrap().surface_id,152);
 s[1].owner_pd=9;assert_eq!(g.winner(&s,888,150,180,1000,800).unwrap().surface_id,888);
 s[1].owner_pd=3;s[1].active=false;assert_eq!(g.winner(&s,888,150,180,1000,800).unwrap().surface_id,888);
 let l=Lifecycle::new();assert_eq!(l.admit(&[],9,152),Admission::Denied);assert_eq!(l.admit(&[SURFACE_EMPTY],3,152),Admission::Vacant(0,3));
}
#[test]fn clipped_and_tiny_frames_never_select_children_outside_body_or_framebuffer(){
 for (x,y,w,h) in [(-30,-20,400,300),(950,750,400,300),(10,70,15,43),(10,70,0,0),(i32::MAX,i32::MAX,400,300)]{
  let mut g=FrameGroups::new();g.register(cat(9));let mut s=[Surface{x,y,w,h,..frame()},surf(9,156)];g.observe(&s);g.fit_children(&mut s);
  let (fx,fy,fw,fh)=surface_groups::rect(&s[0],1000,800);
  for py in 0..801{for px in [0,7,8,fx,fx.saturating_add(7),fx.saturating_add(8),999,1000]{
   let content=g.content(&s[0],&s,px,py,1000,800);
   if content.surface_id==156{assert!(px>=fx+8&&px-fx-8<fw.saturating_sub(16));assert!(py>=fy+36&&py-fy-36<fh.saturating_sub(44));assert!(px<1000&&py<800);}
  }}
 }
}
#[test]fn frame_slot_reuse_does_not_unbind_children_and_grid_restore_preserves_payloads(){
 let mut g=FrameGroups::new();g.register(cat(12));let mut s=[surf(3,153),surf(12,154),surf(12,160),surf(12,161),surf(12,162)];g.observe(&s);g.fit_children(&mut s);
 for(i,c)in s[1..].iter_mut().enumerate(){c.text_buf[0]=b'A'+i as u8;c.text_len=1;c.color=i as u32+1;}
 s[0]=surf(8,777);assert!(s[1..].iter().all(|s|!g.independent(s)));
 s[0]=Surface{x:300,y:200,w:600,h:500,..surf(3,153)};g.observe(&s);g.fit_children(&mut s);
 for(i,c)in s[1..].iter().enumerate(){assert_eq!(c.text_buf[0],b'A'+i as u8);assert_eq!(c.color,i as u32+1);assert!(c.x>=308&&c.y>=236);}
}
#[test]fn fresh_spindle_grid_uses_distinct_owner_frame_and_exact_quadrants(){
 let c=Catalog{slot:50,target_pd:17,task_id:17,surface_id:302,coordinator_pd:3};let mut g=FrameGroups::new();g.register(c);
 let mut s=[surf(3,306),surf(17,302),surf(17,303),surf(17,304),surf(17,305)];g.observe(&s);g.fit_children(&mut s);
 for (i,expected) in [(108,136),(304,136),(108,268),(304,268)].into_iter().enumerate(){let child=&s[i+1];assert_eq!((child.x,child.y),expected);
  let px=child.x as usize+10;let py=child.y as usize+14;let frame=g.winner(&s,302,px,py,1000,800).unwrap();assert_eq!(frame.surface_id,306);assert_eq!(g.content(frame,&s,px,py,1000,800).surface_id,302+i as u64);
 }
 assert_eq!(native_pixel_bit(17,302),16);assert_eq!(native_pixel_bit(12,302),0);
 s[0]=SURFACE_EMPTY;assert!(g.winner(&s,302,120,150,1000,800).is_none());
}
