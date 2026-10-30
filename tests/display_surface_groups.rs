// SPDX-License-Identifier: GPL-3.0-only
//! Actual compositor body extracted by gate. Glass shaders are deterministic
//! sentinels; surface selection, clipping, fill, font and presentation bits are real.
#![allow(dead_code,unused_variables,unused_unsafe,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod bitmap_transaction;
static mut BITMAP_STAGE:bitmap_transaction::Stage=bitmap_transaction::Stage::EMPTY;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
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
include!("fixtures/compositor_reference.rs");
#[test]fn printable_rom_preserves_case_and_rejects_every_unsupported_byte(){
 assert_eq!(FONT_ASCII_5X7.len(),95);
 assert_eq!((desktop_style::GLYPH_WIDTH,desktop_style::GLYPH_HEIGHT,desktop_style::ADVANCE,desktop_style::LINE),(5,7,6,9));
 for byte in 0u8..=255 {
  assert_eq!(ascii_glyph_index(byte),if (32..=126).contains(&byte){Some((byte-32)as usize)}else{None});
 }
 assert_eq!(FONT_ASCII_5X7[0],[0;7]);
 for glyph in &FONT_ASCII_5X7[1..]{assert!(glyph.iter().any(|r|*r!=0));assert!(glyph.iter().all(|r|*r<32));}
 for lower in b'a'..=b'z'{assert_ne!(FONT_ASCII_5X7[(lower-32)as usize],FONT_ASCII_5X7[(lower-b'a'+b'A'-32)as usize],"byte={lower}");}
 assert_eq!(FONT_ASCII_5X7[(b'a'-32)as usize],[0,0,14,1,15,17,15]);
 assert_eq!(FONT_ASCII_5X7[(b'g'-32)as usize],[0,0,15,17,15,1,14]);
}
#[test]fn actual_text_shader_covers_all_bytes_at_both_scales_without_rewriting_selection(){
 for width in [240,400] {for byte in 0u8..=255 {for selected in [false,true] {
  let mut s=Surface{x:0,y:50,w:width,h:100,text_len:1,text_color:0xfedcba,..surf(9,156)};
  s.text_buf[0]=byte;if selected{s.selection_end=1;}recompute_text_lines(&mut s);
  let scale=surface_text_scale(&s);
  for y in 0..9*scale {for x in 0..6*scale {
   let ink=byte>=32&&byte<=126&&x/scale<5&&y/scale<7
       &&FONT_ASCII_5X7[(byte.saturating_sub(32))as usize][y/scale]&(1<<(4-x/scale))!=0;
   let expected=if byte==b'\n'{None}else if ink{Some(0xfedcba)}else if selected{Some(0xff36577c)}else{None};
   assert_eq!(surface_text_fg_at(8+x,62+y,&s),expected,"w={width} byte={byte} selected={selected} x={x} y={y}");
  }}
  assert_eq!(s.text_buf[0],byte);assert_eq!((s.selection_start,s.selection_end),(0,if selected{1}else{0}));
 }}}
}
#[test]fn actual_toolbar_titles_keep_lowercase_at_fixed_integer_metrics(){
 let x0=FRAME_TOP_BAR_LIGHT_EXCLUSION_PX+12;let y0=(FRAME_TOP_BAR_HEIGHT_PX-14)/2;
 for (sid,title)in[(200,&b"Linen"[..]),(201,&b"Quil"[..]),(300,&b"Browser"[..]),(306,&b"Spindle"[..])]{
  for (column,byte)in title.iter().enumerate(){for y in 0..14 {for x in 0..12 {
   let ink=x/2<5&&FONT_ASCII_5X7[(*byte-32)as usize][y/2]&(1<<(4-x/2))!=0;
   assert_eq!(toolbar_title_fg_at(x0+column*12+x,y0+y,sid),if ink{Some(0xe8ffff)}else{None});
  }}}
 }
}
// Literal accepted64 legacy mutation behavior, separate from new helpers.
fn legacy_draw_reference(slot:&mut Surface,packed:u64,arg2:u64){
 let byte_offset=((arg2&0xFF)|((arg2>>4)&0xF00))as usize;let char_count=((arg2>>8)&0xF)as usize;
 slot.text_color=((arg2>>32)as u32)|0xFF000000;
 let mut i=0;while i<char_count&&i<8&&byte_offset+i<TEXT_BUF_CAP{
  slot.text_buf[byte_offset+i]=((packed>>(i*8))&0xFF)as u8;i+=1;
 }
 let new_len=byte_offset+i;if new_len>slot.text_len as usize{slot.text_len=new_len.min(TEXT_BUF_CAP)as u16;}
 recompute_text_lines(slot);
}
fn legacy_args(offset:usize,count:usize,color:u32)->u64{
 (offset as u64&0xff)|((offset as u64&0xf00)<<4)|((count as u64)<<8)|((color as u64)<<32)
}
unsafe fn publish_bitmap(sequence:u32,g:bitmap_batch::Geometry,pixels:&[u32])->bitmap_transaction::Change{
 use bitmap_batch::Request;
 assert!(BITMAP_STAGE.receive(15,300,Request::Begin{sequence,geometry:g}).is_none());
 for(offset,pair)in pixels.chunks(2).enumerate(){let mut data=[0;2];data[..pair.len()].copy_from_slice(pair);
  assert!(BITMAP_STAGE.receive(15,300,Request::Chunk{sequence,offset:offset*2,count:pair.len(),pixels:data}).is_none());
 }
 let change=BITMAP_STAGE.receive(15,300,Request::Commit{sequence,hash:bitmap_batch::hash(pixels)}).unwrap();
 BITMAP_STAGE.poll_reply(|owner,word|{assert_eq!(owner,15);assert_eq!(bitmap_batch::Applied::decode(1,word).unwrap().sequence,sequence);0});change
}
#[test]fn actual_browser_bitmap_blends_over_fill_under_text_in_both_compositor_branches(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();BITMAP_STAGE=bitmap_transaction::Stage::EMPTY;
 SURFACES[0]=Surface{chrome_flags:SURFACE_CHROME_TOP_BAR,..surf(15,300)};
 fill_damage::update(&mut SURFACES[0],0,0,0,400,300,0x204060).unwrap();
 let before=[composite_pixel(120,120,600,500,0,0),composite_pixel(120,120,600,500,0,300)];
 let g=bitmap_batch::Geometry{x:0,y:0,width:48,height:48,scale:4};
 publish_bitmap(1,g,&vec![0x80ff0000;g.len()]);
 for(index,focus)in[0,300].into_iter().enumerate(){
  assert_eq!(composite_pixel(170,180,600,500,0,focus),bitmap_transaction::blend(0x80ff0000,0x204060));
  // The same image covers this local coordinate but cannot overwrite toolbar chrome.
  assert_eq!(composite_pixel(120,120,600,500,0,focus),before[index]);
 }
 SURFACES[0].text_buf[0]=b'A';SURFACES[0].text_len=1;SURFACES[0].text_color=0xfedcba;recompute_text_lines(&mut SURFACES[0]);
 for focus in[0,300]{assert_eq!(surface_text_fg_at(110,136,&SURFACES[0]),Some(0xfedcba));assert_eq!(composite_pixel(110,136,600,500,0,focus),0xfedcba);}
 // Actual surface ownership and actual winner selection both precede sampling.
 SURFACES[0].owner_pd=9;assert_eq!(composite_pixel(170,180,600,500,0,0),0x204060);SURFACES[0].owner_pd=15;
 SURFACES[1]=Surface{color:0xabcdef,..surf(8,888)};assert_eq!(composite_pixel(170,180,600,500,0,888),0xabcdef);
 SURFACES[1]=Surface{color:0x987654,..surf(3,152)};assert_eq!(composite_pixel(170,180,600,500,0,300),0x987654);
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();BITMAP_STAGE=bitmap_transaction::Stage::EMPTY;
}}
#[test]fn actual_bitmap_move_clear_and_resize_damage_match_full_compositor_pixels(){unsafe{
 let(w,h)=(320,260);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();BITMAP_STAGE=bitmap_transaction::Stage::EMPTY;
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,..surf(15,300)};
 let a=bitmap_batch::Geometry{x:10,y:20,width:40,height:20,scale:1};publish_bitmap(1,a,&vec![0x8000ff00;a.len()]);
 let mut framebuffer=vec![0;w*h];for y in 0..h{for x in 0..w{framebuffer[y*w+x]=composite_pixel(x,y,w,h,bg(y,h),0);}}
 for sequence in 2..=3{
  let change=if sequence==2{let b=bitmap_batch::Geometry{x:120,y:40,width:20,height:30,scale:2};publish_bitmap(sequence,b,&vec![0xffaa0011;b.len()])}
   else{BITMAP_STAGE.receive(15,300,bitmap_batch::Request::Clear{sequence}).unwrap()};
  let mut d=damage::Damage::new();d.add(bitmap_surface_damage(&SURFACES[0],change.old,w,h),w,h);d.add(bitmap_surface_damage(&SURFACES[0],change.new,w,h),w,h);
  FOCUSED_SURFACE_ID=0;DAMAGE_ORDER.clear();redraw_surface_damage(framebuffer.as_mut_ptr(),w,h,d.rect().unwrap());
  assert_eq!(DAMAGE_ORDER,["restore","cursor","launcher","presented"]);
  for y in 0..h{for x in 0..w{assert_eq!(framebuffer[y*w+x],composite_pixel(x,y,w,h,bg(y,h),0),"sequence={sequence} x={x} y={y}");}}
 }
 BITMAP_STAGE.poll_reply(|_,_|0);
 // Resize and movement use the existing structural redraw path. The retained
 // image samples its original pixel coordinates and clips to the current body.
 let g=bitmap_batch::Geometry{x:0,y:0,width:48,height:48,scale:4};publish_bitmap(4,g,&vec![0xffff00ff;g.len()]);
 for (width,height)in[(200,150),(25,40),(7,7)]{SURFACES[0].w=width;SURFACES[0].h=height;
  for y in 51..h{for x in 0..w{
   let selected=FRAME_GROUPS.winner(&SURFACES,0,x,y,w,h).map(|s|s.surface_id);
   let color=composite_pixel(x,y,w,h,0x123,0);
   if selected==Some(300){let in_image=x-20<192&&y-70<192;assert_eq!(color,if in_image{0xffff00ff}else{0x123456});}else{assert_eq!(color,0x123);}
  }}
 }
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();BITMAP_STAGE=bitmap_transaction::Stage::EMPTY;
}}
unsafe fn legacy_reference_pixel(x:usize,y:usize,w:usize,h:usize,focus:u64)->u32{
 let selected=FRAME_GROUPS.winner(&SURFACES,focus,x,y,w,h).map(|s|s.surface_id);
 reference_composite_pixel_selected(x,y,w,h,0x123,focus,selected)
}
#[test]fn legacy_mutation_preserves_real_offset_count_color_wrap_and_clear_semantics(){
 for offset in [0,1,7,255,256,4095]{for count in [0,1,7,8,15]{for color in [0,0xabcdef,0xff112233]{
  let mut actual=surf(9,156);actual.text_len=3;actual.text_buf[..3].copy_from_slice(b"old");recompute_text_lines(&mut actual);let mut expected=actual;
  let args=legacy_args(offset,count,color);let packed=u64::from_le_bytes(*b"A\nB CDEF");
  legacy_text_draw(&mut actual,packed,args);legacy_draw_reference(&mut expected,packed,args);
  assert_eq!((actual.text_len,actual.text_color,actual.line_count,actual.wrap_cols),(expected.text_len,expected.text_color,expected.line_count,expected.wrap_cols));
  assert_eq!(actual.text_buf,expected.text_buf);assert_eq!(actual.line_starts,expected.line_starts);
  assert_eq!(legacy_text_clear(&mut actual),actual.text_len!=expected.text_len);assert_eq!(actual.text_len,0);assert!(actual.text_buf.iter().all(|b|*b==0));
  assert_eq!(actual.line_count,0);assert_eq!(actual.wrap_cols,TEXT_LEGACY_COLS);assert!(!legacy_text_clear(&mut actual));
 }}}
 let mut s=surf(9,156);let args=legacy_args(0,8,0xabcdef);let bytes=u64::from_le_bytes(*b"sameword");assert!(legacy_text_draw(&mut s,bytes,args));assert!(!legacy_text_draw(&mut s,bytes,args));
 assert!(legacy_text_draw(&mut s,bytes,legacy_args(0,8,0x123456)));
}
#[test]fn legacy_clear_draw_damage_matches_literal_compositor_at_each_drain_boundary(){unsafe{
 let(w,h)=(320,260);
 for focus in [201,888,152]{for hidden in [false,true]{for x in [-30,40,290]{for drain in [1,2,4]{
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(9));
  SURFACES[0]=Surface{x,y:60,w:240,h:190,..frame()};SURFACES[1]=surf(9,156);
  SURFACES[2]=Surface{x:180,y:110,w:100,h:100,..surf(8,888)};
  SURFACES[3]=Surface{x:220,y:150,w:80,h:80,..surf(3,152)};
  FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);if hidden{SURFACES[0]=SURFACE_EMPTY;}
  SURFACES[1].text_buf[..20].copy_from_slice(b"old words\nmore words");SURFACES[1].text_len=20;recompute_text_lines(&mut SURFACES[1]);
  let mut actual=vec![0;w*h];for y in 0..h{for px in 0..w{actual[y*w+px]=legacy_reference_pixel(px,y,w,h,focus);}}
  let ops=[None,Some((u64::from_le_bytes(*b"A\nlonger"),legacy_args(0,8,0xabcdef))),Some((u64::from_le_bytes(*b" words!!"),legacy_args(8,8,0xabcdef))),Some((u64::from_le_bytes(*b"A\nlonger"),legacy_args(0,8,0x345678))),None,Some((b'Z' as u64,legacy_args(0,1,0xabcdef)))];
  for batch in ops.chunks(drain){
   let mut d=damage::Damage::new();
   for &op in batch{
    let old=text_surface_damage(&SURFACES[1],w,h);
    let changed=match op{None=>legacy_text_clear(&mut SURFACES[1]),Some((packed,args))=>legacy_text_draw(&mut SURFACES[1],packed,args)};
    if changed{d.add(old,w,h);d.add(text_surface_damage(&SURFACES[1],w,h),w,h);}
   }
   FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();if let Some(r)=d.rect(){redraw_surface_damage(actual.as_mut_ptr(),w,h,r);assert_eq!(DAMAGE_ORDER,["restore","cursor","launcher","presented"]);}
   for y in 0..h{for px in 0..w{assert_eq!(actual[y*w+px],legacy_reference_pixel(px,y,w,h,focus),"focus={focus} hidden={hidden} x={x} drain={drain} px={px} y={y}");}}
  }
 }}}}
}}
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

