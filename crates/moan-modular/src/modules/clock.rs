// SPDX-License-Identifier: GPL-3.0-only
//! ClockDivider and EuclideanSeq modules for EuropaRack.

use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, RATE};

pub struct ClockDivider {
    bpm: f32,
    internal: bool,
    phase: f32,
    pulse_width: f32,
    last_external_clock: f32,
    last_reset: f32,
    step_count: u32,
}

impl Default for ClockDivider {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockDivider {
    pub const fn new() -> Self {
        Self {
            bpm: 120.0,
            internal: true,
            phase: 0.0,
            pulse_width: 0.5,
            last_external_clock: 0.0,
            last_reset: 0.0,
            step_count: 0,
        }
    }
}

const CLOCK_INPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Clock In",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Reset In",
        kind: PortKind::Gate,
    },
];

const CLOCK_OUTPUTS: [PortDesc; 6] = [
    PortDesc {
        id: 0,
        name: "Clock (/1)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Div 2 (/2)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 2,
        name: "Div 4 (/4)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 3,
        name: "Div 8 (/8)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 4,
        name: "Mult 2 (*2)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 5,
        name: "Mult 4 (*4)",
        kind: PortKind::Gate,
    },
];

impl Module for ClockDivider {
    fn kind(&self) -> ModuleKind {
        ModuleKind::ClockDivider
    }

    fn inputs(&self) -> &[PortDesc] {
        &CLOCK_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &CLOCK_OUTPUTS
    }

    fn param_count(&self) -> usize {
        3
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.bpm,
            1 => {
                if self.internal {
                    1.0
                } else {
                    0.0
                }
            }
            2 => self.pulse_width,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.bpm = value.clamp(30.0, 300.0),
            1 => self.internal = value >= 0.5,
            2 => self.pulse_width = value.clamp(0.05, 0.95),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.step_count = 0;
        self.last_external_clock = 0.0;
        self.last_reset = 0.0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 6 {
            return;
        }

        let dt = (self.bpm / 60.0) / (RATE as f32);

        for f in 0..BLOCK_FRAMES {
            let clk_in = if !inputs.is_empty() {
                inputs[0].get_cv(0, f)
            } else {
                0.0
            };
            let reset_in = if inputs.len() > 1 {
                inputs[1].get_cv(0, f)
            } else {
                0.0
            };

            if reset_in >= 2.5 && self.last_reset < 2.5 {
                self.phase = 0.0;
                self.step_count = 0;
            }
            self.last_reset = reset_in;

            let mut pulse_rose = false;

            if self.internal {
                let old_phase = self.phase;
                self.phase += dt;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                    pulse_rose = true;
                } else if old_phase < 0.01 && self.phase >= 0.01 {
                    pulse_rose = true;
                }
            } else {
                if clk_in >= 2.5 && self.last_external_clock < 2.5 {
                    pulse_rose = true;
                    self.phase = 0.0;
                }
                self.last_external_clock = clk_in;
            }

            if pulse_rose {
                self.step_count = self.step_count.wrapping_add(1);
            }

            let g1 = if self.phase < self.pulse_width { 10.0 } else { 0.0 };

            // Divisions (/2, /4, /8)
            let g2 = if (self.step_count / 2) % 2 == 0 { 10.0 } else { 0.0 };
            let g4 = if (self.step_count / 4) % 2 == 0 { 10.0 } else { 0.0 };
            let g8 = if (self.step_count / 8) % 2 == 0 { 10.0 } else { 0.0 };

            // Multiplications (*2, *4)
            let p2 = (self.phase * 2.0) - (self.phase * 2.0) as i32 as f32;
            let gm2 = if p2 < self.pulse_width { 10.0 } else { 0.0 };

            let p4 = (self.phase * 4.0) - (self.phase * 4.0) as i32 as f32;
            let gm4 = if p4 < self.pulse_width { 10.0 } else { 0.0 };

            outputs[0].set_cv(0, f, g1);
            outputs[1].set_cv(0, f, g2);
            outputs[2].set_cv(0, f, g4);
            outputs[3].set_cv(0, f, g8);
            outputs[4].set_cv(0, f, gm2);
            outputs[5].set_cv(0, f, gm4);
        }
    }
}

