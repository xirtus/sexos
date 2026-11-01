#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
use launcher::*;
fn id()->Identity {Identity{domain:9,task:91,surface:156,coordinator:3}}
#[test]fn exact_live_instance_only(){assert_eq!(resolve(id(),Some(id()),Evidence::Live,false,false),Status::Running);}
#[test]fn missing_and_mismatched_catalog_never_focus(){
 for c in [None,Some(Identity{task:9,..id()}),Some(Identity{domain:12,..id()}),Some(Identity{surface:201,..id()}),Some(Identity{coordinator:15,..id()})] {assert_eq!(resolve(id(),c,Evidence::Live,false,false),Status::Unavailable);}
}
#[test]fn terminal_or_closing_cache_cannot_become_live(){assert_eq!(resolve(id(),Some(id()),Evidence::Live,false,true),Status::Unavailable);assert_eq!(resolve(id(),Some(id()),Evidence::Live,true,false),Status::Closing);}
#[test]fn exact_dead_is_closed_even_before_all_service_acks(){for acked in [0,1,31]{assert_eq!(resolve(id(),Some(id()),Evidence::Dead{domain:9,task:91,required:31,acked},false,true),Status::Closed);}}
#[test]fn malformed_dead_and_errors_never_authorize(){for e in [Evidence::Invalid,Evidence::Dead{domain:9,task:9,required:31,acked:31},Evidence::Dead{domain:9,task:91,required:1,acked:1},Evidence::Dead{domain:9,task:91,required:31,acked:63}]{assert_eq!(resolve(id(),Some(id()),e,false,false),Status::Unavailable);}assert_eq!(resolve(id(),Some(id()),Evidence::Retry,false,false),Status::Closing);}
#[test]fn only_running_is_enabled_and_closed_explains_missing_restart(){for s in [Status::Closing,Status::Closed,Status::Unavailable]{assert!(!s.presentation().0);}assert!(Status::Running.presentation().0);assert_eq!(Status::Closed.presentation().2,"no_boot_successor");}
#[test]fn selection_scrolls_every_command_into_visible_window(){let rows=[Row{name:"Quil",status:"RUNNING / FOCUS"};10];for selected in 0..10 {let v=text(&rows,selected,480,280);assert_eq!(v.rows,5);assert!(selected>=v.first&&selected<v.first+v.rows);let s=std::str::from_utf8(&v.bytes[..v.len]).unwrap();assert_eq!(s.matches("> Quil").count(),1);assert_eq!((v.row_y,v.row_h,v.scale),(72,36,2));}}
#[test]fn bounded_sanitized_text_never_double_wraps(){let rows=[Row{name:"very long\nname that must not inject another status line",status:"CLOSED / REBOOT TO REOPEN"};10];let v=text(&rows,9,256,280);assert!(v.len<=512);let s=std::str::from_utf8(&v.bytes[..v.len]).unwrap();assert!(s.lines().all(|l|l.len()<20));assert!(!s.contains("long\nname"));assert_eq!(s.lines().count(),12);}
#[test]fn tiny_or_empty_view_is_bounded(){for (w,h) in [(0,0),(12,280),(480,40)]{assert_eq!(text(&[Row{name:"Quil",status:"Closed"}],0,w,h).rows,0);}assert_eq!(text(&[],0,480,280).len,0);}
#[test]fn wide_future_labels_still_emit_every_complete_line(){let value="x".repeat(200);let rows=[Row{name:&value,status:&value};10];let v=text(&rows,9,1920,800);assert!(v.len<=512);assert_eq!(v.bytes[..v.len].iter().filter(|&&b|b==b'\n').count(),12);}

#[test]fn reserve_count_describes_only_ordered_boot_states(){
 for first in 0..=4{for third in 0..=4{
  let expected=match(first,third){(0,0)|(1,0)=>Some(2),(2,0)|(3,0)|(3,1)=>Some(1),(3,2)|(3,3)=>Some(0),_=>None};
  assert_eq!(quil_reserve(first,third),expected);
 }}
 assert_eq!(reserve_label("closed",false,"boot_successor_ready",Some(2)),"closed");
 assert_eq!(reserve_label("CLEANING",true,"predecessor_cleanup_pending",Some(2)),"CLEANING");
 for count in [None,Some(0),Some(1),Some(2)]{assert_eq!(reserve_label("UNAVAILABLE",false,"incarnation_unverified",count),"UNAVAILABLE");}
 assert_eq!(reserve_label("RUNNING / FOCUS",true,"original_incarnation_live",Some(0)),"RUNNING / LAST INSTANCE");
}

#[test]fn live_actions_distinguish_open_restore_other_workspace_and_focus(){
 assert_eq!(live_action(None,0),LiveAction::Open);
 for active in 0..5{for scene in 0..5{for minimized in [false,true]{for visible in [false,true]{
  let expected=if scene!=active{LiveAction::BringHere}else if minimized||!visible{LiveAction::Restore}else{LiveAction::Focus};
  assert_eq!(live_action(Some((scene,minimized,visible)),active),expected);
 }}}}
 assert_eq!(LiveAction::BringHere.title(false),"Bring Here Spindle");assert_eq!(LiveAction::Restore.title(true),"Restore Quil");
 for a in [LiveAction::Open,LiveAction::Restore,LiveAction::BringHere,LiveAction::Focus]{for quil in [false,true]{assert!(a.title(quil).len()<39);}}
}

