// SPDX-License-Identifier: GPL-3.0-only
//! Actual compositor body extracted by gate. Glass shaders are deterministic
//! sentinels; surface selection, clipping, fill, font and presentation bits are real.
#![allow(dead_code,unused_variables,unused_unsafe,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_stack.rs"]pub mod desktop_stack;
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../servers/sexdisplay/src/text_transaction.rs"]mod text_transaction;
static mut TEXT_STAGES:[text_transaction::Stage;MAX_SURFACES]=[text_transaction::Stage::EMPTY;MAX_SURFACES];
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"]mod bitmap_binding;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod bitmap_transaction;
#[path="../servers/sexdisplay/src/bitmap_stores.rs"]mod bitmap_stores;
static mut BITMAP_STORES:bitmap_stores::Stores<{app_catalog::count(app_catalog::App::Browser)}>=bitmap_stores::Stores::UNBOUND;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/desktop_look.rs"]pub mod desktop_look;
#[path="../crates/sex-pdx/src/notify.rs"]pub mod notify;
pub const SLOT_BELL:u64=12;
pub fn pdx_call(_:u64,_:u64,_:u64,_:u64,_:u64)->(u64,u64){unreachable!("display never posts")}
#[path="../servers/sexdisplay/src/glyphs.rs"]mod glyphs;
#[path="../crates/sex-pdx/src/pixel_region.rs"]pub mod pixel_region;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] mod app_catalog;
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"] mod surface_lifecycle;
#[path="../servers/sexdisplay/src/surface_groups.rs"] mod surface_groups;
use surface_lifecycle::*;
use surface_groups::FrameGroups;
macro_rules! serial_println { ($($a:tt)*) => {} }
static mut SURFACES:[Surface;MAX_SURFACES]=[SURFACE_EMPTY;MAX_SURFACES];
static mut FRAME_GROUPS:FrameGroups=FrameGroups::new();
static mut APP_PRESENT_BITS:u64=0;
static RENDER_FRAME_COUNTER:core::sync::atomic::AtomicU64=core::sync::atomic::AtomicU64::new(0);
static mut DISPLAY_TOKENS:RenderTokensV1=RenderTokensV1{frame_top_bar_color:0x111111,active_tab_color:0x222222,inactive_tab_color:0x333333,close_light_color:0x444444,minimize_light_color:0x555555,zoom_light_color:0x666666,frame_rim_color:0x777777,..DEFAULT_RENDER_TOKENS};
fn hot_log()->bool{false}
static mut TEST_GLASS_BACKGROUND:Option<u32>=None;
static mut TEST_BLEND_BACKGROUND:Option<u32>=None;
fn glass_over_bg(c:u32,_x:usize,_y:usize,_a:u8)->u32{unsafe{TEST_GLASS_BACKGROUND.unwrap_or(c)}}
fn alpha_blend_xrgb_over_xrgb(c:u32,_b:u32,_a:u8)->u32{unsafe{TEST_BLEND_BACKGROUND.unwrap_or(c)}}
fn pulse_alpha(base:u8,_a:u8,_f:u64,_p:u64)->u8{base}
include!(env!("DISPLAY_COMPOSITION_SOURCE"));
include!("fixtures/compositor_reference.rs");
include!("fixtures/rounded_reference.rs");
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
  // Every grid scale uses Geist Mono coverage; only unavailable glyphs fall back.
  let scalar=sex_pdx::text_encoding::cell(&s.text_buf[..1],0).map(|c|c.0);
  let geist=scalar.and_then(|c|glyphs::mono_cell(c,6*scale,9*scale));
  for y in 0..9*scale {for x in 0..6*scale {
   let rows=if (32..=126).contains(&byte){FONT_ASCII_5X7[(byte-32)as usize]}else if byte==9{[0;7]}else{[31,17,21,21,21,17,31]};
   let ink=x/scale<5&&y/scale<7&&rows[y/scale]&(1<<(4-x/scale))!=0;
   let back=if selected{Some(0xff36577c)}else{None};
   let expected=if byte==b'\n'{None}else if let Some(cell)=geist{
    let a=cell[y*6*scale+x];if a==0{back}else{Some(coverage_blend(0xfedcba,back.unwrap_or(s.color),a))}
   }else if ink{Some(0xfedcba)}else{back};
   assert_eq!(surface_text_fg_at(8+x,62+y,&s),expected,"w={width} byte={byte} selected={selected} x={x} y={y}");
  }}
  assert_eq!(s.text_buf[0],byte);assert_eq!((s.selection_start,s.selection_end),(0,if selected{1}else{0}));
 }}}
}
#[test]fn actual_toolbar_titles_keep_lowercase_at_natural_13px_metrics(){
 for (sid,title)in[(200,&b"Linen"[..]),(201,&b"Quil"[..]),(300,&b"Browser"[..]),(306,&b"Spindle"[..]),(308,&b"Quil"[..]),(309,&b"Quil"[..]),(310,&b"Quil"[..]),(311,&b"Quil"[..]),(312,&b"Quil"[..])]{
  let end=glyphs::ui_width(title,1).unwrap();let mut count=0;
  for y in 0..26{for x in 81..85+end+4{
   let alpha=if(5..21).contains(&y){glyphs::ui_coverage(title,x as i32-85,y-5,1).unwrap()}else{0};
   let expected=(alpha!=0).then(||coverage_blend(desktop_style::contrast_ink(0),0,alpha));
   assert_eq!(toolbar_title_fg_at(x,y,sid,0),expected);count+=expected.is_some()as usize;
  }}assert!(count>60);
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
// HOST fixture reset only: production initializes static UNBOUND backing once
// from its held query. No packet or compositor operation supplies a binding.
unsafe fn reset_bitmap_stage(){
 for stage in &mut TEXT_STAGES{stage.reset();}
 BITMAP_STORES=bitmap_stores::Stores::UNBOUND;
 let catalog=app_lifecycle::CatalogInfo{target_pd:15,task_id:15,surface_id:300,coordinator_pd:3};
 assert!(BITMAP_STORES.initialize_once(bitmap_binding::Binding::from_catalog(40,catalog).unwrap()));
}
unsafe fn publish_bitmap(sequence:u32,g:bitmap_batch::Geometry,pixels:&[u32])->bitmap_transaction::Change{
 use bitmap_batch::Request;
 assert!(BITMAP_STORES.receive(15,15,300,Request::Begin{sequence,geometry:g}).is_none());
 for(offset,pair)in pixels.chunks(2).enumerate(){let mut data=[0;2];data[..pair.len()].copy_from_slice(pair);
  assert!(BITMAP_STORES.receive(15,15,300,Request::Chunk{sequence,offset:offset*2,count:pair.len(),pixels:data}).is_none());
 }
 let change=BITMAP_STORES.receive(15,15,300,Request::Commit{sequence,hash:bitmap_batch::hash(pixels)}).unwrap();
 // This pixel fixture models an explicit completion boundary; actual Main
 // Full publication/SceneLease/receipt ordering is tested by display_scene.
 BITMAP_STORES.release_after_publication();
 BITMAP_STORES.poll_reply(|owner,word|{assert_eq!(owner,15);assert_eq!(bitmap_batch::Applied::decode(1,word).unwrap().sequence,sequence);0});change
}
#[test]fn actual_browser_bitmap_blends_over_fill_under_text_in_both_compositor_branches(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{chrome_flags:SURFACE_CHROME_TOP_BAR,..surf(15,300)};
 fill_damage::update(&mut SURFACES[0],0,0,0,400,300,0x204060).unwrap();
 let before=[composite_pixel(120,120,600,500,0,0, &capture_scene(600,500,0)),composite_pixel(120,120,600,500,0,300, &capture_scene(600,500,300))];
 let g=bitmap_batch::Geometry{x:0,y:0,width:48,height:48,scale:4};
 publish_bitmap(1,g,&vec![0x80ff0000;g.len()]);
 for(index,focus)in[0,300].into_iter().enumerate(){
  assert_eq!(composite_pixel(170,180,600,500,0,focus, &capture_scene(600,500,focus)),bitmap_transaction::blend(0x80ff0000,0x204060));
  // The same image covers this local coordinate but cannot overwrite toolbar chrome.
  assert_eq!(composite_pixel(120,120,600,500,0,focus, &capture_scene(600,500,focus)),before[index]);
 }
 SURFACES[0].text_buf[0]=b'A';SURFACES[0].text_len=1;SURFACES[0].text_color=0xfedcba;recompute_text_lines(&mut SURFACES[0]);
 let (ax,ay)=solid_a();let (px,py)=(SURFACES[0].x as usize+8+ax,SURFACES[0].y as usize+FRAME_TOP_BAR_HEIGHT_PX+8+ay);
 for focus in[0,300]{assert_eq!(surface_text_fg_at(px,py,&SURFACES[0]),Some(0xfedcba));assert_eq!(composite_pixel(px,py,600,500,0,focus, &capture_scene(600,500,focus)),0xfedcba);}
 // Actual surface ownership and actual winner selection both precede sampling.
 SURFACES[0].owner_pd=9;assert_eq!(composite_pixel(170,180,600,500,0,0, &capture_scene(600,500,0)),0x204060);SURFACES[0].owner_pd=15;
 SURFACES[1]=Surface{color:0xabcdef,..surf(8,888)};assert_eq!(composite_pixel(170,180,600,500,0,888, &capture_scene(600,500,888)),0xabcdef);
 SURFACES[1]=Surface{color:0x987654,..surf(3,152)};assert_eq!(composite_pixel(170,180,600,500,0,300, &capture_scene(600,500,300)),0x987654);
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}
#[test]fn actual_bitmap_move_clear_and_resize_damage_match_full_compositor_pixels(){unsafe{
 let(w,h)=(320,260);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,..surf(15,300)};
 let a=bitmap_batch::Geometry{x:10,y:20,width:40,height:20,scale:1};publish_bitmap(1,a,&vec![0x8000ff00;a.len()]);
 let mut framebuffer=vec![0;w*h];let pixel_scene_1=capture_scene(w,h,0);
 for y in 0..h{for x in 0..w{framebuffer[y*w+x]=composite_pixel(x,y,w,h,bg(y,h),0, &pixel_scene_1);}}
 for sequence in 2..=3{
  let change=if sequence==2{let b=bitmap_batch::Geometry{x:120,y:40,width:20,height:30,scale:2};publish_bitmap(sequence,b,&vec![0xffaa0011;b.len()])}
   else{BITMAP_STORES.receive(15,15,300,bitmap_batch::Request::Clear{sequence}).unwrap()};
  let mut d=damage::Damage::new(28);d.add(bitmap_surface_damage(&SURFACES[0],change.old,w,h),w,h);d.add(bitmap_surface_damage(&SURFACES[0],change.new,w,h),w,h);
  FOCUSED_SURFACE_ID=0;DAMAGE_ORDER.clear();redraw_surface_damage(framebuffer.as_mut_ptr(),w,h,d.rect().unwrap());
  assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);
  let pixel_scene_2=capture_scene(w,h,0);
 for y in 0..h{for x in 0..w{assert_eq!(framebuffer[y*w+x],composite_pixel(x,y,w,h,bg(y,h),0, &pixel_scene_2),"sequence={sequence} x={x} y={y}");}}
 }
 BITMAP_STORES.release_after_publication();BITMAP_STORES.poll_reply(|_,_|0);
 // Resize and movement use the existing structural redraw path. The retained
 // image samples its original pixel coordinates and clips to the current body.
 let g=bitmap_batch::Geometry{x:0,y:0,width:48,height:48,scale:4};publish_bitmap(4,g,&vec![0xffff00ff;g.len()]);
 for (width,height)in[(200,150),(25,40),(7,7)]{SURFACES[0].w=width;SURFACES[0].h=height;
  let pixel_scene_3=capture_scene(w,h,0);
 for y in 51..h{for x in 0..w{
   let selected=FRAME_GROUPS.winner(&SURFACES,0,x,y,w,h).map(|s|s.surface_id);
   let color=composite_pixel(x,y,w,h,0x123,0, &pixel_scene_3);
   if selected==Some(300){let in_image=x-20<192&&y-70<192;let plain=if in_image{0xffff00ff}else{0x123456};
    assert_eq!(color,reference_round_blend(plain,0x123,reference_round_alpha(&SURFACES[0],1,x,y)));}else{assert_eq!(color,0x123);}
  }}
 }
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
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
   let mut d=damage::Damage::new(28);
   for &op in batch{
    let old=text_surface_damage(&SURFACES[1],w,h);
    let changed=match op{None=>legacy_text_clear(&mut SURFACES[1]),Some((packed,args))=>legacy_text_draw(&mut SURFACES[1],packed,args)};
    if changed{d.add(old,w,h);d.add(text_surface_damage(&SURFACES[1],w,h),w,h);}
   }
   FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();if let Some(r)=d.rect(){redraw_surface_damage(actual.as_mut_ptr(),w,h,r);assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);}
   for y in 0..h{for px in 0..w{assert_eq!(actual[y*w+px],legacy_reference_pixel(px,y,w,h,focus),"focus={focus} hidden={hidden} x={x} drain={drain} px={px} y={y}");}}
  }
 }}}}
}}
fn cat(pd:u32)->Catalog{Catalog{slot:pd as usize,target_pd:pd,task_id:pd,surface_id:if pd==9{156}else{154},coordinator_pd:3}}
/// Every text scale uses Geist Mono where available: its coverage over `back`.
/// None means the 5x7 font applies.
fn geist_pixel(scalar:u32,scale:usize,x:usize,y:usize,text:u32,back:u32)->Option<u32>{
 let cell=glyphs::mono_cell(scalar,6*scale,9*scale)?;let a=cell[y*6*scale+x];
 Some(if a==0{back}else{coverage_blend(text,back,a)})
}
/// A pixel Geist covers fully in 'A', relative to its cell: drawn exactly in the text colour.
fn solid_a()->(usize,usize){let c=glyphs::mono_cell(b'A' as u32,12,18).unwrap();let i=c.iter().position(|a|*a==255).unwrap();(i%12,i/12)}
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
 assert_eq!((s[1].x,s[1].y,s[1].w,s[1].h),(108,134,384,258));assert!(s[2..].iter().all(|c|c.h==0));
 for c in &s[1..]{assert!(c.x>=108&&c.y>=134);assert!(c.x as u32+c.w<=492);assert!(c.y as u32+c.h<=392);}
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
 for surface in&mut s{surface.text_len=2;surface.text_buf[..2].copy_from_slice("é".as_bytes());recompute_text_lines(surface);assert_eq!(surface.cell_count,1);}
 assert_eq!(l.prepare(0,12,12,&mut s,&mut refs),Some(5));
 for surface in&s{assert_eq!(surface.cell_count,0);assert!(surface.cell_starts.iter().chain(&surface.line_cells).chain(&surface.line_starts).all(|&x|x==0));}
 for sid in[153,154,160,161,162]{assert!(!l.sid_live(sid));assert_eq!(l.admit(&s,3,sid),Admission::Denied);assert_eq!(l.admit(&s,12,sid),Admission::Denied);}
 assert_eq!(l.admit(&s,9,999),Admission::Vacant(0,9));
}
#[test]fn actual_compositor_keeps_child_color_text_parent_chrome_and_exact_pixel_proof(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(cat(9));
 SURFACES[0]=surf(9,156);SURFACES[1]=frame();FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
 SURFACES[0].text_len=1;SURFACES[0].text_buf[0]=b'A';SURFACES[0].text_color=0xfedcba;recompute_text_lines(&mut SURFACES[0]);
 for focus in[0,201,156]{APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0,focus, &capture_scene(1000,800,focus)),0x123456);assert_eq!(APP_PRESENT_BITS,1);
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(120,125,1000,800,0,focus, &capture_scene(1000,800,focus)),FRAME_TOP_BAR_DIVIDER_COLOR);assert_eq!(APP_PRESENT_BITS,0);
 // A top-row glyph foreground uses actual scaled font coordinates.
 let (ax,ay)=solid_a();let (px,py)=(SURFACES[0].x as usize+8+ax,SURFACES[0].y as usize+12+ay);
 assert_eq!(composite_pixel(px,py,1000,800,0,focus, &capture_scene(1000,800,focus)),0xfedcba);}
 SURFACES[1]=SURFACE_EMPTY;APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0xabcdef,156, &capture_scene(1000,800,156)),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);
 SURFACES[1]=frame();FRAME_GROUPS.observe(&SURFACES);assert_eq!(composite_pixel(150,180,1000,800,0,201, &capture_scene(1000,800,201)),0x123456);
 // Another focused window occludes children: no false presented bit survives.
 SURFACES[2]=surf(8,888);APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0,888, &capture_scene(1000,800,888)),0x123456);assert_eq!(APP_PRESENT_BITS,0);
 // Modal launcher beats the focused app without producing native-app bits.
 SURFACES[3]=Surface{color:0xaabbcc,..surf(3,152)};
 SURFACES[3].text_len=1;SURFACES[3].text_buf[0]=b'A';SURFACES[3].text_color=0xf0f8ff;recompute_text_lines(&mut SURFACES[3]);
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(150,180,1000,800,0,888, &capture_scene(1000,800,888)),0xaabbcc);assert_eq!(APP_PRESENT_BITS,0);
 let (ax,ay)=solid_a();assert_eq!(composite_pixel(108+ax,112+ay,1000,800,0,888, &capture_scene(1000,800,888)),0xf0f8ff);
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
   if content.surface_id==156{
    let(lx,ly)=(px as i64-x as i64,py as i64-y as i64);
    assert!(lx>=8&&lx< w.saturating_sub(8)as i64);assert!(ly>=34&&ly<h.saturating_sub(8)as i64);
    assert!(px<1000&&py<800);
   }
  }}
 }
}
#[test]fn frame_slot_reuse_does_not_unbind_children_and_grid_restore_preserves_payloads(){
 let mut g=FrameGroups::new();g.register(cat(12));let mut s=[surf(3,153),surf(12,154),surf(12,160),surf(12,161),surf(12,162)];g.observe(&s);g.fit_children(&mut s);
 for(i,c)in s[1..].iter_mut().enumerate(){c.text_buf[0]=b'A'+i as u8;c.text_len=1;c.color=i as u32+1;}
 s[0]=surf(8,777);assert!(s[1..].iter().all(|s|!g.independent(s)));
 s[0]=Surface{x:300,y:200,w:600,h:500,..surf(3,153)};g.observe(&s);g.fit_children(&mut s);
 for(i,c)in s[1..].iter().enumerate(){assert_eq!(c.text_buf[0],b'A'+i as u8);assert_eq!(c.color,i as u32+1);assert!(c.x>=308&&c.y>=234);}
}
#[test]fn fresh_spindle_grid_uses_distinct_owner_frame_and_exact_quadrants(){
 let c=Catalog{slot:50,target_pd:17,task_id:17,surface_id:302,coordinator_pd:3};let mut g=FrameGroups::new();g.register(c);
 let mut s=[surf(3,306),surf(17,302),surf(17,303),surf(17,304),surf(17,305)];g.observe(&s);g.fit_children(&mut s);
 for (i,expected) in [(108,134)].into_iter().enumerate(){let child=&s[i+1];assert_eq!((child.x,child.y),expected);
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
 let mut d=damage::Damage::new(28);d.add((usize::MAX,0,usize::MAX,usize::MAX),1000,800);assert_eq!(d.rect(),None);
 d.add((10,0,30,28),1000,800);assert_eq!(d.rect(),None);
 d.add((10,20,30,50),1000,800);d.add((900,750,usize::MAX,usize::MAX),1000,800);
 assert_eq!(d.rect(),Some(damage::Rect{x:10,y:28,right:1000,bottom:800}));
 let mut empty=damage::Damage::new(28);empty.add((0,0,1000,800),0,0);assert_eq!(empty.rect(),None);
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
  let pixel_scene_4=capture_scene(w,h,focus);
  for py in 0..h {for px in 0..w {actual.push(composite_pixel(px,py,w,h,0x123,focus, &pixel_scene_4));}}
  let old_damage=text_surface_damage(&SURFACES[1],w,h);
  SURFACES[1].text_buf=[0;TEXT_BUF_CAP];SURFACES[1].text_buf[..1].copy_from_slice(b"Z");SURFACES[1].text_len=1;recompute_text_lines(&mut SURFACES[1]);
  let mut d=damage::Damage::new(28);d.add(old_damage,w,h);d.add(text_surface_damage(&SURFACES[1],w,h),w,h);
  FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();
  if let Some(r)=d.rect(){redraw_surface_damage(actual.as_mut_ptr(),w,h,r);assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);}
  let pixel_scene_4=capture_scene(w,h,focus);
  for py in 0..h {for px in 0..w {assert_eq!(actual[py*w+px],composite_pixel(px,py,w,h,0x123,focus, &pixel_scene_4),"focus={focus} hidden={hidden} x={x} px={px} py={py}");}}
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
// Cursor save-under is a declared callback boundary in this pixel fixture.
// Its sentinel catches entry during pure Settle; actual Main cursor storage and
// publication effects are tested separately by the frame-output/scene gates.
static mut CURSOR_UNDERLAY_SENTINEL:[u32;4]=[0;4];
fn restore_cursor_under(_: *mut u32,_:usize,_:usize,_:usize,_:desktop_style::chrome::Metrics){unsafe{DAMAGE_ORDER.push("restore");CURSOR_UNDERLAY_SENTINEL[0]=0;}}
fn draw_cursor_z_top(_: *mut u32,_:usize,_:usize,_:usize,save:bool,_:&scene::SceneSnapshot){assert!(save);unsafe{DAMAGE_ORDER.push("cursor");CURSOR_UNDERLAY_SENTINEL[1]=0;}}
fn draw_launcher_panel(_: *mut u32,_:usize,_:usize,_:usize,_:&scene::SceneSnapshot){unsafe{DAMAGE_ORDER.push("launcher");}}
fn report_app_presented(){unsafe{DAMAGE_ORDER.push("presented");}}
#[path="../servers/sexdisplay/src/paint_batch.rs"]mod paint_batch;
use paint_batch::{Content,Plan};
static mut ATLAS_CALLS:usize=0;
fn draw_atlas_cards_pass(_:*mut u32,_:usize,_:usize,_:usize,_:&scene::SceneSnapshot){unsafe{ATLAS_CALLS+=1;}}
fn draw_atlas_frame_previews_pass(_:*mut u32,_:usize,_:usize,_:usize,_:&scene::SceneSnapshot){unsafe{ATLAS_CALLS+=1;}}
fn civil_clock_caption(_:&silkbar_model::civil_clock::CivilClock)->[u8;20]{TEST_CLOCK_LINES}
#[test]
fn glyph_footprint_covers_all_actual_ink_and_erases_empty_or_shorter_replacements(){
 for (x,y,w,h) in [(-30,-30,144,90),(0,0,220,150),(100,100,624,331),(950,750,624,331)] {
  for chrome in [0,SURFACE_CHROME_TOP_BAR] {
   let mut s=Surface{x,y,w,h,chrome_flags:chrome,text_len:512,text_buf:[b'A';TEXT_BUF_CAP],..SURFACE_EMPTY};
   for i in (12..512).step_by(13){s.text_buf[i]=b'\n';}recompute_text_lines(&mut s);
   let r=text_surface_damage(&s,1000,800);let mut d=damage::Damage::new(28);d.add(r,1000,800);
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
  let pixel_scene_5=capture_scene(w,h,focus);
  for py in 0..h {for px in 0..w {actual.push(composite_pixel(px,py,w,h,0x123,focus, &pixel_scene_5));}}
  for (sx,sy,sw,sh,color) in [(0,0,80,40,0x333333),(30,50,120,120,0xabcdef),(-100,-100,60,80,0x123456),(0,0,0,0,0)]{
   let(old,new)=fill_damage::update(&mut SURFACES[1],0,sx,sy,sw,sh,color).unwrap().unwrap();
   let mut d=damage::Damage::new(28);d.add(old.screen(&SURFACES[1],w,h),w,h);d.add(new.screen(&SURFACES[1],w,h),w,h);
   FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();
   if let Some(r)=d.rect(){redraw_surface_damage(actual.as_mut_ptr(),w,h,r);assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);}
  let pixel_scene_5=capture_scene(w,h,focus);
   for py in 0..h {for px in 0..w {assert_eq!(actual[py*w+px],composite_pixel(px,py,w,h,0x123,focus, &pixel_scene_5),"focus={focus} hidden={hidden} x={x} px={px} py={py}");}}
  }
 }}}
}}

