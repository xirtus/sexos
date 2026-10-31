//! Compile the actual renderer functions and shared geometry, using public
//! counter values only. Native presentation and time authority are separate.
#[path="../crates/silkbar-model/src/lib.rs"]mod silkbar_model;
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
use silkbar_model::{SilkBar,ModuleSlot,DEFAULT_THEME,Action};
include!(env!("DISPLAY_CLOCK_PRODUCTION"));
const LABEL:[[u8;7];4]=[
    [30,17,17,17,17,17,30], // D
    [31,16,16,30,16,16,31], // E
    [17,27,21,21,17,17,17], // M
    [14,17,17,17,17,17,14], // O
];
#[test]fn full_demo_prefix_is_visible_at_both_actual_widths(){
 for width in [1024,1280]{let bar=silkbar_model::for_desktop_width(width).unwrap();let(cx,cy,_,_)=module_rect(&bar,ModuleSlot::Clock);
  for y in 0..7{for x in 0..30{let cell=x/6;let px=x%6;let expected=cell<4&&px<5&&(LABEL[cell][y]>>(4-px))&1!=0;
   assert_eq!(clock_fg_at(cx+x,cy+1+y,&bar),expected.then_some(DEFAULT_THEME.text),"width={width} x={x} y={y}");}}
 }
}
#[test]fn whole_clock_stays_inside_chip_and_prefix_survives_counter_updates(){
 for width in [1024,1280]{let mut bar=silkbar_model::for_desktop_width(width).unwrap();let(cx,cy,cw,ch)=module_rect(&bar,ModuleSlot::Clock);assert!(cx+cw<=width&&cw>=76&&ch>=8);
  for value in [0,(23<<16)|(59<<8)|58]{bar.clock_hh=(value>>16)as u8;bar.clock_mm=(value>>8)as u8;bar.clock_ss=value as u8;
   for y in cy..cy+ch{for x in cx..cx+cw{if y==cy||y>=cy+8||x>=cx+76{assert_eq!(clock_fg_at(x,y,&bar),None);}}}
   assert!(clock_fg_at(cx,cy+1,&bar).is_some());assert!(clock_fg_at(cx+31,cy+1,&bar).is_some());
  }
 }
}
#[test]fn fitted_clock_and_all_status_hit_regions_share_display_geometry(){
 for(width,expected_x)in[(1024,928),(1280,1090)]{let bar=silkbar_model::for_desktop_width(width).unwrap();let(cx,cy,_,_)=module_rect(&bar,ModuleSlot::Clock);assert_eq!(cx,expected_x);assert!(silkbar_model::hit_test_action(&bar,cx+40,cy+10)==Action::OpenClock);
  for b in &bar.layout[6..]{assert!(b.x+b.w<=width);assert!(silkbar_model::hit_test_action(&bar,b.x,b.y)!=Action::None);}
  let left_end=bar.layout[..6].iter().map(|b|b.x+b.w).max().unwrap();assert!(left_end+16<=bar.layout[6].x);
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
