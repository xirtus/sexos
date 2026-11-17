// SPDX-License-Identifier: GPL-3.0-only
//! LadderVcf and StateVariableVcf modules for EuropaRack.

use crate::math::{cv_to_freq, soft_clip, tan, PI};
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, MAX_POLY, RATE};

pub struct LadderVcf {
    cutoff_hz: f32,
    resonance: f32, // 0.0..1.0
    drive: f32,     // 1.0..4.0
    cutoff_attenuverter: f32,
    res_attenuverter: f32,
    // 4 poles per channel, stereo left & right
    s: [[[f32; 4]; 2]; MAX_POLY],
}

impl Default for LadderVcf {
    fn default() -> Self {
        Self::new()
    }
}

impl LadderVcf {
    pub const fn new() -> Self {
        Self {
            cutoff_hz: 1000.0,
            resonance: 0.2,
            drive: 1.0,
            cutoff_attenuverter: 1.0,
            res_attenuverter: 0.0,
            s: [[[0.0; 4]; 2]; MAX_POLY],
        }
    }
}

const LADDER_INPUTS: [PortDesc; 3] = [
    PortDesc {
        id: 0,
        name: "Audio In",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Cutoff CV",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 2,
        name: "Resonance CV",
        kind: PortKind::ModCv,
    },
];

const LADDER_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for LadderVcf {
    fn kind(&self) -> ModuleKind {
        ModuleKind::LadderVcf
    }

    fn inputs(&self) -> &[PortDesc] {
        &LADDER_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &LADDER_OUTPUTS
    }

    fn param_count(&self) -> usize {
        5
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.cutoff_hz,
            1 => self.resonance,
            2 => self.drive,
            3 => self.cutoff_attenuverter,
            4 => self.res_attenuverter,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.cutoff_hz = value.clamp(20.0, 20_000.0),
            1 => self.resonance = value.clamp(0.0, 1.0),
            2 => self.drive = value.clamp(0.5, 4.0),
            3 => self.cutoff_attenuverter = value.clamp(-2.0, 2.0),
            4 => self.res_attenuverter = value.clamp(-1.0, 1.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.s = [[[0.0; 4]; 2]; MAX_POLY];
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() {
            return;
        }

        let poly_ch = if !inputs.is_empty() {
            inputs[0].poly_channels.clamp(1, MAX_POLY as u8) as usize
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
                let cutoff_mod = if inputs.len() > 1 {
                    inputs[1].get_cv(ch, f) * self.cutoff_attenuverter
                } else {
                    0.0
                };
                let res_mod = if inputs.len() > 2 {
                    inputs[2].get_cv(ch, f) * 0.1 * self.res_attenuverter
                } else {
                    0.0
                };

                let effective_cutoff =
                    (self.cutoff_hz * cv_to_freq(cutoff_mod) / 261.6256).clamp(20.0, 20_000.0);
                let effective_res = (self.resonance + res_mod).clamp(0.0, 0.99);

                // Moog 4-pole normalized coefficient
                let wc = (2.0 * PI * effective_cutoff / (RATE as f32)).clamp(0.001, 0.9);
                let g = (wc * 0.5).clamp(0.001, 0.95);
                let k = 4.0 * effective_res;

                let mut out_stereo = [0.0f32; 2];

                for c in 0..2 {
                    let x = soft_clip(in_audio[c] * self.drive);
                    let s = &mut self.s[ch][c];

                    // Tanh saturation in feedback
                    let feedback = soft_clip(s[3] * k);
                    let u = x - feedback;

                    let y0 = s[0] + g * (u - s[0]);
                    s[0] = y0 + g * (u - s[0]);

                    let y1 = s[1] + g * (y0 - s[1]);
                    s[1] = y1 + g * (y0 - s[1]);

                    let y2 = s[2] + g * (y1 - s[2]);
                    s[2] = y2 + g * (y1 - s[2]);

                    let y3 = s[3] + g * (y2 - s[3]);
                    s[3] = y3 + g * (y2 - s[3]);

                    out_stereo[c] = soft_clip(y3);
                }

                if ch == 0 {
                    outputs[0].set_audio(f, out_stereo);
                }
                outputs[0].set_cv(ch, f, out_stereo[0] * 5.0);
            }
        }
    }
}

pub struct StateVariableVcf {
    cutoff_hz: f32,
    res_q: f32,
    ic1eq: [[f32; 2]; MAX_POLY],
    ic2eq: [[f32; 2]; MAX_POLY],
}