#[test]fn real_compositor_draws_title_after_tab_background_for_all_quil_incarnations(){unsafe{
 for (owner,sid) in [(3,201),(16,301),(18,307)]{
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
  SURFACES[0]=Surface{owner_pd:owner,surface_id:sid,x:100,y:100,w:400,h:300,active:true,tab_count:1,chrome_flags:1,..SURFACE_EMPTY};
  let mut ink=0;let pixel_scene_6=capture_scene(1000,800,sid);
 for y in 0..28{for x in 62..112{if let Some(color)=toolbar_title_fg_at(x,y,sid,DISPLAY_TOKENS.active_tab_color){ink+=1;assert_eq!(composite_pixel(x+100,y+100,1000,800,0,sid, &pixel_scene_6),color);}}}assert!(ink>0);
 }
}}
#[test]fn visual_family_style_does_not_replace_exact_surface_ownership(){
 // Compile-time use remains valid; visual metadata never supplies an owner.
 const NEWEST_LABEL:Option<&str>=desktop_style::native_label(312);
 const NEWEST_INDICATOR:Option<u32>=desktop_style::native_indicator(312);
 assert_eq!(NEWEST_LABEL,Some("Quil"));assert_eq!(NEWEST_INDICATOR,Some(0x00cba6f7));
 for row in &app_catalog::INCARNATIONS {
  let(label,color)=match row.app {
   app_catalog::App::Quil=>("Quil",0x00cba6f7),
   app_catalog::App::Spindle=>("Spindle",0x0089b4fa),
   app_catalog::App::Browser=>("Browser",0x005ed9c3),
   app_catalog::App::Program=>("Program",0x00f9e2af),
  };
  for sid in [row.frame(),row.surface].into_iter().chain(row.children.iter().copied()) {
   assert_eq!(desktop_style::native_label(sid),Some(label),"sid={sid}");
   assert_eq!(desktop_style::native_indicator(sid),Some(color),"sid={sid}");
  }
  for sid in std::iter::once(row.surface).chain(row.children.iter().copied()) {
   let canvas=if row.app==app_catalog::App::Browser{None}else{Some(desktop_style::NATIVE_CANVAS)};
   assert_eq!(desktop_style::native_canvas(row.target,sid),canvas);
   assert_eq!(desktop_style::native_canvas(3,sid),None);
   for other in app_catalog::INCARNATIONS.iter().filter(|other|other.target!=row.target){
    assert_eq!(desktop_style::native_canvas(other.target,sid),None);
   }
  }
  if row.shell_surface.is_some(){assert_eq!(desktop_style::native_canvas(row.target,row.frame()),None);}
 }
 assert_eq!(desktop_style::native_label(200),Some("Linen"));assert_eq!(desktop_style::native_indicator(200),Some(0x00a6e3a1));
 for sid in [0,100,152,999,u64::MAX]{assert_eq!(desktop_style::native_label(sid),None);assert_eq!(desktop_style::native_indicator(sid),None);}
}