#[test]
fn scanline_cache_matches_winner_at_every_pixel_with_focus_clipping_and_minimize(){
 let mut g=FrameGroups::new();g.register(cat(9));g.register(cat(12));
 let mut s=[SURFACE_EMPTY;MAX_SURFACES];s[0]=frame();s[1]=surf(9,156);
 s[2]=Surface{x:-30,y:40,w:111,h:160,..surf(8,888)};
 s[3]=Surface{x:190,y:65,w:140,h:95,..surf(3,152)};
 s[4]=Surface{x:200,y:80,w:199,h:150,..surf(3,153)};
 s[5]=surf(12,154);g.observe(&s);g.fit_children(&mut s);
 for minimized in [false,true]{
  s[0].active=!minimized;
  for focus in [0,201,156,153,154,888]{for y in [49,50,51,64,65,99,100,127,136,159,180,229,250,399]{
   let mut row=g.row_winners(&s,focus,y,420,410).unwrap();
   for x in 0..420{assert_eq!(row.selected(x),g.winner(&s,focus,x,y,420,410).map(|s|s.surface_id),"{focus} {x} {y} {minimized}");}
  }}
 }
 assert!(g.row_winners(&s,0,0,0,10).is_none());
 assert!(g.row_winners(&s,0,10,10,10).is_none());
}

