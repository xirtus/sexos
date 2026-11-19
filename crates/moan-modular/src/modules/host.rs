// SPDX-License-Identifier: GPL-3.0-only
//! BidulePluginHost module for EuropaRack.

use crate::math::soft_clip;
use crate::modules::{Module, ModuleKind, PortDesc};
use crate::signal::{PortKind, SignalBuffer, BLOCK_FRAMES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PluginFormat {
    Clap = 0,
    Vst3 = 1,
}

pub struct BidulePluginHost {
    plugin_path: [u8; 64],
    plugin_path_len: usize,
    format: PluginFormat,
    params: [f32; 8], // 0.0..1.0
    wet_dry: f32,     // 0.0..1.0
    bypass: bool,
}

impl Default for BidulePluginHost {
    fn default() -> Self {
        Self::new()
    }
}

impl BidulePluginHost {
    pub const fn new() -> Self {
        Self {
            plugin_path: [0; 64],
            plugin_path_len: 0,
            format: PluginFormat::Clap,
            params: [0.5; 8],
            wet_dry: 1.0,
            bypass: false,
        }
    }

    pub fn set_plugin_path(&mut self, path: &str, format: PluginFormat) {
        let bytes = path.as_bytes();
        let len = bytes.len().min(64);
        self.plugin_path[..len].copy_from_slice(&bytes[..len]);
        self.plugin_path_len = len;
        self.format = format;
    }

    pub fn plugin_path_str(&self) -> &str {
        core::str::from_utf8(&self.plugin_path[..self.plugin_path_len]).unwrap_or("")
    }

    pub fn format(&self) -> PluginFormat {
        self.format
    }
}

const HOST_INPUTS: [PortDesc; 9] = [
    PortDesc {
        id: 0,
        name: "Audio In",
        kind: PortKind::Audio,
    },
    PortDesc {
        id: 1,
        name: "Param 1 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 2,
        name: "Param 2 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 3,
        name: "Param 3 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 4,
        name: "Param 4 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 5,
        name: "Param 5 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 6,
        name: "Param 6 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 7,
        name: "Param 7 CV",
        kind: PortKind::ModCv,
    },
    PortDesc {
        id: 8,
        name: "Param 8 CV",
        kind: PortKind::ModCv,
    },
];

const HOST_OUTPUTS: [PortDesc; 1] = [PortDesc {
    id: 0,
    name: "Audio Out",
    kind: PortKind::Audio,
}];

impl Module for BidulePluginHost {
    fn kind(&self) -> ModuleKind {
        ModuleKind::BidulePluginHost
    }

    fn inputs(&self) -> &[PortDesc] {
        &HOST_INPUTS
    }

    fn outputs(&self) -> &[PortDesc] {
        &HOST_OUTPUTS
    }

    fn param_count(&self) -> usize {
        10 // 8 params + wet_dry + bypass
    }

    fn get_param(&self, index: usize) -> f32 {
        match index {
            0..=7 => self.params[index],
            8 => self.wet_dry,
            9 => {
                if self.bypass {
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
            0..=7 => self.params[index] = value.clamp(0.0, 1.0),
            8 => self.wet_dry = value.clamp(0.0, 1.0),
            9 => self.bypass = value >= 0.5,
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

            if self.bypass {
                outputs[0].set_audio(f, in_audio);
                outputs[0].set_cv(0, f, in_audio[0] * 5.0);
                continue;
            }

            // Staged processor emulation:
            // Param 0 = Gain (0..2.0)
            // Param 1 = Pan (-1..+1)
            // Param 2 = Drive
            let p0_mod = if inputs.len() > 1 {
                inputs[1].get_cv(0, f) * 0.1
            } else {
                0.0
            };
            let p1_mod = if inputs.len() > 2 {
                inputs[2].get_cv(0, f) * 0.1
            } else {
                0.0
            };
            let p2_mod = if inputs.len() > 3 {
                inputs[3].get_cv(0, f) * 0.1
            } else {
                0.0
            };

            let gain = (self.params[0] * 2.0 + p0_mod).clamp(0.0, 4.0);
            let pan = ((self.params[1] - 0.5) * 2.0 + p1_mod).clamp(-1.0, 1.0);
            let drive = (self.params[2] * 3.0 + 1.0 + p2_mod).clamp(1.0, 5.0);

            let pan_l = (1.0 - pan.max(0.0)).clamp(0.0, 1.0);
            let pan_r = (1.0 + pan.min(0.0)).clamp(0.0, 1.0);

            let proc_l = soft_clip(in_audio[0] * gain * pan_l * drive);
            let proc_r = soft_clip(in_audio[1] * gain * pan_r * drive);

            let out_l = in_audio[0] * (1.0 - self.wet_dry) + proc_l * self.wet_dry;
            let out_r = in_audio[1] * (1.0 - self.wet_dry) + proc_r * self.wet_dry;

            outputs[0].set_audio(f, [out_l, out_r]);
            outputs[0].set_cv(0, f, out_l * 5.0);
        }
    }
}