use silkbar_model::{SilkBar,ModuleSlot,DEFAULT_SILK_BAR,DEFAULT_THEME};
#[test]
fn normal_app_indicator_uses_actual_focus_and_clears_with_it(){unsafe{
 let bar=DEFAULT_SILK_BAR;
 for sid in [201,301,307,308,309,310,311,312]{FOCUSED_SURFACE_ID=sid;let pixel_scene_7=capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID);
 for y in 12..16{for x in 40..44{assert_eq!(phase5_active_app_at(x,y,&bar, &pixel_scene_7),Some(0xCBA6F7));}}}
 for sid in [153,306]{FOCUSED_SURFACE_ID=sid;assert_eq!(phase5_active_app_at(40,12,&bar, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),Some(0x89B4FA));}
 FOCUSED_SURFACE_ID=0;assert_eq!(phase5_active_app_at(40,12,&bar, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),None);
 FOCUSED_SURFACE_ID=307;assert_eq!(phase5_active_app_at(44,12,&bar, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),None);assert_eq!(phase5_active_app_at(40,16,&bar, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),None);
}}

fn repaint_begin()->u64{0}
fn repaint_end(_kind:&str,_started:u64){}
const TEST_CLOCK_LINES:[u8;20]=[b'C';20];
fn clock_fg_at_scaled(x:usize,y:usize,_:&SilkBar,lines:&[u8;20],_:u32,_:u32)->Option<u32>{assert_eq!(*lines,TEST_CLOCK_LINES);(x==0&&y==0).then_some(1)}
fn bell_badge_at_scaled(x:usize,y:usize,_:&SilkBar,_:u32,_:u32)->Option<u32>{(x==1&&y==0).then_some(2)}
fn phase5_tint_swatch_at(x:usize,y:usize,_:&SilkBar)->Option<u32>{(x==2&&y==0).then_some(3)}
fn phase5_palette_dot_at(x:usize,y:usize,_:&SilkBar)->Option<u32>{(x==3&&y==0).then_some(4)}
fn bar_color(_:usize,_:usize,_:&SilkBar,_:&scene::SceneSnapshot)->u32{9}
#[test]
fn shared_full_and_incremental_strip_uses_current_focus(){unsafe{
 let bar=DEFAULT_SILK_BAR;
 for sid in [301,307,308,309,310,311,312]{FOCUSED_SURFACE_ID=sid;assert_eq!(top_strip_pixel(40,12,&bar,&TEST_CLOCK_LINES, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),0xCBA6F7);}
 FOCUSED_SURFACE_ID=306;assert_eq!(top_strip_pixel(40,12,&bar,&TEST_CLOCK_LINES, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),0x89B4FA);
 FOCUSED_SURFACE_ID=0;assert_eq!(top_strip_pixel(40,12,&bar,&TEST_CLOCK_LINES, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),9);
 for (x,c)in [(0,1),(1,2),(2,3),(3,4),(4,9)]{assert_eq!(top_strip_pixel(x,0,&bar,&TEST_CLOCK_LINES, &capture_scene(MAX_FB_W,MAX_FB_H,FOCUSED_SURFACE_ID)),c);}
}}

