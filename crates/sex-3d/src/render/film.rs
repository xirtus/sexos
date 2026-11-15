// SPDX-License-Identifier: GPL-3.0-only
//! The film: Cycles' importance-sampled Blackman-Harris pixel filter and the
//! View Layer render passes (data passes, light passes, mist).
use crate::math::Vec3;

/// Render passes, named as Blender's View Layer pass checkboxes. The
/// combined pass is always rendered ([`super::RenderResult::color`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pass {
    Combined,
    /// Z: distance along the view axis (along the ray for panoramas), 1e10 on the background.
    Depth,
    /// 0 at the mist start, 1 at start + depth and on the background.
    Mist,
    Normal,
    Position,
    Uv,
    DenoisingAlbedo,
    DenoisingNormal,
    ObjectIndex,
    MaterialIndex,
    DiffuseDirect,
    DiffuseIndirect,
    DiffuseColor,
    GlossyDirect,
    GlossyIndirect,
    GlossyColor,
    TransmissionDirect,
    TransmissionIndirect,
    TransmissionColor,
    VolumeDirect,
    VolumeIndirect,
    Emission,
    Environment,
    AmbientOcclusion,
}

impl Pass {
    pub const ALL: [Pass; 24] = [
        Pass::Combined,
        Pass::Depth,
        Pass::Mist,
        Pass::Normal,
        Pass::Position,
        Pass::Uv,
        Pass::DenoisingAlbedo,
        Pass::DenoisingNormal,
        Pass::ObjectIndex,
        Pass::MaterialIndex,
        Pass::DiffuseDirect,
        Pass::DiffuseIndirect,
        Pass::DiffuseColor,
        Pass::GlossyDirect,
        Pass::GlossyIndirect,
        Pass::GlossyColor,
        Pass::TransmissionDirect,
        Pass::TransmissionIndirect,
        Pass::TransmissionColor,
        Pass::VolumeDirect,
        Pass::VolumeIndirect,
        Pass::Emission,
        Pass::Environment,
        Pass::AmbientOcclusion,
    ];

    /// The pass's name in a multilayer image (Blender's short names).
    pub fn name(self) -> &'static str {
        match self {
            Pass::Combined => "Combined",
            Pass::Depth => "Depth",
            Pass::Mist => "Mist",
            Pass::Normal => "Normal",
            Pass::Position => "Position",
            Pass::Uv => "UV",
            Pass::DenoisingAlbedo => "Denoising Albedo",
            Pass::DenoisingNormal => "Denoising Normal",
            Pass::ObjectIndex => "IndexOB",
            Pass::MaterialIndex => "IndexMA",
            Pass::DiffuseDirect => "DiffDir",
            Pass::DiffuseIndirect => "DiffInd",
            Pass::DiffuseColor => "DiffCol",
            Pass::GlossyDirect => "GlossDir",
            Pass::GlossyIndirect => "GlossInd",
            Pass::GlossyColor => "GlossCol",
            Pass::TransmissionDirect => "TransDir",
            Pass::TransmissionIndirect => "TransInd",
            Pass::TransmissionColor => "TransCol",
            Pass::VolumeDirect => "VolumeDir",
            Pass::VolumeIndirect => "VolumeInd",
            Pass::Emission => "Emit",
            Pass::Environment => "Env",
            Pass::AmbientOcclusion => "AO",
        }
    }

    /// Channels stored (1 for scalars, 3 for colours and vectors).
    pub fn channels(self) -> usize {
        match self {
            Pass::Depth | Pass::Mist | Pass::ObjectIndex | Pass::MaterialIndex => 1,
            _ => 3,
        }
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }

    /// Passes holding the value of the nearest sample rather than an average
    /// (Blender writes them unfiltered).
    pub(crate) fn nearest(self) -> bool {
        matches!(self, Pass::Depth | Pass::ObjectIndex | Pass::MaterialIndex)
    }
}

/// A set of [`Pass`]es.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Passes(pub u32);

