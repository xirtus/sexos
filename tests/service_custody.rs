#![allow(dead_code)]
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod production;
use production::*;use std::cell::Cell;
struct Auth{catalog:CatalogInfo,info:Result<LifecycleInfo,u64>,ack:Result<(),u64>,queries:usize,acks:usize}
impl Auth{fn new()->Self{Self{catalog:CatalogInfo{target_pd:15,task_id:71,surface_id:300,coordinator_pd:3},info:Ok(LifecycleInfo{target_pd:15,task_id:71,required_mask:31,acked_mask:0}),ack:Ok(()),queries:0,acks:0}}}
impl Authority for Auth{
 fn catalog(&mut self,slot:usize)->Result<CatalogInfo,u64>{self.queries+=1;if slot==38{Ok(self.catalog)}else{Err(INVALID)}}
 fn info(&mut self,_:usize)->Result<LifecycleInfo,u64>{self.queries+=1;self.info}
 fn ack(&mut self,slot:usize,task:u32)->Result<(),u64>{assert_eq!((slot,task),(38,71));self.acks+=1;self.ack}
}
#[test]fn forged_hints_have_no_queries_fences_or_effects(){
 for(op,caller,args)in[(0x108,0,[15,71,0]),(0x107,15,[15,71,0]),(0x107,0,[0,71,0]),(0x107,0,[15,0,0]),(0x107,0,[1<<32,71,0]),(0x107,0,[15,1<<32,0]),(0x107,0,[15,71,1])]{
 let mut c=ServiceCustody::new();let mut a=Auth::new();assert!(c.receive(&mut a,op,caller,args,|_|panic!("forged effect")).is_none());assert_eq!(a.queries,0);assert_eq!(a.acks,0);assert!(!c.is_retired(15));}
}
#[test]fn live_retry_unknown_or_mismatching_authority_has_no_effects(){
 for result in [Err(LIVE),Err(RETRY),Err(DENIED),Ok(LifecycleInfo{target_pd:9,task_id:71,required_mask:31,acked_mask:0}),Ok(LifecycleInfo{target_pd:15,task_id:72,required_mask:31,acked_mask:0}),Ok(LifecycleInfo{target_pd:15,task_id:71,required_mask:63,acked_mask:0})]{
 let mut c=ServiceCustody::new();let mut a=Auth::new();a.info=result;assert!(c.receive(&mut a,0x107,0,[15,71,0],|_|panic!("unattested effect")).is_none());assert_eq!(a.acks,0);assert!(!c.is_retired(15));}
 let mut c=ServiceCustody::new();let mut a=Auth::new();a.catalog.task_id=72;assert!(c.receive(&mut a,0x107,0,[15,71,0],|_|panic!()).is_none());assert!(!c.is_retired(15));
}
#[test]fn fence_is_installed_before_cleanup_callback_begins(){
 let mut c=ServiceCustody::new();let mut a=Auth::new();
 let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{c.receive(&mut a,0x107,0,[15,71,0],|_|panic!("observe before cleanup effects"));}));
 assert!(result.is_err());assert!(c.is_retired(15));assert_eq!(a.acks,0);
}
#[test]fn ack_retry_does_not_repeat_cleanup_or_drop_terminal_fence(){
 let mut c=ServiceCustody::new();let mut a=Auth::new();a.ack=Err(RETRY);let effects=Cell::new(0);
 for _ in 0..4{let result=c.receive(&mut a,0x107,0,[15,71,0],|_|effects.set(effects.get()+1));assert_eq!(result,Some((ExitNotice{target_pd:15,task_id:71},false)));assert!(c.is_retired(15));}
 assert_eq!(effects.get(),1);assert_eq!(a.acks,4);a.ack=Ok(());
 assert_eq!(c.receive(&mut a,0x107,0,[15,71,0],|_|effects.set(99)),Some((ExitNotice{target_pd:15,task_id:71},true)));
 assert_eq!(effects.get(),1);assert_eq!(a.acks,5);assert!(!c.is_retired(9));
}
#[test]fn catalog_rebinding_cannot_overwrite_terminal_subject(){
 let mut c=ServiceCustody::new();let mut a=Auth::new();c.receive(&mut a,0x107,0,[15,71,0],|_|{}).unwrap();
 a.catalog.target_pd=9;a.info=Ok(LifecycleInfo{target_pd:9,task_id:71,required_mask:31,acked_mask:0});
 assert!(c.receive(&mut a,0x107,0,[9,71,0],|_|panic!("rebound catalog effect")).is_none());assert!(c.is_retired(15));assert!(!c.is_retired(9));assert_eq!(a.acks,1);
}
#[test]fn acknowledged_replays_are_quiet_and_cannot_repeat_ack_or_effects(){
 let mut c=ServiceCustody::new();let mut a=Auth::new();
 assert!(c.receive(&mut a,OP_APP_EXIT,0,[15,71,0],|_|{}).unwrap().1);
 assert!(c.receive(&mut a,OP_APP_EXIT,0,[15,71,0],|_|panic!("duplicate effect")).is_none());
 assert_eq!(a.acks,1);assert!(c.is_retired(15));
}
#[test]fn cooperative_close_requires_exact_boot_coordinator_and_original_task(){
 assert!(close_requested(OP_APP_CLOSE,3,[15,15,0],15,15));
 for(op,caller,args)in[(OP_APP_EXIT,3,[15,15,0]),(OP_APP_CLOSE,15,[15,15,0]),(OP_APP_CLOSE,0,[15,15,0]),(OP_APP_CLOSE,3,[(1<<32)|15,15,0]),(OP_APP_CLOSE,3,[15,(1<<32)|15,0]),(OP_APP_CLOSE,3,[15,15,1]),(OP_APP_CLOSE,3,[12,12,0])]{
  assert!(!close_requested(op,caller,args,15,15));
 }
 assert!(!close_requested(OP_APP_CLOSE,3,[0,0,0],0,0));
}

