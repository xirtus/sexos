// SPDX-License-Identifier: GPL-3.0-only
//! The point cache (Blender's PointCache): simulated frames kept in memory
//! per simulation, keyed by the scene, plus baking and an optional disk copy.
//!
//! Every simulation has a key ("rigidbody_world", "Cube/Cloth", ...) and a
//! signature: a hash of everything its result depends on except time
//! (settings, rest geometry, starting transforms). When the signature
//! changes the frames are freed and the cache is marked outdated, as
//! Blender does when a setting changes. A baked cache keeps its frames and
//! ignores changes until "Free Bake".
//!
//! The scene holds a [`Caches`] handle: clones of the scene (undo steps,
//! the viewport's copies, renders) share the same frames, and the handle
//! compares equal to any other so scenes still compare by their data.
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// One simulated frame of one simulation.
#[derive(Clone, Debug)]
pub enum SimFrame {
    Rigid(super::rigid::RigidFrame),
    /// Cloth and soft body: world-space positions and velocities.
    Points(super::cloth::PointsFrame),
    Particles(super::particles::ParticlesFrame),
    Fluid(super::fluid::FluidFrame),
    Liquid(super::liquid::LiquidFrame),
    /// Dynamic Paint canvas values per vertex.
    Paint(super::dynamic_paint::PaintFrame),
}

/// What the cache of one simulation holds.
#[derive(Clone, Debug, Default)]
pub struct CacheEntry {
    pub frames: BTreeMap<i32, Arc<SimFrame>>,
    pub signature: u64,
    /// Signature of the start state (transforms at the start frame).
    pub init_signature: u64,
    pub baked: bool,
    /// Settings changed after frames were simulated (they were freed).
    pub outdated: bool,
    /// Full solver state to continue from, when frames only keep what is
    /// drawn (fluids): (frame, state).
    pub resume: Option<(i32, Arc<SimFrame>)>,
    /// Bytes held, roughly, for the cache panel.
    pub bytes: usize,
}

impl CacheEntry {
    pub fn last_frame(&self) -> Option<i32> {
        self.frames.keys().next_back().copied()
    }

    /// Blender's cache info line ("120 frames in memory (1.2 MB)").
    pub fn info(&self) -> String {
        let n = self.frames.len();
        let mb = self.bytes as f64 / (1024.0 * 1024.0);
        let what = if self.baked { "baked" } else { "in memory" };
        let mut s = format!("{n} frames {what} ({mb:.1} MB)");
        if self.outdated {
            s.push_str(", cache is outdated!");
        }
        s
    }

    fn free(&mut self) {
        self.frames.clear();
        self.resume = None;
        self.bytes = 0;
    }
}

#[derive(Default, Debug)]
struct Store {
    entries: HashMap<String, CacheEntry>,
    /// Directory for disk caches (Blender's `//cache/`), set by the app
    /// from the document path.
    dir: Option<String>,
}

/// Shared handle to a scene's point caches.
#[derive(Clone, Default)]
pub struct Caches(Arc<Mutex<Store>>);

impl std::fmt::Debug for Caches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Caches")
    }
}

impl PartialEq for Caches {
    fn eq(&self, _: &Caches) -> bool {
        true
    }
}

impl Caches {
    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The frame `f` of simulation `key`, or the last cached frame before it
    /// (a simulation shown past its cached range holds its last state).
    pub fn frame_at(&self, key: &str, f: i32) -> Option<Arc<SimFrame>> {
        let s = self.lock();
        let e = s.entries.get(key)?;
        e.frames.range(..=f).next_back().map(|(_, v)| v.clone())
    }

    /// Exactly frame `f`.
    pub fn exact(&self, key: &str, f: i32) -> Option<Arc<SimFrame>> {
        self.lock().entries.get(key)?.frames.get(&f).cloned()
    }

    pub fn entry(&self, key: &str) -> Option<CacheEntry> {
        self.lock().entries.get(key).cloned()
    }

    /// Info for the cache panel: (info line, baked, outdated, frames).
    pub fn status(&self, key: &str) -> (String, bool, bool, usize) {
        match self.lock().entries.get(key) {
            Some(e) => (e.info(), e.baked, e.outdated, e.frames.len()),
            None => ("0 frames in memory (0.0 MB)".into(), false, false, 0),
        }
    }

    pub fn is_baked(&self, key: &str) -> bool {
        self.lock().entries.get(key).is_some_and(|e| e.baked)
    }

