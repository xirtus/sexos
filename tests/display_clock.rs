//! Production formatting/painting with literal UI geometry and font advance goldens.
#[path="../crates/sex-pdx/src/app_catalog.rs"]mod app_catalog;
#[path="../crates/silkbar-model/src/lib.rs"]mod silkbar_model;
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
use silkbar_model::{SilkBar,ModuleSlot,DEFAULT_THEME,Action};
#[path="../servers/sexdisplay/src/glyphs.rs"]mod glyphs;
include!(env!("DISPLAY_CLOCK_PRODUCTION"));
fn bar(width:usize,ui:u32)->SilkBar{
 let mut b=silkbar_model::DEFAULT_SILK_BAR;b.civil_enabled=true;
 assert!(silkbar_model::reflow_chrome(&mut b,width,28*ui as usize,24*ui as usize));b
}
fn clock(state:silkbar_model::civil_clock::State,seconds:u32)->silkbar_model::civil_clock::CivilClock{
 use silkbar_model::civil_clock::*;let mut c=CivilClock::new();let u=Update{state,seconds,sequence:1}.encode();
 c.receive((2,2),(2,2),[u.kind as u64,(u.index as u64)<<32|u.a as u64,u.b as u64],0,u32::MAX,0).unwrap();c
}
#[test]fn single_line_shows_same_admitted_utc_date_and_honest_unhealthy_state(){
 use silkbar_model::civil_clock::State;
 let c=clock(State::Snapshot,1794320670);assert_eq!(&civil_clock_caption(&c),b"14:24 UTC | 10 NOV  ");
 for (state,text)in[(State::Unknown,"UTC unknown"),(State::Unavailable,"UTC unavailable"),(State::Rollback,"UTC blocked"),(State::Exhausted,"UTC ended")]{
  let c=clock(state,0);let caption=civil_clock_caption(&c);assert_eq!(core::str::from_utf8(&caption).unwrap().trim_end(),text);
 }
 let mut c=clock(State::Snapshot,1794320670);assert!(c.health(620));
 assert_eq!(core::str::from_utf8(&civil_clock_caption(&c)).unwrap().trim_end(),"UTC stale");
}
#[test]fn natural_13px_clock_has_room_for_widest_time_date_without_shrinking(){
 let f=sex_font::Font::new(sex_font::GEIST_SANS).unwrap();let size=13*64;
 let g:Vec<_>=(32..127).map(|c|f.glyph(c).unwrap()).collect();
 let advances:Vec<_>=g.iter().map(|v|f.advance(*v,size)).collect();
 let pairs:Vec<Vec<_>>=g.iter().map(|a|g.iter().map(|b|f.kerning(*a,*b,size)).collect()).collect();
 let width=|bytes:&[u8]|{let mut pen=0;for(i,&b)in bytes.iter().enumerate(){let k=(b-32)as usize;if i>0{pen+=pairs[(bytes[i-1]-32)as usize][k];}pen+=advances[k];}((pen+255)>>8)as usize};
 let mut max=0;for h in 0..24{for m in 0..60{for d in 1..=31{for month in["JAN","FEB","MAR","APR","MAY","JUN","JUL","AUG","SEP","OCT","NOV","DEC"]{max=max.max(width(format!("{h:02}:{m:02} UTC | {d:02} {month}").as_bytes()));}}}}
 assert_eq!(max,125);assert!(max<=136);
 for ui in 1..=3{for text in[b"00:00 UTC | 02 MAR".as_slice(),b"UTC unavailable",b"UTC blocked",b"UTC stale",b"99"]{assert!(glyphs::ui_width(text,ui).unwrap()<=136*ui as usize);}}
}
#[test]fn clock_and_bell_ink_stay_within_shared_target_and_have_visible_stems(){
 for (width,ui)in[(1024,1),(1280,1),(2560,2),(1024,3)]{
  let mut b=bar(width,ui);b.civil_clock=clock(silkbar_model::civil_clock::State::Snapshot,1794320670);
  let label=civil_clock_caption(&b.civil_clock);let(cx,cy,cw,ch)=module_rect(&b,ModuleSlot::Clock);let mut ink=0;
  for y in cy-1..cy+ch+1{for x in cx-1..cx+cw+1{let p=clock_fg_at_scaled(x,y,&b,&label,0x182638,ui);if x<cx||x>=cx+cw||y<cy||y>=cy+ch{assert_eq!(p,None);}else{ink+=p.is_some()as usize;}}}
  assert!(ink>100*ui as usize);
  b.bell_state.total_visible=99;b.bell_state.flags=1;let(bx,by,bw,bh)=module_rect(&b,ModuleSlot::Bell);let mut ink=0;
  for y in by-1..by+bh+1{for x in bx-1..bx+bw+1{let p=bell_badge_at_scaled(x,y,&b,0xffd700,ui);if x<bx||x>=bx+bw||y<by||y>=by+bh{assert_eq!(p,None);}else{ink+=p.is_some()as usize;}}}assert!(ink>20*ui as usize);
 }
}
#[test]fn status_hits_match_shared_geometry_without_offscreen_or_hidden_targets(){
 for (width,ui)in[(1024,1),(1280,1),(2560,2),(1024,3)]{
  let b=bar(width,ui);for cell in b.layout.iter().filter(|c|c.w>0){assert!(cell.x+cell.w<=width);assert!(cell.y+cell.h<=28*ui as usize);assert!(silkbar_model::hit_test_action(&b,cell.x,cell.y)!=Action::None);}
  let(cx,cy,cw,ch)=module_rect(&b,ModuleSlot::Clock);assert!(silkbar_model::hit_test_action(&b,cx+cw/2,cy+ch/2)==Action::OpenClock);
 }
}
static WIDTH:core::sync::atomic::AtomicI32=core::sync::atomic::AtomicI32::new(1280);
static HEIGHT:core::sync::atomic::AtomicI32=core::sync::atomic::AtomicI32::new(800);
fn desktop_width()->i32{WIDTH.load(core::sync::atomic::Ordering::Relaxed)}
fn desktop_height()->i32{HEIGHT.load(core::sync::atomic::Ordering::Relaxed)}
fn desktop_bar_height()->i32{28}
#[test]fn panel_clamp_preserves_reachable_clock_and_status(){
 for(width,height,x)in[(1024,768,784),(1280,800,1000)]{
  WIDTH.store(width,core::sync::atomic::Ordering::Relaxed);HEIGHT.store(height,core::sync::atomic::Ordering::Relaxed);
  assert_eq!(clamp_position(1000,30,240,300),(x,30));
  assert_eq!(clamp_position(-10,-10,240,300),(0,28));
 }
}
