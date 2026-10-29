//! Immutable BOOT-sanctioned data recovery relationship, not handle transfer.
pub const SLOT:usize=53;
pub const SYS_QUERY:u64=67;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Grant{pub holder:u32,pub logical:u32,pub original:u32,pub original_task:u32,pub successor:u32,pub successor_task:u32,pub base:u64,pub sectors:u64}
impl Grant{
 pub fn valid(self)->bool{self.holder!=0&&self.logical!=0&&self.original!=0&&self.successor!=0&&self.original!=self.successor&&self.original_task!=0&&self.successor_task!=0&&self.original_task!=self.successor_task&&self.sectors==18&&self.base.checked_add(self.sectors).is_some()}
 pub fn permits(self,holder:u32,target:u32,task:u32)->bool{self.valid()&&self.holder==holder&&((target,task)==(self.original,self.original_task)||(target,task)==(self.successor,self.successor_task))}
 pub fn query(self,holder:u32,kind:u64)->[u64;5]{
  if !self.valid()||self.holder!=holder{return [3,0,0,0,0]}
  match kind{0=>[0,self.logical as u64,self.original as u64,self.original_task as u64,self.holder as u64],1=>[0,self.successor as u64,self.successor_task as u64,self.base,self.sectors],_=>[1,0,0,0,0]}
 }
}
#[cfg(target_arch="x86_64")]
pub fn get(slot:usize)->Result<Grant,u64>{
 fn query(slot:usize,kind:u64)->Result<[u64;4],u64>{let(status,a,b,c,d):(u64,u64,u64,u64,u64);unsafe{core::arch::asm!("syscall",inlateout("rax") SYS_QUERY=>status,in("rdi")slot as u64,inlateout("rsi")kind=>a,lateout("rdx")b,lateout("r10")c,lateout("r8")d,out("rcx")_,out("r11")_,options(nostack));}if status!=0{return Err(status)}Ok([a,b,c,d])}
 let a=query(slot,0)?;let b=query(slot,1)?;
 let g=Grant{holder:u32::try_from(a[3]).map_err(|_|1u64)?,logical:u32::try_from(a[0]).map_err(|_|1u64)?,original:u32::try_from(a[1]).map_err(|_|1u64)?,original_task:u32::try_from(a[2]).map_err(|_|1u64)?,successor:u32::try_from(b[0]).map_err(|_|1u64)?,successor_task:u32::try_from(b[1]).map_err(|_|1u64)?,base:b[2],sectors:b[3]};if !g.valid(){return Err(3)}Ok(g)
}
