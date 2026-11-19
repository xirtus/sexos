// SPDX-License-Identifier: GPL-3.0-only
//! MatrixMixer and StereoDelay modules for EuropaRack.

extern crate alloc;
use alloc::vec::Vec;

use crate::math::soft_clip;
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, RATE};

pub struct MatrixMixer {
    // 4 inputs x 4 outputs = 16 attenuverter gains (-1.0..+1.0)
    gains: [[f32; 4]; 4],
}

impl Default for MatrixMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl MatrixMixer {
    pub const fn new() -> Self {
        Self {
            gains: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}

const MATRIX_INPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "In 1",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "In 2",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 2,
        name: "In 3",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 3,
        name: "In 4",
        kind: PortKind::Audio,
    },
];

const MATRIX_OUTPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Out 1",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Out 2",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 2,
        name: "Out 3",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 3,
        name: "Out 4",
        kind: PortKind::Audio,
    },
];

impl Module for MatrixMixer {
    fn kind(&self) -> ModuleKind {
        ModuleKind::MatrixMixer
    }

    fn inputs(&self) -> &[PortDesc] {
        &MATRIX_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &MATRIX_OUTPUTS
    }

    fn param_count(&self) -> usize {
        16
    }

    fn get_param(&self, index: usize) -> f32 {
        if index < 16 {
            let row = index / 4;
            let col = index % 4;
            self.gains[row][col]
        } else {
            0.0
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index < 16 {
            let row = index / 4;
            let col = index % 4;
            self.gains[row][col] = value.clamp(-2.0, 2.0);
        }
    }

    fn reset(&mut self) {}

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 4 {
            return;
        }

        for out_idx in 0..4 {
            outputs[out_idx].clear();
        }

        for f in 0..BLOCK_FRAMES {
            let mut in_audio = [[0.0f32; 2]; 4];
            let mut in_cv = [0.0f32; 4];

            for in_idx in 0..4 {
                if in_idx < inputs.len() {
                    in_audio[in_idx] = inputs[in_idx].get_audio(f);
                    in_cv[in_idx] = inputs[in_idx].get_cv(0, f);
                }
            }

            for out_idx in 0..4 {
                let mut sum_audio = [0.0f32; 2];
                let mut sum_cv = 0.0f32;

                for in_idx in 0..4 {
                    let g = self.gains[in_idx][out_idx];
                    sum_audio[0] += in_audio[in_idx][0] * g;
                    sum_audio[1] += in_audio[in_idx][1] * g;
                    sum_cv += in_cv[in_idx] * g;
                }

                outputs[out_idx].set_audio(f, sum_audio);
                outputs[out_idx].set_cv(0, f, sum_cv);
            }
        }
    }
}

pub const MAX_DELAY_FRAMES: usize = 48_000; // 1.0 second at 48kHz

pub struct StereoDelay {
    time_ms: f32,
    feedback: f32, // 0.0..0.95
    dry_wet: f32,  // 0.0..1.0
    ping_pong: bool,
    buffer: Vec<[f32; 2]>,
    write_pos: usize,
}

impl Default for StereoDelay {
    fn default() -> Self {
        Self::new()
    }
}

impl StereoDelay {
    pub fn new() -> Self {
        let mut buf = Vec::new();
        let _ = buf.try_reserve_exact(MAX_DELAY_FRAMES);
        buf.resize(MAX_DELAY_FRAMES, [0.0, 0.0]);
        Self {
            time_ms: 300.0,
            feedback: 0.4,
            dry_wet: 0.5,
            ping_pong: true,
            buffer: buf,
            write_pos: 0,
        }
    }
}

const DELAY_INPUTS: [PortDesc; 3] = [
    PortDesc {
        id: 0,
        name: "Audio In",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Time CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Feedback CV",
        kind: PortKind::ModCv,
    },
];

const DELAY_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for StereoDelay {
    fn kind(&self) -> ModuleKind {
        ModuleKind::StereoDelay
    }

    fn inputs(&self) -> &[PortDesc] {
        &DELAY_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &DELAY_OUTPUTS
    }

    fn param_count(&self) -> usize {
        4
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.time_ms,
            1 => self.feedback,
            2 => self.dry_wet,
            3 => {
                if self.ping_pong {
                    1.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.time_ms = value.clamp(1.0, 990.0),
            1 => self.feedback = value.clamp(0.0, 0.95),
            2 => self.dry_wet = value.clamp(0.0, 1.0),
            3 => self.ping_pong = value >= 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        for frame in &mut self.buffer {
            *frame = [0.0, 0.0];
        }
        self.write_pos = 0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() || self.buffer.is_empty() {
            return;
        }

        let buf_len = self.buffer.len();

        for f in 0..BLOCK_FRAMES {
            let in_audio = if !inputs.is_empty() {
                inputs[0].get_audio(f)
            } else {
                [0.0, 0.0]
            };
            let time_mod = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 50.0
            } else {
                0.0
            };
            let fb_mod = if inputs.len() > 2 {
                inputs[2].get_cv(0, f) * 0.05
            } else {
                0.0
            };

            let t_ms = (self.time_ms + time_mod).clamp(1.0, 990.0);
            let fb = (self.feedback + fb_mod).clamp(0.0, 0.95);

            let delay_samples = ((t_ms * (RATE as f32) / 1000.0) as usize).clamp(1, buf_len - 1);
            let read_pos = (self.write_pos + buf_len - delay_samples) % buf_len;

            let delayed = self.buffer[read_pos];

            // Ping-pong or straight stereo feedback
            let (fb_l, fb_r) = if self.ping_pong {
                (delayed[1] * fb, delayed[0] * fb)
            } else {
                (delayed[0] * fb, delayed[1] * fb)
            };

            // Soft-clip in feedback loop prevents runaway
            let write_l = soft_clip(in_audio[0] + fb_l);
            let write_r = soft_clip(in_audio[1] + fb_r);

            self.buffer[self.write_pos] = [write_l, write_r];
            self.write_pos = (self.write_pos + 1) % buf_len;

            let wet_l = delayed[0];
            let wet_r = delayed[1];

            let out_l = in_audio[0] * (1.0 - self.dry_wet) + wet_l * self.dry_wet;
            let out_r = in_audio[1] * (1.0 - self.dry_wet) + wet_r * self.dry_wet;

            outputs[0].set_audio(f, [out_l, out_r]);
            outputs[0].set_cv(0, f, out_l * 5.0);
        }
    }
}
