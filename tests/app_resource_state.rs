#![allow(dead_code)]
#[path="../kernel/src/app_resource_state.rs"] mod production;
use production::*;
use std::cell::RefCell;
fn page(va:u64,leaf:u64,pa:u64)->PrivatePage{PrivatePage{va,leaf,expected:pa|5|(15u64<<59)}}
#[test]fn final_elf_flags_preserve_own_pkey_and_permissions(){
 let before=0x8000000000000007;for pkey in 0..16 {let flags=final_elf_flags(before,pkey);assert_eq!((flags>>59)&15,pkey as u64);assert_eq!(flags&!(15<<59),before);}
}
#[test]fn loader_final_flags_regression_detects_pkey_loss(){
 let initial=final_elf_flags(7,15);let old_final=0x8000000000000005;
 assert_ne!((initial>>59)&15,(old_final>>59)&15);
 assert_eq!((final_elf_flags(old_final,15)>>59)&15,15);
 assert_eq!(final_elf_flags(old_final,15)&2,0);
}
#[test]fn role_ack_is_exact_holder_task_and_single_role(){
 let grant=LifecycleGrant{target_pd:15,task_id:77,holder_pd:3,rights:QUERY|ACK,ack_bit:1};
 assert_eq!(grant.authorize(3,15,77,ACK),Ok(()));
 for (holder,target,task,right) in [(1,15,77,ACK),(3,9,77,ACK),(3,15,78,ACK),(3,15,77,RETIRE)]{assert_eq!(grant.authorize(holder,target,task,right),Err(DENIED));}
 for bit in [0,3,32,255]{assert_eq!(LifecycleGrant{ack_bit:bit,..grant}.authorize(3,15,77,ACK),Err(DENIED));}
}
#[test]fn all_private_pages_preflight_before_first_clear(){
 let pages=[page(0x4000,1,0x1000),page(0x5000,2,0x2000)];let writes=RefCell::new(Vec::new());
 let result=retire_private_pages(&pages,|leaf|if leaf==1{pages[0].expected}else{pages[1].expected^2},|leaf|writes.borrow_mut().push(leaf),|_|panic!("flush on failure"));
 assert_eq!(result,Err(DENIED));assert!(writes.borrow().is_empty());
}
#[test]fn stale_physical_pkey_and_execute_permissions_are_denied(){
 let p=page(0x4000,1,0x1000);for xor in [4096,1<<59,1<<63,2,4]{assert_eq!(retire_private_pages(&[p],|_|p.expected^xor,|_|panic!("clear mismatch"),|_|panic!("flush mismatch")),Err(DENIED));}
}
#[test]fn accessed_dirty_allowed_clear_flush_every_page_before_handoff(){
 let pages=[page(0x4000,1,0x1000),page(0x5000,2,0x2000)];let events=RefCell::new(Vec::new());
 let count=retire_private_pages(&pages,|leaf|pages[(leaf-1)as usize].expected|0x60,|leaf|events.borrow_mut().push(("clear",leaf)),|va|events.borrow_mut().push(("flush",va))).unwrap();
 events.borrow_mut().push(("handoff",count as u64));assert_eq!(*events.borrow(),vec![("clear",1),("flush",0x4000),("clear",2),("flush",0x5000),("handoff",2)]);
}
#[test]fn duplicate_physical_or_virtual_or_leaf_identity_cannot_retire(){
 let a=page(0x4000,1,0x1000);
 for b in [page(0x4000,2,0x2000),page(0x5000,1,0x2000),page(0x5000,2,0x1000)]{
 assert_eq!(retire_private_pages(&[a,b],|leaf|if leaf==1{a.expected}else{b.expected},|_|panic!("clear alias"),|_|panic!("flush alias")),Err(DENIED));}
}
#[test]fn query_only_observer_cannot_ack_or_retire(){
 let grant=production::LifecycleGrant{target_pd:12,task_id:12,holder_pd:7,rights:production::QUERY,ack_bit:0};
 assert_eq!(grant.authorize(7,12,12,production::QUERY),Ok(()));
 assert_eq!(grant.authorize(7,12,12,production::ACK),Err(production::DENIED));
 assert_eq!(grant.authorize(7,12,12,production::RETIRE),Err(production::DENIED));
}
#[test]fn activation_grant_binds_holder_logical_incarnation_and_both_tasks(){
 let g=ActivationGrant{holder:3,logical_app:9,incarnation:2,predecessor:9,predecessor_task:91,successor:16,successor_task:161};
 assert_eq!(g.authorize(3,9,91,16,161),Ok(()));
 for (h,p,t,n,nt) in [(1,9,91,16,161),(3,12,91,16,161),(3,9,92,16,161),(3,9,91,17,161),(3,9,91,16,162),(3,9,91,9,161)]{
  assert_eq!(g.authorize(h,p,t,n,nt),Err(DENIED));
 }
 assert_eq!(ActivationGrant{incarnation:3,..g}.authorize(3,9,91,16,161),Err(DENIED));
 assert_eq!(ActivationGrant{logical_app:0,..g}.authorize(3,9,91,16,161),Err(DENIED));
}
#[test]fn successor_all_absent_leaves_preflight_before_any_restore(){
 let pages=[page(0x4000,1,0x1000),page(0x5000,2,0x2000)];let writes=RefCell::new(Vec::new());
 assert_eq!(restore_reserved_pages(&pages,|leaf|if leaf==2{0x60}else{0},|l,v|writes.borrow_mut().push((l,v)),|_|panic!("flush failed preflight")),Err(DENIED));
 assert!(writes.borrow().is_empty());
}
#[test]fn successor_restore_exact_boot_frames_flags_then_flushes(){
 let pages=[page(0x4000,1,0x1000),page(0x5000,2,0x2000)];let events=RefCell::new(Vec::new());
 assert_eq!(restore_reserved_pages(&pages,|_|0,|l,v|events.borrow_mut().push((l,v)),|va|events.borrow_mut().push((va,0))),Ok(2));
 assert_eq!(*events.borrow(),vec![(1,pages[0].expected),(0x4000,0),(2,pages[1].expected),(0x5000,0)]);
}
#[test]fn successor_rejects_aliases_and_empty_reservation(){
 let p=page(0x4000,1,0x1000);
 assert_eq!(restore_reserved_pages(&[],|_|0,|_,_|panic!(),|_|panic!()),Err(INVALID));
 for q in [page(0x4000,2,0x2000),page(0x5000,1,0x2000),page(0x5000,2,0x1000)]{
  assert_eq!(restore_reserved_pages(&[p,q],|_|0,|_,_|panic!(),|_|panic!()),Err(DENIED));
 }
}
