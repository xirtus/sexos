#![allow(dead_code)]
macro_rules! serial_println{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
const CMD_MAX:usize=256;struct History;
fn history_nav(_: &mut CmdLine,_:&History,_:bool)->bool{false}
fn ghost_accept(_: &mut CmdLine,_:&History)->bool{false}
fn scrollback_page(_:bool){}
include!(env!("SPINDLE_EDITOR_SOURCE"));
#[test]fn real_insert_navigation_and_middle_backspace_preserve_expected_text(){
 let mut l=CmdLine::new();let h=History;l.set_from_slice(b"abcd");handle_key(0x47,&mut l,&h);assert_eq!(l.cur,0);
 handle_key(0x4D,&mut l,&h);handle_key(0x4D,&mut l,&h);assert_eq!(l.cur,2);
 handle_key(0x2D,&mut l,&h);assert_eq!(l.as_bytes(),b"abxcd");assert_eq!(l.cur,3);
 handle_key(0x0E,&mut l,&h);assert_eq!(l.as_bytes(),b"abcd");assert_eq!(l.cur,2);
 handle_key(0x4B,&mut l,&h);handle_key(0x0E,&mut l,&h);assert_eq!(l.as_bytes(),b"bcd");assert_eq!(l.cur,0);
 handle_key(0x0E,&mut l,&h);assert_eq!(l.as_bytes(),b"bcd");handle_key(0x4F,&mut l,&h);assert_eq!(l.cur,3);
}
#[test]fn real_arrow_and_home_end_controls_work_in_normal_mode_without_editing(){
 let mut l=CmdLine::new();let h=History;l.set_from_slice(b"keep text");l.mode=ViMode::Normal;
 for (key,pos)in[(0x47,0),(0x4D,1),(0x4F,9),(0x4B,8)]{handle_key(key,&mut l,&h);assert_eq!(l.cur,pos);assert_eq!(l.as_bytes(),b"keep text");assert!(l.mode==ViMode::Normal);}
}
#[test]fn full_editor_rejects_overflow_and_retains_cursor_bounds(){let mut l=CmdLine::new();let h=History;l.set_from_slice(&[b'z';256]);handle_key(0x2D,&mut l,&h);assert_eq!(l.as_bytes(),&[b'z';256]);assert_eq!(l.cur,256);handle_key(0x47,&mut l,&h);handle_key(0x0E,&mut l,&h);assert_eq!(l.cur,0);assert_eq!(l.len,256);}
