// SPDX-License-Identifier: GPL-3.0-only
//! Display-local preparation over two already-held BOOT frame spans.
//! Publication is a caller-supplied CPU output operation, not a hardware flip.
//! A false callback may already have changed external output; only this store's
//! front/generation rollback is guaranteed. No receipts or authority are minted.
//!
//! Buffer age: the span that is not the front lags the front only in the rows
//! the last publication changed. A partial preparation copies just those rows,
//! and publication is told which rows differ from the front it replaces, so a
//! typed letter moves a few rows of memory, not two whole screens.
use core::marker::PhantomData;
use sex_pdx::display_frames::{FrameBacking,Layout};

/// Rows `first..end` of a frame. Empty when `first==end`.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Rows{pub first:usize,pub end:usize}
impl Rows{
    pub const NONE:Self=Self{first:0,end:0};
    pub fn is_empty(self)->bool{self.first>=self.end}
}

/// What a publication must make visible. `previous` is the frame the output
/// already shows when the candidate began as its copy; rows outside `rows`
/// are identical to it. Without `previous` every row must be written.
pub struct Publication<'a>{
    pub pixels:&'a [u32],
    pub previous:Option<&'a [u32]>,
    pub rows:Rows,
}

pub struct Store {
    backing:FrameBacking,
    front:Option<usize>,
    generation:u64,
    /// Rows where the non-front span may differ from the front.
    behind:Rows,
}
impl Store {
    /// # Safety
    /// Both spans must remain mapped, writable and exclusively owned for this
    /// Store's lifetime and every borrowed reference. No other Store, retained
    /// descriptor, task or raw accessor may concurrently read/write them in
    /// conflict with these Rust borrows. Resolving the same PD capability twice
    /// does not provide two exclusive owners; !Copy is not physical uniqueness.
    pub unsafe fn new(backing:FrameBacking)->Self{
        let height=backing.layout().height();
        Self{backing,front:None,generation:0,behind:Rows{first:0,end:height}}
    }
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
    fn all_rows(&self)->Rows{Rows{first:0,end:self.layout().height()}}
    fn erase(&mut self,index:usize){
        for offset in 0..self.span_len(){unsafe{core::ptr::write_volatile(self.pointer(index).add(offset),0);}}
        if Some(index)!=self.front{self.behind=self.all_rows();}
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
            let width=self.layout().width();
            let rows=self.behind;
            if !rows.is_empty(){
                let start=rows.first*width;let count=(rows.end-rows.first)*width;
                unsafe{core::ptr::copy_nonoverlapping(self.pointer(front).add(start),self.pointer(candidate).add(start),count);}
            }
            self.behind=Rows::NONE;
            for offset in self.len()..self.span_len(){unsafe{core::ptr::write_volatile(self.pointer(candidate).add(offset),0);}}
        }
        Some(Preparation{store:self,candidate,next_generation,full,committed:false})
    }
    /// Cleanup removes both visible payloads and page padding. The generation
    /// is not reset: a later publication cannot reuse an old generation value.
    pub fn scrub(&mut self){self.front=None;self.erase(0);self.erase(1);self.behind=self.all_rows();}
    #[cfg(test)]
    pub(crate) fn set_generation_for_test(&mut self,value:u64){self.generation=value;}
}

/// First and one-past-last rows where two frames of `width` differ.
pub fn changed_rows(a:&[u32],b:&[u32],width:usize)->Rows{
    if width==0||a.len()!=b.len(){return Rows{first:0,end:if width==0{0}else{a.len()/width}};}
    let height=a.len()/width;
    let row=|y:usize|y*width..(y+1)*width;
    let Some(first)=(0..height).find(|&y|a[row(y)]!=b[row(y)])else{return Rows::NONE};
    let last=(first..height).rev().find(|&y|a[row(y)]!=b[row(y)]).unwrap_or(first);
    Rows{first,end:last+1}
}

pub struct Preparation<'a>{
    store:&'a mut Store,
    candidate:usize,
    next_generation:u64,
    full:bool,
    committed:bool,
}
impl Preparation<'_>{
    /// Raw drawing requires the caller's unsafe writes to remain within len and
    /// this borrow. No independently constructible target or ambient backing.
    pub fn target(&mut self)->Target<'_>{
        Target{pointer:self.store.pointer(self.candidate),layout:self.store.layout(),_borrow:PhantomData}
    }
    pub fn commit(mut self,publish:impl FnOnce(Publication<'_>)->bool)->bool{
        let store=&*self.store;
        let pixels=unsafe{core::slice::from_raw_parts(store.pointer(self.candidate),store.len())};
        // A full preparation erased the candidate; the output may show anything.
        let previous=if self.full{None}else{store.front_pixels()};
        let rows=match previous{Some(old)=>changed_rows(pixels,old,store.layout().width()),None=>store.all_rows()};
        if !publish(Publication{pixels,previous,rows}){return false;}
        // The old front now lags the new one exactly where this frame changed it.
        let behind=if previous.is_some(){rows}else{store.all_rows()};
        self.store.front=Some(self.candidate);
        self.store.generation=self.next_generation;
        self.store.behind=behind;
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
