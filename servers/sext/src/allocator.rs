// SPDX-License-Identifier: GPL-3.0-only
//! Sext's bounded frame policy. Backing is reusable only after confirmed kernel detach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State { Free, Reserved, Committed, Reclaiming }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame { state: State, token: u64, target: u32, page: u64, zeroed: bool }
const FREE: Frame = Frame { state: State::Free, token: 0, target: 0, page: 0, zeroed: false };
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationError { Invalid, Conflict, Quota, Exhausted }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct Allocation { pub index: usize, pub needs_zero: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding { pub index: usize, pub token: u64, pub target: u32, pub page: u64 }
pub struct FrameAllocator<const N: usize> { frames: [Frame; N], quota: usize }
impl<const N: usize> FrameAllocator<N> {
    pub const fn new(quota: usize) -> Self { Self { frames: [FREE; N], quota } }
    pub fn reserve(&mut self, token: u64, target: u32, page: u64) -> Result<Allocation, AllocationError> {
        if token == 0 || target == 0 || page == 0 || page % 4096 != 0 { return Err(AllocationError::Invalid); }
        if let Some((index, frame)) = self.frames.iter().enumerate().find(|(_, f)| f.state != State::Free && f.token == token) {
            if frame.state != State::Reserved || frame.target != target || frame.page != page { return Err(AllocationError::Conflict); }
            return Ok(Allocation { index, needs_zero: !frame.zeroed });
        }
        if self.frames.iter().any(|f| f.state != State::Free && f.target == target && f.page == page) { return Err(AllocationError::Conflict); }
        if self.frames.iter().filter(|f| f.state != State::Free && f.target == target).count() >= self.quota { return Err(AllocationError::Quota); }
        let index = self.frames.iter().position(|f| f.state == State::Free).ok_or(AllocationError::Exhausted)?;
        self.frames[index] = Frame { state: State::Reserved, token, target, page, zeroed: false };
        Ok(Allocation { index, needs_zero: true })
    }
    pub fn mark_zeroed(&mut self, index: usize, token: u64) -> bool {
        let Some(frame) = self.frames.get_mut(index) else { return false; };
        if frame.state != State::Reserved || frame.token != token { return false; }
        frame.zeroed = true; true
    }
    pub fn commit(&mut self, index: usize, token: u64) -> bool {
        let Some(frame) = self.frames.get_mut(index) else { return false; };
        if frame.state != State::Reserved || frame.token != token || !frame.zeroed { return false; }
        frame.state = State::Committed; true
    }
    /// Only after an explicit kernel failure guaranteeing no mapping was installed.
    pub fn rollback_unpublished(&mut self, index: usize, token: u64) -> bool {
        let Some(frame) = self.frames.get_mut(index) else { return false; };
        if frame.state != State::Reserved || frame.token != token { return false; }
        *frame = FREE; true
    }
    pub fn begin_reclaim(&mut self, target: u32, page: u64) -> Option<Binding> {
        let (index, frame) = self.frames.iter_mut().enumerate().find(|(_, f)|
            f.state == State::Committed && f.target == target && f.page == page)?;
        frame.state = State::Reclaiming;
        Some(Binding { index, token: frame.token, target, page })
    }
    /// The kernel has cleared the exact old PTE and invalidated its translation.
    pub fn finish_reclaim(&mut self, binding: Binding) -> bool {
        let Some(frame) = self.frames.get_mut(binding.index) else { return false; };
        if frame.state != State::Reclaiming || frame.token != binding.token
            || frame.target != binding.target || frame.page != binding.page { return false; }
        *frame = FREE;
        true
    }
    pub fn bindings_for(&self, target: u32) -> [Option<Binding>; N] {
        core::array::from_fn(|index| {
            let frame = self.frames[index];
            if frame.state != State::Free && frame.target == target {
                Some(Binding { index, token: frame.token, target, page: frame.page })
            } else { None }
        })
    }
    pub fn target_mask(&self, target: u32) -> u64 {
        self.frames.iter().enumerate().fold(0, |mask, (index, frame)| {
            if index < 64 && frame.state != State::Free && frame.target == target { mask | (1u64 << index) } else { mask }
        })
    }
    /// Only after whole-target kernel cleanup confirms permanent range closure.
    /// Clear by current local ownership, never by a replayed kernel bitmap.
    pub fn finish_target_cleanup(&mut self, target: u32) -> usize {
        let mut released = 0;
        for frame in &mut self.frames {
            if frame.state != State::Free && frame.target == target {
                *frame = FREE;
                released += 1;
            }
        }
        released
    }
    pub fn used_by(&self, target: u32) -> usize {
        self.frames.iter().filter(|f| f.state != State::Free && f.target == target).count()
    }
    pub fn used(&self) -> usize { self.frames.iter().filter(|f| f.state != State::Free).count() }
}
