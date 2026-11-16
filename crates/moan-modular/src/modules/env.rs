// SPDX-License-Identifier: GPL-3.0-only
//! PolyAdsr and TempoLfo modules for EuropaRack.

use crate::math::{sin, TWO_PI};
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, MAX_POLY, RATE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy, Debug)]
struct VoiceEnv {
    stage: EnvStage,
    value: f32,
    last_gate: f32,
}

impl VoiceEnv {
    const fn new() -> Self {
        Self {
            stage: EnvStage::Idle,
            value: 0.0,
            last_gate: 0.0,
        }
    }
}

pub struct PolyAdsr {
    attack_ms: f32,
    decay_ms: f32,
    sustain_lvl: f32,
    release_ms: f32,
    voices: [VoiceEnv; MAX_POLY],
}

impl Default for PolyAdsr {
    fn default() -> Self {
        Self::new()
    }
}

impl PolyAdsr {
    pub const fn new() -> Self {
        Self {
            attack_ms: 20.0,
            decay_ms: 200.0,
            sustain_lvl: 0.5,
            release_ms: 300.0,
            voices: [VoiceEnv::new(); MAX_POLY],
        }
    }
}

const ADSR_INPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Gate (10V)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Retrigger",
        kind: PortKind::Gate,
    },
];

const ADSR_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Envelope Out (0..10V)",
    kind: PortKind::ModCv,
}];

impl Module for PolyAdsr {
    fn kind(&self) -> ModuleKind {
        ModuleKind::PolyAdsr
    }

    fn inputs(&self) -> &[PortDesc] {
        &ADSR_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &ADSR_OUTPUTS
    }

    fn param_count(&self) -> usize {
        4
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => self.attack_ms,
            1 => self.decay_ms,
            2 => self.sustain_lvl,
            3 => self.release_ms,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.attack_ms = value.clamp(1.0, 10_000.0),
            1 => self.decay_ms = value.clamp(1.0, 10_000.0),
            2 => self.sustain_lvl = value.clamp(0.0, 1.0),
            3 => self.release_ms = value.clamp(1.0, 20_000.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.voices = [VoiceEnv::new(); MAX_POLY];
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

        let attack_rate = 1000.0 / (self.attack_ms * (RATE as f32));
        let decay_rate = 1000.0 / (self.decay_ms * (RATE as f32));
        let release_rate = 1000.0 / (self.release_ms * (RATE as f32));

        for ch in 0..poly_ch {
            let v = &mut self.voices[ch];

            for f in 0..BLOCK_FRAMES {
                let gate_in = if !inputs.is_empty() {
                    inputs[0].get_cv(ch, f)
                } else {
                    0.0
                };
                let retrig = if inputs.len() > 1 {
                    inputs[1].get_cv(ch, f)
                } else {
                    0.0
                };

                let gate_active = gate_in >= 2.5;
                let gate_rose = gate_active && v.last_gate < 2.5;
                let retrig_rose = retrig >= 2.5;

                if gate_rose || retrig_rose {
                    v.stage = EnvStage::Attack;
                } else if !gate_active && v.last_gate >= 2.5 {
                    v.stage = EnvStage::Release;
                }
                v.last_gate = gate_in;

                match v.stage {
                    EnvStage::Idle => {
                        v.value = 0.0;
                    }
                    EnvStage::Attack => {
                        v.value += attack_rate;
                        if v.value >= 1.0 {
                            v.value = 1.0;
                            v.stage = EnvStage::Decay;
                        }
                    }
                    EnvStage::Decay => {
                        v.value -= decay_rate;
                        if v.value <= self.sustain_lvl {
                            v.value = self.sustain_lvl;
                            v.stage = EnvStage::Sustain;
                        }
                    }
                    EnvStage::Sustain => {
                        v.value = self.sustain_lvl;
                        if !gate_active {
                            v.stage = EnvStage::Release;
                        }
                    }
                    EnvStage::Release => {
                        v.value -= release_rate;
                        if v.value <= 0.0001 {
                            v.value = 0.0;
                            v.stage = EnvStage::Idle;
                        }
                    }
                }

                outputs[0].set_cv(ch, f, v.value * 10.0);
            }
        }
    }
}

pub struct TempoLfo {
    sync_mode: bool,
    rate: f32, // Hz (free) or beats (sync)
    bpm: f32,
    phase: f32,
    noise_seed: u32,
    last_sh: f32,
    last_reset: f32,
}

impl Default for TempoLfo {
    fn default() -> Self {
        Self::new()
    }
}

impl TempoLfo {
    pub const fn new() -> Self {
        Self {
            sync_mode: false,
            rate: 2.0,
            bpm: 120.0,
            phase: 0.0,
            noise_seed: 0xDEAD_BEEF,
            last_sh: 0.0,
            last_reset: 0.0,
        }
    }

