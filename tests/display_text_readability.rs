// SPDX-License-Identifier: GPL-3.0-only
//! Exact production glyph table, layout and pixel functions, extracted by gate.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
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
 for(width,cols)in[(1,1),(200,30),(256,20),(300,23),(640,52),(8192,80)]{
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