impl Default for StateVariableVcf {
    fn default() -> Self {
        Self::new()
    }
}

impl StateVariableVcf {
    pub const fn new() -> Self {
        Self {
            cutoff_hz: 2000.0,
            res_q: 1.0,
            ic1eq: [[0.0; 2]; MAX_POLY],
            ic2eq: [[0.0; 2]; MAX_POLY],
        }
    }
}

const SVF_INPUTS: [PortDesc; 3] = [
    PortDesc {
        id: 0,
        name: "Audio In",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Cutoff CV",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 2,
        name: "Resonance CV",
        kind: PortKind::ModCv,
    },
];

const SVF_OUTPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Lowpass Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Bandpass Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 2,
        name: "Highpass Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 3,
        name: "Notch Out",
        kind: PortKind::Audio,
    },
];

impl Module for StateVariableVcf {
    fn kind(&self) -> ModuleKind {
        ModuleKind::StateVariableVcf
    }

    fn inputs(&self) -> &[PortDesc] {
        &SVF_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &SVF_OUTPUTS
    }

    fn param_count(&self) -> usize {
        2
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.cutoff_hz,
            1 => self.res_q,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.cutoff_hz = value.clamp(20.0, 20_000.0),
            1 => self.res_q = value.clamp(0.5, 10.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.ic1eq = [[0.0; 2]; MAX_POLY];
        self.ic2eq = [[0.0; 2]; MAX_POLY];
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 4 {
            return;
        }

        let poly_ch = if !inputs.is_empty() {
            inputs[0].poly_channels.clamp(1, MAX_POLY as u8) as usize
        } else {
            1
        };

        for out in outputs.iter_mut() {
            out.poly_channels = poly_ch as u8;
        }

        for ch in 0..poly_ch {
            for f in 0..BLOCK_FRAMES {
                let in_audio = if !inputs.is_empty() {
                    inputs[0].get_audio(f)
                } else {
                    [0.0, 0.0]
                };
                let cutoff_mod = if inputs.len() > 1 {
                    inputs[1].get_cv(ch, f)
                } else {
                    0.0
                };
                let q_mod = if inputs.len() > 2 {
                    inputs[2].get_cv(ch, f) * 0.2
                } else {
                    0.0
                };

                let effective_cutoff =
                    (self.cutoff_hz * cv_to_freq(cutoff_mod) / 261.6256).clamp(20.0, 20_000.0);
                let q = (self.res_q + q_mod).clamp(0.5, 20.0);

                let g = tan(PI * effective_cutoff / (RATE as f32)).clamp(0.0001, 10.0);
                let k = 1.0 / q;
                let a1 = 1.0 / (1.0 + g * (g + k));
                let a2 = g * a1;
                let a3 = g * a2;

                let mut lp = [0.0f32; 2];
                let mut bp = [0.0f32; 2];
                let mut hp = [0.0f32; 2];
                let mut notch = [0.0f32; 2];

                for c in 0..2 {
                    let v0 = in_audio[c];
                    let v1 = a1 * self.ic1eq[ch][c] + a2 * (v0 - self.ic2eq[ch][c]);
                    let v2 = self.ic2eq[ch][c] + a3 * self.ic1eq[ch][c] + a2 * a2 * (v0 - self.ic2eq[ch][c]);

                    let hp_val = v0 - k * v1 - v2;
                    let bp_val = v1;
                    let lp_val = v2;
                    let notch_val = hp_val + lp_val;

                    self.ic1eq[ch][c] = 2.0 * v1 - self.ic1eq[ch][c];
                    self.ic2eq[ch][c] = 2.0 * v2 - self.ic2eq[ch][c];

                    lp[c] = soft_clip(lp_val);
                    bp[c] = soft_clip(bp_val);
                    hp[c] = soft_clip(hp_val);
                    notch[c] = soft_clip(notch_val);
                }

                if ch == 0 {
                    outputs[0].set_audio(f, lp);
                    outputs[1].set_audio(f, bp);
                    outputs[2].set_audio(f, hp);
                    outputs[3].set_audio(f, notch);
                }

                outputs[0].set_cv(ch, f, lp[0] * 5.0);
                outputs[1].set_cv(ch, f, bp[0] * 5.0);
                outputs[2].set_cv(ch, f, hp[0] * 5.0);
                outputs[3].set_cv(ch, f, notch[0] * 5.0);
            }
        }
    }
}