#[path = "../servers/sexdisplay/src/damage.rs"]
mod damage;
#[test]
fn damage_union_clips_overflow_and_never_touches_silkbar() {
 let mut d=damage::Damage::new();d.add((usize::MAX,0,usize::MAX,usize::MAX),1000,800);assert_eq!(d.rect(),None);
 d.add((10,0,30,30),1000,800);assert_eq!(d.rect(),None);
 d.add((10,40,30,30),1000,800);d.add((900,750,usize::MAX,usize::MAX),1000,800);
 assert_eq!(d.rect(),Some(damage::Rect{x:10,y:51,right:1000,bottom:800}));
 let mut empty=damage::Damage::new();empty.add((0,0,1000,800),0,0);assert_eq!(empty.rect(),None);
}
#[test]
fn text_damage_matches_full_compositor_with_focus_clipping_overlay_and_hidden_parent(){unsafe{
 // Existing screen, replace only app text, then compare damage to a fresh full
 // composite pixel-for-pixel. No screenshot occupancy substitutes for glyphs.
 for focus in [201,888,152] {for hidden in [false,true] {for x in [-30,100,950] {
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(9));
  SURFACES[0]=Surface{x,..frame()};SURFACES[1]=surf(9,156);
  SURFACES[2]=Surface{x:300,y:160,w:200,h:100,color:0xabcdef,..surf(8,888)};
  SURFACES[3]=Surface{x:480,y:220,w:200,h:200,color:0xaabbcc,..surf(3,152)};
  FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
  if hidden {SURFACES[0]=SURFACE_EMPTY;}
  SURFACES[1].text_buf[..9].copy_from_slice(b"old\nwords");SURFACES[1].text_len=9;recompute_text_lines(&mut SURFACES[1]);
  let (w,h)=(1000,800);let mut actual=Vec::with_capacity(w*h);
  for py in 0..h {for px in 0..w {actual.push(composite_pixel(px,py,w,h,0x123,focus));}}
  let old_damage=text_surface_damage(&SURFACES[1],w,h);
  SURFACES[1].text_buf=[0;TEXT_BUF_CAP];SURFACES[1].text_buf[..1].copy_from_slice(b"Z");SURFACES[1].text_len=1;recompute_text_lines(&mut SURFACES[1]);
  let mut d=damage::Damage::new();d.add(old_damage,w,h);d.add(text_surface_damage(&SURFACES[1],w,h),w,h);
  FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();
  if let Some(r)=d.rect(){redraw_surface_damage(actual.as_mut_ptr(),w,h,r);assert_eq!(DAMAGE_ORDER,["restore","cursor","launcher","presented"]);}
  for py in 0..h {for px in 0..w {assert_eq!(actual[py*w+px],composite_pixel(px,py,w,h,0x123,focus),"focus={focus} hidden={hidden} x={x} px={px} py={py}");}}
 }}}
}}

