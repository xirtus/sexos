// SPDX-License-Identifier: GPL-3.0-only
//! Disk caches: a baked simulation as one little-endian binary file in the
//! document's `cache` folder (Blender's `//cache/`; Blender writes one
//! `.bphys` per frame, here one `.pcache` per simulation). The app does the
//! file I/O; this module only encodes and decodes.
use super::cache::{file_stem, Caches, SimFrame};
use super::cloth::PointsFrame;
use super::dynamic_paint::PaintFrame;
use super::fluid::FluidFrame;
use super::liquid::LiquidFrame;
use super::particles::{Alive, Particle, ParticlesFrame};
use super::rigid::{BodyState, RigidFrame};
use crate::math::{Mat4, Quat, Vec3};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"PCACHE01";

pub fn file_name(key: &str) -> String {
    format!("{}.pcache", file_stem(key))
}

struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn v3(&mut self, v: Vec3) {
        self.f(v.x);
        self.f(v.y);
        self.f(v.z);
    }
    fn q(&mut self, q: Quat) {
        for x in [q.w, q.x, q.y, q.z] {
            self.f(x);
        }
    }
    fn s(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn fs(&mut self, v: &[f32]) {
        self.u32(v.len() as u32);
        for &x in v {
            self.f(x);
        }
    }
    fn v3s(&mut self, v: &[Vec3]) {
        self.u32(v.len() as u32);
        for &x in v {
            self.v3(x);
        }
    }
}

struct R<'a>(&'a [u8], usize);

impl R<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.0.get(self.1..self.1 + n)?;
        self.1 += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn v3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f()?, self.f()?, self.f()?))
    }
    fn q(&mut self) -> Option<Quat> {
        let (w, x, y, z) = (self.f()?, self.f()?, self.f()?, self.f()?);
        Some(Quat { w, x, y, z })
    }
    fn s(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn fs(&mut self) -> Option<Vec<f32>> {
        let n = self.u32()? as usize;
        if n > self.0.len() {
            return None;
        }
        (0..n).map(|_| self.f()).collect()
    }
    fn v3s(&mut self) -> Option<Vec<Vec3>> {
        let n = self.u32()? as usize;
        if n > self.0.len() {
            return None;
        }
        (0..n).map(|_| self.v3()).collect()
    }
}

fn encode_frame(w: &mut W, f: &SimFrame) {
    match f {
        SimFrame::Rigid(r) => {
            w.u32(0);
            let mut names: Vec<&String> = r.bodies.keys().collect();
            names.sort();
            w.u32(names.len() as u32);
            for n in names {
                let b = &r.bodies[n];
                w.s(n);
                w.v3(b.pos);
                w.q(b.rot);
                w.v3(b.vel);
                w.v3(b.ang);
                w.u32(b.sleeping as u32);
                w.f(b.sleep_time);
            }
            w.u32(r.broken.len() as u32);
            for b in &r.broken {
                w.s(b);
            }
        }
        SimFrame::Points(p) => {
            w.u32(1);
            w.v3s(&p.pos);
            w.v3s(&p.vel);
            w.v3s(&p.target);
        }
        SimFrame::Particles(p) => {
            w.u32(2);
            w.u32(p.particles.len() as u32);
            for x in &p.particles {
                w.v3(x.pos);
                w.v3(x.vel);
                w.q(x.rot);
                w.v3(x.ang);
                w.f(x.size);
                w.f(x.birth);
                w.f(x.die);
                w.u32(match x.state {
                    Alive::Unborn => 0,
                    Alive::Alive => 1,
                    Alive::Dead => 2,
                });
            }
            match &p.emitter {
                Some(m) => {
                    w.u32(1);
                    for x in m.to_cols_array() {
                        w.f(x);
                    }
                }
                None => w.u32(0),
            }
        }
        SimFrame::Fluid(f) => {
            w.u32(3);
            for r in f.res {
                w.u32(r as u32);
            }
            w.v3(f.min);
            w.f(f.dx);
            w.fs(&f.density);
            match &f.flame {
                Some(fl) => {
                    w.u32(1);
                    w.fs(fl);
                }
                None => w.u32(0),
            }
        }
        SimFrame::Liquid(f) => {
            w.u32(4);
            for r in f.res {
                w.u32(r as u32);
            }
            w.v3(f.min);
            w.f(f.dx);
            w.v3s(&f.mesh.positions);
            w.u32(f.mesh.faces.len() as u32);
            for fc in &f.mesh.faces {
                w.u32(fc.verts.len() as u32);
                for &v in &fc.verts {
                    w.u32(v);
                }
            }
            w.u32(f.count as u32);
            w.f(f.particle_volume);
            match &f.particles {
                Some(p) => {
                    w.u32(1);
                    w.v3s(p);
                }
                None => w.u32(0),
            }
        }
        SimFrame::Paint(p) => {
            w.u32(5);
            w.u32(p.color.len() as u32);
            for c in p.color.iter() {
                for &x in c {
                    w.f(x);
                }
            }
            w.fs(&p.wet);
            w.fs(&p.value);
            w.fs(&p.vel);
        }
    }
}

