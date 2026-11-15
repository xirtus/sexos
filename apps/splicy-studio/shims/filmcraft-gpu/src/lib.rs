//! FilmCraft's GPU compositor interface (filmcraft-gpu), with no GPU behind it. FilmCraft
//! starts its compositor only when eframe hands it a wgpu device, which never happens on
//! SexOS, so it composites every frame on the CPU. MIT OR Apache-2.0, as FilmCraft.
#![forbid(unsafe_code)]

use eframe::wgpu;
use filmcraft_render::plan::{DeviceCaps, FramePlan};

/// A plan's uploads, made ahead of compositing. Empty here.
#[derive(Clone, Default)]
pub struct PreparedPlan(());

impl PreparedPlan {
    pub fn bytes(&self) -> usize {
        0
    }
}

pub fn prepare(_plan: &FramePlan) -> PreparedPlan {
    PreparedPlan(())
}

/// The compositor's counters, as the real one keeps them; all zero.
pub struct GpuCompositor {
    pub gpu_transitions: u64,
    pub cpu_transitions: u64,
    pub gpu_adjustments: u64,
    pub cpu_adjustments: u64,
    pub uploaded_bytes: u64,
}

impl GpuCompositor {
    pub fn new(_device: &wgpu::Device, _queue: &wgpu::Queue) -> Self {
        Self { gpu_transitions: 0, cpu_transitions: 0, gpu_adjustments: 0, cpu_adjustments: 0, uploaded_bytes: 0 }
    }
    pub fn caps(&self) -> DeviceCaps {
        DeviceCaps { adjust_side: 0 }
    }
    pub fn composite_prepared(&mut self, plan: &FramePlan, _prep: Option<&PreparedPlan>) -> (&wgpu::TextureView, (u32, u32)) {
        unreachable!("no GPU on SexOS: {} px frame asked of the GPU compositor", plan.max_side())
    }
}
