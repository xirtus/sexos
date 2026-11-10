// Appended by the source-pin gate as a child of the unchanged production module.
// Registry, page frames and loader input are explicit host doubles from runtime.rs.
use super::*;
use crate::{capability::ProtectionDomain,boot_memory,program_frames,LEAVES};
#[repr(align(4096))]
struct Frame([u8;4096]);
fn image()->Vec<u8>{
 let mut b=vec![0u8;512];b[..6].copy_from_slice(&[127,b'E',b'L',b'F',2,1]);
 b[16..18].copy_from_slice(&3u16.to_le_bytes());b[18..20].copy_from_slice(&62u16.to_le_bytes());
 b[24..32].copy_from_slice(&0x1100u64.to_le_bytes());b[32..40].copy_from_slice(&64u64.to_le_bytes());
 b[54..56].copy_from_slice(&56u16.to_le_bytes());b[56..58].copy_from_slice(&2u16.to_le_bytes());
 b[64..68].copy_from_slice(&1u32.to_le_bytes());b[68..72].copy_from_slice(&5u32.to_le_bytes());
 b[80..88].copy_from_slice(&0x1000u64.to_le_bytes());b[96..104].copy_from_slice(&512u64.to_le_bytes());b[104..112].copy_from_slice(&4096u64.to_le_bytes());
 b[120..124].copy_from_slice(&0x6474_e551u32.to_le_bytes());b[160..168].copy_from_slice(&4096u64.to_le_bytes());b[256]=0xc3;b
}
fn snapshot(pd:&ProtectionDomain)->String{
 let a=APPS.lock();let a=&a[1];let main=unsafe{&*pd.main_task.load(Ordering::Acquire)};
 format!("{:?}",(((a.task,a.generation,a.count,a.image_pages,a.grown,a.arena,a.arena_pages,a.entry),(a.stack_top,a.activated,a.retired,a.rebirthing,a.acked,a.hinted,a.kicked,a.forced)),
 a.pages.clone(),NEXT_RUN_TASK.load(Ordering::Acquire),DORMANT[1].load(Ordering::Acquire),TERMINAL[1].load(Ordering::Acquire),main.id,main.context.rip,main.context.rsp,main.state.snapshot(),
 LEAVES.lock().values().map(|p|unsafe{*(*p as *const u64)}).collect::<Vec<_>>(),pd.incoming_replies.lock().iter().map(|r|(r.value,r.caller_pd,r.caller_task)).collect::<Vec<_>>()))
}
#[test]
fn actual_first_activation_and_rebirth_reject_source_before_mutation(){
 let shell=ProtectionDomain::new_key(3,3);let old=ProtectionDomain::new_key(15,5);let pd=ProtectionDomain::new_key(16,5);
 let approved=image();let source=unsafe{ImageSource::boot_private(approved.as_ptr() as usize,approved.len())};
 shell.grant_capability(44,CapabilityData::AppActivation(ActivationGrant{image_source:Some(source),holder:3,logical_app:15,incarnation:2,predecessor:15,predecessor_task:15,successor:16,successor_task:16}));
 let mut leaves:Vec<Box<u64>>=(0..8).map(|_|Box::new(0u64)).collect();let arena=0x8000_0000;
 for (i,leaf) in leaves.iter_mut().enumerate(){LEAVES.lock().insert(arena+i as u64*4096,&mut **leaf as *mut u64 as usize);}
 let frames:Vec<Box<Frame>>=(0..8).map(|_|Box::new(Frame([0;4096]))).collect();
 for frame in &frames{program_frames::FRAMES.lock().push(frame.0.as_ptr() as u64);}
 {
  let mut a=APPS.lock();a[0]=App::new();a[0].target=15;a[0].task=15;a[0].boot_task=15;a[0].main=old.main_task.load(Ordering::Acquire) as usize;a[0].ready=true;a[0].reusable=true;a[0].activated=true;a[0].identity=Incarnation::original(15);
  a[1]=App::new();a[1].target=16;a[1].task=16;a[1].boot_task=16;a[1].main=pd.main_task.load(Ordering::Acquire) as usize;a[1].trampoline=pd.trampoline_task.load(Ordering::Acquire) as usize;a[1].ready=true;a[1].reusable=true;a[1].pkey=5;a[1].identity=Incarnation{logical_app:15,number:2};a[1].predecessor=15;a[1].arena=arena;a[1].arena_pages=8;a[1].stack_top=65536;
 }
 pd.incoming_replies.lock().push_back(crate::messages::IpcReply{value:0x12345678,caller_pd:11,caller_task:11});
 for rebirth in [false,true]{
  if rebirth{
   unsafe{(*pd.main_task.load(Ordering::Acquire)).state.exit();}
   let mut a=APPS.lock();a[1].retired=true;a[1].acked=REQUIRED;
   for page in &a[1].pages{unsafe{*(page.leaf as *mut u64)=0;}}
  }
  for wrong_length in [false,true]{
   let mut changed=approved.clone();if wrong_length{changed.pop();}else{changed[256]^=1;}
   let n=changed.len();*boot_memory::CANDIDATE.lock()=Some(changed);
   let before=snapshot(pd);let available=program_frames::available();
   assert_eq!(activation(shell,44,3,n as u64)[0],DENIED,"rebirth={rebirth} length={wrong_length}");
   assert_eq!(snapshot(pd),before,"source rejection changed task/generation/manifest/arena/leaves/receipts");
   assert_eq!(program_frames::available(),available,"source rejection consumed image/stack frames");
  }
  *boot_memory::CANDIDATE.lock()=Some(approved.clone());
  assert_eq!(activation(shell,44,3,approved.len() as u64)[0],0,"original approved source must still load");
  {
   let a=APPS.lock();assert_eq!((a[1].image_pages,a[1].stack_pages,a[1].count),(1,1,2));
   let frame=a[1].pages[0].expected&PHYSICAL_MASK;
   assert_eq!(unsafe{core::slice::from_raw_parts(frame as *const u8,approved.len())},&approved[..]);
   assert_eq!(a[1].generation,if rebirth{1}else{0});
  }
  if !rebirth{assert_eq!(activation(shell,44,1,0)[0],0);}else{
   let mut changed=approved.clone();changed[256]^=1;*boot_memory::CANDIDATE.lock()=Some(changed);
   let before=snapshot(pd);assert_eq!(activation(shell,44,3,approved.len() as u64)[0],RETRY);
   assert_eq!(snapshot(pd),before,"kind3 cannot advance/reload a pending rebirth with an ignored candidate");
  }
 }
}
