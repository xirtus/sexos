#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../apps/spindle/src/viewport.rs"]mod viewport;
use spindle_view::*;
fn row(t:&viewport::Text,index:usize)->&[u8]{let start=index*(t.cols+1);&t.bytes[start..start+t.cols]}
fn bottom_prompt(t:&viewport::Text)->Vec<u8>{assert!(t.rows>0);row(t,t.rows-1).to_vec()}
#[test]fn startup_tile_keeps_full_prompt_in_one_visible_bottom_row(){let v=Layout::new(1280,134);assert_eq!(v.rows,[4,0]);assert_eq!(v.cols,103);let t=viewport::render(v,b"echo spindle17",2,0,|_|b"old output");assert!(bottom_prompt(&t).starts_with(b"> echo spindle17|"));assert!(row(&t,t.rows-2).starts_with(b"old output"));}
#[test]fn minimum_height_keeps_the_prompt_in_the_body(){let v=Layout::new(120,80);assert_eq!(v.rows,[2,0]);let t=viewport::render(v,b"a long command",0,0,|_|panic!());assert_eq!(&bottom_prompt(&t)[..3],b"> <");}
#[test]fn terminal_pane_fills_the_body_and_other_surfaces_stay_empty(){for w in 120..=1280{for h in [80,90,100,133,134,180,335,670]{let v=Layout::new(w,h);let t=viewport::render(v,&[b'x';256],128,0,|_|b"some output longer than a tile");let p=v.panes[0];assert_eq!((p.x,p.y,p.w,p.h),(8,36,w-16,h-44));assert!(v.panes[1..].iter().all(|q|q.h==0));assert!(p.x+p.w<=w-8);assert!(p.y+p.h<=h-8);let s=scale(p.w);if t.len==0{continue}assert!(8+v.cols as u32*6*s<=p.w);assert!(12+(v.rows[0]-1)as u32*9*s+7*s<=p.h);assert!(t.len<=MAX_TEXT);}}}
#[test]fn shrinking_and_expanding_reconstructs_original_scrollback_and_input(){let command=b"echo unchanged";let small=viewport::render(Layout::new(120,80),command,20,0,|_|b"line retained");let large=viewport::render(Layout::new(1280,670),command,20,0,|_|b"line retained");assert!(small.rows<large.rows);assert!(core::str::from_utf8(&large.bytes[..large.len]).unwrap().contains("> echo unchanged"));assert_eq!(command,b"echo unchanged");}
#[test]fn only_exact_shell_and_incarnation_accept_newer_layout(){let n=Notice{target:17,task:17,w:640,h:134,sequence:8};assert_eq!(Notice::decode(3,17,17,n.args(),7),Some(n));for(c,p,t,last)in[(7,17,17,7),(3,12,12,7),(3,17,12,7),(3,17,17,8)]{assert!(Notice::decode(c,p,t,n.args(),last).is_none())}assert!(Notice::decode(3,17,17,[n.args()[0],0,8],0).is_none());}
#[test]fn full_retains_exact_notice_and_coalesces_newest_geometry(){let mut d=Delivery::new();d.queue(17,17,640,134);let mut first=None;d.poll(true,true,|n|{first=Some(n);Err(ERR_SERVICE_NOT_READY)});d.poll(true,true,|n|{assert_eq!(Some(n),first);Err(ERR_SERVICE_NOT_READY)});d.queue(17,17,640,335);d.poll(true,true,|n|{assert_eq!(n.h,335);assert!(n.sequence>first.unwrap().sequence);Ok(())});d.poll(true,true,|_|panic!());}
#[test]fn reserve_can_wait_without_being_retired_and_death_fences_pending(){let mut d=Delivery::new();d.poll(false,false,|_|panic!());d.queue(17,17,640,134);d.poll(true,false,|_|panic!());d.poll(false,true,|_|panic!());d.queue(17,17,640,335);d.poll(true,true,|_|panic!());}
#[test]fn bounded_formatter_suppresses_all_text_below_physical_glyph_limit(){let t=viewport::render(Layout::new(16,44),b"must not clip",2,0,|_|b"line");assert_eq!(t.len,0);}

