// SPDX-License-Identifier: GPL-3.0-only
//! Piecewise-linear track automation curves for volume, pan, and filter cutoff.
//! Bounded memory (<= 256 points) and zero-alloc interpolation during audio playback.

use crate::Error;
use alloc::vec::Vec;

pub const MAX_AUTOMATION_POINTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationPoint {
    pub frame: usize,
    pub value: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationLane {
    points: Vec<AutomationPoint>,
    default_value: u16,
}

impl Default for AutomationLane {
    fn default() -> Self {
        Self::new(100)
    }
}

impl AutomationLane {
    pub fn new(default_value: u16) -> Self {
        Self {
            points: Vec::new(),
            default_value,
        }
    }

    pub fn points(&self) -> &[AutomationPoint] {
        &self.points
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    pub fn default_value(&self) -> u16 {
        self.default_value
    }

    pub fn set_default_value(&mut self, val: u16) {
        self.default_value = val;
    }

    /// Add or update an automation point, maintaining chronological order.
    pub fn add_point(&mut self, point: AutomationPoint) -> Result<(), Error> {
        // If point with same frame exists, replace value
        if let Some(pos) = self.points.iter().position(|p| p.frame == point.frame) {
            self.points[pos].value = point.value;
            return Ok(());
        }

        if self.points.len() >= MAX_AUTOMATION_POINTS {
            return Err(Error::Range);
        }

        let idx = self.points.partition_point(|p| p.frame < point.frame);
        self.points.insert(idx, point);
        Ok(())
    }

    pub fn remove_at_frame(&mut self, frame: usize) -> bool {
        if let Some(pos) = self.points.iter().position(|p| p.frame == frame) {
            self.points.remove(pos);
            true
        } else {
            false
        }
    }

    /// Sample the curve at a timeline frame via zero-alloc piecewise linear interpolation.
    #[inline]
    pub fn sample_at(&self, frame: usize) -> u16 {
        if self.points.is_empty() {
            return self.default_value;
        }

        if frame <= self.points[0].frame {
            return self.points[0].value;
        }

        let last = &self.points[self.points.len() - 1];
        if frame >= last.frame {
            return last.value;
        }

        // Binary search for the segment [p0, p1]
        let idx = self.points.partition_point(|p| p.frame <= frame);
        let p0 = &self.points[idx - 1];
        let p1 = &self.points[idx];

        let dx = (p1.frame - p0.frame) as i64;
        if dx == 0 {
            return p0.value;
        }
        let dy = p1.value as i64 - p0.value as i64;
        let offset = (frame - p0.frame) as i64;
        let interpolated = p0.value as i64 + (dy * offset / dx);

        interpolated.clamp(0, u16::MAX as i64) as u16
    }
}
