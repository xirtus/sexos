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