pub struct EuclideanSeq {
    steps: u8,   // 1..16
    pulses: u8,  // 0..16
    rotation: u8, // 0..15
    pitches: [f32; 16], // 1V/oct offsets
    current_step: u8,
    last_clock: f32,
    last_reset: f32,
    cached_pattern: [bool; 16],
}

impl Default for EuclideanSeq {
    fn default() -> Self {
        Self::new()
    }
}

impl EuclideanSeq {
    pub const fn new() -> Self {
        Self {
            steps: 16,
            pulses: 4,
            rotation: 0,
            pitches: [
                0.0, 0.0, 0.25, 0.0,
                0.5, 0.25, 0.0, 0.75,
                0.0, -0.25, 0.0, 0.25,
                0.5, 0.75, 1.0, 0.0,
            ],
            current_step: 0,
            last_clock: 0.0,
            last_reset: 0.0,
            cached_pattern: [
                true, false, false, false,
                true, false, false, false,
                true, false, false, false,
                true, false, false, false,
            ],
        }
    }

    /// Bjorklund Euclidean distribution algorithm
    pub fn rebuild_pattern(&mut self) {
        let n = self.steps.clamp(1, 16) as usize;
        let k = (self.pulses as usize).min(n);

        let mut pattern = [false; 16];
        if k > 0 {
            for i in 0..n {
                // Bresenham / Bjorklund equivalence: (i * k) % n < k
                let idx = (i + self.rotation as usize) % n;
                pattern[idx] = (i * k) % n < k;
            }
        }
        self.cached_pattern = pattern;
    }
}

const EUCLID_INPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Clock In",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Reset In",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 2,
        name: "Steps CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Pulses CV",
        kind: PortKind::ModCv,
    },
];

const EUCLID_OUTPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Gate Out",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Pitch CV",
        kind: PortKind::PitchCv,
    },
];

impl Module for EuclideanSeq {
    fn kind(&self) -> ModuleKind {
        ModuleKind::EuclideanSeq
    }

    fn inputs(&self) -> &[PortDesc] {
        &EUCLID_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &EUCLID_OUTPUTS
    }

    fn param_count(&self) -> usize {
        19 // steps, pulses, rotation, 16 step pitches
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.steps as f32,
            1 => self.pulses as f32,
            2 => self.rotation as f32,
            3..=18 => self.pitches[index - 3],
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => {
                self.steps = value.clamp(1.0, 16.0) as u8;
                self.rebuild_pattern();
            }
            1 => {
                self.pulses = value.clamp(0.0, 16.0) as u8;
                self.rebuild_pattern();
            }
            2 => {
                self.rotation = value.clamp(0.0, 15.0) as u8;
                self.rebuild_pattern();
            }
            3..=18 => {
                self.pitches[index - 3] = value.clamp(-3.0, 3.0);
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.current_step = 0;
        self.last_clock = 0.0;
        self.last_reset = 0.0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 2 {
            return;
        }

        let n = self.steps.clamp(1, 16) as usize;

        for f in 0..BLOCK_FRAMES {
            let clk_in = if !inputs.is_empty() {
                inputs[0].get_cv(0, f)
            } else {
                0.0
            };
            let reset_in = if inputs.len() > 1 {
                inputs[1].get_cv(0, f)
            } else {
                0.0
            };

            if reset_in >= 2.5 && self.last_reset < 2.5 {
                self.current_step = 0;
            }
            self.last_reset = reset_in;

            if clk_in >= 2.5 && self.last_clock < 2.5 {
                self.current_step = ((self.current_step as usize + 1) % n) as u8;
            }
            self.last_clock = clk_in;

            let is_pulse = self.cached_pattern[self.current_step as usize % 16];
            let gate_val = if is_pulse && clk_in >= 2.5 { 10.0 } else { 0.0 };
            let pitch_val = self.pitches[self.current_step as usize % 16];

            outputs[0].set_cv(0, f, gate_val);
            outputs[1].set_cv(0, f, pitch_val);
        }
    }
}
