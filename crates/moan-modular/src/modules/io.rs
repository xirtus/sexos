// SPDX-License-Identifier: GPL-3.0-only
//! TrackIn and MasterOut modules for EuropaRack.

use crate::math::{abs, soft_clip};
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES};

pub struct TrackIn {
    gain: f32,
    pub external_input: [[f32; 2]; BLOCK_FRAMES],
}

impl Default for TrackIn {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackIn {
    pub const fn new() -> Self {
        Self {
            gain: 1.0,
            external_input: [[0.0; 2]; BLOCK_FRAMES],
        }
    }

    pub fn set_external_input(&mut self, audio: &[[f32; 2]; BLOCK_FRAMES]) {
        self.external_input = *audio;
    }
}

const TRACK_IN_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for TrackIn {
    fn kind(&self) -> ModuleKind {
        ModuleKind::TrackIn
    }

    fn inputs(&self) -> &[PortDesc] {
        &[]
    }

    fn outputs(&self) -> &[PortDesc] {
        &TRACK_IN_OUTPUTS
    }

    fn param_count(&self) -> usize {
        1
    }

    fn get_param(&self, index: usize) -> f32 {
        if index == 0 {
            self.gain
        } else {
            0.0
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.gain = value.clamp(0.0, 4.0);
        }
    }

    fn reset(&mut self) {
        self.external_input = [[0.0; 2]; BLOCK_FRAMES];
    }

    fn process_block(&mut self, _inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() {
            return;
        }
        for f in 0..BLOCK_FRAMES {
            outputs[0].audio[f][0] = self.external_input[f][0] * self.gain;
            outputs[0].audio[f][1] = self.external_input[f][1] * self.gain;
        }
    }
}

pub struct MasterOut {
    volume: f32,
    drive: f32,
    pub peak_meter: [f32; 2],
    pub clipping: bool,
    pub output_frames: [[f32; 2]; BLOCK_FRAMES],
}

impl Default for MasterOut {
    fn default() -> Self {
        Self::new()
    }
}

impl MasterOut {
    pub const fn new() -> Self {
        Self {
            volume: 1.0,
            drive: 1.0,
            peak_meter: [0.0, 0.0],
            clipping: false,
            output_frames: [[0.0; 2]; BLOCK_FRAMES],
        }
    }

    pub fn peak(&self) -> [f32; 2] {
        self.peak_meter
    }

    pub fn is_clipping(&self) -> bool {
        self.clipping
    }
}

const MASTER_OUT_INPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio In",
    kind: PortKind::Audio,
}];

impl Module for MasterOut {
    fn kind(&self) -> ModuleKind {
        ModuleKind::MasterOut
    }

    fn inputs(&self) -> &[PortDesc] {
        &MASTER_OUT_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &[]
    }

    fn param_count(&self) -> usize {
        2
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.volume,
            1 => self.drive,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.volume = value.clamp(0.0, 4.0),
            1 => self.drive = value.clamp(0.1, 10.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.peak_meter = [0.0, 0.0];
        self.clipping = false;
        self.output_frames = [[0.0; 2]; BLOCK_FRAMES];
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], _outputs: &mut [SignalBuffer]) {
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        let scale = self.volume * self.drive;

        for f in 0..BLOCK_FRAMES {
            let in_l = if !inputs.is_empty() {
                inputs[0].audio[f][0]
            } else {
                0.0
            };
            let in_r = if !inputs.is_empty() {
                inputs[0].audio[f][1]
            } else {
                0.0
            };

            let out_l = soft_clip(in_l * scale);
            let out_r = soft_clip(in_r * scale);

            if abs(out_l) > peak_l {
                peak_l = abs(out_l);
            }
            if abs(out_r) > peak_r {
                peak_r = abs(out_r);
            }

            self.output_frames[f] = [out_l, out_r];
        }

        self.peak_meter = [peak_l, peak_r];
        self.clipping = peak_l >= 0.99 || peak_r >= 0.99;
    }
}