// Framebuffer/overlay primitives are sentinels; damage-loop, winner and glyph
// code are extracted production bodies. Tests run serially over their globals.
const HIGH_HALF_BASE:u64=0;
const MAX_FB_W:usize=1920;
const MAX_FB_H:usize=1080;
const ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED:bool=false;
const ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED:bool=false;
static mut FOCUSED_SURFACE_ID:u64=0;
static mut DAMAGE_ORDER:Vec<&str>=Vec::new();
fn bg(_:usize,_:usize)->u32{0x123}
fn restore_cursor_under(_: *mut u32,_:usize,_:usize,_:usize){unsafe{DAMAGE_ORDER.push("restore");}}
fn draw_cursor_z_top(_: *mut u32,_:usize,_:usize,_:usize,save:bool){assert!(save);unsafe{DAMAGE_ORDER.push("cursor");}}
fn draw_launcher_panel(_: *mut u32,_:usize,_:usize,_:usize){unsafe{DAMAGE_ORDER.push("launcher");}}
fn report_app_presented(){unsafe{DAMAGE_ORDER.push("presented");}}
fn redraw_surface_area(_: *mut u32,_:usize,_:usize){panic!("unexpected full redraw");}
#[test]
fn glyph_footprint_covers_all_actual_ink_and_erases_empty_or_shorter_replacements(){
 for (x,y,w,h) in [(-30,-30,144,90),(0,0,220,150),(100,100,624,331),(950,750,624,331)] {
  for chrome in [0,SURFACE_CHROME_TOP_BAR] {
   let mut s=Surface{x,y,w,h,chrome_flags:chrome,text_len:512,text_buf:[b'A';TEXT_BUF_CAP],..SURFACE_EMPTY};
   for i in (12..512).step_by(13){s.text_buf[i]=b'\n';}recompute_text_lines(&mut s);
   let r=text_surface_damage(&s,1000,800);let mut d=damage::Damage::new();d.add(r,1000,800);
   let bounds=surface_groups::rect(&s,1000,800);
   for py in 51..800 {for px in bounds.0..bounds.0+bounds.2 {
    if surface_groups::contains(bounds,px,py) && surface_text_fg_at(px,py,&s).is_some(){
     let r=d.rect().unwrap();assert!(px>=r.x&&px<r.right&&py>=r.y&&py<r.bottom,"{x} {y} {w} {h} {px} {py}");
    }
   }}
   s.text_len=0;recompute_text_lines(&mut s);assert_eq!(text_surface_damage(&s,1000,800),(0,0,0,0));
  }
 }
 let mut s=Surface{x:8,y:86,w:624,h:706,text_len:7,text_buf:[0;TEXT_BUF_CAP],..SURFACE_EMPTY};
 s.text_buf[..7].copy_from_slice(b"a\nb\nc\nd");recompute_text_lines(&mut s);
 let (_,_,width,height)=text_surface_damage(&s,1280,800);assert_eq!((width,height),(624,72));assert!(width*height<1280*750/20);
}

#[path="../servers/sexdisplay/src/fill_damage.rs"]mod fill_damage;
#[test]fn fill_state_clamps_clears_noops_and_rejects_before_effects(){
 let mut s=surf(9,156);
 let (old,new)=fill_damage::update(&mut s,7,-99,999,999,25,0xabcdef).unwrap().unwrap();
 assert_eq!(old.w,0);assert_eq!((new.x,new.y,new.w,new.h),(0,275,400,25));assert_eq!(s.fill_count,8);
 assert_eq!(fill_damage::update(&mut s,7,0,275,400,25,0xabcdef),Ok(None));
 let before=(s.fill_count,s.fill_sw,s.fill_color);assert_eq!(fill_damage::update(&mut s,8,0,0,1,1,0),Err(()));assert_eq!((s.fill_count,s.fill_sw,s.fill_color),before);
 let (old,new)=fill_damage::update(&mut s,7,0,0,0,25,0).unwrap().unwrap();assert_eq!((old.w,new.w,s.fill_count),(400,0,0));
 assert_eq!(fill_damage::update(&mut s,7,0,0,0,0,0),Ok(None));
}
#[test]fn fill_damage_matches_every_full_compositor_pixel_for_recolor_move_and_clear(){unsafe{
 for focus in [201,888,152] {for hidden in [false,true] {for x in [-30,100,950] {
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(9));
  SURFACES[0]=Surface{x,..frame()};SURFACES[1]=surf(9,156);
  SURFACES[2]=Surface{x:300,y:160,w:200,h:100,color:0xabcdef,..surf(8,888)};
  SURFACES[3]=Surface{x:480,y:220,w:200,h:200,color:0xaabbcc,..surf(3,152)};
  FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
  if hidden {SURFACES[0]=SURFACE_EMPTY;}
  SURFACES[1].text_buf[..9].copy_from_slice(b"old\nwords");SURFACES[1].text_len=9;recompute_text_lines(&mut SURFACES[1]);
  fill_damage::update(&mut SURFACES[1],0,0,0,80,40,0x121212).unwrap();
  let(w,h)=(1000,800);let mut actual=Vec::with_capacity(w*h);
  for py in 0..h {for px in 0..w {actual.push(composite_pixel(px,py,w,h,0x123,focus));}}
  for (sx,sy,sw,sh,color) in [(0,0,80,40,0x333333),(30,50,120,120,0xabcdef),(-100,-100,60,80,0x123456),(0,0,0,0,0)]{
   let(old,new)=fill_damage::update(&mut SURFACES[1],0,sx,sy,sw,sh,color).unwrap().unwrap();
   let mut d=damage::Damage::new();d.add(old.screen(&SURFACES[1],w,h),w,h);d.add(new.screen(&SURFACES[1],w,h),w,h);
   FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();
   if let Some(r)=d.rect(){redraw_surface_damage(actual.as_mut_ptr(),w,h,r);assert_eq!(DAMAGE_ORDER,["restore","cursor","launcher","presented"]);}
   for py in 0..h {for px in 0..w {assert_eq!(actual[py*w+px],composite_pixel(px,py,w,h,0x123,focus),"focus={focus} hidden={hidden} x={x} px={px} py={py}");}}
  }
 }}}
}}

