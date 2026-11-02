#![allow(dead_code)]
macro_rules! serial_println{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
const CMD_MAX:usize=256;struct History;
fn history_nav(_: &mut CmdLine,_:&History,_:bool)->bool{false}
fn ghost_accept(_: &mut CmdLine,_:&History)->bool{false}
fn scrollback_page(_:bool){}
include!(env!("SPINDLE_EDITOR_SOURCE"));
#[test]fn real_insert_navigation_and_middle_backspace_preserve_expected_text(){
 let mut l=CmdLine::new();let h=History;l.set_from_slice(b"abcd");handle_key(0x47,false,&mut l,&h);assert_eq!(l.cur,0);
 handle_key(0x4D,false,&mut l,&h);handle_key(0x4D,false,&mut l,&h);assert_eq!(l.cur,2);
 handle_key(0x2D,false,&mut l,&h);assert_eq!(l.as_bytes(),b"abxcd");assert_eq!(l.cur,3);
 handle_key(0x0E,false,&mut l,&h);assert_eq!(l.as_bytes(),b"abcd");assert_eq!(l.cur,2);
 handle_key(0x4B,false,&mut l,&h);handle_key(0x0E,false,&mut l,&h);assert_eq!(l.as_bytes(),b"bcd");assert_eq!(l.cur,0);
 handle_key(0x0E,false,&mut l,&h);assert_eq!(l.as_bytes(),b"bcd");handle_key(0x4F,false,&mut l,&h);assert_eq!(l.cur,3);
}
#[test]fn real_arrow_and_home_end_controls_work_in_normal_mode_without_editing(){
 let mut l=CmdLine::new();let h=History;l.set_from_slice(b"keep text");l.mode=ViMode::Normal;
 for (key,pos)in[(0x47,0),(0x4D,1),(0x4F,9),(0x4B,8)]{handle_key(key,false,&mut l,&h);assert_eq!(l.cur,pos);assert_eq!(l.as_bytes(),b"keep text");assert!(l.mode==ViMode::Normal);}
}
#[test]fn full_editor_rejects_overflow_and_retains_cursor_bounds(){let mut l=CmdLine::new();let h=History;l.set_from_slice(&[b'z';256]);handle_key(0x2D,false,&mut l,&h);assert_eq!(l.as_bytes(),&[b'z';256]);assert_eq!(l.cur,256);handle_key(0x47,false,&mut l,&h);handle_key(0x0E,false,&mut l,&h);assert_eq!(l.cur,0);assert_eq!(l.len,256);}
#[test]fn shift_layer_types_capitals_and_operator_symbols(){
 let mut l=CmdLine::new();let h=History;
 for (code,shift) in [(0x2E,true),(0x1E,false),(0x09,true),(0x0A,true),(0x04,false),(0x0D,true),(0x05,false),(0x0B,true),(0x35,false),(0x06,true)]{handle_key(code,shift,&mut l,&h);}
 assert_eq!(l.as_bytes(),b"Ca*(3+4)/%");
}
