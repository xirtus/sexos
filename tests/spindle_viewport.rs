#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../apps/spindle/src/viewport.rs"]mod viewport;
use spindle_view::*;
#[test]fn startup_tile_keeps_full_prompt_in_one_visible_bottom_row(){let v=Layout::new(1280,134);assert_eq!(v.rows,[1,1]);let t=viewport::render(v,b"echo spindle17",2,0,|_|b"old output");assert_eq!(core::str::from_utf8(&t[2].bytes[..t[2].len]).unwrap(),"> echo spindle17|   \n");}
#[test]fn minimum_height_collapses_upper_pair_and_gives_prompt_the_body(){let v=Layout::new(120,80);assert_eq!(v.rows[0],0);assert!(v.rows[1]>0);let t=viewport::render(v,b"a long command",0,0,|_|panic!());assert_eq!(t[0].len,0);assert_eq!(t[1].len,0);let start=(t[2].rows-1)*(t[2].cols+1);assert_eq!(&t[2].bytes[start..start+3],b"> <");}
#[test]fn layout_and_all_nonblank_cells_fit_actual_panes(){for w in 120..=1280{for h in [80,90,100,133,134,180,335,670]{let v=Layout::new(w,h);let t=viewport::render(v,&[b'x';256],128,0,|_|b"some output longer than a tile");for(i,p)in v.panes.iter().enumerate(){assert!(p.x+p.w<=w-8);assert!(p.y+p.h<=h-8);let s=scale(p.w);if t[i].len==0{continue}assert!(8+v.cols as u32*6*s<=p.w);assert!(12+(v.rows[i/2]-1)as u32*9*s+7*s<=p.h);assert!(t[i].len<=MAX_TEXT);}}}}
#[test]fn shrinking_and_expanding_reconstructs_original_scrollback_and_input(){let command=b"echo unchanged";let small=viewport::render(Layout::new(120,80),command,20,0,|_|b"line retained");let large=viewport::render(Layout::new(1280,670),command,20,0,|_|b"line retained");assert!(small[0].len==0);assert!(large[0].len>0);assert!(core::str::from_utf8(&large[2].bytes[..large[2].len]).unwrap().contains("> echo unchanged"));assert_eq!(command,b"echo unchanged");}
#[test]fn only_exact_shell_and_incarnation_accept_newer_layout(){let n=Notice{target:17,task:17,w:640,h:134,sequence:8};assert_eq!(Notice::decode(3,17,17,n.args(),7),Some(n));for(c,p,t,last)in[(7,17,17,7),(3,12,12,7),(3,17,12,7),(3,17,17,8)]{assert!(Notice::decode(c,p,t,n.args(),last).is_none())}assert!(Notice::decode(3,17,17,[n.args()[0],0,8],0).is_none());}
#[test]fn full_retains_exact_notice_and_coalesces_newest_geometry(){let mut d=Delivery::new();d.queue(17,17,640,134);let mut first=None;d.poll(true,true,|n|{first=Some(n);Err(ERR_SERVICE_NOT_READY)});d.poll(true,true,|n|{assert_eq!(Some(n),first);Err(ERR_SERVICE_NOT_READY)});d.queue(17,17,640,335);d.poll(true,true,|n|{assert_eq!(n.h,335);assert!(n.sequence>first.unwrap().sequence);Ok(())});d.poll(true,true,|_|panic!());}
#[test]fn reserve_can_wait_without_being_retired_and_death_fences_pending(){let mut d=Delivery::new();d.poll(false,false,|_|panic!());d.queue(17,17,640,134);d.poll(true,false,|_|panic!());d.poll(false,true,|_|panic!());d.queue(17,17,640,335);d.poll(true,true,|_|panic!());}
#[test]fn bounded_formatter_suppresses_all_text_below_physical_glyph_limit(){let t=viewport::render(Layout::new(16,44),b"must not clip",2,0,|_|b"line");assert!(t.iter().all(|t|t.len==0));}

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
 assert_eq!(v.panes,[Rect{x:8,y:36,w:244,h:333},Rect{x:260,y:36,w:244,h:333},
  Rect{x:8,y:377,w:244,h:333},Rect{x:260,y:377,w:244,h:333}]);
 assert_eq!((v.cols,v.columns(),v.rows),(18,36,[6,6]));
 let text=viewport::render(v,b"echo readable",0,0,|_|panic!());
 assert!(text.iter().all(|t|t.len==114&&t.cols==18&&t.rows==6));
}

