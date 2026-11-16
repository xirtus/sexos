// SPDX-License-Identifier: GPL-3.0-only
//! PolyVca and Wavefolder modules for EuropaRack.

use crate::math::soft_clip;
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, MAX_POLY};

pub struct PolyVca {
    gain: f32,
    exp_curve: f32, // 0.0 = Linear, 1.0 = Exponential
}

impl Default for PolyVca {
    fn default() -> Self {
        Self::new()
    }
}

impl PolyVca {
    pub const fn new() -> Self {
        Self {
            gain: 0.0,
            exp_curve: 0.5,
        }
    }
}

const VCA_INPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Audio In",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Control CV (0..10V)",
        kind: PortKind::ModCv,
    },
];

const VCA_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for PolyVca {
    fn kind(&self) -> ModuleKind {
        ModuleKind::PolyVca
    }

    fn inputs(&self) -> &[PortDesc] {
        &VCA_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &VCA_OUTPUTS
    }

    fn param_count(&self) -> usize {
        2
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.gain,
            1 => self.exp_curve,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.gain = value.clamp(0.0, 1.0),
            1 => self.exp_curve = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn reset(&mut self) {}

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() {
            return;
        }

        let poly_ch = if !inputs.is_empty() {
            let ch_in0 = inputs[0].poly_channels;
            let ch_in1 = if inputs.len() > 1 {
                inputs[1].poly_channels
            } else {
                1
            };
            ch_in0.max(ch_in1).clamp(1, MAX_POLY as u8) as usize
        } else {
            1
        };

        outputs[0].poly_channels = poly_ch as u8;

        for ch in 0..poly_ch {
            for f in 0..BLOCK_FRAMES {
                let in_audio = if !inputs.is_empty() {
                    inputs[0].get_audio(f)
                } else {
                    [0.0, 0.0]
                };
                let cv_raw = if inputs.len() > 1 {
                    inputs[1].get_cv(ch, f) * 0.1
                } else {
                    0.0
                };

                let norm_cv = (cv_raw + self.gain).clamp(0.0, 1.5);
                // Linear vs exponential response blend
                let lin_gain = norm_cv;
                let exp_gain = norm_cv * norm_cv;
                let amp = lin_gain * (1.0 - self.exp_curve) + exp_gain * self.exp_curve;

                let out_l = in_audio[0] * amp;
                let out_r = in_audio[1] * amp;

                if ch == 0 {
                    outputs[0].set_audio(f, [out_l, out_r]);
                }
                outputs[0].set_cv(ch, f, out_l * 5.0);
            }
        }
    }
}

pub struct Wavefolder {
    fold_amount: f32, // 1.0..10.0
    bias: f32,        // -1.0..+1.0
}

impl Default for Wavefolder {
    fn default() -> Self {
        Self::new()
    }
}

impl Wavefolder {
    pub const fn new() -> Self {
        Self {
            fold_amount: 2.0,
            bias: 0.0,
        }
    }

    #[inline]
    fn fold(x: f32) -> f32 {
        // Multi-stage folding diode approximation
        let mut v = x;
        for _ in 0..3 {
            if v > 1.0 {
                v = 2.0 - v;
            } else if v < -1.0 {
                v = -2.0 - v;
            }
        }
        soft_clip(v)
    }
}

const FOLDER_INPUTS: [PortDesc; 3] = [
    PortDesc {
        id: 0,
        name: "Audio In",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Fold CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Bias CV",
        kind: PortKind::ModCv,
    },
];

const FOLDER_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for Wavefolder {
    fn kind(&self) -> ModuleKind {
        ModuleKind::Wavefolder
    }

    fn inputs(&self) -> &[PortDesc] {
        &FOLDER_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &FOLDER_OUTPUTS
    }

    fn param_count(&self) -> usize {
        2
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.fold_amount,
            1 => self.bias,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.fold_amount = value.clamp(1.0, 10.0),
            1 => self.bias = value.clamp(-2.0, 2.0),
            _ => {}
        }
    }

    fn reset(&mut self) {}

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() {
            return;
        }

        for f in 0..BLOCK_FRAMES {
            let in_audio = if !inputs.is_empty() {
                inputs[0].get_audio(f)
            } else {
                [0.0, 0.0]
            };
            let fold_cv = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 0.5
            } else {
                0.0
            };
            let bias_cv = if inputs.len() > 2 {
                inputs[2].get_cv(0, f) * 0.1
            } else {
                0.0
            };

            let fold = (self.fold_amount + fold_cv).clamp(1.0, 12.0);
            let b = (self.bias + bias_cv).clamp(-2.0, 2.0);

            let folded_l = Self::fold((in_audio[0] + b) * fold);
            let folded_r = Self::fold((in_audio[1] + b) * fold);

            outputs[0].set_audio(f, [folded_l, folded_r]);
            outputs[0].set_cv(0, f, folded_l * 5.0);
        }
    }
}