// Literal image-58 two-pass shader is an independent oracle. The row cache
// must preserve both visible pixels and the exact owner-bound presentation mask.
unsafe fn assert_span_reference(w:usize,h:usize,focuses:&[u64]) {
 for &focus in focuses {let pixel_scene_8=capture_scene(w,h,focus);
 for y in 0..h {
  let mut row=pixel_scene_8.groups.row_winners(&pixel_scene_8.surfaces,focus,y,w,h).unwrap();
  for x in 0..w {
   let selected=FRAME_GROUPS.winner(&SURFACES,focus,x,y,w,h).map(|s|s.surface_id);
   APP_PRESENT_BITS=0;
   let expected=reference_composite_pixel_selected(x,y,w,h,0x192837,focus,selected);
   let expected_mask=APP_PRESENT_BITS;
   APP_PRESENT_BITS=0;
   let actual=composite_pixel_source(x,y,w,h,0x192837,row.source(x), &pixel_scene_8);
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
 assert!(core::mem::size_of::<surface_groups::RowWinners<'_>>()<=8192);
}}

#[test]fn actual_selection_cells_cover_spaces_and_spacing_skip_newlines_preserve_ink(){
 let mut s=Surface{x:20,y:70,w:200,h:150,text_len:5,text_color:0xfffafafa,selection_start:0,selection_end:5,..SURFACE_EMPTY};
 s.text_buf[..5].copy_from_slice(b"A \n B");recompute_text_lines(&mut s);
 let scale=surface_text_scale(&s);let tx=20+desktop_style::INSET as usize;let ty=70+desktop_style::TEXT_TOP as usize;
 // Full selected cells extend through spacing and baseline padding.
 for row in[0,1]{for col in[0,1]{assert_eq!(surface_text_fg_at(tx+col*6*scale+5*scale,ty+row*9*scale+8*scale,&s),Some(0xff36577c));}}
 assert_eq!(surface_text_fg_at(tx+2*6*scale,ty,&s),None); // newline cell
 assert_eq!(surface_text_fg_at(tx+2*6*scale,ty+9*scale,&s),None); // past line end
 let mut ink=0;for y in ty..ty+7*scale{for x in tx..tx+5*scale{if surface_text_fg_at(x,y,&s).is_some_and(|color|color!=0xff36577c){ink+=1;}}}assert!(ink>0);
 s.erase_selection();assert_eq!((s.selection_start,s.selection_end),(0,0));
 assert_eq!(surface_text_fg_at(tx+6*scale,ty,&s),None);
 // Legacy mutation also erases former selection so it cannot drift over bytes.
 s.selection_end=5;assert!(legacy_text_clear(&mut s));assert_eq!((s.selection_start,s.selection_end),(0,0));
 s.selection_end=1;assert!(legacy_text_draw(&mut s,b'X' as u64,legacy_args(0,1,0xfffafafa)));assert_eq!(s.selection_end,0);
}
#[test]fn selection_only_changes_match_full_and_incremental_compositor_and_clear_old_pixels(){unsafe{
 for focus in[0,300]{let(w,h)=(320,260);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,color:0x123456,text_len:16,text_color:0xfffafafa,..surf(15,300)};
 SURFACES[0].text_buf[..16].copy_from_slice(b"alpha beta\ngamma");recompute_text_lines(&mut SURFACES[0]);
 let mut actual=vec![0;w*h];let pixel_scene_9=capture_scene(w,h,focus);
 for y in 0..h{for x in 0..w{actual[y*w+x]=composite_pixel(x,y,w,h,bg(y,h),focus, &pixel_scene_9);}}
 for(start,end)in[(0,16),(3,8),(0,0)]{
  SURFACES[0].selection_start=start;SURFACES[0].selection_end=end;
  let mut d=damage::Damage::new(28);d.add(text_surface_damage(&SURFACES[0],w,h),w,h);
  FOCUSED_SURFACE_ID=focus;DAMAGE_ORDER.clear();redraw_surface_damage(actual.as_mut_ptr(),w,h,d.rect().unwrap());
  assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);
  let pixel_scene_10=capture_scene(w,h,focus);
 for y in 0..h{for x in 0..w{assert_eq!(actual[y*w+x],composite_pixel(x,y,w,h,bg(y,h),focus, &pixel_scene_10),"focus={focus} selection={start}..{end} x={x} y={y}");}}
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
    if index>1{assert_eq!(child.h,0);continue}
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
   if x>=w as i32{assert_eq!((sw,sh),(0,0));}
   for focus in[0,sid,spindle_catalog(pd).frame_surface_id().unwrap()]{let pixel_scene_11=capture_scene(w,h,focus);
 for py in sy..sy+sh{
    let mut row=pixel_scene_11.groups.row_winners(&pixel_scene_11.surfaces,focus,py,w,h).unwrap();
    for px in sx..sx+sw{
     APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,focus, &pixel_scene_11),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);
     let source=row.source(px);assert!(source.is_none());assert_eq!(composite_pixel_source(px,py,w,h,0xabcdef,source, &pixel_scene_11),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);
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
  for slot in 1..2{recompute_text_lines(&mut SURFACES[slot]);let child=SURFACES[slot];let(px,py)=(child.x as usize+child.w as usize/2,child.y as usize+child.h as usize/2);
   for focus in[0,parent,child.surface_id]{
    APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,focus, &capture_scene(w,h,focus)),child.color);assert_eq!(APP_PRESENT_BITS,native_pixel_bit(pd,child.surface_id));
    let pane_scene=capture_scene(w,h,focus);let mut row=pane_scene.groups.row_winners(&pane_scene.surfaces,focus,py,w,h).unwrap();let source=row.source(px).unwrap();assert_eq!(source.surface.surface_id,parent);assert_eq!(source.content.surface_id,child.surface_id);
    APP_PRESENT_BITS=0;assert_eq!(composite_pixel_source(px,py,w,h,0xabcdef,Some(source), &pane_scene),child.color);assert_eq!(APP_PRESENT_BITS,native_pixel_bit(pd,child.surface_id));
   }
   // Wrong-owner content cannot borrow the held child group or native bits.
   SURFACES[slot].owner_pd=99;APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,parent, &capture_scene(w,h,parent)),SURFACES[0].color);assert_eq!(APP_PRESENT_BITS,0);SURFACES[slot]=child;
  }
  SURFACES[0].active=false;
  for child in SURFACES[1..5].iter(){let(px,py)=(child.x as usize+10,child.y as usize+10);APP_PRESENT_BITS=0;assert_eq!(composite_pixel(px,py,w,h,0xabcdef,child.surface_id, &capture_scene(w,h,child.surface_id)),0xabcdef);assert_eq!(APP_PRESENT_BITS,0);}
 }}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}

// Independent host-only sRGB/WCAG oracle; production uses a finite integer LUT.
fn reference_luminance(color:u32)->f64{
 let linear=|v:u32|{let x=v as f64/255.0;if x<=0.04045{x/12.92}else{((x+0.055)/1.055).powf(2.4)}};
 linear((color>>16)&255)*0.2126+linear((color>>8)&255)*0.7152+linear(color&255)*0.0722
}
fn reference_contrast(a:u32,b:u32)->f64{
 let(a,b)=(reference_luminance(a),reference_luminance(b));(a.max(b)+0.05)/(a.min(b)+0.05)
}
#[test]fn integer_contrast_matches_independent_srgb_at_all_channel_samples_and_thresholds(){
 for channel in 0..=255{for shift in[0,8,16]{
  let color=channel<<shift;
  assert!((desktop_style::relative_luminance(color)as f64/1_000_000.0-reference_luminance(color)).abs()<0.000002);
 }}
 for r in(0..=255).step_by(8).chain([255]){for g in(0..=255).step_by(8).chain([255]){for b in(0..=255).step_by(8).chain([255]){
  let background=(r<<16)|(g<<8)|b;let ink=desktop_style::contrast_ink(background);
  assert!(ink==0||ink==0xffffff);
  assert!(reference_contrast(ink,background)>=4.5,"background={background:06x} ink={ink:06x}");
  assert_eq!(ink,desktop_style::contrast_ink(background|0xff000000));
 }}}
 for channel in 0..=255{let background=channel*0x010101;assert!(reference_contrast(desktop_style::contrast_ink(background),background)>=4.5);}
 assert_eq!(desktop_style::contrast_ink(0x757575),0xffffff);
 assert_eq!(desktop_style::contrast_ink(0x767676),0);
 assert_eq!(desktop_style::relative_luminance(0xffffff),1_000_000);
}
#[test]fn actual_toolbar_pixels_choose_contrast_after_blending_in_scalar_scanline_and_damage_paths(){unsafe{
 let(w,h)=(640,480);
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
 for background in[0,0xffffff,0xb4f7de,0x1e1e2e,0x757575,0x767676,0x808080,0xff0000,0x00ff00,0x0000ff]{
  // Neither shader token has this controlled post-blend color. This tests the
  // real compositor's background dependency, not the host shader implementation.
  for sid in[200,201,300,306,301,307]{for tabs in[0,1,2]{
   // Tabs deliberately replace an opposite-luminance toolbar glass pixel.
   TEST_GLASS_BACKGROUND=Some(if tabs==0{background}else{if desktop_style::contrast_ink(background)==0{0}else{0xffffff}});
   TEST_BLEND_BACKGROUND=Some(background);
   SURFACES[0]=Surface{chrome_flags:SURFACE_CHROME_TOP_BAR,tab_count:tabs,active_tab:0,..surf(3,sid)};
   FOCUSED_SURFACE_ID=sid;
   let expected=desktop_style::contrast_ink(background);let mut glyph_pixels=0;assert!(reference_contrast(expected,background)>=4.5);
   let mut framebuffer=vec![0xdeadbeef;w*h];DAMAGE_ORDER.clear();
   redraw_surface_damage(framebuffer.as_mut_ptr(),w,h,damage::Rect{x:183,y:105,right:269,bottom:121});
   let pixel_scene_13=capture_scene(w,h,sid);

 for y in 105..121{
    let mut row=pixel_scene_13.groups.row_winners(&pixel_scene_13.surfaces,sid,y,w,h).unwrap();
    for x in 183..269{
     let foreground=toolbar_title_fg_at(x-100,y-100,sid,background);
     let want=foreground.unwrap_or(background);
     // Contrast applies to the nominal ink; antialiased edge pixels have
     // partial coverage, and13px lowercase need not contain an opaque pixel.
     if let Some(ink)=foreground{glyph_pixels+=usize::from(ink!=background);}
     assert_eq!(composite_pixel(x,y,w,h,0x987654,sid, &pixel_scene_13),want,"scalar bg={background:06x} sid={sid} tabs={tabs}");
     assert_eq!(composite_pixel_source(x,y,w,h,0x987654,row.source(x), &pixel_scene_13),want,"scanline bg={background:06x} sid={sid} tabs={tabs}");
     assert_eq!(framebuffer[y*w+x],want,"damage bg={background:06x} sid={sid} tabs={tabs}");
    }
   }
   assert!(glyph_pixels>20,"bg={background:06x} sid={sid} tabs={tabs}");
  }}
 }
 TEST_GLASS_BACKGROUND=None;TEST_BLEND_BACKGROUND=None;SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}
#[test]fn actual_native_body_and_selection_ink_keep_contrast_on_declared_canvas(){
 for ink in[desktop_style::TEXT,0x00cdd6f4,0x00e8ffff,0x00f0f8ff]{
  let mut surface=Surface{x:0,y:50,w:400,h:100,text_color:ink,text_len:1,..surf(9,156)};
  surface.text_buf[0]=b'A';recompute_text_lines(&mut surface);
  let mut found_ink=false;let mut found_background=false;
  surface.selection_end=1;
  for y in 62..80{for x in 8..20{if let Some(color)=surface_text_fg_at(x,y,&surface){
   if color==ink{found_ink=true;assert!(reference_contrast(color,desktop_style::NATIVE_CANVAS)>=4.5);assert!(reference_contrast(color,0x36577c)>=4.5);}
   // Glyph edges blend the ink into the selection; whatever else shows is the selection.
   else if color==0xff36577c{found_background=true;}
  }}}
  assert!(found_ink&&found_background);
 }
}

