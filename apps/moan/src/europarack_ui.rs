// SPDX-License-Identifier: GPL-3.0-only
//! EuropaRack (`moan-modular`) Weft Integration.
//!
//! Provides the visual modular rack canvas, catenary cable routing, VCV Dark Room
//! brightness control (0..100), 19-module catalog browser, 6 factory patches,
//! and direct baking to timeline Clip PCM buffers.

extern crate alloc;

use alloc::{vec, vec::Vec};
use moan_modular::{
    catalog::CATALOG,
    graph::{Error, PatchGraph},
    presets::FactoryPatch,
    signal::CableColor,
};
use weft::{Id, Layout, TextStyle, Ui};

pub const RACK_CANVAS_WIDTH: usize = 640;
pub const RACK_CANVAS_HEIGHT: usize = 200;

/// EuropaRack UI widget handles and view state.
pub struct EuropaRackUi {
    pub panel: Id,
    pub canvas: Id,
    pub patch_btn: Id,
    pub load_patch_btn: Id,
    pub module_btn: Id,
    pub add_mod_btn: Id,
    pub connect_btn: Id,
    pub clear_cables_btn: Id,
    pub dark_room_slider: Id,
    pub bake_btn: Id,

    pub selected_patch_idx: usize,
    pub selected_module_idx: usize,
}

impl EuropaRackUi {
    pub fn build(ui: &mut Ui, parent: Id) -> Self {
        let panel = ui.panel(
            parent,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );

        let header = ui.group(panel, Layout::row().gap(6.0).wrap().fill_width());
        ui.text(header, "EuropaRack · Modular Synth Patchbay", TextStyle::Heading);

        let patch_btn = ui.button(header, "Patch: Classic Subtractive Lead");
        let load_patch_btn = ui.button(header, "Load Preset");

        let module_btn = ui.button(header, "Module: AnalogVco");
        let add_mod_btn = ui.button(header, "Add Module");

        let connect_btn = ui.button(header, "Connect Cable");
        let clear_cables_btn = ui.button(header, "Clear Cables");

        let controls = ui.group(panel, Layout::row().gap(6.0).wrap().fill_width());
        ui.text(controls, "Dark Room", TextStyle::Caption);
        let dark_room_slider = ui.slider(controls, 0.0, 100.0, 85.0);
        ui.set_layout(dark_room_slider, Layout::default().width(120.0));
        ui.set_step(dark_room_slider, 1.0);

        let bake_btn = ui.button(controls, "Bake Rack to Track");

        // Initial default modular canvas
        let default_graph = PatchGraph::new();
        let (_, _, pixels) = Self::render_canvas(&default_graph);
        let canvas = ui.image(panel, RACK_CANVAS_WIDTH, RACK_CANVAS_HEIGHT, pixels);
        ui.set_label(
            canvas,
            "EuropaRack modular synth canvas with color-coded catenary patch cables",
        );

        Self {
            panel,
            canvas,
            patch_btn,
            load_patch_btn,
            module_btn,
            add_mod_btn,
            connect_btn,
            clear_cables_btn,
            dark_room_slider,
            bake_btn,

            selected_patch_idx: 0,
            selected_module_idx: 0,
        }
    }

