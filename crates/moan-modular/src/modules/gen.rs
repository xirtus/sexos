// SPDX-License-Identifier: GPL-3.0-only
//! TuringQuantizer and SampleAndHoldSlew modules for EuropaRack.

use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, RATE};

pub struct TuringQuantizer {
    lock_prob: f32, // 0.0..1.0
    scale: u8,      // 0 = Chromatic, 1 = Major, 2 = Minor, 3 = Pentatonic Minor, 4 = Dorian, 5 = Blues
    root: u8,       // 0..11 (C = 0, C# = 1, etc.)
    octaves: u8,    // 1..4
    shift_reg: u16,
    rng_state: u32,
    last_clock: f32,
    current_cv: f32,
    current_raw: f32,
}

impl Default for TuringQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

const SCALE_CHROMATIC: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const SCALE_MAJOR: &[u8] = &[0, 2, 4, 5, 7, 9, 11];
const SCALE_MINOR: &[u8] = &[0, 2, 3, 5, 7, 8, 10];
const SCALE_PENTATONIC_MINOR: &[u8] = &[0, 3, 5, 7, 10];
const SCALE_DORIAN: &[u8] = &[0, 2, 3, 5, 7, 9, 10];
const SCALE_BLUES: &[u8] = &[0, 3, 5, 6, 7, 10];

impl TuringQuantizer {
    pub const fn new() -> Self {
        Self {
            lock_prob: 0.6,
            scale: 2, // Minor
            root: 0,  // C
            octaves: 2,
            shift_reg: 0xACE1,
            rng_state: 0x1234_5678,
            last_clock: 0.0,
            current_cv: 0.0,
            current_raw: 0.0,
        }
    }

    fn prng(&mut self) -> f32 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 17;
        self.rng_state ^= self.rng_state << 5;
        (self.rng_state as f32) / (u32::MAX as f32)
    }

    fn quantize(&self, semitone: i32) -> f32 {
        let scale_notes = match self.scale {
            1 => SCALE_MAJOR,
            2 => SCALE_MINOR,
            3 => SCALE_PENTATONIC_MINOR,
            4 => SCALE_DORIAN,
            5 => SCALE_BLUES,
            _ => SCALE_CHROMATIC,
        };

        let octave = if semitone >= 0 {
            semitone / 12
        } else {
            (semitone - 11) / 12
        };
        let mut note_in_oct = semitone - octave * 12;
        if note_in_oct < 0 {
            note_in_oct += 12;
        }

        // Relative to root
        let rel_note = (note_in_oct - self.root as i32 + 12) % 12;

        // Find closest scale degree
        let mut closest = scale_notes[0] as i32;
        let mut min_dist = 999;
        for &s in scale_notes {
            let dist = (s as i32 - rel_note).abs();
            if dist < min_dist {
                min_dist = dist;
                closest = s as i32;
            }
        }

        let quantized_semitone = octave * 12 + self.root as i32 + closest;
        (quantized_semitone as f32) / 12.0
    }
}

const TURING_INPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Clock In",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Lock/Prob CV",
        kind: PortKind::ModCv,
    },
];

const TURING_OUTPUTS: [PortDesc; 3] = [
    PortDesc {
        id: 0,
        name: "Pitch CV (1V/oct)",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 1,
        name: "Raw CV (0..10V)",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Gate Out",
        kind: PortKind::Gate,
    },
];

impl Module for TuringQuantizer {
    fn kind(&self) -> ModuleKind {
        ModuleKind::TuringQuantizer
    }

    fn inputs(&self) -> &[PortDesc] {
        &TURING_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &TURING_OUTPUTS
    }

    fn param_count(&self) -> usize {
        4
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.lock_prob,
            1 => self.scale as f32,
            2 => self.root as f32,
            3 => self.octaves as f32,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.lock_prob = value.clamp(0.0, 1.0),
            1 => self.scale = value.clamp(0.0, 5.0) as u8,
            2 => self.root = value.clamp(0.0, 11.0) as u8,
            3 => self.octaves = value.clamp(1.0, 4.0) as u8,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.shift_reg = 0xACE1;
        self.last_clock = 0.0;
        self.current_cv = 0.0;
        self.current_raw = 0.0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 3 {
            return;
        }

        for f in 0..BLOCK_FRAMES {
            let clk_in = if !inputs.is_empty() {
                inputs[0].get_cv(0, f)
            } else {
                0.0
            };
            let prob_mod = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 0.1
            } else {
                0.0
            };

            let prob = (self.lock_prob + prob_mod).clamp(0.0, 1.0);

            // Clock rising edge
            if clk_in >= 2.5 && self.last_clock < 2.5 {
                let last_bit = (self.shift_reg >> 15) & 1;
                let rnd = self.prng();

                let new_bit = if prob >= 0.999 {
                    // 100% locked loop
                    last_bit
                } else if prob <= 0.001 {
                    // Inverted loop (sub-octave loop)
                    last_bit ^ 1
                } else if rnd < prob {
                    last_bit
                } else {
                    last_bit ^ 1
                };

                self.shift_reg = (self.shift_reg << 1) | new_bit;

                // Extract 8 bits DAC value
                let dac = (self.shift_reg & 0xFF) as f32 / 255.0;
                self.current_raw = dac * 10.0;

                // Scale to semitones
                let max_semitones = (self.octaves as i32) * 12;
                let semitone = (dac * max_semitones as f32) as i32;
                self.current_cv = self.quantize(semitone);
            }
            self.last_clock = clk_in;

            let gate = if clk_in >= 2.5 { 10.0 } else { 0.0 };

            outputs[0].set_cv(0, f, self.current_cv);
            outputs[1].set_cv(0, f, self.current_raw);
            outputs[2].set_cv(0, f, gate);
        }
    }
}