impl Passes {
    pub const NONE: Passes = Passes(0);
    pub fn contains(self, p: Pass) -> bool {
        self.0 & p.bit() != 0
    }
    pub fn with(self, p: Pass) -> Passes {
        Passes(self.0 | p.bit())
    }
    pub fn without(self, p: Pass) -> Passes {
        Passes(self.0 & !p.bit())
    }
    pub fn set(&mut self, p: Pass, on: bool) {
        *self = if on { self.with(p) } else { self.without(p) };
    }
    pub fn iter(self) -> impl Iterator<Item = Pass> {
        Pass::ALL.into_iter().filter(move |p| *p != Pass::Combined && self.contains(*p))
    }
    pub fn is_empty(self) -> bool {
        self.iter().next().is_none()
    }
    /// Any of the light passes (which need the per-lobe split of every path).
    pub fn any_light(self) -> bool {
        self.iter().any(|p| p >= Pass::DiffuseDirect)
    }
}

impl FromIterator<Pass> for Passes {
    fn from_iter<T: IntoIterator<Item = Pass>>(iter: T) -> Passes {
        iter.into_iter().fold(Passes::NONE, Passes::with)
    }
}

/// Mist pass falloff (Blender's World > Mist Pass).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MistFalloff {
    #[default]
    Quadratic,
    Linear,
    InverseQuadratic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MistSettings {
    pub start: f32,
    pub depth: f32,
    pub falloff: MistFalloff,
}

impl Default for MistSettings {
    fn default() -> Self {
        MistSettings { start: 5.0, depth: 25.0, falloff: MistFalloff::Quadratic }
    }
}

impl MistSettings {
    pub(crate) fn eval(&self, dist: f32) -> f32 {
        if !dist.is_finite() {
            return 1.0;
        }
        let m = ((dist - self.start) / self.depth.max(1e-6)).clamp(0.0, 1.0);
        match self.falloff {
            MistFalloff::Linear => m,
            MistFalloff::Quadratic => m * m,
            MistFalloff::InverseQuadratic => m.sqrt(),
        }
    }
}

/// Where each enabled pass lives in a pixel's float record.
#[derive(Clone, Debug, Default)]
pub(crate) struct PassLayout {
    offsets: [Option<u16>; 24],
    /// Floats per pixel (0 when no passes).
    pub stride: usize,
    /// Slot of the nearest-sample depth key, if any nearest pass is on.
    pub key: Option<usize>,
    pub passes: Passes,
}

impl PassLayout {
    pub fn new(passes: Passes) -> PassLayout {
        let mut l = PassLayout { passes, ..PassLayout::default() };
        let mut at = 0usize;
        if passes.iter().any(Pass::nearest) {
            l.key = Some(0);
            at = 1;
        }
        for p in passes.iter() {
            l.offsets[p as usize] = Some(at as u16);
            at += p.channels();
        }
        l.stride = at;
        l
    }
    pub fn offset(&self, p: Pass) -> Option<usize> {
        self.offsets[p as usize].map(|o| o as usize)
    }
    /// A fresh pixel record.
    pub fn init(&self, rec: &mut [f32]) {
        rec.iter_mut().for_each(|v| *v = 0.0);
        if let Some(k) = self.key {
            rec[k] = f32::INFINITY;
        }
        for p in [Pass::Depth, Pass::ObjectIndex, Pass::MaterialIndex] {
            if let Some(o) = self.offset(p) {
                rec[o] = if p == Pass::Depth { BACKGROUND_DEPTH } else { 0.0 };
            }
        }
    }
    /// Add one sample's values.
    pub fn add(&self, rec: &mut [f32], s: &SamplePasses) {
        if self.stride == 0 {
            return;
        }
        if let Some(k) = self.key {
            // Nearest-sample passes: the closest hit wins (ties keep the first).
            if s.depth < rec[k] {
                rec[k] = s.depth;
                if let Some(o) = self.offset(Pass::Depth) {
                    rec[o] = if s.depth.is_finite() { s.depth } else { BACKGROUND_DEPTH };
                }
                if let Some(o) = self.offset(Pass::ObjectIndex) {
                    rec[o] = s.object_index;
                }
                if let Some(o) = self.offset(Pass::MaterialIndex) {
                    rec[o] = s.material_index;
                }
            }
        }
        for p in self.passes.iter() {
            if p.nearest() {
                continue;
            }
            let Some(o) = self.offset(p) else { continue };
            if p == Pass::Mist {
                rec[o] += s.mist;
                continue;
            }
            let v = s.get(p);
            rec[o] += v.x;
            rec[o + 1] += v.y;
            rec[o + 2] += v.z;
        }
    }
    /// Sum two records of the same pixel (progressive accumulation).
    pub fn merge(&self, into: &mut [f32], from: &[f32]) {
        let key = self.key.unwrap_or(usize::MAX);
        if let Some(k) = self.key {
            if from[k] < into[k] {
                for p in [Pass::Depth, Pass::ObjectIndex, Pass::MaterialIndex] {
                    if let Some(o) = self.offset(p) {
                        into[o] = from[o];
                    }
                }
                into[k] = from[k];
            }
        }
        for p in self.passes.iter().filter(|p| !p.nearest()) {
            let o = self.offset(p).unwrap_or(key);
            for c in 0..p.channels() {
                into[o + c] += from[o + c];
            }
        }
    }
}