#[test]fn utf8_scalar_cell_tables_preserve_source_offsets_wrapping_and_bounded_reset(){
 let text="Aé€😀\tZ\nöß…".as_bytes();
 for width in[40,240,400]{
  let mut surface=Surface{x:20,y:70,w:width,h:300,text_len:text.len()as u16,..surf(9,156)};
  surface.text_buf[..text.len()].copy_from_slice(text);recompute_text_lines(&mut surface);
  assert_eq!(&surface.text_buf[..text.len()],text);
  assert_eq!(surface.cell_count,10);
  assert_eq!(&surface.cell_starts[..10],&[0,1,3,6,10,11,12,13,15,17]);
  let columns=surface.wrap_cols as usize;
  for row in 0..surface.line_count as usize{
   assert_eq!(surface.line_starts[row],surface.cell_starts[surface.line_cells[row]as usize]);
   let begin=surface.line_cells[row]as usize;
   let end=if row+1<surface.line_count as usize{surface.line_cells[row+1]as usize}else{surface.cell_count as usize};
   assert!(end-begin<=columns+1);
   assert!(text_encoding::is_boundary(text,surface.line_starts[row]as usize));
  }
  // Reflow is a derived-table update, never a source-byte edit.
  surface.w=124;recompute_text_lines(&mut surface);assert_eq!(&surface.text_buf[..text.len()],text);
  assert!(legacy_text_clear(&mut surface));assert_eq!(surface.cell_count,0);
  assert!(surface.cell_starts.iter().chain(&surface.line_cells).chain(&surface.line_starts).all(|&x|x==0));
 }
 let mut maximum=Surface{w:8192,text_len:TEXT_BUF_CAP as u16,..surf(9,156)};maximum.text_buf.fill(b'A');maximum.text_buf[TEXT_BUF_CAP-1]=b'\n';recompute_text_lines(&mut maximum);
 assert_eq!(maximum.cell_count as usize,TEXT_BUF_CAP);assert_eq!(maximum.cell_starts[TEXT_BUF_CAP-1]as usize,TEXT_BUF_CAP-1);
 assert_eq!(maximum.cell_starts.len(),TEXT_BUF_CAP);assert_eq!(maximum.line_cells.len(),48);
 maximum.text_len=1;maximum.text_buf[0]=b'x';recompute_text_lines(&mut maximum);
 assert_eq!(maximum.cell_count,1);assert!(maximum.cell_starts[1..].iter().all(|&x|x==0));
}
#[test]fn actual_utf8_glyphs_fallback_tab_and_byte_selection_match_scalar_scanline_and_damage(){unsafe{
 let text="Aé€😀\tZ\nöß…".as_bytes();
 let expected=[FONT_ASCII_5X7[(b'A'-32)as usize],[2,4,14,17,31,16,14],[7,8,30,8,30,8,7],[31,17,21,21,21,17,31],[0;7],FONT_ASCII_5X7[(b'Z'-32)as usize],[10,0,0,14,17,17,14],[12,18,18,28,18,18,28],[0,0,0,0,0,0,21]];
 for width in[240,400]{
  let(w,h)=(600,300);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
  SURFACES[0]=Surface{x:20,y:70,w:width,h:160,text_len:text.len()as u16,text_color:0xe0f0ff,selection_start:1,selection_end:11,..surf(9,156)};
  SURFACES[0].text_buf[..text.len()].copy_from_slice(text);recompute_text_lines(&mut SURFACES[0]);FOCUSED_SURFACE_ID=0;
  let factor=surface_text_scale(&SURFACES[0]);let mut framebuffer=vec![0;w*h];DAMAGE_ORDER.clear();
  redraw_surface_damage(framebuffer.as_mut_ptr(),w,h,damage::Rect{x:20,y:70,right:20+width as usize,bottom:230});
  let pixel_scene_15=capture_scene(w,h,0);
  for row in 0..2{for column in 0..8{let glyph=if row==0&&column<6{expected[column]}else if row==1&&column<3{expected[6+column]}else{[0;7]};

 for py in 0..9*factor{let y=82+row*9*factor+py;let mut cache=pixel_scene_15.groups.row_winners(&pixel_scene_15.surfaces,0,y,w,h).unwrap();
    for px in 0..6*factor{let x=28+column*6*factor+px;
     let ink=py/factor<7&&px/factor<5&&glyph[py/factor]&(1<<(4-px/factor))!=0;
     let selected=row==0&&(1..=4).contains(&column);
     let scalars=[['A','é','€','😀','\t','Z','\0','\0'],['ö','ß','…','\0','\0','\0','\0','\0']];
     let back=if selected{0xff36577c}else{SURFACES[0].color};
     let color=match scalars[row][column]{'\0'=>back,c=>geist_pixel(c as u32,factor,px,py,0xe0f0ff,back).unwrap_or(if ink{0xe0f0ff}else{back})};
     assert_eq!(composite_pixel(x,y,w,h,0,0, &pixel_scene_15),color,"scalar width={width} row={row} col={column}");
     assert_eq!(composite_pixel_source(x,y,w,h,0,cache.source(x), &pixel_scene_15),color,"scanline width={width} row={row} col={column}");
     assert_eq!(framebuffer[y*w+x],color,"damage width={width} row={row} col={column}");
    }
   }
  }}
  assert_eq!(&SURFACES[0].text_buf[..text.len()],text);
 }
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}
#[test]fn finite_scalar_font_is_explicit_bounded_and_never_aliases_unknown_to_question_mark(){
 assert_eq!(FONT_UNICODE_5X7.len(),94);
 for(code,rows)in FONT_UNICODE_5X7{
  assert!(char::from_u32(*code).is_some());assert!(rows.iter().all(|&row|row<32));
  assert_eq!(scalar_glyph(*code),*rows);
 }
 assert_ne!(scalar_glyph(0x00f6),scalar_glyph(0x00d6));
 assert_eq!(scalar_glyph(9),[0;7]);
 assert_eq!(scalar_glyph(0x1f600),[31,17,21,21,21,17,31]);
 assert_eq!(scalar_glyph(0xfffd),[31,17,21,21,21,17,31]);
 assert_ne!(scalar_glyph(0x1f600),FONT_ASCII_5X7[(b'?'-32)as usize]);
}

#[test]fn scalar_wrap_never_splits_multibyte_source_at_legacy_or_width_derived_columns(){
 let text="é".repeat(21);let mut s=Surface{w:124,text_len:text.len()as u16,..surf(9,156)};
 s.text_buf[..text.len()].copy_from_slice(text.as_bytes());recompute_text_lines(&mut s);
 assert_eq!(s.wrap_cols,20);assert_eq!(s.line_starts[1],40);assert_eq!(s.line_cells[1],20);
 let text=format!("{text}\n");s.text_len=text.len()as u16;s.text_buf[..text.len()].copy_from_slice(text.as_bytes());recompute_text_lines(&mut s);
 assert_eq!(s.wrap_cols,18);assert_eq!(s.line_starts[1],36);assert_eq!(s.line_cells[1],18);
 assert!(s.line_starts[..s.line_count as usize].iter().all(|&at|text_encoding::is_boundary(text.as_bytes(),at as usize)));
}

// Host allocation/lease boundary only. All payload copies and sampling use the
// production SceneSnapshot and PublishedStores; no pixel adapter captures state.
#[path="../servers/sexdisplay/src/scene.rs"] mod scene;
fn capture_scene(w:usize,h:usize,focus:u64)->Box<scene::SceneSnapshot>{unsafe{
 let mut snapshot=Box::new(scene::SceneSnapshot::EMPTY);
 assert!(snapshot.capture(&SURFACES,&FRAME_GROUPS,&BITMAP_STORES,&TEXT_STAGES,&DEFAULT_SILK_BAR,DISPLAY_TOKENS,focus,RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed),w,h));
 snapshot
}}
struct SceneLease { snapshot:Box<scene::SceneSnapshot> }
impl SceneLease {
 fn capture(w:usize,h:usize,_:&SilkBar)->Option<Self>{Some(Self{snapshot:capture_scene(w,h,unsafe{FOCUSED_SURFACE_ID})})}
 fn view(&self)->&scene::SceneSnapshot{&self.snapshot}
}
impl Drop for SceneLease {fn drop(&mut self){self.snapshot.scrub();}}
// OutputBoundary: actual Store/Target and raster_batch, with a host-only CPU
// publication callback. This is not the Main wrapper/receipt/cleanup proof.
#[path="../crates/sex-pdx/src/memory_region.rs"] pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
pub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;
#[path="../crates/sex-pdx/src/display_frames.rs"] pub mod display_frames;
#[path="../servers/sexdisplay/src/frame_output.rs"] mod frame_output;
struct OutputBoundary{pointer:*mut u8,allocation:std::alloc::Layout}
impl Drop for OutputBoundary{fn drop(&mut self){unsafe{std::alloc::dealloc(self.pointer,self.allocation);}}}
fn redraw_surface_damage(fb:*mut u32,w:usize,h:usize,r:damage::Rect){paint_output_batch(fb,w,h,Plan{content:Content::Damage(r),strip:false});}
fn paint_output_batch(fb:*mut u32,w:usize,h:usize,plan:Plan){unsafe{
 let layout=display_frames::Layout::new(w as u64,h as u64,(w*4)as u64).unwrap();
 let allocation=std::alloc::Layout::from_size_align(layout.total_bytes()as usize,4096).unwrap();
 let pointer=std::alloc::alloc(allocation);assert!(!pointer.is_null());
 let output=OutputBoundary{pointer,allocation};
 // Explicit simulated held replies over a uniquely owned page-aligned host
 // allocation. This does not establish a native capability or task isolation.
 use memory_region::{RegionGrant,ViewRequest,QUERY_VIEW,READ,WRITE};
 let grant=RegionGrant{region_id:display_frames::REGION_ID,virtual_base:output.pointer as u64,length:layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE,dma_base:None,peer_pd:None};
 let info=memory_region::resolve_info(Some(&grant)).unwrap();
 let view=memory_region::resolve_view(Some(&grant),ViewRequest{offset:0,length:layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE}).unwrap();
 let backing=display_frames::FrameBacking::from_resolved(layout,info,view).unwrap();
 let mut store=frame_output::Store::new(backing);
 // The fixture's current framebuffer is the already-visible baseline. Seed it
 // once, then use actual partial preparation; never recapture once per pixel.
 let mut baseline=store.prepare(true).unwrap();
 {let target=baseline.target();core::ptr::copy_nonoverlapping(fb,target.pointer(),target.len());}
 assert!(baseline.commit(|_|true));
 let captured=capture_scene(w,h,FOCUSED_SURFACE_ID);
 let mut preparation=store.prepare(plan.is_full()).unwrap();
 {let target=preparation.target();assert!(raster_batch(&target,&captured,plan));}
 assert!(preparation.commit(|frame|{core::ptr::copy_nonoverlapping(frame.pixels.as_ptr(),fb,frame.pixels.len());true}));
 if !plan.settle_only(){report_app_presented();}
 store.scrub();
 // Store drops before OutputBoundary deallocates its mapped lifetime.
}}
fn scene_glass_over_bg(c:u32,x:usize,y:usize,a:u8,_:&scene::SceneSnapshot)->u32{glass_over_bg(c,x,y,a)}