fn rendered_bottom_prompt(text:&[viewport::Text;4])->Vec<u8>{
 let mut prompt=Vec::new();
 for pane in &text[2..]{assert!(pane.rows>0);let start=(pane.rows-1)*(pane.cols+1);
  prompt.extend_from_slice(&pane.bytes[start..start+pane.cols]);}
 prompt
}
#[test]fn compact_prompt_home_middle_and_tail_keep_caret_and_exact_source_input(){
 let command=b"echo abcdefghijklmnopqrstuvwxyz0123456789 abcdefghijklmnopqrstuvwxyz";
 let original=*command;let v=Layout::new(512,335);assert_eq!(v.columns(),36);
 let home=rendered_bottom_prompt(&viewport::render_at(v,command,0,0,0,|_|panic!()));
 assert_eq!(&home[..3],b"> |");assert_eq!(&home[3..35],&command[..32]);assert_eq!(home[35],b'>');
 let middle=rendered_bottom_prompt(&viewport::render_at(v,command,34,0,0,|_|panic!()));
 assert_eq!(&middle[..3],b"> <");assert_eq!(&middle[3..34],&command[3..34]);assert_eq!(&middle[34..],b"|>");
 let tail=rendered_bottom_prompt(&viewport::render_at(v,command,command.len(),0,0,|_|panic!()));
 assert_eq!(&tail[..3],b"> <");assert_eq!(&tail[3..35],&command[command.len()-32..]);assert_eq!(tail[35],b'|');
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
  assert!(compact.iter().all(|t|t.cols==18));assert!(zoom.iter().all(|t|t.cols==20));
  assert_eq!(rendered_bottom_prompt(&compact).len(),36);assert_eq!(rendered_bottom_prompt(&zoom).len(),40);
  for text in [&compact,&zoom]{let prompt=rendered_bottom_prompt(text);
   assert_eq!(prompt.iter().filter(|&&ch|ch==b'|').count(),1);
   assert!(text.iter().all(|t|t.len<=MAX_TEXT));}
  assert_eq!(command,&original);
 }
}
#[test]fn asymmetric_scale_boundary_and_short_tiles_keep_prompt_in_uncropped_lower_band(){
 // Odd width279 straddles the scale boundary: left127 remains1x, right128
 // uses2x. Both halves share the smaller safe column/row budget.
 let narrow=Layout::new(278,90);assert_eq!((narrow.cols,narrow.rows),(17,[1,1]));
 let asymmetric=Layout::new(279,104);
 assert_eq!((asymmetric.panes[0].w,asymmetric.panes[1].w),(127,128));
 assert_eq!((scale(127),scale(128)),(1,2));assert_eq!((asymmetric.cols,asymmetric.rows),(8,[1,1]));
 let command=b"echo keep the complete command while the view becomes short";let original=*command;
 for w in [279,280,512]{for h in 90..=103{
  let v=Layout::new(w,h);assert_eq!(v.rows,[0,2]);
  assert_eq!((v.panes[0].h,v.panes[1].h),(0,0));assert_eq!(v.panes[2].y,36);assert_eq!(v.panes[2].h,h-44);
  let text=viewport::render_at(v,command,command.len(),0,0,|_|panic!());assert_eq!((text[0].len,text[1].len),(0,0));
  let prompt=rendered_bottom_prompt(&text);assert_eq!(&prompt[..3],b"> <");assert_eq!(prompt.last(),Some(&b'|'));
  for i in 2..4{let p=v.panes[i];let s=scale(p.w);assert!(8+v.cols as u32*6*s<=p.w);
   assert!(12+(text[i].rows-1)as u32*9*s+7*s<=p.h);assert!(text[i].len<=MAX_TEXT);}
  assert_eq!(command,&original);
 }}
 assert_eq!(Layout::new(512,104).rows,[1,1]);
}
#[test]fn readable_frames_keep_finite_text_limits_from_tiny_to_extreme_dimensions(){
 for w in [0,1,16,120,127,128,278,279,280,512,1024,4096,u32::MAX]{
  for h in [0,1,44,80,90,103,104,718,4096,u32::MAX]{
   let v=Layout::new(w,h);assert!(v.cols<=20);assert!(v.rows.iter().all(|&n|n<=6));
   let text=viewport::render_at(v,&[b'x';256],128,1,0,|_|b"retained output");
   for t in text{assert!(t.len<=MAX_TEXT);if t.len>0{assert_eq!(t.len,t.rows*(t.cols+1));
    assert!(t.bytes[..t.len].chunks_exact(t.cols+1).all(|row|row[t.cols]==b'\n'));}}
  }
 }
 let largest=viewport::render(Layout::new(4096,4096),b"",0,0,|_|panic!());
 assert!(largest.iter().all(|t|t.len==MAX_TEXT));
}
