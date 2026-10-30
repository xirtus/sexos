// SPDX-License-Identifier: GPL-3.0-only
//! Bounded own-task deadlines and an immutable BOOT task inventory.
//! No allocation, target lookup, task activation or authority creation.
use core::sync::atomic::{AtomicBool,AtomicPtr,AtomicU64,Ordering};
pub const MAX_TICKS:u64=64;
pub struct Deadline{at:AtomicU64}
impl Deadline{
 pub const fn new()->Self{Self{at:AtomicU64::new(0)}}
 pub fn arm(&self,now:u64,ticks:u64)->Option<u64>{
  if ticks==0||ticks>MAX_TICKS{return None;}
  let at=now.checked_add(ticks)?;
  self.at.compare_exchange(0,at,Ordering::AcqRel,Ordering::Acquire).ok().map(|_|at)
 }
 pub fn cancel(&self){self.at.store(0,Ordering::Release);}
 /// One claim only; competing IPC wake/cancellation can win instead. A timer
 /// wake may coalesce with another event and never means an inbox was consumed.
 pub fn take_due(&self,now:u64)->Option<u64>{
  let at=self.at.load(Ordering::Acquire);
  if at==0||now<at{return None;}
  self.at.compare_exchange(at,0,Ordering::AcqRel,Ordering::Acquire).ok().map(|_|at)
 }
}
pub struct BootInventory<T,const N:usize>{tasks:[AtomicPtr<T>;N],sealed:AtomicBool}
impl<T,const N:usize> BootInventory<T,N>{
 pub const fn new()->Self{Self{tasks:[const{AtomicPtr::new(core::ptr::null_mut())};N],sealed:AtomicBool::new(false)}}
 /// BOOT's single construction lane only, before seal and interrupt use.
 pub fn register(&self,index:usize,task:*mut T)->bool{
  if self.sealed.load(Ordering::Acquire)||index>=N||task.is_null()||self.tasks.iter().any(|s|s.load(Ordering::Acquire)==task){return false;}
  self.tasks[index].compare_exchange(core::ptr::null_mut(),task,Ordering::AcqRel,Ordering::Acquire).is_ok()
 }
 pub fn seal(&self)->bool{self.sealed.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_ok()}
 pub fn contains(&self,task:*mut T)->bool{!task.is_null()&&self.sealed.load(Ordering::Acquire)&&self.tasks.iter().any(|s|s.load(Ordering::Acquire)==task)}
 pub fn visit(&self,mut f:impl FnMut(*mut T)){
  if !self.sealed.load(Ordering::Acquire){return;}
  for slot in &self.tasks{let task=slot.load(Ordering::Acquire);if !task.is_null(){f(task);}}
 }
}