#[test]fn sixth_catalog_entry_retains_terminal_fence_independently(){
 struct Sixth;
 impl Authority for Sixth{
  fn catalog(&mut self,slot:usize)->Result<CatalogInfo,u64>{if slot==54{Ok(CatalogInfo{target_pd:18,task_id:180,surface_id:307,coordinator_pd:3})}else{Err(INVALID)}}
  fn info(&mut self,_:usize)->Result<LifecycleInfo,u64>{Ok(LifecycleInfo{target_pd:18,task_id:180,required_mask:31,acked_mask:0})}
  fn ack(&mut self,slot:usize,task:u32)->Result<(),u64>{assert_eq!((slot,task),(54,180));Ok(())}
 }
 let mut c=ServiceCustody::new();let mut a=Sixth;let mut calls=0;
 assert!(c.receive(&mut a,OP_APP_EXIT,0,[18,18,0],|_|panic!()).is_none());
 assert_eq!(c.receive(&mut a,OP_APP_EXIT,0,[18,180,0],|_|calls+=1),Some((ExitNotice{target_pd:18,task_id:180},true)));
 assert!(c.is_retired(18));assert!(!c.is_retired(16));
 assert!(c.receive(&mut a,OP_APP_EXIT,0,[18,180,0],|_|panic!()).is_none());assert_eq!(calls,1);
}

#[test]fn kernel_catalog_subject_keeps_task_namespace_separate(){
 let c=CatalogInfo{target_pd:18,task_id:180,surface_id:307,coordinator_pd:3};
 assert!(c.is_subject(18,307,3));
 for bad in [CatalogInfo{task_id:0,..c},CatalogInfo{target_pd:16,..c},CatalogInfo{surface_id:301,..c},CatalogInfo{coordinator_pd:9,..c}]{assert!(!bad.is_subject(18,307,3));}
 assert!(!c.is_subject(0,307,3));assert!(!c.is_subject(18,0,3));assert!(!c.is_subject(18,307,18));
}
