// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure receiver, source bounds and Display boundary/glyph functions.
//! Host IPC metadata and tick inputs are controlled; no native authority claim.
#![allow(dead_code,static_mut_refs,unused_attributes)]
extern crate self as sex_pdx;
thread_local! {static DRAW_LOG:std::cell::RefCell<Vec<String>>=const{std::cell::RefCell::new(Vec::new())};}
#[macro_export]macro_rules! serial_record {($($arg:tt)*)=>{DRAW_LOG.with(|l|l.borrow_mut().push(format!($($arg)*)))};}
#[path="../crates/silkbar-model/src/lib.rs"]mod silkbar_model;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
use silkbar_model::{SilkBar,ModuleSlot,DEFAULT_THEME,DEFAULT_SILK_BAR,UpdateKind,SilkBarUpdate};
use silkbar_model::civil_clock::{Applied,CivilClock,Reject,State,Update,HEALTH_TICKS,STAGNANT_TURNS};
include!(env!("SILKBAR_CIVIL_DISPLAY"));
const OWNER:(u32,u32)=(6,6);
const MIN:u32=clock_source::MIN_SECONDS as u32;const MAX:u32=clock_source::MAX_SECONDS as u32;
fn wire(state:State,seconds:u32,sequence:u32)->[u64;3]{let u=Update{state,seconds,sequence}.encode();[u.kind as u64,(u.index as u64)<<32|u.a as u64,u.b as u64]}
fn receive(c:&mut CivilClock,args:[u64;3],now:u64)->Result<Applied,Reject>{c.receive(OWNER,OWNER,args,MIN,MAX,now)}
fn text(line:&[u8])->&str{core::str::from_utf8(line).unwrap().trim_end()}
#[test]fn initial_unknown_uses_no_old_demo_digits_and_encoder_keeps_16byte_shape(){
 let c=CivilClock::new();assert_eq!((c.state(),c.seconds(),c.sequence(),c.high_water(),c.is_stale()),(State::Unknown,0,0,0,false));assert_eq!(civil_clock_lines(&c),[ *b"UTC unknown  ",*b"src? prec?   "]);
 assert_eq!(core::mem::size_of::<SilkBarUpdate>(),16);
 for state in[State::Unknown,State::Snapshot,State::Unavailable,State::Rollback,State::Exhausted]{let seconds=if state==State::Snapshot{MIN}else{0};let args=wire(state,seconds,17);assert_eq!(args,[11,((0x10|state as u64)<<32)|seconds as u64,17]);}
}
#[test]fn actual_display_boundary_authenticates_both_identity_words_before_effect(){
 for caller in[(0,0),(0,6),(6,0),(3,3),(6,3),(3,6),(6,60),(60,6),(u32::MAX,6),(6,u32::MAX)]{
  let mut bar=DEFAULT_SILK_BAR;let before=bar.civil_clock;
  assert_eq!(handle_civil_clock_update(&mut bar,caller.0,caller.1,wire(State::Snapshot,MIN,1),0),Err(Reject::Publisher));assert_eq!(bar.civil_clock,before);assert!(!bar.civil_enabled);
 }
 let mut bar=DEFAULT_SILK_BAR;assert_eq!(handle_civil_clock_update(&mut bar,6,6,wire(State::Snapshot,MIN,1),0),Ok(Applied::New));assert!(bar.civil_enabled);
}
#[test]fn every_reserved_bit_and_version_state_alias_is_rejected_atomically(){
 let good=wire(State::Snapshot,MIN,1);let mut cases=Vec::new();
 for bit in 32..64{let mut w=good;w[0]|=1u64<<bit;cases.push(w);}
 for bit in 40..64{let mut w=good;w[1]|=1u64<<bit;cases.push(w);}
 for bit in 32..64{let mut w=good;w[2]|=1u64<<bit;cases.push(w);}
 for index in 0..=255u64{if !(0x10..=0x14).contains(&index){let mut w=good;w[1]=index<<32|MIN as u64;cases.push(w);}}
 cases.extend([[11,0x11u64<<32|MIN as u64,0],wire(State::Snapshot,MIN-1,1),wire(State::Snapshot,MAX+1,1)]);
 for state in[State::Unknown,State::Unavailable,State::Rollback,State::Exhausted]{cases.push(wire(state,1,1));}
 for args in cases{let mut c=CivilClock::new();let before=c;assert_eq!(receive(&mut c,args,7),Err(Reject::Framing),"{args:x?}");assert_eq!(c,before);}
}
#[test]fn exact_replay_is_idempotent_conflict_and_old_sequences_never_refresh_health(){
 let mut c=CivilClock::new();let first=wire(State::Snapshot,MIN,2);assert_eq!(receive(&mut c,first,10),Ok(Applied::New));
 let before=c;assert_eq!(receive(&mut c,first,500),Ok(Applied::Duplicate));assert_eq!(c,before);
 assert_eq!(receive(&mut c,wire(State::Snapshot,MIN+1,2),500),Err(Reject::Conflict));assert_eq!(c,before);
 assert_eq!(receive(&mut c,wire(State::Unknown,0,1),500),Err(Reject::Stale));assert_eq!(c,before);
 assert!(c.health(10+HEALTH_TICKS));assert_eq!((c.state(),c.seconds(),c.is_stale()),(State::Unknown,0,true));
 let stale=c;assert_eq!(receive(&mut c,first,900),Ok(Applied::Duplicate));assert_eq!(c,stale);assert!(text(&civil_clock_lines(&c)[0]).contains("stale"));
 assert_eq!(receive(&mut c,wire(State::Snapshot,MIN,3),901),Ok(Applied::New));assert!(!c.is_stale());assert_eq!(c.seconds(),MIN);
}
#[test]fn ticks_and_stagnant_turns_only_hide_a_sample_never_advance_its_second(){
 let mut c=CivilClock::new();receive(&mut c,wire(State::Snapshot,MIN+59,1),100).unwrap();
 for tick in 101..100+HEALTH_TICKS{assert!(!c.health(tick));assert_eq!(c.seconds(),MIN+59);}
 assert!(c.health(100+HEALTH_TICKS));assert_eq!(c.seconds(),0);assert_eq!(c.high_water(),MIN+59);
 let mut c=CivilClock::new();receive(&mut c,wire(State::Snapshot,MIN,1),100).unwrap();
 for _ in 1..STAGNANT_TURNS{assert!(!c.health(100));assert_eq!(c.seconds(),MIN);}
 assert!(c.health(100));assert!(c.is_stale());
 let mut c=CivilClock::new();receive(&mut c,wire(State::Snapshot,MIN,1),100).unwrap();assert!(c.health(99));assert_eq!(c.seconds(),0);
}
#[test]fn midnight_is_derived_from_each_whole_source_sample_and_highwater_survives_failures(){
 let mut c=CivilClock::new();receive(&mut c,wire(State::Snapshot,MIN+86_399,1),0).unwrap();assert_eq!(text(&civil_clock_lines(&c)[0]),"UTC 23:59:59");
 receive(&mut c,wire(State::Snapshot,MIN+86_400,2),0).unwrap();assert_eq!(text(&civil_clock_lines(&c)[0]),"UTC 00:00:00");
 for(sequence,state)in[(3,State::Unknown),(4,State::Unavailable)]{receive(&mut c,wire(state,0,sequence),0).unwrap();assert_eq!(c.seconds(),0);assert_eq!(c.high_water(),MIN+86_400);}
 receive(&mut c,wire(State::Snapshot,MIN+86_399,5),0).unwrap();assert_eq!(c.state(),State::Rollback);assert_eq!(c.seconds(),0);assert_eq!(text(&civil_clock_lines(&c)[0]),"UTC blocked");
 assert_eq!(receive(&mut c,wire(State::Snapshot,MAX,6),0),Err(Reject::Terminal));assert_eq!(c.high_water(),MIN+86_400);
}
#[test]fn explicit_rollback_and_finite_sequence_exhaustion_are_permanent_and_nonnumeric(){
 for(state,sequence)in[(State::Rollback,2),(State::Exhausted,2),(State::Snapshot,u32::MAX)]{
  let mut c=CivilClock::new();receive(&mut c,wire(State::Snapshot,MIN,1),0).unwrap();let args=wire(state,if state==State::Snapshot{MAX}else{0},sequence);receive(&mut c,args,0).unwrap();assert_eq!(c.seconds(),0);
  assert_eq!(receive(&mut c,args,500),Ok(Applied::Duplicate));let before=c;
  if sequence!=u32::MAX{assert_eq!(receive(&mut c,wire(State::Snapshot,MAX,sequence+1),501),Err(Reject::Terminal));}
  assert!(!c.health(1_000_000));assert_eq!(c,before);assert!(!civil_clock_lines(&c)[0].contains(&b':'));
 }
}
#[test]fn legacy_clock_mutations_cannot_modify_civil_mode_or_its_pixels(){
 let mut bar=DEFAULT_SILK_BAR;handle_civil_clock_update(&mut bar,6,6,wire(State::Snapshot,MIN+12345,1),0).unwrap();let lines=civil_clock_lines(&bar.civil_clock);
 assert!(!silkbar_model::apply_update(&mut bar,SilkBarUpdate::new(UpdateKind::SetClock as u32,0,23,(59<<8)|59)));
 assert!(!silkbar_model::apply_update(&mut bar,Update{state:State::Snapshot,seconds:MAX,sequence:2}.encode()));
 bar.clock_hh=23;bar.clock_mm=59;bar.clock_ss=59;clock_canon_apply_to_bar(&mut bar);assert_eq!(civil_clock_lines(&bar.civil_clock),lines);assert_eq!(bar.civil_clock.seconds(),MIN+12345);
 let mut legacy=DEFAULT_SILK_BAR;assert!(!legacy.civil_enabled);assert!(silkbar_model::apply_update(&mut legacy,SilkBarUpdate::new(4,0,23,(59<<8)|59)));assert_eq!((legacy.clock_hh,legacy.clock_mm,legacy.clock_ss),(23,59,59));
}
#[test]fn both_profile_chips_paint_exact_two_line_source_bound_glyphs_and_erase_prior_digits(){
 for width in[1024,1280]{for state in[State::Unknown,State::Snapshot,State::Unavailable,State::Rollback,State::Exhausted]{
  let mut bar=silkbar_model::for_desktop_width(width).unwrap();bar.civil_enabled=true;receive(&mut bar.civil_clock,wire(state,if state==State::Snapshot{MIN+12345}else{0},1),0).unwrap();
  let lines=civil_clock_lines(&bar.civil_clock);let(cx,cy,cw,ch)=module_rect(&bar,ModuleSlot::Clock);assert!(cx+cw<=width);
  for y in cy.saturating_sub(1)..cy+ch+1{for x in cx.saturating_sub(1)..cx+cw+1{
   let mut expected=None;
   for row in 0..2{let yy=cy+if row==0{1}else{11};if x>=cx&&x<cx+78&&y>=yy&&y<yy+7{
    let col=(x-cx)/6;let px=(x-cx)%6;let glyph=(lines[row][col]-32)as usize;
    if px<5&&(FONT_ASCII_5X7[glyph][y-yy]>>(4-px))&1!=0{expected=Some(DEFAULT_THEME.text);}
   }}
   assert_eq!(clock_fg_at(x,y,&bar),expected,"width{width} state{state:?} x{x} y{y}");
  }}
 }}
}