    fn prng(&mut self) -> f32 {
        self.noise_seed ^= self.noise_seed << 13;
        self.noise_seed ^= self.noise_seed >> 17;
        self.noise_seed ^= self.noise_seed << 5;
        ((self.noise_seed as i32 as f32) / 2147483648.0).clamp(-1.0, 1.0)
    }
}

const LFO_INPUTS: [PortDesc; 2] = [
    PortDesc {
        id: 0,
        name: "Sync Reset",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 1,
        name: "Rate CV",
        kind: PortKind::ModCv,
    },
];

const LFO_OUTPUTS: [PortDesc; 5] = [
    PortDesc {
        id: 0,
        name: "Sine Out",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 1,
        name: "Triangle Out",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Saw Out",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Square Out",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 4,
        name: "S&H Out",
        kind: PortKind::ModCv,
    },
];

impl Module for TempoLfo {
    fn kind(&self) -> ModuleKind {
        ModuleKind::TempoLfo
    }

    fn inputs(&self) -> &[PortDesc] {
        &LFO_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &LFO_OUTPUTS
    }

    fn param_count(&self) -> usize {
        3
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0 => {
                if self.sync_mode {
                    1.0
                } else {
                    0.0
                }
            }
            1 => self.rate,
            2 => self.bpm,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.sync_mode = value >= 0.5,
            1 => self.rate = value.clamp(0.01, 50.0),
            2 => self.bpm = value.clamp(30.0, 300.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.last_sh = 0.0;
        self.last_reset = 0.0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 5 {
            return;
        }

        for f in 0..BLOCK_FRAMES {
            let reset_cv = if !inputs.is_empty() {
                inputs[0].get_cv(0, f)
            } else {
                0.0
            };
            let rate_cv = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 0.5
            } else {
                0.0
            };

            if reset_cv >= 2.5 && self.last_reset < 2.5 {
                self.phase = 0.0;
            }
            self.last_reset = reset_cv;

            let base_freq = if self.sync_mode {
                (self.bpm / 60.0) * self.rate
            } else {
                self.rate
            };

            let freq = (base_freq * (1.0 + rate_cv)).clamp(0.001, 100.0);
            let dt = freq / (RATE as f32);

            let sine = sin(self.phase * TWO_PI) * 5.0;
            let tri = (2.0 * (if self.phase < 0.5 { 2.0 * self.phase } else { 2.0 - 2.0 * self.phase }) - 1.0) * 5.0;
            let saw = (2.0 * self.phase - 1.0) * 5.0;
            let square = (if self.phase < 0.5 { 1.0 } else { -1.0 }) * 5.0;

            outputs[0].set_cv(0, f, sine);
            outputs[1].set_cv(0, f, tri);
            outputs[2].set_cv(0, f, saw);
            outputs[3].set_cv(0, f, square);
            outputs[4].set_cv(0, f, self.last_sh * 5.0);

            let old_phase = self.phase;
            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                // Update S&H random step
                self.last_sh = self.prng();
            } else if old_phase < 0.5 && self.phase >= 0.5 {
                self.last_sh = self.prng();
            }
        }
    }
}
