// SPDX-License-Identifier: GPL-3.0-only
//! ArielFormantCv: Klattsch / Ariel Eurorack Vocal Module.

use crate::math::{cos, cv_to_freq, sin, soft_clip, TWO_PI};
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, RATE};

#[derive(Clone, Copy, Debug)]
struct Formants {
    f1: f32,
    f2: f32,
    f3: f32,
}

// Formants for AA, AE, IY, UW, OW
const VOWELS: [Formants; 5] = [
    Formants {
        f1: 730.0,
        f2: 1090.0,
        f3: 2440.0,
    }, // AA
    Formants {
        f1: 660.0,
        f2: 1720.0,
        f3: 2410.0,
    }, // AE
    Formants {
        f1: 270.0,
        f2: 2290.0,
        f3: 3010.0,
    }, // IY
    Formants {
        f1: 300.0,
        f2: 870.0,
        f3: 2240.0,
    }, // UW
    Formants {
        f1: 570.0,
        f2: 840.0,
        f3: 2410.0,
    }, // OW
];

pub struct ArielFormantCv {
    phase: f32,
    noise_seed: u32,
    step_idx: u8,
    last_trigger: f32,
    // Resonator states: [y1, y2] for 3 formants
    resonator_state: [[f32; 2]; 3],
    vowel_x: f32,
    vowel_y: f32,
    effort: f32,
    breath: f32,
}

impl Default for ArielFormantCv {
    fn default() -> Self {
        Self::new()
    }
}

impl ArielFormantCv {
    pub const fn new() -> Self {
        Self {
            phase: 0.0,
            noise_seed: 0x51A7_CAFE,
            step_idx: 0,
            last_trigger: 0.0,
            resonator_state: [[0.0; 2]; 3],
            vowel_x: 0.0,
            vowel_y: 0.0,
            effort: 0.5,
            breath: 0.1,
        }
    }

    fn prng(&mut self) -> f32 {
        self.noise_seed ^= self.noise_seed << 13;
        self.noise_seed ^= self.noise_seed >> 17;
        self.noise_seed ^= self.noise_seed << 5;
        ((self.noise_seed as i32 as f32) / 2147483648.0).clamp(-1.0, 1.0)
    }

    fn current_formants(&self, x: f32, y: f32, use_step: bool) -> Formants {
        if use_step {
            return VOWELS[(self.step_idx as usize) % 5];
        }

        // Map X/Y in [-1.0, 1.0] across vowel space
        let u = (x * 0.5 + 0.5).clamp(0.0, 0.999);
        let v = (y * 0.5 + 0.5).clamp(0.0, 0.999);

        let idx1 = (u * 4.0) as usize;
        let frac = (u * 4.0) - (idx1 as f32);
        let idx2 = (idx1 + 1).min(4);

        let base_f1 = VOWELS[idx1].f1 * (1.0 - frac) + VOWELS[idx2].f1 * frac;
        let base_f2 = VOWELS[idx1].f2 * (1.0 - frac) + VOWELS[idx2].f2 * frac;
        let base_f3 = VOWELS[idx1].f3 * (1.0 - frac) + VOWELS[idx2].f3 * frac;

        let y_scale = 1.0 + (v - 0.5) * 0.3;

        Formants {
            f1: (base_f1 * y_scale).clamp(150.0, 1200.0),
            f2: (base_f2 * y_scale).clamp(500.0, 3500.0),
            f3: (base_f3 * y_scale).clamp(1800.0, 4200.0),
        }
    }
}

const ARIEL_INPUTS: [PortDesc; 6] = [
    PortDesc {
        id: 0,
        name: "Pitch CV (1V/oct)",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 1,
        name: "Vowel X",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Vowel Y",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Effort CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 4,
        name: "Breath CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 5,
        name: "Phoneme Step Trig",
        kind: PortKind::Gate,
    },
];

const ARIEL_OUTPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Audio Out",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Phoneme CV",
        kind: PortKind::ModCv,
    },
];

impl Module for ArielFormantCv {
    fn kind(&self) -> ModuleKind {
        ModuleKind::ArielFormantCv
    }

    fn inputs(&self) -> &[PortDesc] {
        &ARIEL_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &ARIEL_OUTPUTS
    }

