// SPDX-License-Identifier: GPL-3.0-only
//! Exact production glyph table, layout and pixel functions, extracted by gate.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] mod app_catalog;
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"] mod state;
use state::{Surface,SURFACE_EMPTY,TEXT_BUF_CAP,TEXT_MAX_ROWS,TEXT_LEGACY_COLS};
const FRAME_TOP_BAR_LIGHT_EXCLUSION_PX:usize=50;
const FRAME_TOP_BAR_HEIGHT_PX:usize=28;
const SURFACE_CHROME_TOP_BAR:u8=1;
include!(env!("DISPLAY_TEXT_SOURCE"));
fn surface(width:u32,text:&[u8])->Surface{
 let mut s=Surface{active:true,x:20,y:60,w:width,h:400,text_color:0x123456,..SURFACE_EMPTY};
 s.text_buf[..text.len()].copy_from_slice(text);s.text_len=text.len()as u16;recompute_text_lines(&mut s);s
}
#[test]fn readable_scale_preserves_narrow_overlay_density(){
 assert_eq!(surface_text_scale(&surface(255,b"A")),1);assert_eq!(surface_text_scale(&surface(256,b"A")),2);
 assert_eq!(20*6*2+16,256);
}
#[test]fn scaled_glyph_is_exact_four_pixel_replication_with_preserved_color(){
 let s=surface(640,b"A");let glyph=ascii_glyph_index(b'A').unwrap();
 for row in 0..7{for col in 0..5{let want=if(FONT_ASCII_5X7[glyph][row]>>(4-col))&1!=0{Some(s.text_color)}else{None};
 for dy in 0..2{for dx in 0..2{assert_eq!(surface_text_fg_at(28+col*2+dx,72+row*2+dy,&s),want);}}}}
 for y in 72..86{assert_eq!(surface_text_fg_at(38,y,&s),None);assert_eq!(surface_text_fg_at(39,y,&s),None);}
}
#[test]fn narrow_glyph_keeps_original_pixel_grid(){
 let s=surface(200,b"A");let glyph=ascii_glyph_index(b'A').unwrap();
 for row in 0..7{for col in 0..5{let want=if(FONT_ASCII_5X7[glyph][row]>>(4-col))&1!=0{Some(s.text_color)}else{None};assert_eq!(surface_text_fg_at(28+col,72+row,&s),want);}}
}
#[test]fn lowercase_titles_draw_all_letters_inside_existing_chrome(){
 for(letter,ch)in b"Linen".iter().enumerate(){let mut pixels=0;for y in 0..28{for x in 62+letter*12..62+(letter+1)*12{
     if toolbar_title_fg_at(x,y,200,0).is_some(){assert!((7..21).contains(&y));pixels+=1;}
 }}assert!(pixels>0,"letter {} ({}) disappeared",letter,*ch);}
 assert_ne!(ascii_glyph_index(b'i'),ascii_glyph_index(b'I'));assert_eq!(ascii_glyph_index(b'\n'),None);
}
#[test]fn newline_wrap_uses_scaled_advance_and_clamps_before_narrowing(){
 for(width,cols)in[(1,1),(200,30),(256,20),(300,23),(640,52),(8192,160)]{
     let s=surface(width,b"a\nb");assert_eq!(s.wrap_cols,cols,"width {}",width);
 }
}
#[test]fn legacy_rows_keep_twenty_columns_and_reflow_tracks_resize(){
 let s=surface(640,b"abcdefghijklmnopqrstU");assert_eq!(s.wrap_cols,20);assert_eq!(s.line_starts[1],20);
 let mut s=surface(640,b"one\ntwo");assert_eq!(s.wrap_cols,52);s.w=300;recompute_text_lines(&mut s);assert_eq!(s.wrap_cols,23);
}
#[test]fn body_text_is_below_chrome_and_line_spacing_is_scaled(){
 let mut s=surface(640,b"A\nB");s.chrome_flags=SURFACE_CHROME_TOP_BAR;
 for y in 60..96{for x in 28..40{assert_eq!(surface_text_fg_at(x,y,&s),None);}}
 let mut row2pixels=0;for y in 114..128{for x in 28..38{row2pixels+=surface_text_fg_at(x,y,&s).is_some()as usize;}}
 assert!(row2pixels>0);for y in 110..114{for x in 28..40{assert_eq!(surface_text_fg_at(x,y,&s),None);}}
}
#[test]fn native_titles_share_existing_font_and_contrast_ink(){
 for sid in[156,154,300]{let mut pixels=0;for y in 0..28{for x in 62..158{if let Some(color)=toolbar_title_fg_at(x,y,sid,0){assert_eq!(color,0x00FFFFFF);pixels+=1;}}}assert!(pixels>0);}
}
#[test]fn filename_punctuation_keeps_old_indices_and_has_visible_pixels(){
 for ch in b' '..=b'Z'{assert_eq!(ascii_glyph_index(ch),Some((ch-b' ')as usize));}
 for ch in b"[\\]^_`{|}~"{let index=ascii_glyph_index(*ch).unwrap();assert!(index<FONT_ASCII_5X7.len());assert!(FONT_ASCII_5X7[index].iter().any(|row|*row!=0));}
 for ch in [b'_',b'~']{let s=surface(200,&[ch]);let glyph=ascii_glyph_index(ch).unwrap();let mut pixels=0;
 for y in 0..7{for x in 0..5{let want=if (FONT_ASCII_5X7[glyph][y]>>(4-x))&1!=0{pixels+=1;Some(s.text_color)}else{None};assert_eq!(surface_text_fg_at(28+x,72+y,&s),want);}}assert!(pixels>0);}
 for ch in [0,10,31,127,128,255]{assert_eq!(ascii_glyph_index(ch),None);}
}