#[test]fn completed_draw_receipt_is_bounded_to_changed_fitting_civil_stamps(){
 let mut bar=silkbar_model::for_desktop_width(1280).unwrap();bar.civil_enabled=true;
 DRAW_LOG.with(|l|l.borrow_mut().clear());
 civil_clock_drawn(&bar,100,100,"full");assert!(DRAW_LOG.with(|l|l.borrow().is_empty()));
 civil_clock_drawn(&bar,1280,800,"full");civil_clock_drawn(&bar,1280,800,"strip");
 assert_eq!(DRAW_LOG.with(|l|l.borrow().len()),1);
 assert!(DRAW_LOG.with(|l|l.borrow()[0].contains("sequence=0 state=Unknown seconds=0 stale=0 pixels=written path=full")));
 handle_civil_clock_update(&mut bar,6,6,wire(State::Snapshot,MIN,1),10).unwrap();
 civil_clock_drawn(&bar,1280,800,"strip");assert_eq!(DRAW_LOG.with(|l|l.borrow().len()),2);
 assert!(DRAW_LOG.with(|l|l.borrow()[1].contains("sequence=1 state=Snapshot")));
 assert!(bar.civil_clock.health(10+HEALTH_TICKS));civil_clock_drawn(&bar,1280,800,"strip");
 assert_eq!(DRAW_LOG.with(|l|l.borrow().len()),3);
 assert!(DRAW_LOG.with(|l|l.borrow()[2].contains("sequence=1 state=Unknown seconds=0 stale=1 pixels=written")));
 bar.civil_enabled=false;civil_clock_drawn(&bar,1280,800,"full");assert_eq!(DRAW_LOG.with(|l|l.borrow().len()),3);
}
