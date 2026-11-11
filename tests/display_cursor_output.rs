// SPDX-License-Identifier: GPL-3.0-only
//! Actual cursor helpers/stencil over actual Scene and held-frame preparation.
//! Background/menu shading and CPU output are declared host boundaries.
#![allow(dead_code,unused_unsafe,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_stack.rs"]pub mod desktop_stack;
fn bar_height()->usize{28}
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/desktop_look.rs"]pub mod desktop_look;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
pub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/display_frames.rs"]pub mod display_frames;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"]mod bitmap_binding;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod bitmap_transaction;
#[path="../servers/sexdisplay/src/bitmap_stores.rs"]mod bitmap_stores;
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"]mod surface_lifecycle;
#[path="../servers/sexdisplay/src/surface_groups.rs"]mod surface_groups;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../servers/sexdisplay/src/text_transaction.rs"]mod text_transaction;
#[path="../servers/sexdisplay/src/damage.rs"]mod damage;
#[path="../servers/sexdisplay/src/scene.rs"]mod scene;
#[path="../servers/sexdisplay/src/frame_output.rs"]mod frame_output;
use surface_lifecycle::{Surface,MAX_SURFACES,SURFACE_EMPTY,TEXT_BUF_CAP,TEXT_MAX_ROWS,TEXT_LEGACY_COLS};
use surface_groups::FrameGroups;
use silkbar_model::{DEFAULT_THEME,DEFAULT_SILK_BAR};
include!(env!("DISPLAY_CURSOR_SOURCE"));
const W:usize=128;const H:usize=112;
const MENU_OLD:u32=0x004488aa;const MENU_NEW:u32=0x00aa8844;
struct Allocation{pointer:*mut u8,layout:std::alloc::Layout}
impl Drop for Allocation{fn drop(&mut self){unsafe{std::alloc::dealloc(self.pointer,self.layout);}}}
struct OutputBoundary{store:frame_output::Store,allocation:Allocation,visible:Vec<u32>}
impl OutputBoundary{
 fn new()->Self{unsafe{
  let layout=display_frames::Layout::new(W as u64,H as u64,(W*4)as u64).unwrap();
  let allocation=std::alloc::Layout::from_size_align(layout.total_bytes()as usize,4096).unwrap();
  let pointer=std::alloc::alloc(allocation);assert!(!pointer.is_null());
  use memory_region::*;
  // Explicit simulated held replies, not authority inferred from these values.
  let grant=RegionGrant{region_id:display_frames::REGION_ID,virtual_base:pointer as u64,length:layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE,dma_base:None,peer_pd:None};
  let info=resolve_info(Some(&grant)).unwrap();let view=resolve_view(Some(&grant),ViewRequest{offset:0,length:layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE}).unwrap();
  let backing=display_frames::FrameBacking::from_resolved(layout,info,view).unwrap();
  Self{store:frame_output::Store::new(backing),allocation:Allocation{pointer,layout:allocation},visible:vec![0;W*H]}
 }}
 fn paint(&mut self,mode:Mode,cursor:(i32,i32),menu:u32){
  let captured=capture(cursor);let mut preparation=self.store.prepare(matches!(mode,Mode::Full)).unwrap();
  {let target=preparation.target();let fb=target.pointer();
   match mode{
    Mode::Full=>{shade(&target,0,H);menu_rect(&target,menu);},
    Mode::Move=>restore_cursor_under(fb,W,H,target.len(),captured.metrics),
    Mode::Strip=>{restore_cursor_under(fb,W,H,target.len(),captured.metrics);shade(&target,0,28);menu_rect(&target,menu);},
   }
   draw_cursor_z_top(fb,W,H,target.len(),true,&captured);
  }
  let output=&mut self.visible;assert!(preparation.commit(|frame|{output.copy_from_slice(frame.pixels);true}));
  assert_eq!(self.store.front_pixels().unwrap(),self.visible.as_slice());
 }
 fn padding_is_zero(&self){unsafe{
  let layout=self.store.layout();for bank in 0..2{for byte in layout.frame_bytes()..layout.span_bytes(){assert_eq!(*self.allocation.pointer.add((bank*layout.span_bytes()+byte)as usize),0);}}
 }}
}
impl Drop for OutputBoundary{fn drop(&mut self){self.store.scrub();}}
#[derive(Clone,Copy)]enum Mode{Full,Move,Strip}
fn capture((x,y):(i32,i32))->Box<scene::SceneSnapshot>{
 let mut surfaces=[SURFACE_EMPTY;MAX_SURFACES];surfaces[0]=Surface{active:true,owner_pd:3,surface_id:CURSOR_SURFACE_ID,x,y,w:8,h:16,..SURFACE_EMPTY};
 let mut captured=Box::new(scene::SceneSnapshot::EMPTY);
 assert!(captured.capture(&surfaces,&FrameGroups::new(),&bitmap_stores::Stores::UNBOUND,&[text_transaction::Stage::EMPTY;MAX_SURFACES],&DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,0,0,W,H));captured
}
fn background(x:usize,y:usize)->u32{0x00112200+(y as u32)*257+x as u32}
fn underlay(x:usize,y:usize,menu:u32)->u32{if(20..100).contains(&x)&&(55..105).contains(&y){menu}else{background(x,y)}}
fn shade(target:&frame_output::Target<'_>,first:usize,end:usize){for y in first..end{for x in 0..W{unsafe{target.pointer().add(y*W+x).write(background(x,y));}}}}
fn menu_rect(target:&frame_output::Target<'_>,color:u32){for y in 55..105{for x in 20..100{unsafe{target.pointer().add(y*W+x).write(color);}}}}
fn reset(){scrub_cursor_under();unsafe{PREPARED_CURSOR=None;INPUT_TRACE_LAST_SEQ=73;INPUT_TRACE_LAST_SHELL_TICK=91;}}
// Independent fresh full-frame baseline for sequencing comparisons. It uses
// the actual arrow renderer with save_under=false, never a replacement stencil.
// Preserve pending metadata; importantly this never alters the live saved patch.
fn fresh_full(cursor:(i32,i32),menu:u32)->Vec<u32>{
 let mut output=OutputBoundary::new();let captured=capture(cursor);let mut p=output.store.prepare(true).unwrap();
 unsafe{let pending=PREPARED_CURSOR;{let target=p.target();shade(&target,0,H);menu_rect(&target,menu);draw_cursor_z_top(target.pointer(),W,H,target.len(),false,&captured);}PREPARED_CURSOR=pending;}
 assert!(p.commit(|frame|{output.visible.copy_from_slice(frame.pixels);true}));output.visible.clone()
}
#[test]fn full_overlap_is_visible_and_move_restores_exact_menu_without_arrow_trail(){
 reset();let mut output=OutputBoundary::new();output.paint(Mode::Full,(40,70),MENU_OLD);
 assert_eq!(output.visible[70*W+40],0x00ffffff);assert_eq!(output.visible[70*W+47],MENU_OLD);
 output.paint(Mode::Move,(110,90),MENU_OLD);
 for y in 70..86{for x in 40..48{assert_eq!(output.visible[y*W+x],MENU_OLD,"old arrow {x}/{y}");}}
 assert_eq!(output.visible,fresh_full((110,90),MENU_OLD));assert_eq!(output.store.generation(),2);output.padding_is_zero();reset();
}
#[test]fn strip_repaint_recaptures_new_menu_before_cursor_moves_away(){
 reset();let mut output=OutputBoundary::new();output.paint(Mode::Full,(40,70),MENU_OLD);
 output.paint(Mode::Strip,(40,70),MENU_NEW);assert_eq!(output.visible[70*W+40],0x00ffffff);
 output.paint(Mode::Move,(110,90),MENU_NEW);
 for y in 70..86{for x in 40..48{assert_eq!(output.visible[y*W+x],MENU_NEW);}}
 assert_eq!(output.visible,fresh_full((110,90),MENU_NEW));assert_eq!(output.store.generation(),3);reset();
}
#[test]fn clipped_edges_and_reserved_strip_are_safe_and_restore_exactly(){
 for cursor in [(127,111),(125,103),(0,49),(-3,45),(128,112)]{
  reset();let mut output=OutputBoundary::new();output.paint(Mode::Full,cursor,MENU_OLD);
  let(ox,oy)=(cursor.0.max(0)as usize,cursor.1.max(0)as usize);
  for y in 0..H{for x in 0..W{if y<28||x<ox||x>=ox+8||y<oy||y>=oy+16{assert_eq!(output.visible[y*W+x],underlay(x,y,MENU_OLD),"{cursor:?} {x}/{y}");}}}
  if cursor==(127,111){assert_eq!(output.visible[111*W+127],0x00ffffff);}
  output.paint(Mode::Move,(60,80),MENU_OLD);assert_eq!(output.visible,fresh_full((60,80),MENU_OLD));output.padding_is_zero();
 }reset();
}
#[test]fn full_and_partial_sequences_are_byte_equal_at_every_completed_frame(){
 reset();let mut output=OutputBoundary::new();
 let steps=[(Mode::Full,(40,70),MENU_OLD),(Mode::Move,(60,85),MENU_OLD),(Mode::Strip,(60,85),MENU_NEW),(Mode::Move,(127,111),MENU_NEW),(Mode::Move,(0,49),MENU_NEW),(Mode::Full,(40,70),MENU_OLD)];
 for(index,(mode,cursor,menu))in steps.into_iter().enumerate(){output.paint(mode,cursor,menu);assert_eq!(output.visible,fresh_full(cursor,menu),"step {index}");assert_eq!(output.store.generation(),index as u64+1);}
 reset();
}
#[test]fn cleanup_scrubs_all_saved_words_and_coordinates(){unsafe{
 reset();CURSOR_UNDER_SAVED.fill(0xaabbccdd);CURSOR_UNDER_X=23;CURSOR_UNDER_Y=82;CURSOR_UNDER_VALID=true;
 scrub_cursor_under();assert!(CURSOR_UNDER_SAVED.iter().all(|&p|p==0));assert_eq!(CURSOR_UNDER_SAVED.len(),128*9);assert_eq!((CURSOR_UNDER_X,CURSOR_UNDER_Y,CURSOR_UNDER_VALID),(0,0,false));
}}