#[test]fn real_compositor_draws_title_after_tab_background_for_all_quil_incarnations(){unsafe{
 for (owner,sid) in [(3,201),(16,301),(18,307)]{
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
  SURFACES[0]=Surface{owner_pd:owner,surface_id:sid,x:100,y:100,w:400,h:300,active:true,tab_count:1,chrome_flags:1,..SURFACE_EMPTY};
  let mut ink=0;for y in 0..28{for x in 62..112{if let Some(color)=toolbar_title_fg_at(x,y,sid){ink+=1;assert_eq!(composite_pixel(x+100,y+100,1000,800,0,sid),color);}}}assert!(ink>0);
 }
}}
#[test]fn visual_family_style_does_not_replace_exact_surface_ownership(){
 for sid in [156,201,301,307]{assert_eq!(desktop_style::native_label(sid),Some("Quil"));assert_eq!(desktop_style::native_indicator(sid),Some(0x00cba6f7));}
 for(owner,sid)in[(9,156),(16,301),(18,307)]{assert_eq!(desktop_style::native_canvas(owner,sid),Some(desktop_style::NATIVE_CANVAS));}
 for(owner,sid)in[(3,307),(9,307),(18,301),(17,156),(18,201)]{assert_eq!(desktop_style::native_canvas(owner,sid),None);}
 assert_eq!(desktop_style::native_label(999),None);
}

struct SilkBar;
#[test]
fn normal_app_indicator_uses_actual_focus_and_clears_with_it(){unsafe{
 let bar=SilkBar;
 for sid in [201,301,307]{FOCUSED_SURFACE_ID=sid;for y in 22..26{for x in 155..159{assert_eq!(phase5_active_app_at(x,y,&bar),Some(0xCBA6F7));}}}
 for sid in [153,306]{FOCUSED_SURFACE_ID=sid;assert_eq!(phase5_active_app_at(155,22,&bar),Some(0x89B4FA));}
 FOCUSED_SURFACE_ID=0;assert_eq!(phase5_active_app_at(155,22,&bar),None);
 FOCUSED_SURFACE_ID=307;assert_eq!(phase5_active_app_at(159,22,&bar),None);assert_eq!(phase5_active_app_at(155,26,&bar),None);
}}

fn repaint_begin()->u64{0}
fn repaint_end(_kind:&str,_started:u64){}
fn clock_fg_at(x:usize,y:usize,_:&SilkBar)->Option<u32>{(x==0&&y==0).then_some(1)}
fn bell_badge_at(x:usize,y:usize,_:&SilkBar)->Option<u32>{(x==1&&y==0).then_some(2)}
fn phase5_tint_swatch_at(x:usize,y:usize,_:&SilkBar)->Option<u32>{(x==2&&y==0).then_some(3)}
fn phase5_palette_dot_at(x:usize,y:usize,_:&SilkBar)->Option<u32>{(x==3&&y==0).then_some(4)}
fn bar_color(_:usize,_:usize,_:&SilkBar)->u32{9}
#[test]
fn shared_full_and_incremental_strip_uses_current_focus(){unsafe{
 let bar=SilkBar;FOCUSED_SURFACE_ID=307;
 assert_eq!(top_strip_pixel(155,22,&bar),0xCBA6F7);
 FOCUSED_SURFACE_ID=306;assert_eq!(top_strip_pixel(155,22,&bar),0x89B4FA);
 FOCUSED_SURFACE_ID=0;assert_eq!(top_strip_pixel(155,22,&bar),9);
 for (x,c)in [(0,1),(1,2),(2,3),(3,4),(4,9)]{assert_eq!(top_strip_pixel(x,0,&bar),c);}
}}