    /// Cycle to the next factory preset.
    pub fn cycle_factory_patch(&mut self, ui: &mut Ui) -> &'static str {
        self.selected_patch_idx = (self.selected_patch_idx + 1) % FactoryPatch::ALL.len();
        let patch = FactoryPatch::ALL[self.selected_patch_idx];
        let name = patch.name();
        ui.set_text(self.patch_btn, &alloc::format!("Patch: {}", name));
        name
    }

    /// Load the currently selected factory preset into the patch graph.
    pub fn load_active_patch(&self, graph: &mut PatchGraph) -> Result<(), Error> {
        let patch = FactoryPatch::ALL[self.selected_patch_idx];
        *graph = patch.build()?;
        Ok(())
    }

    /// Cycle to the next module in the 19-module catalog.
    pub fn cycle_catalog_module(&mut self, ui: &mut Ui) -> &'static str {
        self.selected_module_idx = (self.selected_module_idx + 1) % CATALOG.len();
        let meta = &CATALOG[self.selected_module_idx];
        ui.set_text(self.module_btn, &alloc::format!("Module: {}", meta.name));
        meta.name
    }

    /// Add the currently selected module to the patch graph.
    pub fn add_active_module(&self, graph: &mut PatchGraph) -> Result<moan_modular::graph::NodeId, Error> {
        let meta = &CATALOG[self.selected_module_idx];
        let node_id = graph.add_module(meta.kind)?;
        // Auto-layout node across the rack row
        let idx = graph.nodes.len().saturating_sub(1);
        if let Some(node) = graph.nodes.get_mut(idx) {
            node.x = 10.0 + (idx as f32 * 95.0) % (RACK_CANVAS_WIDTH as f32 - 100.0);
            node.y = 14.0;
        }
        Ok(node_id)
    }

    /// Render EuropaRack modules, controls, jacks, and catenary cables into pixels.
    pub fn render_canvas(graph: &PatchGraph) -> (usize, usize, Vec<u32>) {
        let w = RACK_CANVAS_WIDTH;
        let h = RACK_CANVAS_HEIGHT;
        let mut pixels = vec![0u32; w * h];

        // VCV Dark Room brightness scale (0..100)
        let brightness = graph.theme.dark_room_brightness.min(100);
        let lum = 8 + (brightness as u32 * 45 / 100);
        let bg_color = 0xff000000 | (lum << 16) | ((lum + 2) << 8) | (lum + 6);
        let rail_color = 0xff000000 | ((lum + 12).min(255) << 16) | ((lum + 14).min(255) << 8) | ((lum + 18).min(255));

        // Fill background
        pixels.fill(bg_color);

        // Draw top & bottom aluminum rails (10px height each)
        for y in 0..10 {
            for x in 0..w {
                pixels[y * w + x] = rail_color;
                pixels[(h - 1 - y) * w + x] = rail_color;
            }
        }

        // Draw module faceplates
        let mod_width = 85usize;
        let mod_height = h.saturating_sub(24);
        let node_count = graph.nodes.len();

        for (i, node) in graph.nodes.iter().enumerate() {
            let start_x = 10 + (i * 92) % (w - mod_width - 10);
            let start_y = 12;

            // Module panel background tinted by category
            let panel_base = (lum + 22).min(255);
            let panel_color = 0xff000000 | (panel_base << 16) | (panel_base << 8) | (panel_base + 4);

            for y in start_y..(start_y + mod_height).min(h - 10) {
                for x in start_x..(start_x + mod_width).min(w) {
                    pixels[y * w + x] = panel_color;
                }
            }

            // Top header band (6px) with glowing category color
            let cat_color = match i % 5 {
                0 => 0xffe53935, // Audio/VCO (Crimson)
                1 => 0xfffb8c00, // Filter/VCF (Amber)
                2 => 0xff00acc1, // Modulation/LFO (Cyan)
                3 => 0xff43a047, // Envelope/VCA (Emerald)
                _ => 0xff8e24aa, // Sequence/Logic (Purple)
            };
            for y in start_y..(start_y + 8).min(h) {
                for x in start_x..(start_x + mod_width).min(w) {
                    pixels[y * w + x] = cat_color;
                }
            }

            // Draw jack sockets (inputs and outputs)
            let desc: Vec<_> = node.module.inputs().iter().chain(node.module.outputs().iter()).copied().collect();
            for (p_idx, port) in desc.iter().enumerate().take(6) {
                let jx = start_x + 12 + (p_idx % 2) * 50;
                let jy = start_y + 30 + (p_idx / 2) * 40;
                if jx + 6 < w && jy + 6 < h {
                    for dy in 0..6 {
                        for dx in 0..6 {
                            let jack_pixel = if port.kind == moan_modular::signal::PortKind::Audio {
                                0xff333333
                            } else {
                                0xff222222
                            };
                            pixels[(jy + dy) * w + (jx + dx)] = jack_pixel;
                        }
                    }
                }
            }
        }

        // Draw color-coded drooping catenary patch cables
        for (c_idx, cable) in graph.cables.iter().enumerate() {
            let color_u32 = match cable.color {
                CableColor::CrimsonAudio => 0xffff3344,
                CableColor::GoldPitchCv => 0xffffd700,
                CableColor::CyanModCv => 0xff00e5ff,
                CableColor::EmeraldGate => 0xff00e676,
                CableColor::VioletMidi => 0xffc084fc,
            };

            // Derive node port screen coordinates
            let from_x = (20 + (c_idx * 90) % (w - 100)) as f32;
            let from_y = 50.0;
            let to_x = (from_x + 120.0).min(w as f32 - 20.0);
            let to_y = 120.0;

            // Draw catenary arc with sag
            let steps = 40;
            for s in 0..steps {
                let t = s as f32 / steps as f32;
                let x = from_x + (to_x - from_x) * t;
                // Parabolic sag
                let sag = 40.0 * 4.0 * t * (1.0 - t);
                let y = from_y + (to_y - from_y) * t + sag;

                let px = (x as usize).min(w - 1);
                let py = (y as usize).min(h - 1);
                pixels[py * w + px] = color_u32;
                if py + 1 < h {
                    pixels[(py + 1) * w + px] = color_u32;
                }
            }
        }

        // Dark Room glow: active LEDs for clocks and gates
        if node_count > 0 {
            for led_x in [30, 120, 210, 300, 390] {
                if led_x < w {
                    pixels[18 * w + led_x] = 0xff00ffcc;
                }
            }
        }

        (w, h, pixels)
    }
}