#[test]fn count_labels_cover_every_u8_count_with_exact_short_ascii() {
 for count in 0..=u8::MAX {
  let label=reserve_count_label(true,"original_incarnation_live",Some(count)).unwrap();
  let expected=match count{0=>"LAST INSTANCE".to_owned(),1=>"1 REOPEN LEFT".to_owned(),_=>format!("{count} REOPENS LEFT")};
  assert_eq!(label.as_str(),expected);assert!(label.as_str().is_ascii());assert!(label.as_str().len()+2<=19);
  let ready=reserve_count_label(true,"boot_successor_ready",Some(count));
  if count==0 {assert!(ready.is_none());}else{
   let expected=if count==1{"LAST RESERVED".to_owned()}else{format!("{count} RESERVED LEFT")};
   assert_eq!(ready.as_ref().unwrap().as_str(),expected);assert!(expected.is_ascii());assert!(expected.len()+2<=19);
  }
 }
}
#[test]fn count_labels_reject_unknown_unavailable_and_inconsistent_states() {
 for reason in ["original_incarnation_live","boot_successor_ready"] {
  assert!(reserve_count_label(true,reason,None).is_none());
  for count in 0..=u8::MAX {assert!(reserve_count_label(false,reason,Some(count)).is_none());}
 }
 for reason in ["", "ORIGINAL_INCARNATION_LIVE", " original_incarnation_live", "original_incarnation_live\n> OPEN", "boot_successor_ready\0", "predecessor_cleanup_pending", "incarnation_closing", "incarnation_unverified"] {
  for count in [None,Some(0),Some(1),Some(255)] {assert!(reserve_count_label(true,reason,count).is_none());}
 }
 assert!(reserve_count_label(true,"boot_successor_ready",Some(0)).is_none());
}
#[test]fn count_labels_remain_owned_when_later_values_are_formatted() {
 let first=reserve_count_label(true,"original_incarnation_live",Some(15)).unwrap();
 let second=reserve_count_label(true,"boot_successor_ready",Some(255)).unwrap();
 let copy=first;let _third=reserve_count_label(true,"original_incarnation_live",Some(0)).unwrap();
 assert_eq!(first.as_str(),"15 REOPENS LEFT");assert_eq!(copy,first);assert_eq!(second.as_str(),"255 RESERVED LEFT");
}
#[test]fn count_labels_fit_actual_256_and480_launcher_rows_without_aliasing() {
 let names=["Spindle","Quil","Browser","Spindle next","Quil next","Browser next","Last instance"];
 let labels=[reserve_count_label(true,"original_incarnation_live",Some(8)).unwrap(),reserve_count_label(true,"original_incarnation_live",Some(15)).unwrap(),reserve_count_label(true,"boot_successor_ready",Some(255)).unwrap(),reserve_count_label(true,"boot_successor_ready",Some(1)).unwrap(),reserve_count_label(true,"original_incarnation_live",Some(1)).unwrap(),reserve_count_label(true,"boot_successor_ready",Some(2)).unwrap(),reserve_count_label(true,"original_incarnation_live",Some(0)).unwrap()];
 let rows:Vec<_>=names.iter().zip(labels.iter()).map(|(name,label)|Row{name,status:label.as_str()}).collect();
 for width in [256,480] {for selected in 0..rows.len(){
  let view=text(&rows,selected,width,280);assert!(view.len<=512);assert_eq!(view.rows,5);assert_eq!((view.scale,view.row_y,view.row_h),(2,72,36));
  let lines:Vec<_>=core::str::from_utf8(&view.bytes[..view.len]).unwrap().lines().collect();assert_eq!(lines.len(),12);
  for offset in 0..view.rows {let index=view.first+offset;let prefix=if index==selected{"> "}else{"  "};
   assert_eq!(lines[2+offset*2],format!("{prefix}{}",names[index]));assert_eq!(lines[3+offset*2],format!("  {}",labels[index].as_str()));
  }
  if width==256 {assert!(lines.iter().all(|line|line.len()<=19));}
 }}
}
#[test]fn rejected_count_hint_preserves_old_fallback_injection_sanitization() {
 let fallback="CLOSING\n> forged status";let status=reserve_count_label(false,"original_incarnation_live",Some(255));assert!(status.is_none());
 let rows=[Row{name:"Quil",status:status.as_ref().map_or(fallback,ReserveCountLabel::as_str)}];let view=text(&rows,0,256,160);
 let lines:Vec<_>=core::str::from_utf8(&view.bytes[..view.len]).unwrap().lines().collect();assert_eq!(lines.len(),4);assert!(!lines.iter().any(|line|line.starts_with("> forged")));assert!(lines[3].contains('?'));
}
