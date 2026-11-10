//! Compile the actual renderer functions and shared geometry, using public
//! counter values only. Native presentation and time authority are separate.
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
mod app_catalog;
#[path="../crates/silkbar-model/src/lib.rs"]mod silkbar_model;
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
use silkbar_model::{SilkBar,ModuleSlot,DEFAULT_THEME,Action};
#[path="../servers/sexdisplay/src/glyphs.rs"]mod glyphs;
include!(env!("DISPLAY_CLOCK_PRODUCTION"));
#[test]fn full_demo_prefix_is_visible_at_both_actual_widths(){
 for width in [1024,1280]{let mut bar=silkbar_model::for_desktop_width(width).unwrap();bar.clock_hh=0;bar.clock_mm=0;bar.clock_ss=0;let(cx,cy,cw,ch)=module_rect(&bar,ModuleSlot::Clock);
  for y in 0..ch{for x in 0..cw{let alpha=if x>=2&&y>=1{glyphs::caption_coverage(b"DEMO 00:00:00",x as i32-2,y-1).unwrap()}else{0};
   let expected=(alpha!=0).then(||coverage_blend(desktop_style::contrast_ink(0),0,alpha));
   assert_eq!(clock_fg_at(cx+x,cy+y,&bar,&civil_clock_lines(&bar.civil_clock),0),expected,"width={width} x={x} y={y}");}}
 }
}
#[test]fn whole_clock_stays_inside_chip_and_prefix_survives_counter_updates(){
 for width in [1024,1280]{let mut bar=silkbar_model::for_desktop_width(width).unwrap();let(cx,cy,cw,ch)=module_rect(&bar,ModuleSlot::Clock);
  for value in [0,(23<<16)|(59<<8)|58]{bar.clock_hh=(value>>16)as u8;bar.clock_mm=(value>>8)as u8;bar.clock_ss=value as u8;
   for y in cy..cy+ch{for x in cx..cx+cw{if y==cy||y>=cy+10||x<cx+2{assert_eq!(clock_fg_at(x,y,&bar,&civil_clock_lines(&bar.civil_clock),0),None);}}}
   assert!((0..30).any(|dx|(0..9).any(|dy|clock_fg_at(cx+dx,cy+1+dy,&bar,&civil_clock_lines(&bar.civil_clock),0).is_some())));
  }
 }
}
#[test]fn every_status_caption_fits_existing_boxes_at_natural_size(){
 for text in [b"UTC 23:59:59".as_slice(),b"DEMO 23:59:59",b"30 MAY 8888",b"UTC unknown",b"Awaiting UTC",b"Sample stale",b"Updates ended",b"Source error",b"Time blocked"]{
  assert!(glyphs::caption_width(text).unwrap()<=76,"{:?}",text);
 }
 for text in [b"1".as_slice(),b"8",b"10",b"99"]{assert!(glyphs::caption_width(text).unwrap()<=18);}
 // Captions have real near-opaque stems, not barely-visible miniature cells.
 for text in [b"UTC 23:59:59".as_slice(),b"30 MAY 8888",b"99"]{
  assert!((0..9).any(|y|(0..78).any(|x|glyphs::caption_coverage(text,x,y).unwrap()>240)));
 }
}
#[test]fn widest_real_clock_and_date_captions_fit_without_shrinking_geist(){
 let font=sex_font::Font::new(sex_font::GEIST_SANS).unwrap();
 let glyphs:Vec<_>=(32..127).map(|c|font.glyph(c).unwrap()).collect();
 let advances:Vec<_>=glyphs.iter().map(|g|font.advance(*g,9*64)).collect();
 let pairs:Vec<Vec<_>>=glyphs.iter().map(|a|glyphs.iter().map(|b|font.kerning(*a,*b,9*64)).collect()).collect();
 let width=|text:&str|{let bytes=text.as_bytes();let mut pen=0;
  for(i,&b)in bytes.iter().enumerate(){let c=(b-32)as usize;if i>0{pen+=pairs[(bytes[i-1]-32)as usize][c];}pen+=advances[c];}
  ((pen+255)>>8)as usize};
 let mut widest=(0,String::new());
 for h in 0..24{for m in 0..60{for sec in 0..60{let text=format!("UTC {h:02}:{m:02}:{sec:02}");let w=width(&text);if w>widest.0{widest=(w,text);}}}}
 for year in 2020..=2106{for month in ["JAN","FEB","MAR","APR","MAY","JUN","JUL","AUG","SEP","OCT","NOV","DEC"]{for day in 1..=31{
  let text=format!("{day:02} {month} {year:04}");let w=width(&text);if w>widest.0{widest=(w,text);}
 }}}
 assert!(widest.0<=76,"widest {:?}",widest);
 assert_eq!(glyphs::caption_width(widest.1.as_bytes()),Some(widest.0));
}
#[test]fn bell_counts_use_natural_sans_inside_the_same_status_hit_box(){
 for width in[1024,1280]{let mut bar=silkbar_model::for_desktop_width(width).unwrap();let(bx,by,bw,bh)=module_rect(&bar,ModuleSlot::Bell);
  for count in[1,9,10,99,255]{bar.bell_state.total_visible=count;bar.bell_state.flags=1;let mut ink=0;
   for y in by-1..by+bh+1{for x in bx-1..bx+bw+1{let pixel=bell_badge_at(x,y,&bar,0xffd700);
    if x<bx||x>=bx+bw||y<by||y>=by+bh{assert_eq!(pixel,None);}else{ink+=pixel.is_some()as usize;}
   }}assert!(ink>0);
  }
 }
}
#[test]fn fitted_clock_and_all_status_hit_regions_share_display_geometry(){
 for(width,expected_x)in[(1024,928),(1280,1090)]{let bar=silkbar_model::for_desktop_width(width).unwrap();let(cx,cy,_,_)=module_rect(&bar,ModuleSlot::Clock);assert_eq!(cx,expected_x);assert!(silkbar_model::hit_test_action(&bar,cx+40,cy+10)==Action::OpenClock);
  let status_start=1+silkbar_model::WORKSPACE_COUNT;
  for b in &bar.layout[status_start..]{assert!(b.x+b.w<=width);assert!(silkbar_model::hit_test_action(&bar,b.x,b.y)!=Action::None);}
  let left_end=bar.layout[..status_start].iter().map(|b|b.x+b.w).max().unwrap();assert!(left_end+16<=bar.layout[status_start].x);
  if width==1024{assert!(silkbar_model::hit_test_action(&bar,1095,cy+10)!=Action::OpenClock);}
 }
}
#[test]fn unsupported_narrow_width_rejects_geometry_without_underflow(){
 for width in [0,80,640,998]{assert!(silkbar_model::for_desktop_width(width).is_none());}
 assert!(silkbar_model::for_desktop_width(usize::MAX).is_some());
}

struct Policy{bar_height:i32}
const P:Policy=Policy{bar_height:50};
static WIDTH:core::sync::atomic::AtomicI32=core::sync::atomic::AtomicI32::new(1280);
static HEIGHT:core::sync::atomic::AtomicI32=core::sync::atomic::AtomicI32::new(800);
fn desktop_width()->i32{WIDTH.load(core::sync::atomic::Ordering::Relaxed)}
fn desktop_height()->i32{HEIGHT.load(core::sync::atomic::Ordering::Relaxed)}
#[test]fn actual_panel_clamp_keeps_newly_reachable_clock_and_status_panels_in_view(){
 for(width,height,clock_x)in[(1024,768,784),(1280,800,1000)]{
  WIDTH.store(width,core::sync::atomic::Ordering::Relaxed);HEIGHT.store(height,core::sync::atomic::Ordering::Relaxed);
  assert_eq!(clamp_position(1000,55,240,300),(clock_x,55));
  for(x,y,w,h)in[(1000,55,240,300),(860,55,200,300),(80,55,240,360),(2000,2000,240,300)]{
   let(cx,cy)=clamp_position(x,y,w,h);assert!(cx>=0&&cy>=50&&cx+w as i32<=width&&cy+h as i32<=height);
  }
 }
}