#[test]
fn captured_compositor_pixels_survive_live_surface_token_focus_and_bitmap_changes(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 let saved_tokens=DISPLAY_TOKENS;
 SURFACES[0]=Surface{color:0x204060,..surf(15,300)};
 let geometry=bitmap_batch::Geometry{x:0,y:0,width:48,height:48,scale:4};
 publish_bitmap(1,geometry,&vec![0xffff0000;geometry.len()]);FOCUSED_SURFACE_ID=300;
 let captured=capture_scene(600,500,300);
 let points=[(100,100),(170,180),(120,120)];
 let before:Vec<_>=points.iter().map(|&(x,y)|composite_pixel(x,y,600,500,0,captured.focus,&captured)).collect();
 assert!(before.contains(&0xffff0000));
 SURFACES[0].color=0xabcdef;SURFACES[0].x=300;
 DISPLAY_TOKENS.frame_rim_color=0x00ff00;FOCUSED_SURFACE_ID=0;
 BITMAP_STORES.retire(15,15);FRAME_GROUPS=FrameGroups::new();
 RENDER_FRAME_COUNTER.store(12345,core::sync::atomic::Ordering::Relaxed);
 desktop_style::set_ui_scale(3); // a later live scale cannot move captured pixels
 for (index,&(x,y)) in points.iter().enumerate(){assert_eq!(composite_pixel(x,y,600,500,0,captured.focus,&captured),before[index]);}
 assert_eq!(captured.bitmaps.sample(15,300,0,0),Some(0xffff0000));
 assert_eq!(captured.metrics.scale(),1);assert_eq!(captured.groups.metrics().scale(),1);
 desktop_style::set_ui_scale(1);
 let next=capture_scene(600,500,0);
 assert!(points.iter().enumerate().any(|(i,&(x,y))|composite_pixel(x,y,600,500,0,next.focus,&next)!=before[i]));
 assert_eq!(next.bitmaps.sample(15,300,0,0),None);
 DISPLAY_TOKENS=saved_tokens;SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();FOCUSED_SURFACE_ID=0;RENDER_FRAME_COUNTER.store(0,core::sync::atomic::Ordering::Relaxed);
}}

#[test]
fn captured_strip_focus_does_not_follow_later_global_focus(){unsafe{
 let saved=FOCUSED_SURFACE_ID;FOCUSED_SURFACE_ID=201;
 let previous=capture_scene(1000,800,201);
 assert_eq!(phase5_active_app_at(40,12,&previous.bar,&previous),Some(0xCBA6F7));
 FOCUSED_SURFACE_ID=306;
 assert_eq!(phase5_active_app_at(40,12,&previous.bar,&previous),Some(0xCBA6F7));
 let next=capture_scene(1000,800,306);
 assert_eq!(phase5_active_app_at(40,12,&next.bar,&next),Some(0x89B4FA));
 FOCUSED_SURFACE_ID=saved;
}}

// The legacy reference computes body pixels independently. Strip glyph/color
// sentinels are the established clock/bar boundary, not a replacement Scene.
unsafe fn independent_batch_pixel(x:usize,y:usize,w:usize,h:usize)->u32{
 if y<27{return match(x,y){(0,0)=>1,(1,0)=>2,(2,0)=>3,(3,0)=>4,_=>9};}
 if y==27{return DEFAULT_THEME.panel_glow;}
 legacy_reference_pixel(x,y,w,h,0)
}
unsafe fn combined_fixture()->(usize,usize,Vec<u32>){
 let(w,h)=(320,260);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FOCUSED_SURFACE_ID=0;
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,color:0x345678,..surf(9,156)};
 let mut pixels=vec![0;w*h];for y in 0..h{for x in 0..w{pixels[y*w+x]=independent_batch_pixel(x,y,w,h);}}
 (w,h,pixels)
}
#[test]fn combined_area_strip_matches_independent_whole_frame_once(){unsafe{
 let(w,h,mut pixels)=combined_fixture();SURFACES[0].color=0xabcdef;DAMAGE_ORDER.clear();ATLAS_CALLS=0;
 paint_output_batch(pixels.as_mut_ptr(),w,h,Plan{content:Content::Area,strip:true});
 assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);assert_eq!(ATLAS_CALLS,2);
 for y in 0..h{for x in 0..w{assert_eq!(pixels[y*w+x],independent_batch_pixel(x,y,w,h),"area/strip {x}/{y}");}}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];
}}
#[test]fn combined_damage_strip_preserves_every_pixel_outside_changed_regions(){unsafe{
 let(w,h,mut pixels)=combined_fixture();let old=pixels.clone();
 fill_damage::update(&mut SURFACES[0],0,5,10,30,20,0x00f0e0d0).unwrap();
 let rect=damage::Rect{x:25,y:80,right:55,bottom:100};DAMAGE_ORDER.clear();ATLAS_CALLS=0;
 paint_output_batch(pixels.as_mut_ptr(),w,h,Plan{content:Content::Damage(rect),strip:true});
 assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);assert_eq!(ATLAS_CALLS,2);
 let mut changed=0;for y in 0..h{for x in 0..w{
  assert_eq!(pixels[y*w+x],independent_batch_pixel(x,y,w,h),"damage/strip {x}/{y}");
  if y>=28&&!(x>=25&&x<55&&y>=80&&y<100){assert_eq!(pixels[y*w+x],old[y*w+x]);}
  changed+=usize::from(pixels[y*w+x]!=old[y*w+x]);
 }}assert_eq!(changed,30*20);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];
}}
#[test]fn combined_cursor_strip_keeps_body_exact_and_finishes_overlays_once(){unsafe{
 let(w,h,mut pixels)=combined_fixture();for pixel in &mut pixels[..28*w]{*pixel=0xdeadbeef;}let body=pixels[28*w..].to_vec();DAMAGE_ORDER.clear();ATLAS_CALLS=0;
 paint_output_batch(pixels.as_mut_ptr(),w,h,Plan{content:Content::Cursor,strip:true});
 assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);assert_eq!(ATLAS_CALLS,2);assert_eq!(&pixels[28*w..],body.as_slice());
 for y in 0..h{for x in 0..w{assert_eq!(pixels[y*w+x],independent_batch_pixel(x,y,w,h),"cursor/strip {x}/{y}");}}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];
}}


// Actual register protocol + Stage seal, deliberately without publication or
// Applied release. These are candidate pixels, not canonical text/receipt proof.
unsafe fn seal_pending_text(index:usize,task:u32,sequence:u32,text:&[u8],selection:(usize,usize)){
 use text_batch::Request;
 let surface=SURFACES[index];
 let mut admit=|request:Request|{
  let(op,a,b)=request.wire();let decoded=Request::decode(op,a,b).unwrap();
  TEXT_STAGES[index].receive(surface.owner_pd,task,surface.surface_id,decoded).is_some()
 };
 assert!(!admit(Request::Begin{sequence,len:text.len(),color:0xfffafafa}));
 for(offset,chunk)in text.chunks(8).enumerate(){let mut bytes=[0;8];bytes[..chunk.len()].copy_from_slice(chunk);
  assert!(!admit(Request::Chunk{sequence,offset:offset*8,count:chunk.len(),bytes}));
 }
 assert!(!admit(Request::Select{sequence,start:selection.0,end:selection.1}));
 assert!(admit(Request::Commit{sequence,hash:text_batch::selected_hash(text,selection.0,selection.1).unwrap()}));
 assert!(TEXT_STAGES[index].needs_publication());
}

#[test]fn pending_utf8_scene_pixels_preserve_canonical_text_selection_wrap_and_width_clipping(){unsafe{
 for width in[128,400]{for focus in[0,300]{
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
  SURFACES[0]=Surface{x:20,y:70,w:width,h:160,text_len:3,text_color:0xff112233,..surf(15,300)};
  SURFACES[0].text_buf[..3].copy_from_slice(b"OLD");recompute_text_lines(&mut SURFACES[0]);let canonical=SURFACES[0];
  let bytes="é X abcdefghijklmnopqrs\nß Z".as_bytes();seal_pending_text(0,150,1,bytes,(0,4));
  let snapshot=capture_scene(600,300,focus);let candidate=&snapshot.surfaces[0];
  assert_eq!(&candidate.text_buf[..candidate.text_len as usize],bytes);
  assert_eq!((candidate.selection_start,candidate.selection_end),(0,4));
  assert_eq!(&candidate.cell_starts[..4],&[0,2,3,4]);
  if width==128{assert_eq!(candidate.wrap_cols,18);assert_eq!(candidate.line_starts[1],19);}
  assert_eq!(SURFACES[0].text_buf,canonical.text_buf);assert_eq!(SURFACES[0].text_len,3);
  assert_eq!((SURFACES[0].text_color,SURFACES[0].selection_start,SURFACES[0].selection_end),(0xff112233,0,0));
  assert_eq!(SURFACES[0].cell_starts,canonical.cell_starts);
  let scale=if width==128{1}else{2};assert_eq!(surface_text_scale(candidate),scale);
  let(tx,ty)=(28,82);let mut ink=0;
  // Independent source-cell positions: multibyte é, selected blank, then X.
  // Font bitmaps are the actual fixed ROM; layout/selection expectations here
  // do not read the candidate's cell or line tables.
  for(col,scalar)in['é',' ','X'].into_iter().enumerate(){for y in 0..9*scale{for x in 0..6*scale{
   let rows=scalar_glyph(scalar as u32);
   let marked=x/scale<5&&y/scale<7&&rows[y/scale]&(1<<(4-x/scale))!=0;
   let expected=match geist_pixel(scalar as u32,scale,x,y,0xfffafafa,0xff36577c){
    Some(c)=>{if c!=0xff36577c{ink+=1;}c}
    None=>if marked{ink+=1;0xfffafafa}else{0xff36577c}};
   assert_eq!(composite_pixel(tx+col*6*scale+x,ty+y,600,300,0xabcdef,focus,&snapshot),expected,"width={width} focus={focus} col={col} x={x} y={y}");
  }}}
  assert!(ink>0);
  assert_eq!(composite_pixel(20+width as usize,ty,600,300,0xabcdef,focus,&snapshot),0xabcdef);
  // Source Stage remains unreleased; merely sampling a scene cannot emit Applied.
  let mut replies=0;TEXT_STAGES[0].poll_reply(|_,_|{replies+=1;0});assert_eq!(replies,0);
 }}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];reset_bitmap_stage();
}}