/// Z of the background, as Blender writes it.
pub const BACKGROUND_DEPTH: f32 = 1e10;

/// Light pass slots in [`SamplePasses::light`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightSlot {
    DiffuseDirect = 0,
    DiffuseIndirect,
    GlossyDirect,
    GlossyIndirect,
    TransmissionDirect,
    TransmissionIndirect,
    VolumeDirect,
    VolumeIndirect,
    Emission,
    Environment,
}

/// Everything one camera sample contributes to the passes.
#[derive(Clone, Debug)]
pub(crate) struct SamplePasses {
    pub depth: f32,
    pub mist: f32,
    pub normal: Vec3,
    pub position: Vec3,
    pub uv: Vec3,
    pub object_index: f32,
    pub material_index: f32,
    pub diffuse_color: Vec3,
    pub glossy_color: Vec3,
    pub transmission_color: Vec3,
    pub ao: Vec3,
    pub light: [Vec3; 10],
    pub albedo: Vec3,
    pub dnormal: Vec3,
}

impl Default for SamplePasses {
    fn default() -> Self {
        SamplePasses {
            depth: f32::INFINITY,
            mist: 1.0,
            normal: Vec3::ZERO,
            position: Vec3::ZERO,
            uv: Vec3::ZERO,
            object_index: 0.0,
            material_index: 0.0,
            diffuse_color: Vec3::ZERO,
            glossy_color: Vec3::ZERO,
            transmission_color: Vec3::ZERO,
            ao: Vec3::ZERO,
            light: [Vec3::ZERO; 10],
            albedo: Vec3::ZERO,
            dnormal: Vec3::ZERO,
        }
    }
}

impl SamplePasses {
    fn get(&self, p: Pass) -> Vec3 {
        match p {
            Pass::Normal => self.normal,
            Pass::Position => self.position,
            Pass::Uv => self.uv,
            Pass::DenoisingAlbedo => self.albedo,
            Pass::DenoisingNormal => self.dnormal,
            Pass::DiffuseColor => self.diffuse_color,
            Pass::GlossyColor => self.glossy_color,
            Pass::TransmissionColor => self.transmission_color,
            Pass::AmbientOcclusion => self.ao,
            Pass::DiffuseDirect => self.light[LightSlot::DiffuseDirect as usize],
            Pass::DiffuseIndirect => self.light[LightSlot::DiffuseIndirect as usize],
            Pass::GlossyDirect => self.light[LightSlot::GlossyDirect as usize],
            Pass::GlossyIndirect => self.light[LightSlot::GlossyIndirect as usize],
            Pass::TransmissionDirect => self.light[LightSlot::TransmissionDirect as usize],
            Pass::TransmissionIndirect => self.light[LightSlot::TransmissionIndirect as usize],
            Pass::VolumeDirect => self.light[LightSlot::VolumeDirect as usize],
            Pass::VolumeIndirect => self.light[LightSlot::VolumeIndirect as usize],
            Pass::Emission => self.light[LightSlot::Emission as usize],
            Pass::Environment => self.light[LightSlot::Environment as usize],
            _ => Vec3::ZERO,
        }
    }
}

