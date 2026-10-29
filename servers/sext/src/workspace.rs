// SPDX-License-Identifier: GPL-3.0-only
//! Bounded pager journal backed by detached app bytes. No frame/VA allocation.
//! Only the capability retirement result admits storage; never a hint or bitmap.
#[derive(Clone,Copy)]
pub struct Workspace {base:u64,bytes:usize,next:usize,records:u64}
impl Workspace {
    /// Caller must hold the kernel's completed retirement grant for this alias.
    pub unsafe fn admit(base:u64,bytes:u64)->Self {
        assert!(base!=0 && base%4096==0 && bytes>=4096 && bytes%4096==0);
        let bytes=usize::try_from(bytes).expect("WORKSPACE_SIZE");
        core::ptr::write_bytes(base as *mut u8,0,bytes);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        for offset in (0..bytes).step_by(8) {
            assert_eq!((base as *const u64).add(offset/8).read_volatile(),0,"WORKSPACE_ZERO");
        }
        Self {base,bytes,next:0,records:0}
    }
    /// Real retained fault/audit records rotate through the reclaimed pool.
    /// Each reused record is scrubbed before new pager-owned content is stored.
    pub unsafe fn record(&mut self,target:u32,token:u64)->u64 {
        let offset=self.next;
        let p=(self.base as *mut u8).add(offset).cast::<u64>();
        core::ptr::write_bytes(p.cast::<u8>(),0,24);
        p.write_volatile(target as u64);p.add(1).write_volatile(token);
        self.records=self.records.checked_add(1).expect("WORKSPACE_RECORD_WRAP");
        p.add(2).write_volatile(self.records);
        assert_eq!(p.read_volatile(),target as u64);
        assert_eq!(p.add(1).read_volatile(),token);
        assert_eq!(p.add(2).read_volatile(),self.records);
        self.next=if offset+48<=self.bytes {offset+24}else{0};
        self.records
    }
    pub fn rewind(&mut self){self.next=0;}
}