#[test]fn every_cursor_position_remains_visible_without_mutating_long_input(){
 for len in 0..=256{let command=vec![b'x';len];let before=command.clone();for cursor in 0..=len{for cols in 8..=40{let p=viewport::prompt(&command,cursor,cols);let caret=p.caret.unwrap();assert!(caret>=2&&caret<cols);assert_eq!(p.bytes[caret],b'|');assert!(p.start<=cursor&&cursor<=p.end);assert!(p.end<=len);assert_eq!(command,before);}}}
}
#[test]fn horizontal_window_shifts_to_home_middle_and_end(){let command=b"echo spindle17zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
 let home=viewport::prompt(command,0,40);assert_eq!(&home.bytes[..16],b"> |echo spindle1");assert_eq!(home.bytes[39],b'>');
 let middle=viewport::prompt(command,14,40);assert_eq!(middle.bytes[middle.caret.unwrap()],b'|');assert_eq!(middle.start,0);
 let end=viewport::prompt(command,command.len(),40);assert!(end.start>0);assert_eq!(end.bytes[2],b'<');assert_eq!(end.bytes[39],b'|');
}

#[test]fn notice_carries_actual_catalog_task_and_rejects_domain_alias(){
 let mut d=Delivery::new();d.queue(17,170,640,335);let mut n=None;
 d.poll(true,true,|v|{n=Some(v);Ok(())});let n=n.unwrap();
 assert_eq!(n.task,170);assert!(Notice::decode(3,17,170,n.args(),0).is_some());
 assert!(Notice::decode(3,17,17,n.args(),0).is_none());
}

#[test]fn same_size_restore_forces_fresh_nonwrapping_notice_after_create(){
 let mut d=Delivery::new();d.queue(17,170,640,335);let mut first=0;
 d.poll(true,true,|n|{first=n.sequence;Ok(())});d.queue(17,170,640,335);assert!(!d.pending());
 d.refresh();d.queue(17,170,640,335);d.poll(true,false,|_|panic!());
 d.poll(true,true,|n|{assert!(n.sequence>first);assert_eq!(n.task,170);Ok(())});
 d.retire();d.refresh();d.queue(17,170,640,335);assert!(!d.pending());
}