// Literal image-58 two-pass shader is an independent oracle. The row cache
// must preserve both visible pixels and the exact owner-bound presentation mask.
unsafe fn assert_span_reference(w:usize,h:usize,focuses:&[u64]) {
 for &focus in focuses {for y in 0..h {
  let mut row=FRAME_GROUPS.row_winners(&SURFACES,focus,y,w,h).unwrap();
  for x in 0..w {
   let selected=FRAME_GROUPS.winner(&SURFACES,focus,x,y,w,h).map(|s|s.surface_id);
   APP_PRESENT_BITS=0;
   let expected=reference_composite_pixel_selected(x,y,w,h,0x192837,focus,selected);
   let expected_mask=APP_PRESENT_BITS;
   APP_PRESENT_BITS=0;
   let actual=composite_pixel_source(x,y,w,h,0x192837,row.source(x));
   assert_eq!((actual,APP_PRESENT_BITS),(expected,expected_mask),"focus={focus} pixel={x},{y}");
  }
 }}
}
unsafe fn span_text_fill(index:usize) {
 let s=&mut SURFACES[index];
 s.text_buf[..8].copy_from_slice(b"AZ\nold\nz");s.text_len=8;s.text_color=0xfedcba;
 s.fill_count=2;s.fill_sx[0]=0;s.fill_sy[0]=20;s.fill_sw[0]=41;s.fill_sh[0]=35;s.fill_color[0]=0x345678;
 s.fill_sx[1]=12;s.fill_sy[1]=24;s.fill_sw[1]=31;s.fill_sh[1]=16;s.fill_color[1]=0x876543;
 recompute_text_lines(s);
}
#[test]fn span_shader_matches_accepted_pixels_for_all_native_incarnations_and_pane_gaps(){unsafe{
 for (pd,frame_id,children) in [
  (9,201,&[156][..]),(12,153,&[154,160,161,162][..]),
  (16,301,&[301][..]),(17,306,&[302,303,304,305][..]),(18,307,&[307][..])
 ] {
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
  FRAME_GROUPS.register(Catalog{slot:pd as usize,target_pd:pd,task_id:pd+100,surface_id:children[0],coordinator_pd:3});
  let bound=matches!(pd,9|12|17);
  SURFACES[0]=Surface{x:20,y:55,w:250,h:150,owner_pd:if bound{3}else{pd},surface_id:frame_id,..frame()};
  if bound {for (i,&sid) in children.iter().enumerate(){SURFACES[i+1]=surf(pd,sid);}}
  FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
  for i in 0..=if bound{children.len()}else{0} {span_text_fill(i);}
  // Overlap with a real foreign standalone surface and the modal shell launcher.
  SURFACES[8]=Surface{x:150,y:90,w:115,h:100,color:0xabcdef,..surf(8,888)};
  SURFACES[9]=Surface{x:190,y:60,w:45,h:55,color:0x112233,..surf(3,152)};
  assert_span_reference(300,220,&[0,frame_id,children[0],888,152]);
  SURFACES[9].active=false;
  assert_span_reference(300,220,&[frame_id,children[0]]);
 }
}}
#[test]fn span_shader_preserves_unfitted_body_boundaries_clipping_and_tiny_frames(){unsafe{
 for (x,y,w,h) in [(-20,-30,170,140),(280,200,170,140),(20,55,15,43),(20,55,17,45),(20,55,0,0),(i32::MAX,i32::MAX,100,100)] {
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(9));
  SURFACES[0]=Surface{x,y,w,h,..frame()};
  // Deliberately crosses every body edge: child edges alone cannot describe
  // the content source at x=frame+8 and x=frame+width-8.
  SURFACES[1]=Surface{x:-40,y:0,w:1000,h:1000,color:0x934567,..surf(9,156)};
  span_text_fill(1);FRAME_GROUPS.observe(&SURFACES);
  assert_span_reference(300,220,&[0,201,156]);
 }
}}
#[test]fn span_cache_is_rebuilt_after_hide_restore_slot_reuse_and_full_capacity(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(12));
 SURFACES[0]=Surface{x:5,y:53,w:180,h:135,..surf(3,153)};
 for (i,sid) in [154,160,161,162].into_iter().enumerate(){SURFACES[i+1]=surf(12,sid);}
 for i in 5..MAX_SURFACES {SURFACES[i]=Surface{x:(i*17) as i32,y:50+(i*7) as i32,w:90,h:80,color:0x102030+i as u32,..surf(8,800+i as u64)};}
 FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
 assert_span_reference(320,240,&[0,153,154,815]);
 SURFACES[0]=SURFACE_EMPTY;assert_span_reference(320,240,&[153,154]);
 SURFACES[0]=Surface{x:30,y:85,w:160,h:140,..surf(8,999)};
 assert_span_reference(320,240,&[999,154]);
 SURFACES[0]=Surface{x:60,y:70,w:220,h:160,..surf(3,153)};
 FRAME_GROUPS.fit_children(&mut SURFACES);assert_span_reference(320,240,&[153,154]);
 let oversized=[SURFACE_EMPTY;MAX_SURFACES+1];
 assert!(FRAME_GROUPS.row_winners(&oversized,0,60,320,240).is_none());
 // Fixed storage remains bounded even when every active rectangle contributes
 // both outer edges; no dynamic allocation or pointer crosses an IPC boundary.
 assert!(core::mem::size_of::<surface_groups::RowWinners<'_>>()<=4096);
}}

