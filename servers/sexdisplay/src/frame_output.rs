// SPDX-License-Identifier: GPL-3.0-only
//! Display-local preparation over two already-held BOOT frame spans.
//! Publication is a caller-supplied CPU output operation, not a hardware flip.
//! A false callback may already have changed external output; only this store's
//! front/generation rollback is guaranteed. No receipts or authority are minted.
use core::marker::PhantomData;
use sex_pdx::display_frames::{FrameBacking,Layout};

pub struct Store {
    backing:FrameBacking,
    front:Option<usize>,
    generation:u64,
}
impl Store {
    /// # Safety
    /// Both spans must remain mapped, writable and exclusively owned for this
    /// Store's lifetime and every borrowed reference. No other Store, retained
    /// descriptor, task or raw accessor may concurrently read/write them in
    /// conflict with these Rust borrows. Resolving the same PD capability twice
    /// does not provide two exclusive owners; !Copy is not physical uniqueness.
    pub unsafe fn new(backing:FrameBacking)->Self{Self{backing,front:None,generation:0}}
    pub fn layout(&self)->Layout{self.backing.layout()}
    pub fn generation(&self)->u64{self.generation}
    pub fn front_pixels(&self)->Option<&[u32]>{
        let index=self.front?;
        // A front becomes valid only after its complete visible span has been
        // initialized during preparation and the publication callback succeeds.
        Some(unsafe{core::slice::from_raw_parts(self.pointer(index),self.len())})
    }
    fn pointer(&self,index:usize)->*mut u32{self.backing.address(index).expect("FRAME_INDEX") as *mut u32}
    fn len(&self)->usize{self.layout().frame_bytes() as usize/4}
    fn span_len(&self)->usize{self.layout().span_bytes() as usize/4}
    fn erase(&mut self,index:usize){
        for offset in 0..self.span_len(){unsafe{core::ptr::write_volatile(self.pointer(index).add(offset),0);}}
    }
    pub fn prepare(&mut self,full:bool)->Option<Preparation<'_>>{
        // Exhaustion and absent baseline refuse before modifying either span.
        let next_generation=self.generation.checked_add(1)?;
        if !full&&self.front.is_none(){return None;}
        let candidate=self.front.map_or(0,|front|1-front);
        if full{
            // Held address metadata alone does not promise initialized pixels.
            self.erase(candidate);
        }else{
            let front=self.front.expect("FRAME_BASELINE");
            unsafe{core::ptr::copy_nonoverlapping(self.pointer(front),self.pointer(candidate),self.len());}
            for offset in self.len()..self.span_len(){unsafe{core::ptr::write_volatile(self.pointer(candidate).add(offset),0);}}
        }
        Some(Preparation{store:self,candidate,next_generation,committed:false})
    }
    /// Cleanup removes both visible payloads and page padding. The generation
    /// is not reset: a later publication cannot reuse an old generation value.
    pub fn scrub(&mut self){self.front=None;self.erase(0);self.erase(1);}
    #[cfg(test)]
    pub(crate) fn set_generation_for_test(&mut self,value:u64){self.generation=value;}
}

pub struct Preparation<'a>{
    store:&'a mut Store,
    candidate:usize,
    next_generation:u64,
    committed:bool,
}
impl Preparation<'_>{
    /// Raw drawing requires the caller's unsafe writes to remain within len and
    /// this borrow. No independently constructible target or ambient backing.
    pub fn target(&mut self)->Target<'_>{
        Target{pointer:self.store.pointer(self.candidate),layout:self.store.layout(),_borrow:PhantomData}
    }
    pub fn commit(mut self,publish:impl FnOnce(&[u32])->bool)->bool{
        let pixels=unsafe{core::slice::from_raw_parts(self.store.pointer(self.candidate),self.store.len())};
        if !publish(pixels){return false;}
        self.store.front=Some(self.candidate);
        self.store.generation=self.next_generation;
        self.committed=true;
        true
    }
}
impl Drop for Preparation<'_>{
    fn drop(&mut self){if !self.committed{self.store.erase(self.candidate);}}
}

/// Non-Copy, non-Clone borrowed drawing target; not a capability or new region.
pub struct Target<'a>{pointer:*mut u32,layout:Layout,_borrow:PhantomData<&'a mut [u32]>}
impl Target<'_>{
    pub fn pointer(&self)->*mut u32{self.pointer}
    pub fn width(&self)->usize{self.layout.width()}
    pub fn height(&self)->usize{self.layout.height()}
    pub fn len(&self)->usize{self.layout.frame_bytes() as usize/4}
}