pub struct SampleAndHoldSlew {
    is_pink: bool,
    rise_ms: f32,
    fall_ms: f32,
    noise_seed: u32,
    b0: f32,
    b1: f32,
    b2: f32,
    sampled_val: f32,
    slewed_val: f32,
    last_trigger: f32,
}

impl Default for SampleAndHoldSlew {
    fn default() -> Self {
        Self::new()
    }
}

impl SampleAndHoldSlew {
    pub const fn new() -> Self {
        Self {
            is_pink: false,
            rise_ms: 10.0,
            fall_ms: 10.0,
            noise_seed: 0x1984_CAFE,
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            sampled_val: 0.0,
            slewed_val: 0.0,
            last_trigger: 0.0,
        }
    }

    fn next_noise(&mut self) -> f32 {
        self.noise_seed ^= self.noise_seed << 13;
        self.noise_seed ^= self.noise_seed >> 17;
        self.noise_seed ^= self.noise_seed << 5;
        let white = ((self.noise_seed as i32 as f32) / 2147483648.0).clamp(-1.0, 1.0);

        if self.is_pink {
            // Paul Kellet's filter for pink noise (-3dB/oct)
            self.b0 = 0.99886 * self.b0 + white * 0.0555179;
            self.b1 = 0.99332 * self.b1 + white * 0.0750759;
            self.b2 = 0.96900 * self.b2 + white * 0.1538520;
            (self.b0 + self.b1 + self.b2 + white * 0.5362) * 0.4
        } else {
            white
        }
    }
}

const SH_INPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Signal In",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 1,
        name: "Trigger In",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 2,
        name: "Rise Slew CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Fall Slew CV",
        kind: PortKind::ModCv,
    },
];

const SH_OUTPUTS: [PortDesc; 3] = [
    PortDesc {
        id: 0,
        name: "Noise Out",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 1,
        name: "Sampled Out",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Slewed Out",
        kind: PortKind::ModCv,
    },
];

impl Module for SampleAndHoldSlew {
    fn kind(&self) -> ModuleKind {
        ModuleKind::SampleAndHoldSlew
    }

    fn inputs(&self) -> &[PortDesc] {
        &SH_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &SH_OUTPUTS
    }

    fn param_count(&self) -> usize {
        3
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => {
                if self.is_pink {
                    1.0
                } else {
                    0.0
                }
            }
            1 => self.rise_ms,
            2 => self.fall_ms,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.is_pink = value >= 0.5,
            1 => self.rise_ms = value.clamp(0.1, 5000.0),
            2 => self.fall_ms = value.clamp(0.1, 5000.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.b0 = 0.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.sampled_val = 0.0;
        self.slewed_val = 0.0;
        self.last_trigger = 0.0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 3 {
            return;
        }

        let has_signal = !inputs.is_empty();
        let has_trigger = inputs.len() > 1;

        for f in 0..BLOCK_FRAMES {
            let noise = self.next_noise() * 5.0;

            let in_sig = if has_signal {
                inputs[0].get_cv(0, f)
            } else {
                noise
            };
            let trig_in = if has_trigger {
                inputs[1].get_cv(0, f)
            } else {
                10.0
            };
            let rise_mod = if inputs.len() > 2 {
                inputs[2].get_cv(0, f) * 10.0
            } else {
                0.0
            };
            let fall_mod = if inputs.len() > 3 {
                inputs[3].get_cv(0, f) * 10.0
            } else {
                0.0
            };

            // Sample on rising edge
            if trig_in >= 2.5 && self.last_trigger < 2.5 {
                self.sampled_val = in_sig;
            }
            self.last_trigger = trig_in;

            // Slew limiter calculation
            let r_ms = (self.rise_ms + rise_mod).clamp(0.1, 10_000.0);
            let f_ms = (self.fall_ms + fall_mod).clamp(0.1, 10_000.0);
            let rise_step = 1000.0 * 10.0 / (r_ms * RATE as f32);
            let fall_step = 1000.0 * 10.0 / (f_ms * RATE as f32);

            if self.sampled_val > self.slewed_val {
                self.slewed_val = (self.slewed_val + rise_step).min(self.sampled_val);
            } else if self.sampled_val < self.slewed_val {
                self.slewed_val = (self.slewed_val - fall_step).max(self.sampled_val);
            }

            outputs[0].set_cv(0, f, noise);
            outputs[1].set_cv(0, f, self.sampled_val);
            outputs[2].set_cv(0, f, self.slewed_val);
        }
    }
}