#[test]fn actual_selection_cells_cover_spaces_and_spacing_skip_newlines_preserve_ink(){
 let mut s=Surface{x:20,y:70,w:200,h:150,text_len:5,text_color:0xfffafafa,selection_start:0,selection_end:5,..SURFACE_EMPTY};
 s.text_buf[..5].copy_from_slice(b"A \n B");recompute_text_lines(&mut s);
 let scale=surface_text_scale(&s);let tx=20+desktop_style::INSET as usize;let ty=70+desktop_style::TEXT_TOP as usize;
 // Full selected cells extend through spacing and baseline padding.
 for row in[0,1]{for col in[0,1]{assert_eq!(surface_text_fg_at(tx+col*6*scale+5*scale,ty+row*9*scale+8*scale,&s),Some(0xff36577c));}}
 assert_eq!(surface_text_fg_at(tx+2*6*scale,ty,&s),None); // newline cell
 assert_eq!(surface_text_fg_at(tx+2*6*scale,ty+9*scale,&s),None); // past line end
 let mut ink=0;for y in ty..ty+7*scale{for x in tx..tx+5*scale{if surface_text_fg_at(x,y,&s)==Some(0xfffafafa){ink+=1;}}}assert!(ink>0);
 s.erase_selection();assert_eq!((s.selection_start,s.selection_end),(0,0));
 assert_eq!(surface_text_fg_at(tx+6*scale,ty,&s),None);
 // Legacy mutation also erases former selection so it cannot drift over bytes.
 s.selection_end=5;assert!(legacy_text_clear(&mut s));assert_eq!((s.selection_start,s.selection_end),(0,0));
 s.selection_end=1;assert!(legacy_text_draw(&mut s,b'X' as u64,legacy_args(0,1,0xfffafafa)));assert_eq!(s.selection_end,0);
}
#[test]fn selection_only_changes_match_full_and_incremental_compositor_and_clear_old_pixels(){unsafe{
 for focus in[0,300]{let(w,h)=(320,260);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();BITMAP_STAGE=bitmap_transaction::Stage::EMPTY;
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,color:0x123456,text_len:16,text_color:0xfffafafa,..surf(15,300)};
 SURFACES[0].text_buf[..16].copy_from_slice(b"alpha beta\ngamma");recompute_text_lines(&mut SURFACES[0]);
 let mut actual=vec![0;w*h];for y in 0..h{for x in 0..w{actual[y*w+x]=composite_pixel(x,y,w,h,bg(y,h),focus);}}
 for(start,end)in[(0,16),(3,8),(0,0)]{
  SURFACES[0].selection_start=start;SURFACES[0].selection_end=end;
  let mut d=damage::Damage::new();d.add(text_surface_damage(&SURFACES[0],w,h),w,h);
  FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();redraw_surface_damage(actual.as_mut_ptr(),w,h,d.rect().unwrap());
  assert_eq!(DAMAGE_ORDER,["restore","cursor","launcher","presented"]);
  for y in 0..h{for x in 0..w{assert_eq!(actual[y*w+x],composite_pixel(x,y,w,h,bg(y,h),focus),"focus={focus} selection={start}..{end} x={x} y={y}");}}
 }
 }
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}

fn spindle_catalog(pd:u32)->Catalog {
 assert!(matches!(pd,12|17));
 Catalog{slot:if pd==12{39}else{50},target_pd:pd,task_id:pd,surface_id:if pd==12{154}else{302},coordinator_pd:3}
}
fn spindle_ids(pd:u32)->[u64;4] {if pd==12{[154,160,161,162]}else{[302,303,304,305]}}
fn spindle_children(pd:u32)->[Surface;5] {
 let mut surfaces=[SURFACE_EMPTY;5];
 for(index,sid)in spindle_ids(pd).into_iter().enumerate(){
  surfaces[index+1]=Surface{x:900+(index%2)as i32*200,y:500+(index/2)as i32*260,w:200,h:240,color:0xa10000+index as u32,
   text_len:1,text_buf:{let mut bytes=[0;TEXT_BUF_CAP];bytes[0]=b'A'+index as u8;bytes},selection_start:0,selection_end:1,..surf(pd,sid)};
 }
 surfaces
}
#[test]fn exact_spindle_children_never_become_standalone_before_parent_at_either_profile(){
 for(w,h)in[(1280,800),(1024,768)] {for pd in[12,17] {
  let catalog=spindle_catalog(pd);let parent=catalog.frame_surface_id().unwrap();let mut g=FrameGroups::new();g.register(catalog);
  let mut surfaces=spindle_children(pd);let before=surfaces;g.observe(&surfaces);g.fit_children(&mut surfaces);
  for(index,child)in surfaces.iter().enumerate().skip(1){
   assert!(!g.independent(child));assert_eq!(g.awaiting_parent(child),Some(parent));
   assert_eq!((child.x,child.y,child.w,child.h),(before[index].x,before[index].y,before[index].w,before[index].h));
   assert_eq!(child.text_buf,before[index].text_buf);assert_eq!(child.selection_end,1);
   let(x,y,sw,sh)=surface_groups::rect(child,w,h);
   for focus in[0,parent,child.surface_id]{
    for(px,py)in[(x,y),(x+sw-1,y+sh-1)]{
     assert!(g.winner(&surfaces,focus,px,py,w,h).is_none(),"pd={pd} pane={} focus={focus} framebuffer={w}x{h}",child.surface_id);
     let mut row=g.row_winners(&surfaces,focus,py,w,h).unwrap();assert!(row.source(px).is_none());
    }
   }
  }
  // A parent record alone is not an observation. Before observe, it supplies
  // its own body only; children remain unfit and cannot supply its content.
  surfaces[0]=Surface{x:10,y:60,w:600,h:500,..surf(3,parent)};
  assert_eq!(g.content(&surfaces[0],&surfaces,30,110,w,h).surface_id,parent);
  assert_eq!(g.awaiting_parent(&surfaces[1]),Some(parent));
 }}
}
#[test]fn spindle_parent_gate_does_not_reclassify_unknown_catalogs_wrong_owners_or_quil_early_surfaces(){
 for(w,h)in[(1280,800),(1024,768)]{for pd in[12,17]{
  let catalog=spindle_catalog(pd);let frame_id=catalog.frame_surface_id().unwrap();let mut g=FrameGroups::new();g.register(catalog);
  let mut surfaces=spindle_children(pd);surfaces[0]=surf(99,frame_id);g.observe(&surfaces);g.fit_children(&mut surfaces);
  for child in&surfaces[1..]{assert_eq!(g.awaiting_parent(child),Some(frame_id));assert!(!g.independent(child));}
  assert!(g.independent(&surfaces[0]));assert_eq!(g.content(&surfaces[0],&surfaces,120,150,w,h).owner_pd,99);
  for sid in spindle_ids(pd){let foreign=surf(99,sid);assert!(g.independent(&foreign));assert_eq!(g.awaiting_parent(&foreign),None);}
  let unknown=surf(pd,999);assert!(g.independent(&unknown));assert_eq!(g.awaiting_parent(&unknown),None);
  let absent=FrameGroups::new();assert!(absent.independent(&surf(pd,catalog.surface_id)));
  let mut invalid=FrameGroups::new();invalid.register(Catalog{surface_id:999,..catalog});assert!(invalid.independent(&surf(pd,catalog.surface_id)));
  surfaces[0]=Surface{active:false,..surf(3,frame_id)};g.observe(&surfaces);assert_eq!(g.awaiting_parent(&surfaces[1]),Some(frame_id));
  let mut quil=FrameGroups::new();quil.register(cat(9));assert!(quil.independent(&surf(9,156)));assert_eq!(quil.awaiting_parent(&surf(9,156)),None);
 }}
}
#[test]fn spindle_late_parent_hide_slot_reuse_and_recreation_keep_sticky_binding_and_inline_content(){
 for(w,h)in[(1280,800),(1024,768)]{for pd in[12,17]{
  let catalog=spindle_catalog(pd);let parent=catalog.frame_surface_id().unwrap();let mut g=FrameGroups::new();g.register(catalog);
  let mut surfaces=spindle_children(pd);let before=surfaces;
  for pass in 0..2 {
   surfaces[0]=Surface{x:20+pass*40,y:60+pass*20,w:w as u32-100,h:h as u32-150,..surf(3,parent)};
   g.observe(&surfaces);g.fit_children(&mut surfaces);let layout=spindle_view::Layout::new(surfaces[0].w,surfaces[0].h);
   for(index,child)in surfaces.iter().enumerate().skip(1){
    let pane=layout.panes[index-1];assert_eq!((child.x,child.y,child.w,child.h),(surfaces[0].x+pane.x as i32,surfaces[0].y+pane.y as i32,pane.w,pane.h));
    assert_eq!(child.text_buf,before[index].text_buf);assert_eq!(child.color,before[index].color);assert_eq!((child.selection_start,child.selection_end),(0,1));
    assert!(!g.independent(child));assert_eq!(g.awaiting_parent(child),None);
    let(px,py)=(child.x as usize+child.w as usize/2,child.y as usize+child.h as usize/2);
    let winner=g.winner(&surfaces,child.surface_id,px,py,w,h).unwrap();assert_eq!(winner.surface_id,parent);assert_eq!(g.content(winner,&surfaces,px,py,w,h).surface_id,child.surface_id);
   }
   surfaces[0]=SURFACE_EMPTY;g.observe(&surfaces);
   for child in&surfaces[1..]{assert!(!g.independent(child));assert!(g.winner(&surfaces,child.surface_id,child.x as usize+10,child.y as usize+10,w,h).is_none());}
   surfaces[0]=surf(99,777);g.observe(&surfaces);
   assert!(surfaces[1..].iter().all(|child|!g.independent(child)));
  }
 }}
}
#[test]fn actual_scalar_and_scanline_compositors_erase_every_preparent_spindle_edge_sliver_without_present_bits(){unsafe{
 for(w,h)in[(1280,800),(1024,768)]{for pd in[12,17]{for sid in spindle_ids(pd){
  for(x,y)in[(w as i32-4,h as i32-3),(w as i32+64,h as i32+64)]{
   SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(spindle_catalog(pd));
   SURFACES[0]=Surface{x,y,w:200,h:200,text_len:1,text_buf:{let mut b=[0;TEXT_BUF_CAP];b[0]=b'X';b},..surf(pd,sid)};recompute_text_lines(&mut SURFACES[0]);
   FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
   let(sx,sy,sw,sh)=surface_groups::rect(&SURFACES[0],w,h);
   if x>=w as i32{assert_eq!((sw,sh),(1,1));}
   for focus in[0,sid,spindle_catalog(pd).frame_surface_id().unwrap()]{for py in sy..sy+sh{
    let mut row=FRAME_GROUPS.row_winners(&SURFACES,focus,py,w,h).unwrap();
    for px in sx..sx+sw{
     APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,focus),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);
     let source=row.source(px);assert!(source.is_none());assert_eq!(composite_pixel_source(px,py,w,h,0xabcdef,source),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);
    }
   }}
  }
 }}}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}