#[test]fn spindle_scale_boundary_and_compact_tile_have_independent_readability_goldens(){
 assert_eq!(scale(0),1);assert_eq!(scale(127),1);assert_eq!(scale(128),2);
 assert_eq!(scale(244),2);assert_eq!(scale(255),2);assert_eq!(scale(256),2);
 let v=Layout::new(512,718);
 assert_eq!(v.panes[0],Rect{x:8,y:36,w:496,h:674});
 assert_eq!((v.cols,v.columns(),v.rows),(39,39,[37,0]));
 let text=viewport::render(v,b"echo readable",0,0,|_|panic!());
 assert_eq!((text.len,text.cols,text.rows),(1480,39,37));
}
#[test]fn maximized_window_is_one_wide_terminal(){
 let v=Layout::new(1280,750);assert_eq!((v.cols,v.rows),(103,[38,0]));
 let long=[b'q';103];let t=viewport::render(v,b"",1,0,|_|&long);
 assert_eq!(row(&t,t.rows-2),&long[..]);
}
#[test]fn compact_prompt_home_middle_and_tail_keep_caret_and_exact_source_input(){
 let command=b"echo abcdefghijklmnopqrstuvwxyz0123456789 abcdefghijklmnopqrstuvwxyz";
 let original=*command;let v=Layout::new(512,335);assert_eq!(v.columns(),39);
 let home=bottom_prompt(&viewport::render_at(v,command,0,0,0,|_|panic!()));
 assert_eq!(&home[..3],b"> |");assert_eq!(&home[3..38],&command[..35]);assert_eq!(home[38],b'>');
 let middle=bottom_prompt(&viewport::render_at(v,command,50,0,0,|_|panic!()));
 assert_eq!(&middle[..3],b"> <");let caret=middle.iter().position(|&c|c==b'|').unwrap();
 assert_eq!(&middle[caret-3..caret],&command[47..50]);
 let tail=bottom_prompt(&viewport::render_at(v,command,command.len(),0,0,|_|panic!()));
 assert_eq!(&tail[..3],b"> <");assert_eq!(&tail[3..38],&command[command.len()-35..]);assert_eq!(tail[38],b'|');
 for prompt in [home,middle,tail]{assert_eq!(prompt.iter().filter(|&&ch|ch==b'|').count(),1);}
 assert_eq!(command,&original);
}
#[test]fn compact_zoom_and_restore_reconstruct_same_prompt_and_scrollback_at_each_caret(){
 let command=b"echo abcdefghijklmnopqrstuvwxyz0123456789 abcdefghijklmnopqrstuvwxyz";
 let original=*command;
 for cursor in [0,34,command.len()]{
  let render=|w|viewport::render_at(Layout::new(w,718),command,cursor,2,0,|i|
   if i==0{b"first retained scrollback line"}else{b"second retained scrollback line"});
  let compact=render(512);let zoom=render(1024);let restored=render(512);
  assert_eq!(compact,restored);assert_ne!(compact,zoom);
  assert_eq!((compact.cols,zoom.cols),(39,81));
  assert_eq!(bottom_prompt(&compact).len(),39);assert_eq!(bottom_prompt(&zoom).len(),81);
  for text in [&compact,&zoom]{let prompt=bottom_prompt(text);
   assert_eq!(prompt.iter().filter(|&&ch|ch==b'|').count(),1);
   assert!(text.len<=MAX_TEXT);}
  assert_eq!(command,&original);
 }
}
#[test]fn short_tiles_keep_the_prompt_visible(){
 let command=b"echo keep the complete command while the view becomes short";let original=*command;
 for w in [279,280,512]{for h in 90..=103{
  let v=Layout::new(w,h);assert_eq!(v.rows,[2,0]);assert_eq!(v.panes[0].h,h-44);
  let text=viewport::render_at(v,command,command.len(),0,0,|_|panic!());
  let prompt=bottom_prompt(&text);assert_eq!(&prompt[..3],b"> <");assert_eq!(prompt.last(),Some(&b'|'));
  let p=v.panes[0];let s=scale(p.w);assert!(8+v.cols as u32*6*s<=p.w);
  assert!(12+(text.rows-1)as u32*9*s+7*s<=p.h);
  assert_eq!(command,&original);
 }}
}
#[test]fn readable_frames_keep_finite_text_limits_from_tiny_to_extreme_dimensions(){
 for w in [0,1,16,120,127,128,278,279,280,512,1024,4096,u32::MAX]{
  for h in [0,1,44,80,90,103,104,718,4096,u32::MAX]{
   let v=Layout::new(w,h);assert!(v.cols<=MAX_COLUMNS);assert!(v.lines()<=MAX_ROWS);
   let t=viewport::render_at(v,&[b'x';256],128,1,0,|_|b"retained output");
   assert!(t.len<=MAX_TEXT);if t.len>0{assert_eq!(t.len,t.rows*(t.cols+1));
    assert!(t.bytes[..t.len].chunks_exact(t.cols+1).all(|row|row[t.cols]==b'\n'));}
  }
 }
 let largest=viewport::render(Layout::new(4096,4096),b"",0,0,|_|panic!());
 assert_eq!(largest.len,MAX_ROWS*(MAX_COLUMNS+1));assert!(largest.len<=MAX_TEXT);
}