/// Blackman-Harris importance table: offsets from the pixel centre with the
/// filter's density, so every sample weighs the same (Cycles' filter table).
#[derive(Clone, Debug)]
pub(crate) struct PixelFilter {
    table: Vec<f32>,
}

impl PixelFilter {
    pub fn new(width: f32) -> PixelFilter {
        const N: usize = 256;
        // Cycles doubles the Blackman-Harris width so it matches a Gaussian's look.
        let w = if width.is_finite() { (width * 2.0).clamp(0.01, 20.0) } else { 3.0 };
        let steps = 1024;
        let f = |x: f32| {
            let v = core::f32::consts::TAU * (x / w + 0.5);
            (0.35875 - 0.48829 * v.cos() + 0.14128 * (2.0 * v).cos() - 0.01168 * (3.0 * v).cos()).max(0.0)
        };
        let mut cdf = Vec::with_capacity(steps + 1);
        cdf.push(0.0f64);
        for i in 0..steps {
            let x = -w * 0.5 + w * (i as f32 + 0.5) / steps as f32;
            let last = *cdf.last().unwrap();
            cdf.push(last + f(x) as f64);
        }
        let total = *cdf.last().unwrap();
        let mut table = Vec::with_capacity(N);
        let mut j = 0usize;
        for i in 0..N {
            let target = (i as f64 + 0.5) / N as f64 * total;
            while j + 1 < steps && cdf[j + 1] < target {
                j += 1;
            }
            let (c0, c1) = (cdf[j], cdf[j + 1]);
            let frac = if c1 > c0 { ((target - c0) / (c1 - c0)) as f32 } else { 0.5 };
            table.push(-w * 0.5 + w * (j as f32 + frac) / steps as f32);
        }
        PixelFilter { table }
    }

    /// Offset from the pixel centre for a uniform `u`.
    pub fn sample(&self, u: f32) -> f32 {
        let x = u * (self.table.len() - 1) as f32;
        let i = (x as usize).min(self.table.len() - 2);
        let f = x - i as f32;
        self.table[i] * (1.0 - f) + self.table[i + 1] * f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_centred_and_bounded() {
        let f = PixelFilter::new(1.5);
        assert!(f.sample(0.5).abs() < 1e-3);
        assert!(f.sample(0.0) >= -1.5 && f.sample(1.0) <= 1.5);
        assert!(f.sample(0.25) < 0.0 && f.sample(0.75) > 0.0);
        // Most of the weight is within the pixel.
        assert!(f.sample(0.1) > -0.6 && f.sample(0.9) < 0.6);
    }

    #[test]
    fn layout_places_passes() {
        let p = Passes::NONE.with(Pass::Depth).with(Pass::Normal).with(Pass::ObjectIndex);
        let l = PassLayout::new(p);
        assert_eq!(l.stride, 1 + 1 + 3 + 1);
        let mut rec = vec![0.0; l.stride];
        l.init(&mut rec);
        let s = SamplePasses { depth: 2.0, object_index: 3.0, normal: Vec3::Z, ..Default::default() };
        l.add(&mut rec, &s);
        l.add(&mut rec, &SamplePasses { depth: 5.0, object_index: 9.0, ..Default::default() });
        assert_eq!(rec[l.offset(Pass::Depth).unwrap()], 2.0);
        assert_eq!(rec[l.offset(Pass::ObjectIndex).unwrap()], 3.0);
        assert_eq!(rec[l.offset(Pass::Normal).unwrap() + 2], 1.0);
    }
}