#[test]fn actual_late_parent_pixels_use_exact_panes_and_stop_immediately_when_parent_hides(){unsafe{
 for(w,h)in[(1280,800),(1024,768)]{for pd in[12,17]{
  let catalog=spindle_catalog(pd);let parent=catalog.frame_surface_id().unwrap();let mut scene=spindle_children(pd);
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];SURFACES[..5].copy_from_slice(&scene);FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(catalog);
  scene[0]=Surface{x:20,y:60,w:w as u32-100,h:h as u32-150,chrome_flags:SURFACE_CHROME_TOP_BAR,..surf(3,parent)};SURFACES[0]=scene[0];
  FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
  for slot in 1..5{recompute_text_lines(&mut SURFACES[slot]);let child=SURFACES[slot];let(px,py)=(child.x as usize+child.w as usize/2,child.y as usize+child.h as usize/2);
   for focus in[0,parent,child.surface_id]{
    APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,focus),child.color);assert_eq!(APP_PRESENT_BITS,native_pixel_bit(pd,child.surface_id));
    let mut row=FRAME_GROUPS.row_winners(&SURFACES,focus,py,w,h).unwrap();let source=row.source(px).unwrap();assert_eq!(source.surface.surface_id,parent);assert_eq!(source.content.surface_id,child.surface_id);
    APP_PRESENT_BITS=0;assert_eq!(composite_pixel_source(px,py,w,h,0xabcdef,Some(source)),child.color);assert_eq!(APP_PRESENT_BITS,native_pixel_bit(pd,child.surface_id));
   }
   // Wrong-owner content cannot borrow the held child group or native bits.
   SURFACES[slot].owner_pd=99;APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,parent),SURFACES[0].color);assert_eq!(APP_PRESENT_BITS,0);SURFACES[slot]=child;
  }
  SURFACES[0].active=false;
  for child in SURFACES[1..5].iter(){let(px,py)=(child.x as usize+10,child.y as usize+10);APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,child.surface_id),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);}
 }}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}