// Explicit BOOT groups: metadata selects rendering, never grants authority.
const SPINDLE_PANES:[(u32,u64,[u64;4]);8]=[
 (12,153,[154,160,161,162]),(17,306,[302,303,304,305]),
 (24,314,[313,315,316,317]),(25,319,[318,320,321,322]),
 (26,324,[323,325,326,327]),(27,329,[328,330,331,332]),
 (28,334,[333,335,336,337]),(29,339,[338,340,341,342]),
];
fn owned_surface(owner:u32,sid:u64,width:u32,text:&[u8])->Surface{
 let mut s=surface(width,text);s.owner_pd=owner;s.surface_id=sid;recompute_text_lines(&mut s);s
}
#[test]fn only_exact_spindle_content_pairs_use_terminal_scale(){
 for (owner,frame,panes) in SPINDLE_PANES {
  for sid in panes {
   for (width,want,cols) in [(127,1,18),(128,2,9),(244,2,19)] {
    let s=owned_surface(owner,sid,width,b"A\nB");assert_eq!(surface_text_scale(&s),want,"{owner}/{sid}/{width}");assert_eq!(s.wrap_cols,cols);
   }
   for foreign in [0,3,9,15,owner|(1<<31)] {assert_eq!(surface_text_scale(&owned_surface(foreign,sid,244,b"A")),1);}
   for (other,_,_) in SPINDLE_PANES {if other!=owner {assert_eq!(surface_text_scale(&owned_surface(other,sid,244,b"A")),1);}}
   assert_eq!(surface_text_scale(&owned_surface(owner,sid|(1u64<<32),244,b"A")),1);
  }
  for frame_owner in [3,owner]{assert_eq!(surface_text_scale(&owned_surface(frame_owner,frame,244,b"A")),1);}
 }
 for (owner,sid) in [(9,156),(16,301),(18,307),(19,308),(20,309),(21,310),(22,311),(23,312),(15,300)]{
  for (width,want) in [(127,1),(128,1),(244,1),(255,1),(256,2)]{assert_eq!(surface_text_scale(&owned_surface(owner,sid,width,b"A")),want);}
 }
}
#[test]fn terminal_wrap_and_glyph_pixels_use_same_two_x_grid_at_244(){
 for (owner,_,panes) in SPINDLE_PANES {for sid in panes {
  let s=owned_surface(owner,sid,244,b"ABCDEFGHIJKLMNOPQRST\nZ");
  assert_eq!(s.wrap_cols,19);assert_eq!(s.line_starts[1],19);
  let glyph=ascii_glyph_index(b'A').unwrap();
  for row in 0..7{for col in 0..5{
   let want=if (FONT_ASCII_5X7[glyph][row]>>(4-col))&1!=0{Some(0x123456)}else{None};
   for dy in 0..2{for dx in 0..2{assert_eq!(surface_text_fg_at(28+col*2+dx,72+row*2+dy,&s),want);}}
  }}
  for y in 72..86{for x in 38..40{assert_eq!(surface_text_fg_at(x,y,&s),None);}}
 }}
}
#[test]fn terminal_scaled_scalar_controls_and_selection_preserve_cell_boundaries(){
 assert_eq!(scalar_glyph(9),[0;7]);
 assert_eq!(scalar_glyph(1),[31,17,21,21,21,17,31]);
 let text="é\t\u{1}A\nB".as_bytes();let mut s=owned_surface(29,342,244,text);
 assert_eq!(s.cell_count,6);assert_eq!(&s.cell_starts[..6],&[0,2,3,4,5,6]);
 assert_eq!(s.line_starts[1],6);assert_eq!(s.line_cells[1],5);
 s.selection_start=0;s.selection_end=2;
 for (cell,scalar) in [(0,0xe9),(1,9),(2,1),(3,65)] {
  let glyph=scalar_glyph(scalar);
  for row in 0..9 {for col in 0..6 {
   let ink=row<7&&col<5&&(glyph[row]>>(4-col))&1!=0;
   let want=if ink{Some(s.text_color)}else if cell==0{Some(0xff36577c)}else{None};
   for dy in 0..2{for dx in 0..2{assert_eq!(surface_text_fg_at(28+cell*12+col*2+dx,72+row*2+dy,&s),want);}}
  }}
 }
 for y in 72..90{for x in 76..88{assert_eq!(surface_text_fg_at(x,y,&s),None);}}
}