fn decode_frame(r: &mut R) -> Option<SimFrame> {
    Some(match r.u32()? {
        0 => {
            let n = r.u32()? as usize;
            let mut bodies = HashMap::new();
            for _ in 0..n {
                let name = r.s()?;
                let st = BodyState { pos: r.v3()?, rot: r.q()?, vel: r.v3()?, ang: r.v3()?, sleeping: r.u32()? != 0, sleep_time: r.f()? };
                bodies.insert(name, st);
            }
            let nb = r.u32()? as usize;
            let broken = (0..nb).map(|_| r.s()).collect::<Option<Vec<_>>>()?;
            SimFrame::Rigid(RigidFrame { bodies, broken, warm: Arc::default() })
        }
        1 => SimFrame::Points(PointsFrame { pos: r.v3s()?, vel: r.v3s()?, target: r.v3s()? }),
        2 => {
            let n = r.u32()? as usize;
            let mut particles = Vec::with_capacity(n.min(1 << 24));
            for _ in 0..n {
                let (pos, vel, rot, ang, size, birth, die) = (r.v3()?, r.v3()?, r.q()?, r.v3()?, r.f()?, r.f()?, r.f()?);
                let state = match r.u32()? {
                    0 => Alive::Unborn,
                    1 => Alive::Alive,
                    _ => Alive::Dead,
                };
                particles.push(Particle { pos, vel, rot, ang, size, birth, die, state });
            }
            let emitter = if r.u32()? == 1 {
                let mut a = [0.0f32; 16];
                for x in &mut a {
                    *x = r.f()?;
                }
                Some(Mat4::from_cols_array(a))
            } else {
                None
            };
            SimFrame::Particles(ParticlesFrame { particles, emitter })
        }
        3 => {
            let res = [r.u32()? as usize, r.u32()? as usize, r.u32()? as usize];
            let min = r.v3()?;
            let dx = r.f()?;
            let density = r.fs()?;
            if density.len() != res[0] * res[1] * res[2] {
                return None;
            }
            let flame = if r.u32()? == 1 { Some(Arc::new(r.fs()?)) } else { None };
            SimFrame::Fluid(FluidFrame { res, min, dx, density: Arc::new(density), flame, state: None })
        }
        4 => {
            let res = [r.u32()? as usize, r.u32()? as usize, r.u32()? as usize];
            let (min, dx) = (r.v3()?, r.f()?);
            let positions = r.v3s()?;
            let nf = r.u32()? as usize;
            if nf > r.0.len() {
                return None;
            }
            let mut faces = Vec::with_capacity(nf);
            for _ in 0..nf {
                let k = r.u32()? as usize;
                if k > 64 {
                    return None;
                }
                let verts = (0..k).map(|_| r.u32().filter(|&v| (v as usize) < positions.len())).collect::<Option<Vec<u32>>>()?;
                let mut fc = crate::mesh::Face::new(verts);
                fc.smooth = true;
                faces.push(fc);
            }
            let mut mesh = crate::mesh::Mesh::new();
            mesh.positions = positions;
            mesh.faces = faces;
            let count = r.u32()? as usize;
            let particle_volume = r.f()?;
            let particles = if r.u32()? == 1 { Some(Arc::new(r.v3s()?)) } else { None };
            SimFrame::Liquid(LiquidFrame { res, min, dx, mesh: Arc::new(mesh), particles, count, particle_volume, state: None })
        }
        5 => {
            let n = r.u32()? as usize;
            if n > r.0.len() {
                return None;
            }
            let color = (0..n).map(|_| Some([r.f()?, r.f()?, r.f()?, r.f()?])).collect::<Option<Vec<_>>>()?;
            let (wet, value, vel) = (r.fs()?, r.fs()?, r.fs()?);
            SimFrame::Paint(PaintFrame { color: Arc::new(color), wet: Arc::new(wet), value: Arc::new(value), vel: Arc::new(vel) })
        }
        _ => return None,
    })
}

/// The cache of `key` as file bytes.
pub fn encode(c: &Caches, key: &str) -> Option<Vec<u8>> {
    let frames = c.frames_of(key);
    if frames.is_empty() {
        return None;
    }
    let mut w = W(Vec::new());
    w.0.extend_from_slice(MAGIC);
    w.s(key);
    w.u32(frames.len() as u32);
    for (f, data) in frames {
        w.i32(f);
        encode_frame(&mut w, &data);
    }
    Some(w.0)
}

/// Load file bytes as the baked cache of `key`.
pub fn decode(c: &Caches, key: &str, bytes: &[u8]) -> bool {
    let parse = || -> Option<(BTreeMap<i32, Arc<SimFrame>>, usize)> {
        if bytes.get(..8)? != MAGIC {
            return None;
        }
        let mut r = R(bytes, 8);
        if r.s()? != key {
            return None;
        }
        let n = r.u32()? as usize;
        let mut frames = BTreeMap::new();
        for _ in 0..n {
            let f = r.i32()?;
            frames.insert(f, Arc::new(decode_frame(&mut r)?));
        }
        Some((frames, bytes.len()))
    };
    match parse() {
        Some((frames, size)) => {
            c.set_frames(key, frames, size);
            true
        }
        None => false,
    }
}
