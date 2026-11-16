// SPDX-License-Identifier: GPL-3.0-only
//! PolyMidiToCv module for EuropaRack.

use crate::math::midi_to_cv;
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES, MAX_POLY};

#[derive(Clone, Copy, Debug)]
struct Voice {
    active: bool,
    key: u8,
    velocity: u8,
    age: u32,
    pitch_cv: f32,
    gate: f32,
    retrigger_countdown: u8,
}

impl Voice {
    const fn new() -> Self {
        Self {
            active: false,
            key: 60,
            velocity: 0,
            age: 0,
            pitch_cv: 0.0,
            gate: 0.0,
            retrigger_countdown: 0,
        }
    }
}

pub struct PolyMidiToCv {
    polyphony: u8, // 1..=16
    voices: [Voice; MAX_POLY],
    age_counter: u32,
}

impl Default for PolyMidiToCv {
    fn default() -> Self {
        Self::new()
    }
}

impl PolyMidiToCv {
    pub const fn new() -> Self {
        Self {
            polyphony: 16,
            voices: [Voice::new(); MAX_POLY],
            age_counter: 0,
        }
    }

    fn allocate_voice(&mut self, key: u8, velocity: u8) -> usize {
        let max_v = (self.polyphony as usize).min(MAX_POLY);
        // Check if note is already playing on an active voice (retrigger)
        for i in 0..max_v {
            if self.voices[i].active && self.voices[i].key == key {
                self.age_counter = self.age_counter.wrapping_add(1);
                self.voices[i].velocity = velocity;
                self.voices[i].age = self.age_counter;
                self.voices[i].pitch_cv = midi_to_cv(key);
                self.voices[i].gate = 10.0;
                self.voices[i].retrigger_countdown = 6;
                return i;
            }
        }

        // Look for an inactive voice
        for i in 0..max_v {
            if !self.voices[i].active {
                self.age_counter = self.age_counter.wrapping_add(1);
                self.voices[i].active = true;
                self.voices[i].key = key;
                self.voices[i].velocity = velocity;
                self.voices[i].age = self.age_counter;
                self.voices[i].pitch_cv = midi_to_cv(key);
                self.voices[i].gate = 10.0;
                self.voices[i].retrigger_countdown = 6;
                return i;
            }
        }

        // Steal the oldest voice
        let mut oldest_idx = 0;
        let mut oldest_age = u32::MAX;
        for i in 0..max_v {
            if self.voices[i].age < oldest_age {
                oldest_age = self.voices[i].age;
                oldest_idx = i;
            }
        }

        self.age_counter = self.age_counter.wrapping_add(1);
        self.voices[oldest_idx].active = true;
        self.voices[oldest_idx].key = key;
        self.voices[oldest_idx].velocity = velocity;
        self.voices[oldest_idx].age = self.age_counter;
        self.voices[oldest_idx].pitch_cv = midi_to_cv(key);
        self.voices[oldest_idx].gate = 10.0;
        self.voices[oldest_idx].retrigger_countdown = 6;
        oldest_idx
    }

    fn release_voice(&mut self, key: u8) {
        let max_v = (self.polyphony as usize).min(MAX_POLY);
        for i in 0..max_v {
            if self.voices[i].active && self.voices[i].key == key {
                self.voices[i].active = false;
                self.voices[i].gate = 0.0;
            }
        }
    }
}

const MIDI_INPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "MIDI In",
    kind: PortKind::Midi,
}];

const MIDI_OUTPUTS: [PortDesc; 4] = [
    PortDesc {
        id: 0,
        name: "Pitch CV (1V/oct)",
        kind: PortKind::PitchCv,
    },
    PortDesc {
        id: 1,
        name: "Gate (10V)",
        kind: PortKind::Gate,
    },
    PortDesc {
        id: 2,
        name: "Velocity CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Retrigger",
        kind: PortKind::Gate,
    },
];

impl Module for PolyMidiToCv {
    fn kind(&self) -> ModuleKind {
        ModuleKind::PolyMidiToCv
    }

    fn inputs(&self) -> &[PortDesc] {
        &MIDI_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &MIDI_OUTPUTS
    }

    fn param_count(&self) -> usize {
        1
    }

    fn get_param(&self, index: usize) -> f32 {
        if index == 0 {
            self.polyphony as f32
        } else {
            0.0
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.polyphony = (value.clamp(1.0, 16.0) as u8).max(1);
        }
    }

    fn reset(&mut self) {
        self.voices = [Voice::new(); MAX_POLY];
        self.age_counter = 0;
    }

    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]) {
        if outputs.len() < 4 {
            return;
        }

        // Process incoming MIDI events
        if !inputs.is_empty() {
            for event in inputs[0].midi.events() {
                if event.is_note_on && event.velocity > 0 {
                    self.allocate_voice(event.key, event.velocity);
                } else {
                    self.release_voice(event.key);
                }
            }
        }

        let max_v = (self.polyphony as usize).min(MAX_POLY);
        for out in outputs.iter_mut() {
            out.poly_channels = self.polyphony;
        }

        for ch in 0..max_v {
            let voice = &mut self.voices[ch];
            let pitch = voice.pitch_cv;
            let gate = voice.gate;
            let vel_cv = (voice.velocity as f32 / 127.0) * 10.0;

            for f in 0..BLOCK_FRAMES {
                outputs[0].poly[ch][f] = pitch;
                outputs[1].poly[ch][f] = gate;
                outputs[2].poly[ch][f] = vel_cv;

                let retrig = if voice.retrigger_countdown > 0 {
                    voice.retrigger_countdown -= 1;
                    10.0
                } else {
                    0.0
                };
                outputs[3].poly[ch][f] = retrig;
            }
        }
    }
}
