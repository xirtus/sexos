#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/document_view.rs"]pub mod document_view;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
#[path="../servers/silk-shell/src/desktop_content.rs"] mod desktop_content;
#[path="../servers/silk-shell/src/linen_text_delivery.rs"] mod linen_text_delivery;
use desktop_content::*;
fn items()->[Item<'static>;3]{[Item{id:1,name:b"notes.txt"},Item{id:2,name:b"kernel.rs"},Item{id:3,name:b"report.md"}]}
#[test]fn real_names_header_help_and_selection(){let v=list(&items(),2,640,400);let s=core::str::from_utf8(&v.text[..v.len]).unwrap();assert!(s.contains("LINEN / FILES\nJ/K SELECT  ENTER OPEN\n"));assert!(s.contains("> kernel.rs\n\n"));assert_eq!(v.selected_row,Some(1));assert_eq!(v.row_y,72);assert_eq!(v.row_h,36);}
#[test]fn narrow_rows_never_wrap_before_newline(){for width in [64,128,255,256,320,640]{let v=list(&items(),1,width,400);let scale=if width>=256{2}else{1};let cols=((width-16)/(6*scale)).clamp(1,80)as usize;for line in v.text[..v.len].split(|&c|c==b'\n'){assert!(line.len()<cols);}}}
#[test]fn long_names_truncate_visibly_and_sanitize(){let i=[Item{id:1,name:b"very-long-name-abcdef.txt"},Item{id:2,name:b"bad\n\x01"}];let v=list(&i,1,128,400);let text=core::str::from_utf8(&v.text[..v.len]).unwrap();assert!(text.contains("~\n"));assert!(text.contains("bad??"));}
#[test]fn selected_late_object_scrolls_bounded_window(){let mut i=[Item{id:0,name:b"test"};16];for(n,item)in i.iter_mut().enumerate(){item.id=n as u64+1;}let v=list(&i,14,640,400);assert_eq!(v.first,10);assert_eq!(v.rows,5);assert_eq!(v.selected_row,Some(3));assert!(v.len<=512);}
#[test]fn partial_backend_state_cannot_mutate_published_text(){let published=items();let before=list(&published,2,640,400);let _incomplete_new_names=[Item{id:99,name:b"unpublished"}];let after=list(&published,2,640,400);assert_eq!(before.text,after.text);assert!(!core::str::from_utf8(&after.text[..after.len]).unwrap().contains("unpublished"));}
#[test]fn small_height_clamps_rows_and_scrolls(){let v=list(&items(),3,640,110);assert_eq!(v.rows,1);assert_eq!(v.first,2);assert_eq!(v.selected_row,Some(0));assert_eq!(list(&items(),1,640,80).rows,0);}
#[test]fn empty_view_never_reads_missing_row(){let v=list(&[],1,640,400);assert_eq!(v.rows,0);assert_eq!(v.len,0);}
#[test]fn quil_inset_and_spindle_row_major_geometry(){assert_eq!(children(201,10,20,640,400)[0],Some(Child{sid:156,x:18,y:56,w:624,h:356}));let c=children(153,10,20,640,400);assert_eq!(c.map(|c|c.unwrap().sid),[154,160,161,162]);assert_eq!(c[0].unwrap().w,308);assert_eq!(c[1].unwrap().x,334);assert_eq!(c[2].unwrap().y,238);}
#[test]fn odd_native_dimensions_remain_inside_body(){for w in 25..80{for h in 53..100{for c in children(153,0,0,w,h).iter().flatten(){assert!(c.x>=8&&c.y>=36);assert!(c.x as u32+c.w<=w-8);assert!(c.y as u32+c.h<=h-8);}}}}
#[test]fn tiny_unknown_frames_do_not_allocate_children(){assert_eq!(children(201,0,0,1,1),[None;4]);assert_eq!(children(300,0,0,640,400),[None;4]);}

mod integration {
    #![allow(dead_code,static_mut_refs)]
    use super::desktop_content;
    use super::document_view;
    use super::linen_text_delivery;
    static mut LINEN_TEXT:linen_text_delivery::Outbox=linen_text_delivery::Outbox::new(200);
    use super::app_custody;
    use std::sync::Mutex;
    static LOCK:Mutex<()>=Mutex::new(());
    static CALLS:Mutex<Vec<(u64,u64,u64,u64)>>=Mutex::new(Vec::new());
    const LINEN_FRAME_ID:u32=12;
    unsafe fn set_frame_top_bar(_:u32,enabled:bool){assert!(enabled);}
    unsafe fn send_frame_tab_info(_:u32){}
    const SLOT_DISPLAY:usize=5;const SURFACE_ID_LINEN:u64=200;const LINEN_MAX_OBJECTS:usize=16;
    #[derive(Clone,Copy)]struct Object{object_id:u64,name:[u8;24],name_len:u8,display_name:&'static str}
    static mut LINEN_OBJECTS:[Option<Object>;16]=[None;16];
    static mut SELECTED_LINEN_OBJECT_ID:u64=1;
    static mut SURFACE_200_X:i32=0;static mut SURFACE_200_Y:i32=50;
    static mut SURFACE_200_W:u32=640;static mut SURFACE_200_H:u32=400;
    static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
    macro_rules! serial_println {($($arg:tt)*)=>{{ let _=format_args!($($arg)*); }}}
    #[macro_export]macro_rules! discard_record {($($arg:tt)*)=>{{ let _=format_args!($($arg)*); }}}
    mod sex_pdx {pub use crate::discard_record as serial_record;}
    static mut SPINDLE_VIEWS:[crate::spindle_view::Delivery;2]=[crate::spindle_view::Delivery::new(),crate::spindle_view::Delivery::new()];
    fn pdx_call(_:usize,op:u64,sid:u64,a:u64,b:u64){CALLS.lock().unwrap().push((op,sid,a,b));}
    static mut DOCUMENT_VIEWS:[document_view::Delivery;2]=[document_view::Delivery::new(),document_view::Delivery::new()];
    include!(env!("DESKTOP_CONTENT_PRODUCTION"));
    #[test]fn actual_renderer_emits_real_text_protocol_after_complete_view(){
        let _lock=LOCK.lock().unwrap();CALLS.lock().unwrap().clear();
        unsafe{LINEN_OBJECTS=[None;16];LINEN_OBJECTS[0]=Some(Object{object_id:1,name:[0;24],name_len:0,display_name:"saved.txt"});LINEN_TEXT=linen_text_delivery::Outbox::new(200);linen_render_object_list();
            assert!(!CALLS.lock().unwrap().iter().any(|c|matches!(c.0,0xFA|0xFB)));
            while LINEN_TEXT.pending(){LINEN_TEXT.poll(4,|op|{pdx_call(SLOT_DISPLAY,op.opcode,op.surface,op.packed,op.control);linen_text_delivery::Admission::Accepted});}}
        let calls=CALLS.lock().unwrap();let clear=calls.iter().position(|c|c.0==0xFA).unwrap();
        let mut text=[0u8;512];let mut len=0;
        for &(op,sid,word,desc)in &calls[clear+1..]{if op!=0xFB{continue}assert_eq!(sid,200);let offset=((desc&255)|(((desc>>12)&15)<<8))as usize;let n=((desc>>8)&15)as usize;assert_eq!(desc>>32,0xf0f8ff);for i in 0..n{text[offset+i]=(word>>(i*8))as u8;}len=len.max(offset+n);}
        let text=core::str::from_utf8(&text[..len]).unwrap();assert!(text.contains("> saved.txt\n"));assert!(text.contains("ENTER OPEN"));
    }
    #[test]fn actual_native_proxy_requires_live_catalog_and_preserves_owner_id(){
        let _lock=LOCK.lock().unwrap();CALLS.lock().unwrap().clear();
        unsafe{APP_CUSTODY=[None;3];native_child_geometry(201,0,50,640,400);assert!(CALLS.lock().unwrap().is_empty());
            APP_CUSTODY[0]=Some(app_custody::Entry::new(38,9,9,156));native_child_geometry(201,0,50,640,400);
            assert_eq!(CALLS.lock().unwrap()[0],(0xEC,156,(86u64<<32)|8,(356u64<<32)|624));
            CALLS.lock().unwrap().clear();APP_CUSTODY[0].as_mut().unwrap().close_pending=true;native_child_geometry(201,0,50,640,400);assert!(CALLS.lock().unwrap().is_empty());
            APP_CUSTODY[0].as_mut().unwrap().close_pending=false;APP_CUSTODY[0].as_mut().unwrap().dead=true;native_child_geometry(201,0,50,640,400);assert!(CALLS.lock().unwrap().is_empty());}
    }
}

#[test]fn successor_spindle_geometry_uses_only_reserved_fresh_children(){
 let fresh=children(306,10,50,640,335);let old=children(153,10,50,640,335);
 for (i,(n,p)) in fresh.into_iter().zip(old).enumerate(){let n=n.unwrap();let p=p.unwrap();assert_eq!(n.sid,[302,303,304,305][i]);assert_eq!((n.x,n.y,n.w,n.h),(p.x,p.y,p.w,p.h));}
}