#[test]fn projected_scene_glyphs_are_owned_after_source_stage_and_canonical_surface_reset(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{x:20,y:70,w:200,h:120,text_len:3,..surf(15,300)};
 SURFACES[0].text_buf[..3].copy_from_slice(b"OLD");recompute_text_lines(&mut SURFACES[0]);
 let text="é X\nß Z".as_bytes();seal_pending_text(0,150,7,text,(0,text.len()));
 for focus in[0,300]{let mut snapshot=capture_scene(300,230,focus);
  let before:Vec<_>=(70..190).flat_map(|y|(20..220).map(move|x|(x,y))).map(|(x,y)|composite_pixel(x,y,300,230,0xabcdef,focus,&snapshot)).collect();
  TEXT_STAGES[0].reset();SURFACES[0]=SURFACE_EMPTY;
  assert_eq!(&snapshot.surfaces[0].text_buf[..text.len()],text);
  let after:Vec<_>=(70..190).flat_map(|y|(20..220).map(move|x|(x,y))).map(|(x,y)|composite_pixel(x,y,300,230,0xabcdef,focus,&snapshot)).collect();
  assert_eq!(before,after);
  let alpha=*glyphs::mono_cell('é' as u32,6,9).unwrap().iter().find(|&&a|a>0).unwrap();
  assert!(before.contains(&coverage_blend(0xfffafafa,0xff36577c,alpha)));
  snapshot.scrub();assert!(!snapshot.is_valid());assert!(snapshot.surfaces[0].text_buf.iter().all(|&byte|byte==0));
  if focus==0{SURFACES[0]=Surface{x:20,y:70,w:200,h:120,..surf(15,300)};seal_pending_text(0,150,7,text,(0,text.len()));}
 }
 reset_bitmap_stage();
}}

#[test]fn pending_child_text_projects_without_bypassing_parent_visibility_or_focus_clipping(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();FRAME_GROUPS.register(cat(12));
 SURFACES[1]=Surface{x:20,y:70,w:180,h:130,text_len:3,..surf(12,154)};
 SURFACES[1].text_buf[..3].copy_from_slice(b"OLD");recompute_text_lines(&mut SURFACES[1]);
 let text="é X".as_bytes();seal_pending_text(1,120,1,text,(0,text.len()));
 FRAME_GROUPS.observe(&SURFACES);
 for focus in[0,153,154]{let snapshot=capture_scene(400,300,focus);
  assert_eq!(&snapshot.surfaces[1].text_buf[..text.len()],text);
  for y in 70..200{for x in 20..200{assert_eq!(composite_pixel(x,y,400,300,0xabcdef,focus,&snapshot),0xabcdef);}}
 }
 SURFACES[0]=Surface{x:10,y:55,w:350,h:230,..surf(3,153)};FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);
 for focus in[0,153,154]{let snapshot=capture_scene(400,300,focus);let child=&snapshot.surfaces[1];let scale=surface_text_scale(child);
  let(x,y)=(child.x as usize+8+6*scale,child.y as usize+12+8*scale);
  assert_eq!(composite_pixel(x,y,400,300,0xabcdef,focus,&snapshot),0xff36577c);
  let mut row=snapshot.groups.row_winners(&snapshot.surfaces,focus,y,400,300).unwrap();
  assert_eq!(composite_pixel_source(x,y,400,300,0xabcdef,row.source(x),&snapshot),0xff36577c);
  assert_eq!(&SURFACES[1].text_buf[..3],b"OLD");
 }
 SURFACES[0].active=false;
 for focus in[0,153,154]{let snapshot=capture_scene(400,300,focus);let child=&snapshot.surfaces[1];
  assert_eq!(composite_pixel(child.x as usize+8,child.y as usize+12,400,300,0xabcdef,focus,&snapshot),0xabcdef);
 }
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}


#[test]fn identity_settle_preserves_baseline_pixels_and_bypasses_cursor_underlay_and_overlays(){unsafe{
 let(w,h)=(320,240);SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();FOCUSED_SURFACE_ID=0;
 SURFACES[0]=Surface{x:20,y:70,w:200,h:130,text_len:4,text_color:0xfffafafa,..surf(15,300)};
 SURFACES[0].text_buf[..4].copy_from_slice(b"same");recompute_text_lines(&mut SURFACES[0]);
 seal_pending_text(0,15,1,b"same",(0,0));let candidate=capture_scene(w,h,0);
 assert_eq!(candidate.text_projections[0].unwrap().coverage,scene::TextCoverage::NoPixels);
 // Arbitrary existing pixels include a retained cursor-shaped patch and its
 // independently retained callback underlay, rather than a freshly rendered
 // expected scene. Identity publication must not change any of these values.
 let mut pixels:Vec<_>=(0..w*h).map(|i|0xff000000|((i as u32).wrapping_mul(0x12345)&0xffffff)).collect();
 for y in 100..116{pixels[y*w+80]=0xffffffff;}let baseline=pixels.clone();
 CURSOR_UNDERLAY_SENTINEL=[0x123456,0xabcdef,0x876543,0x102030];let underlay=CURSOR_UNDERLAY_SENTINEL;
 let bits=APP_PRESENT_BITS;let frame=RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed);
 DAMAGE_ORDER.clear();ATLAS_CALLS=0;
 paint_output_batch(pixels.as_mut_ptr(),w,h,Plan{content:Content::Settle,strip:false});
 assert_eq!(pixels,baseline);assert_eq!(CURSOR_UNDERLAY_SENTINEL,underlay);
 assert!(DAMAGE_ORDER.is_empty());assert_eq!(ATLAS_CALLS,0);assert_eq!(APP_PRESENT_BITS,bits);
 assert_eq!(RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed),frame);
 assert!(TEXT_STAGES[0].needs_publication());
 // Combining the same candidate with real strip work must retain the ordinary
 // cursor/overlay tail. Settle cannot suppress another output obligation.
 let body=pixels[28*w..].to_vec();DAMAGE_ORDER.clear();
 paint_output_batch(pixels.as_mut_ptr(),w,h,Plan{content:Content::Settle,strip:true});
 assert_eq!(&pixels[28*w..],body.as_slice());assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);assert_eq!(ATLAS_CALLS,2);
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}

#[test]fn pending_text_coverage_damage_matches_independent_pixels_for_shorter_empty_growing_and_selection_updates(){unsafe{
 let(w,h)=(320,240);
 let cases:[(&[u8],(u16,u16),&[u8],(usize,usize));4]=[
  (b"AB\nCD\nEF",(0,8),b"Z",(0,0)),
  (b"AB\nCD\nEF",(0,0),b"",(0,0)),
  (b"",(0,0),b"AB\nCD\nEF",(0,8)),
  (b"A X",(0,3),b"A X",(0,0))];
 for focus in[0,300]{for(old,selection,new,new_selection)in cases{
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();FOCUSED_SURFACE_ID=focus;
  SURFACES[0]=Surface{x:20,y:70,w:128,h:130,text_len:old.len()as u16,text_color:0xfffafafa,selection_start:selection.0,selection_end:selection.1,..surf(15,300)};
  SURFACES[0].text_buf[..old.len()].copy_from_slice(old);recompute_text_lines(&mut SURFACES[0]);let canonical=SURFACES[0];
  let mut pixels:Vec<_>=(0..h).flat_map(|y|(0..w).map(move|x|(x,y))).map(|(x,y)|legacy_reference_pixel(x,y,w,h,focus)).collect();let before=pixels.clone();
  // Construct the independent expected replacement directly. Neither expected
  // pixels nor expected geometry use apply_pending_text/coverage requirements.
  let mut expected_surface=canonical;expected_surface.text_buf.fill(0);expected_surface.text_buf[..new.len()].copy_from_slice(new);
  expected_surface.text_len=new.len()as u16;expected_surface.selection_start=new_selection.0 as u16;expected_surface.selection_end=new_selection.1 as u16;recompute_text_lines(&mut expected_surface);
  SURFACES[0]=expected_surface;
  let expected:Vec<_>=(0..h).flat_map(|y|(0..w).map(move|x|(x,y))).map(|(x,y)|legacy_reference_pixel(x,y,w,h,focus)).collect();
  SURFACES[0]=canonical;seal_pending_text(0,15,1,new,new_selection);
  let candidate=capture_scene(w,h,focus);let projection=candidate.text_projections[0].unwrap();
  let scene::TextCoverage::Body(required)=projection.coverage else{panic!("changed text did not retain body obligation");};
  assert!(required.right-required.x<=128);assert!(required.bottom-required.y<=27);
  DAMAGE_ORDER.clear();paint_output_batch(pixels.as_mut_ptr(),w,h,Plan{content:Content::Damage(required),strip:false});
  assert_eq!(DAMAGE_ORDER,["restore","launcher","cursor","presented"]);
  assert_eq!(pixels,expected,"focus={focus} old={old:?} new={new:?}");
  let changed=pixels.iter().zip(&before).filter(|(a,b)|a!=b).count();assert!(changed>0);
  // A literal formerly occupied third row must erase on short/empty updates;
  // the replacement's rectangle alone cannot establish this result.
  if old==b"AB\nCD\nEF"{
   assert!((100..109).any(|y|(28..40).any(|x|before[y*w+x]!=canonical.color&&before[y*w+x]!=0xff36577c)));
   assert!((100..109).all(|y|(28..40).all(|x|pixels[y*w+x]==canonical.color)));
  }
  for y in 0..h{for x in 0..w{if x<required.x||x>=required.right||y<required.y||y>=required.bottom{assert_eq!(pixels[y*w+x],before[y*w+x]);}}}
  assert_eq!(SURFACES[0].text_buf,canonical.text_buf);assert_eq!(SURFACES[0].text_len,canonical.text_len);assert!(TEXT_STAGES[0].needs_publication());
 }}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();FOCUSED_SURFACE_ID=0;
}}