    /// Compare the signature; on a change free the frames and mark the
    /// cache outdated (unless it is baked). Returns true when frames were freed.
    pub fn validate(&self, key: &str, signature: u64) -> bool {
        let mut s = self.lock();
        let e = s.entries.entry(key.to_string()).or_default();
        if e.signature == signature || e.baked {
            if e.frames.is_empty() && !e.baked {
                e.signature = signature;
            }
            return false;
        }
        let had = !e.frames.is_empty();
        e.free();
        e.signature = signature;
        e.outdated = had;
        had
    }

    /// At the start frame: compare the start-state signature (where the
    /// objects are) and free the frames when it changed.
    pub fn validate_start(&self, key: &str, init: u64) -> bool {
        let mut s = self.lock();
        let e = s.entries.entry(key.to_string()).or_default();
        if e.baked || e.init_signature == init {
            return false;
        }
        let stale = e.init_signature != 0 && !e.frames.is_empty();
        e.init_signature = init;
        if stale {
            e.free();
            e.outdated = true;
        }
        stale
    }

    /// Replace a cache's frames (reading a disk bake).
    pub(crate) fn set_frames(&self, key: &str, frames: BTreeMap<i32, Arc<SimFrame>>, bytes: usize) {
        let mut s = self.lock();
        let e = s.entries.entry(key.to_string()).or_default();
        e.frames = frames;
        e.bytes = bytes;
        e.baked = true;
        e.outdated = false;
    }

    pub fn insert(&self, key: &str, frame: i32, data: SimFrame, bytes: usize) {
        let mut s = self.lock();
        let e = s.entries.entry(key.to_string()).or_default();
        e.bytes += bytes;
        e.frames.insert(frame, Arc::new(data));
    }

    pub fn set_resume(&self, key: &str, frame: i32, data: Arc<SimFrame>) {
        let mut s = self.lock();
        s.entries.entry(key.to_string()).or_default().resume = Some((frame, data));
    }

    pub fn resume(&self, key: &str) -> Option<(i32, Arc<SimFrame>)> {
        self.lock().entries.get(key)?.resume.clone()
    }

    /// The last cached frame and its data.
    pub fn last(&self, key: &str) -> Option<(i32, Arc<SimFrame>)> {
        let s = self.lock();
        s.entries.get(key)?.frames.iter().next_back().map(|(f, v)| (*f, v.clone()))
    }

    /// Free the frames after `frame` (jumping back before the cached range).
    pub fn truncate_after(&self, key: &str, frame: i32) {
        let mut s = self.lock();
        if let Some(e) = s.entries.get_mut(key) {
            if !e.baked {
                let _ = e.frames.split_off(&(frame + 1));
                if e.resume.as_ref().is_some_and(|r| r.0 > frame) {
                    e.resume = None;
                }
            }
        }
    }

    /// Clear the outdated flag (the simulation was redone from the start).
    pub fn mark_current(&self, key: &str) {
        if let Some(e) = self.lock().entries.get_mut(key) {
            e.outdated = false;
        }
    }

    /// Free one cache, baked or not ("Free Bake", "Delete Bake").
    pub fn free(&self, key: &str) {
        let mut s = self.lock();
        if let Some(e) = s.entries.get_mut(key) {
            e.free();
            e.baked = false;
            e.outdated = false;
        }
    }

    /// Free every cache ("Delete All Bakes" frees baked and unbaked alike).
    pub fn free_all(&self) {
        let mut s = self.lock();
        for e in s.entries.values_mut() {
            e.free();
            e.baked = false;
            e.outdated = false;
        }
    }

    /// Free the unbaked caches whose key starts with `prefix` (an object
    /// was renamed or deleted, or the scene changed wholesale).
    pub fn free_unbaked(&self) {
        let mut s = self.lock();
        for e in s.entries.values_mut() {
            if !e.baked {
                e.free();
            }
        }
    }

    pub fn set_baked(&self, key: &str, baked: bool) {
        let mut s = self.lock();
        s.entries.entry(key.to_string()).or_default().baked = baked;
    }

    pub fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = self.lock().entries.keys().cloned().collect();
        k.sort();
        k
    }

    /// Where disk caches go (`<dir of the .poly>/cache`), if known.
    pub fn set_dir(&self, dir: Option<String>) {
        self.lock().dir = dir;
    }

    pub fn dir(&self) -> Option<String> {
        self.lock().dir.clone()
    }

    pub(crate) fn frames_of(&self, key: &str) -> Vec<(i32, Arc<SimFrame>)> {
        self.lock().entries.get(key).map(|e| e.frames.iter().map(|(f, v)| (*f, v.clone())).collect()).unwrap_or_default()
    }
}

/// A file name for a cache key (Blender's `<name>_<index>`-style names).
pub fn file_stem(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect()
}
