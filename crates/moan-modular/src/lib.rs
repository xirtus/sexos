// SPDX-License-Identifier: GPL-3.0-only
//! EuropaRack (`moan-modular`): Moan's `#![no_std]` Virtual Eurorack (VCV Rack 2)
//! + Modular Audio/MIDI/Plugin Patchbay (Plogue Bidule / Pure Data / Bespoke Synth).
//!
//! Provides polyphonic bounded signals (BLOCK_FRAMES = 64, MAX_POLY = 16, RATE = 48_000),
//! feedback-safe topological scheduling with 1-block feedback delays, 19 built-in
//! modules, sub-patch export/import, binary patch persistence, and direct rendering
//! to Moan timeline Clip PCM buffers.

#![no_std]
extern crate alloc;

pub mod catalog;
pub mod graph;
pub mod math;
pub mod modules;
pub mod presets;
pub mod serialize;
pub mod signal;

pub use catalog::{filter_by_category, find_metadata, search, ModuleCategory, ModuleFavorites, ModuleMetadata, CATALOG};
pub use graph::{Cable, CableId, Error, ModuleNode, NodeId, PatchGraph, PortId};
pub use modules::{create_module, Module, ModuleKind, PortDesc};
pub use presets::FactoryPatch;
pub use signal::{
    CableColor, MidiBuffer, ModularMidiEvent, PortKind, RackTheme, SignalBuffer, BLOCK_FRAMES, MAX_MIDI_EVENTS,
    MAX_POLY, RATE,
};