    fn param_count(&self) -> usize {
        4
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.vowel_x,
            1 => self.vowel_y,
            2 => self.effort,
            3 => self.breath,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.vowel_x = value.clamp(-1.0, 1.0),
            1 => self.vowel_y = value.clamp(-1.0, 1.0),
            2 => self.effort = value.clamp(0.0, 1.0),
            3 => self.breath = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.step_idx = 0;
        self.last_trigger = 0.0;
        self.resonator_state = [[0.0; 2]; 3];
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.is_empty() {
            return;
        }

        let has_step_trig = inputs.len() > 5;

        for f in 0..BLOCK_FRAMES {
            let pitch_cv = if !inputs.is_empty() {
                inputs[0].get_cv(0, f)
            } else {
                0.0
            };
            let vx_cv = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 0.2
            } else {
                0.0
            };
            let vy_cv = if inputs.len() > 2 {
                inputs[2].get_cv(0, f) * 0.2
            } else {
                0.0
            };
            let effort_cv = if inputs.len() > 3 {
                inputs[3].get_cv(0, f) * 0.1
            } else {
                0.0
            };
            let breath_cv = if inputs.len() > 4 {
                inputs[4].get_cv(0, f) * 0.1
            } else {
                0.0
            };
            let trig_in = if has_step_trig {
                inputs[5].get_cv(0, f)
            } else {
                0.0
            };

            // Detect rising edge on step trigger
            if trig_in >= 2.5 && self.last_trigger < 2.5 {
                self.step_idx = (self.step_idx + 1) % 5;
            }
            self.last_trigger = trig_in;

            let x = (self.vowel_x + vx_cv).clamp(-1.0, 1.0);
            let y = (self.vowel_y + vy_cv).clamp(-1.0, 1.0);
            let effort = (self.effort + effort_cv).clamp(0.0, 1.0);
            let breath = (self.breath + breath_cv).clamp(0.0, 1.0);

            let formants = self.current_formants(x, y, has_step_trig && trig_in >= 0.1);

            let f0 = cv_to_freq(pitch_cv).clamp(40.0, 1000.0);
            let dt = f0 / (RATE as f32);

            // Glottal source pulse: Rosenberg-style asymmetric pulse
            let pulse = if self.phase < 0.4 {
                let p = self.phase / 0.4;
                sin(p * 3.14159265) * (0.8 + 0.4 * effort)
            } else if self.phase < 0.6 {
                let p = (self.phase - 0.4) / 0.2;
                cos(p * 1.5707963)
            } else {
                0.0
            };

            let noise = self.prng();
            let excitation = pulse * (1.0 - breath * 0.6) + noise * breath;

            // 3 Formant Resonators:
            let freqs = [formants.f1, formants.f2, formants.f3];
            let bandwidths = [
                80.0 + (1.0 - effort) * 60.0,
                110.0 + (1.0 - effort) * 80.0,
                150.0 + (1.0 - effort) * 100.0,
            ];
            let gains = [1.0, 0.6 + effort * 0.4, 0.3 + effort * 0.5];

            let mut vocal_out = 0.0f32;

            for r in 0..3 {
                let w = TWO_PI * freqs[r] / (RATE as f32);
                let bw = TWO_PI * bandwidths[r] / (RATE as f32);
                // Radius r_decay = exp(-bw/2) ~ 1 - bw/2
                let r_decay = (1.0 - bw * 0.5).clamp(0.8, 0.999);
                let c = 2.0 * r_decay * cos(w);
                let r2 = r_decay * r_decay;
                let g = (1.0 - r_decay) * gains[r];

                let y0 = c * self.resonator_state[r][0] - r2 * self.resonator_state[r][1]
                    + g * excitation;
                self.resonator_state[r][1] = self.resonator_state[r][0];
                self.resonator_state[r][0] = y0;

                vocal_out += y0;
            }

            let vocal_audio = soft_clip(vocal_out * 1.2);
            outputs[0].set_audio_mono(f, vocal_audio);
            outputs[0].set_cv(0, f, vocal_audio * 5.0);

            if outputs.len() > 1 {
                outputs[1].set_cv(0, f, self.step_idx as f32 * 2.0);
            }

            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}
