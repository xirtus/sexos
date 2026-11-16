// SPDX-License-Identifier: GPL-3.0-only
//! AnalogVco and Fm4OpMacro modules for EuropaRack.

use crate::math::{cv_to_freq, poly_blep, sin, TWO_PI};
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, MAX_POLY, RATE};

pub struct AnalogVco {
    octave: i8,
    fine: f32, // -1.0..+1.0 semitones
    pulse_width: f32,
    lin_fm_depth: f32,
    phases: [f32; MAX_POLY],
    last_sync: [f32; MAX_POLY],
}

impl Default for AnalogVco {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalogVco {
    pub const fn new() -> Self {
        Self {
            octave: 0,
            fine: 0.0,
            pulse_width: 0.5,
            lin_fm_depth: 100.0,
            phases: [0.0; MAX_POLY],
            last_sync: [0.0; MAX_POLY],
        }
    }
}

const VCO_INPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Pitch CV (1V/oct)",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 1,
        name: "Linear FM",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "PWM CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Hard Sync",
        kind: PortKind::Gate,
    },
];

const VCO_OUTPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Saw Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Pulse Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 2,
        name: "Triangle Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 3,
        name: "Sine Out",
        kind: PortKind::Audio,
    },
];

impl Module for AnalogVco {
    fn kind(&self) -> ModuleKind {
        ModuleKind::AnalogVco
    }

    fn inputs(&self) -> &[PortDesc] {
        &VCO_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &VCO_OUTPUTS
    }

    fn param_count(&self) -> usize {
        4
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.octave as f32,
            1 => self.fine,
            2 => self.pulse_width,
            3 => self.lin_fm_depth,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.octave = value.clamp(-4.0, 4.0) as i8,
            1 => self.fine = value.clamp(-1.0, 1.0),
            2 => self.pulse_width = value.clamp(0.01, 0.99),
            3 => self.lin_fm_depth = value.clamp(0.0, 1000.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.phases = [0.0; MAX_POLY];
        self.last_sync = [0.0; MAX_POLY];
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

        let fine_oct = (self.octave as f32) + (self.fine / 12.0);

        for ch in 0..poly_ch {
            let mut phase = self.phases[ch];

            for f in 0..BLOCK_FRAMES {
                let pitch_cv = if !inputs.is_empty() {
                    inputs[0].get_cv(ch, f)
                } else {
                    0.0
                };
                let lin_fm = if inputs.len() > 1 {
                    inputs[1].get_cv(ch, f)
                } else {
                    0.0
                };
                let pwm_cv = if inputs.len() > 2 {
                    inputs[2].get_cv(ch, f)
                } else {
                    0.0
                };
                let sync_cv = if inputs.len() > 3 {
                    inputs[3].get_cv(ch, f)
                } else {
                    0.0
                };

                // Hard sync on rising edge
                if sync_cv >= 2.5 && self.last_sync[ch] < 2.5 {
                    phase = 0.0;
                }
                self.last_sync[ch] = sync_cv;

                let base_freq = cv_to_freq(pitch_cv + fine_oct);
                let freq = (base_freq + lin_fm * self.lin_fm_depth).clamp(0.1, (RATE / 2) as f32);
                let dt = freq / (RATE as f32);

                // Pulse width with CV (±5V maps to ±0.4)
                let pw = (self.pulse_width + pwm_cv * 0.08).clamp(0.02, 0.98);

                // Anti-aliased PolyBLEP Saw:
                let mut raw_saw = 2.0 * phase - 1.0;
                raw_saw -= poly_blep(phase, dt);
                let saw = raw_saw;

                // Anti-aliased PolyBLEP Pulse:
                let mut raw_pulse = if phase < pw { 1.0 } else { -1.0 };
                raw_pulse += poly_blep(phase, dt);
                let phase_pw = if phase >= pw { phase - pw } else { phase - pw + 1.0 };
                raw_pulse -= poly_blep(phase_pw, dt);
                let pulse = raw_pulse;

                // Continuous Triangle (integrated square or polyBLEP triangle)
                let tri = 2.0 * (if phase < 0.5 { 2.0 * phase } else { 2.0 - 2.0 * phase }) - 1.0;

                // Pure Sine
                let sine = sin(phase * TWO_PI);

                outputs[0].set_cv(ch, f, saw * 5.0);
                outputs[1].set_cv(ch, f, pulse * 5.0);
                outputs[2].set_cv(ch, f, tri * 5.0);
                outputs[3].set_cv(ch, f, sine * 5.0);

                if ch == 0 {
                    outputs[0].set_audio_mono(f, saw);
                    outputs[1].set_audio_mono(f, pulse);
                    outputs[2].set_audio_mono(f, tri);
                    outputs[3].set_audio_mono(f, sine);
                }

                phase += dt;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
            }

            self.phases[ch] = phase;
        }
    }
}

pub struct Fm4OpMacro {
    ratios: [f32; 4],
    indices: [f32; 3], // Op4->Op3, Op3->Op2, Op2->Op1
    feedback: f32,     // Op4 self feedback
    phases: [f32; 4],
    last_op4: f32,
}

impl Default for Fm4OpMacro {
    fn default() -> Self {
        Self::new()
    }
}

impl Fm4OpMacro {
    pub const fn new() -> Self {
        Self {
            ratios: [1.0, 1.0, 2.0, 3.0],
            indices: [2.0, 1.5, 1.0],
            feedback: 0.2,
            phases: [0.0; 4],
            last_op4: 0.0,
        }
    }
}

const FM_INPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Pitch CV (1V/oct)",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 1,
        name: "Index CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Ratio CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Feedback CV",
        kind: PortKind::ModCv,
    },
];