#[test]fn fast_rows_match_the_pixel_compositor_exactly(){unsafe{
 let check=|name:&str,w:usize,h:usize,focus:u64|{
  let scene=capture_scene(w,h,focus);
  let mut fast=vec![0u32;w*h];let mut slow=vec![0u32;w*h];
  for y in 51..h{
   APP_PRESENT_BITS=0;raster_row(fast.as_mut_ptr(),y,0,w,w,h,&scene);let fast_bits=APP_PRESENT_BITS;
   APP_PRESENT_BITS=0;let mut row=scene.groups.row_winners(&scene.surfaces,focus,y,w,h);
   for x in 0..w{slow[y*w+x]=if let Some(row)=row.as_mut(){composite_pixel_source(x,y,w,h,bg(y,h),row.source(x),&scene)}else{composite_pixel(x,y,w,h,bg(y,h),focus,&scene)};}
   assert_eq!(fast_bits,APP_PRESENT_BITS,"{name} present bits y={y}");
   for x in 0..w{assert_eq!(fast[y*w+x],slow[y*w+x],"{name} x={x} y={y}");}
  }
  // A damage rectangle paints the same pixels and nothing outside it.
  let mut part=vec![7u32;w*h];for y in 60..(h-1).min(200){raster_row(part.as_mut_ptr(),y,w/3,w*2/3,w,h,&scene);}
  for y in 0..h{for x in 0..w{let inside=y>=60&&y<(h-1).min(200)&&x>=w/3&&x<w*2/3;assert_eq!(part[y*w+x],if inside{slow[y*w+x]}else{7},"{name} damage x={x} y={y}");}}
 };
 for(w,h)in[(320,240),(640,400)]{
  let parent=spindle_catalog(12).frame_surface_id().unwrap();
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();FRAME_GROUPS.register(spindle_catalog(12));
  let text="plain ascii row\nA\u{e9}\u{20ac}\u{1f600}\tZ unicode\n\nabcdefghijklmnopqrstuvwxyz0123456789 a long row that wraps at the width of this window and keeps going\n".as_bytes();
  let mut scene=spindle_children(12);
  scene[0]=Surface{x:10,y:60,w:w as u32-20,h:h as u32-70,chrome_flags:SURFACE_CHROME_TOP_BAR,..surf(3,parent)};
  scene[1].text_len=text.len() as u16;scene[1].text_buf=[0;TEXT_BUF_CAP];scene[1].text_buf[..text.len()].copy_from_slice(text);scene[1].selection_start=4;scene[1].selection_end=30;
  SURFACES[..5].copy_from_slice(&scene);FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);recompute_text_lines(&mut SURFACES[1]);
  check("selected text",w,h,parent);check("unfocused",w,h,0);
  SURFACES[1].selection_start=0;SURFACES[1].selection_end=0;check("plain text",w,h,parent);
  // A fill rect over the whole run, then over part of it.
  SURFACES[1].fill_count=1;SURFACES[1].fill_sx[0]=0;SURFACES[1].fill_sy[0]=0;SURFACES[1].fill_sw[0]=4000;SURFACES[1].fill_sh[0]=30;SURFACES[1].fill_color[0]=0xff102030;check("whole fill",w,h,parent);
  SURFACES[1].fill_sx[0]=25;SURFACES[1].fill_sw[0]=40;check("partial fill",w,h,parent);
  SURFACES[1].fill_count=0;
  // Off the screen edge, then a surface without a frame.
  SURFACES[0].x=w as i32-100;FRAME_GROUPS.observe(&SURFACES);FRAME_GROUPS.fit_children(&mut SURFACES);recompute_text_lines(&mut SURFACES[1]);check("clipped",w,h,parent);
  SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
  let mut alone=Surface{x:40,y:70,w:(w as u32).min(200),h:90,..surf(9,156)};alone.text_len=text.len() as u16;alone.text_buf[..text.len()].copy_from_slice(text);SURFACES[0]=alone;recompute_text_lines(&mut SURFACES[0]);
  check("independent",w,h,0);check("independent focused",w,h,156);
 }
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();
}}

#[test]fn rounded_edges_blend_actual_owners_and_only_mark_real_contributors(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,color:0x110000,owner_pd:9,surface_id:156,active:true,..SURFACE_EMPTY};
 SURFACES[1]=Surface{x:0,y:28,w:320,h:230,color:0x000077,owner_pd:15,surface_id:300,active:true,..SURFACE_EMPTY};
 let old=capture_scene(320,260,0);
 for(x,color,bits)in[(20,0x000077,4),(27,0x020068,5),(31,0x110000,1)]{
  APP_PRESENT_BITS=0;assert_eq!(composite_pixel(x,70,320,260,0x123,0,&old),color);assert_eq!(APP_PRESENT_BITS,bits);
  let mut row=old.groups.row_winners(&old.surfaces,0,70,320,260).unwrap();
  APP_PRESENT_BITS=0;assert_eq!(composite_pixel_source(x,70,320,260,0x123,row.source(x),&old),color);assert_eq!(APP_PRESENT_BITS,bits);
 }
 let mut pixels=vec![0;320*260];raster_row(pixels.as_mut_ptr(),70,0,320,320,260,&old);
 assert_eq!((pixels[70*320+20],pixels[70*320+27],pixels[70*320+31]),(0x000077,0x020068,0x110000));
 FRAME_GROUPS.set_scale(3);SURFACES[0].color=0xffffff;
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(27,70,320,260,0x123,0,&old),0x020068);assert_eq!(APP_PRESENT_BITS,5);
 // Two translucent corner samples over a third real owner; a fourth opaque
 // owner is hidden and must never gain APP_PRESENT through speculative shading.
 FRAME_GROUPS.set_scale(1);SURFACES[0]=old.surfaces[0];
 SURFACES[1]=Surface{color:0x007700,owner_pd:12,surface_id:154,..SURFACES[0]};
 SURFACES[2]=Surface{x:0,y:28,w:320,h:230,color:0x0000bb,owner_pd:15,surface_id:300,active:true,..SURFACE_EMPTY};
 SURFACES[3]=Surface{owner_pd:16,surface_id:301,color:0xabcdef,..SURFACES[2]};
 let layered=capture_scene(320,260,0);APP_PRESENT_BITS=0;
 assert_eq!(composite_pixel(27,70,320,260,0x123,0,&layered),0x020c8e);assert_eq!(APP_PRESENT_BITS,7);
 // Legacy overlay underlay reconstruction must skip its own retained layer.
 let source=surface_groups::PixelSource{surface:&layered.surfaces[0],content:&layered.surfaces[0],chrome:false};
 APP_PRESENT_BITS=0;assert_eq!(composite_rounded_edge(27,70,320,260,0x123,source,Some(&layered.surfaces[0]),&layered),0x000ea3);assert_eq!(APP_PRESENT_BITS,6);
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}

#[test]fn legacy_overlay_rounded_edge_reconstructs_underlay_without_double_blending(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{x:20,y:70,w:200,h:150,color:0xdeadbe,owner_pd:3,surface_id:146,active:true,..SURFACE_EMPTY};
 SURFACES[1]=Surface{x:0,y:28,w:320,h:230,color:0x000077,owner_pd:8,surface_id:888,active:true,..SURFACE_EMPTY};
 let scene=capture_scene(320,260,0);let mut pixels=vec![0;320*260];
 for y in 0..260{raster_row(pixels.as_mut_ptr(),y,0,320,320,260,&scene);}
 // Generic surface colour is intentionally different from the legacy pass:
 // reading its prior blended edge would produce the wrong visible result.
 draw_legacy_launcher_pixels(pixels.as_mut_ptr(),320,260,pixels.len(),&scene);
 assert_eq!((pixels[70*320+20],pixels[70*320+27],pixels[70*320+31]),(0x000077,0x080b78,0x405880));
 for y in 28..260{for x in 0..320{
  let a=reference_round_alpha(&SURFACES[0],1,x,y);
  let under=if reference_round_alpha(&SURFACES[1],1,x,y)!=0{reference_round_blend(0x000077,0x123,reference_round_alpha(&SURFACES[1],1,x,y))}else{0x123};
  let(lx,ly)=(x as i64-20,y as i64-70);
  let paint=if lx==0||ly==0||lx==199||ly==149{0x405880}else{0x203058};
  let expected=if a==0{under}else{reference_round_blend(paint,under,a)};
  assert_eq!(pixels[y*320+x],expected,"legacy overlay {x},{y}");
 }}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}

#[test]fn changed_row_damage_uses_original_signed_glyph_origin_after_top_clipping(){unsafe{
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
 SURFACES[0]=Surface{x:20,y:-30,w:120,h:150,color:0x102030,text_color:0xffffff,text_len:17,owner_pd:8,surface_id:888,active:true,..SURFACE_EMPTY};
 SURFACES[0].text_buf[..17].copy_from_slice(b"A\nB\nC\nD\nE\nF\nG\nH\nI");recompute_text_lines(&mut SURFACES[0]);
 assert_eq!(surface_text_scale(&SURFACES[0]),1);
 let old=SURFACES[0];let before=capture_scene(180,160,0);let mut actual=vec![0;180*160];
 for y in 0..160{raster_row(actual.as_mut_ptr(),y,0,180,180,160,&before);}
 SURFACES[0].text_buf[12]=b'Z';recompute_text_lines(&mut SURFACES[0]);
 let mut d=damage::Damage::new(28);d.add(text_surface_damage(&old,180,160),180,160);d.add(text_surface_damage(&SURFACES[0],180,160),180,160);
 let changed=changed_text_rows(&old,&SURFACES[0],d.rect().unwrap());
 assert_eq!((changed.y,changed.bottom),(36,45)); // -30 +12 +6*9, one full line
 let after=capture_scene(180,160,0);
 for y in changed.y..changed.bottom{raster_row(actual.as_mut_ptr(),y,changed.x,changed.right,180,160,&after);}
 for y in 0..160{for x in 0..180{assert_eq!(actual[y*180+x],composite_pixel(x,y,180,160,0x123,0,&after),"signed changed row {x},{y}");}}
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}

#[test]fn rounded_partial_stack_visits_all_32_layers_but_stops_at_opaque_owner(){unsafe{
 FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();let mut expected_mask=0u64;
 for i in 0..MAX_SURFACES{
  let sid=desktop_stack::ORDER[i];let row=app_catalog::INCARNATIONS.iter().enumerate().find(|(_,row)|row.frame()==sid);
  let owner=row.map_or(3,|(_,row)|if row.shell_surface.is_some(){3}else{row.target});
  if let Some((index,row))=row{if owner==row.target&&sid==row.surface{expected_mask|=1u64<<index;}}
  SURFACES[i]=Surface{x:20,y:70,w:200,h:150,color:if i==31{0xff0000}else{0xffffff},owner_pd:owner,surface_id:sid,active:true,..SURFACE_EMPTY};
 }
 assert_ne!(native_pixel_bit(SURFACES[31].owner_pd,SURFACES[31].surface_id),0);
 let scene=capture_scene(320,260,0);
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(27,70,320,260,0x123,0,&scene),0xf8f8f8);assert_eq!(APP_PRESENT_BITS,expected_mask);
 APP_PRESENT_BITS=0;assert_eq!(composite_pixel(31,70,320,260,0x123,0,&scene),0xffffff);assert_eq!(APP_PRESENT_BITS,0);
 SURFACES=[SURFACE_EMPTY;MAX_SURFACES];FRAME_GROUPS=FrameGroups::new();reset_bitmap_stage();
}}