const FM_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for Fm4OpMacro {
    fn kind(&self) -> ModuleKind {
        ModuleKind::Fm4OpMacro
    }

    fn inputs(&self) -> &[PortDesc] {
        &FM_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &FM_OUTPUTS
    }

    fn param_count(&self) -> usize {
        4
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.ratios[1],
            1 => self.ratios[2],
            2 => self.indices[0],
            3 => self.feedback,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.ratios[1] = value.clamp(0.25, 16.0),
            1 => self.ratios[2] = value.clamp(0.25, 16.0),
            2 => self.indices[0] = value.clamp(0.0, 10.0),
            3 => self.feedback = value.clamp(0.0, 0.99),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.phases = [0.0; 4];
        self.last_op4 = 0.0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() {
            return;
        }

        for f in 0..BLOCK_FRAMES {
            let pitch_cv = if !inputs.is_empty() {
                inputs[0].get_cv(0, f)
            } else {
                0.0
            };
            let idx_mod = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 0.2
            } else {
                0.0
            };
            let ratio_mod = if inputs.len() > 2 {
                inputs[2].get_cv(0, f) * 0.1
            } else {
                0.0
            };
            let fb_mod = if inputs.len() > 3 {
                inputs[3].get_cv(0, f) * 0.05
            } else {
                0.0
            };

            let base_freq = cv_to_freq(pitch_cv);
            let fb = (self.feedback + fb_mod).clamp(0.0, 0.95);
            let idx_scale = (1.0 + idx_mod).clamp(0.0, 5.0);

            // Cascade: Op4 (w/ feedback) -> Op3 -> Op2 -> Op1 (Carrier)
            let dt4 = (base_freq * (self.ratios[3] + ratio_mod).max(0.1)) / (RATE as f32);
            self.phases[3] += dt4;
            if self.phases[3] >= 1.0 {
                self.phases[3] -= 1.0;
            }
            let op4_val = sin(self.phases[3] * TWO_PI + self.last_op4 * fb * TWO_PI);
            self.last_op4 = op4_val;

            let dt3 = (base_freq * self.ratios[2].max(0.1)) / (RATE as f32);
            self.phases[2] += dt3;
            if self.phases[2] >= 1.0 {
                self.phases[2] -= 1.0;
            }
            let op3_val = sin(self.phases[2] * TWO_PI + op4_val * self.indices[2] * idx_scale);

            let dt2 = (base_freq * self.ratios[1].max(0.1)) / (RATE as f32);
            self.phases[1] += dt2;
            if self.phases[1] >= 1.0 {
                self.phases[1] -= 1.0;
            }
            let op2_val = sin(self.phases[1] * TWO_PI + op3_val * self.indices[1] * idx_scale);

            let dt1 = (base_freq * self.ratios[0]) / (RATE as f32);
            self.phases[0] += dt1;
            if self.phases[0] >= 1.0 {
                self.phases[0] -= 1.0;
            }
            let carrier = sin(self.phases[0] * TWO_PI + op2_val * self.indices[0] * idx_scale);

            outputs[0].set_audio_mono(f, carrier * 0.8);
            outputs[0].set_cv(0, f, carrier * 5.0);
        }
    }
}
