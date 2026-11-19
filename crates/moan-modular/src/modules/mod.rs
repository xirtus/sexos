// SPDX-License-Identifier: GPL-3.0-only
//! Module trait, catalog kinds, and port definitions for EuropaRack.

pub mod amp;
pub mod clock;
pub mod env;
pub mod filter;
pub mod gen;
pub mod host;
pub mod io;
pub mod midi;
pub mod routing;
pub mod vco;
pub mod vocal;

use crate::signal::{PortKind, SignalBuffer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortDesc {
    pub id: u8,
    pub name: &'static str,
    pub kind: PortKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ModuleKind {
    TrackIn = 0,
    MasterOut = 1,
    PolyMidiToCv = 2,
    AnalogVco = 3,
    Fm4OpMacro = 4,
    ArielFormantCv = 5,
    LadderVcf = 6,
    StateVariableVcf = 7,
    PolyVca = 8,
    Wavefolder = 9,
    PolyAdsr = 10,
    TempoLfo = 11,
    ClockDivider = 12,
    EuclideanSeq = 13,
    TuringQuantizer = 14,
    SampleAndHoldSlew = 15,
    MatrixMixer = 16,
    StereoDelay = 17,
    BidulePluginHost = 18,
}

pub trait Module: Send + Sync {
    fn kind(&self) -> ModuleKind;
    fn inputs(&self) -> &[PortDesc];
    fn outputs(&self) -> &[PortDesc];
    fn param_count(&self) -> usize;
    fn get_param(&self, index: usize) -> f32;
    fn set_param(&mut self, index: usize, value: f32);
    fn reset(&mut self);
    fn process_block(&mut self, inputs: &[SignalBuffer], outputs: &mut [SignalBuffer]);
}

pub fn create_module(kind: ModuleKind) -> alloc::boxed::Box<dyn Module> {
    match kind {
        ModuleKind::TrackIn => alloc::boxed::Box::new(io::TrackIn::new()),
        ModuleKind::MasterOut => alloc::boxed::Box::new(io::MasterOut::new()),
        ModuleKind::PolyMidiToCv => alloc::boxed::Box::new(midi::PolyMidiToCv::new()),
        ModuleKind::AnalogVco => alloc::boxed::Box::new(vco::AnalogVco::new()),
        ModuleKind::Fm4OpMacro => alloc::boxed::Box::new(vco::Fm4OpMacro::new()),
        ModuleKind::ArielFormantCv => alloc::boxed::Box::new(vocal::ArielFormantCv::new()),
        ModuleKind::LadderVcf => alloc::boxed::Box::new(filter::LadderVcf::new()),
        ModuleKind::StateVariableVcf => alloc::boxed::Box::new(filter::StateVariableVcf::new()),
        ModuleKind::PolyVca => alloc::boxed::Box::new(amp::PolyVca::new()),
        ModuleKind::Wavefolder => alloc::boxed::Box::new(amp::Wavefolder::new()),
        ModuleKind::PolyAdsr => alloc::boxed::Box::new(env::PolyAdsr::new()),
        ModuleKind::TempoLfo => alloc::boxed::Box::new(env::TempoLfo::new()),
        ModuleKind::ClockDivider => alloc::boxed::Box::new(clock::ClockDivider::new()),
        ModuleKind::EuclideanSeq => alloc::boxed::Box::new(clock::EuclideanSeq::new()),
        ModuleKind::TuringQuantizer => alloc::boxed::Box::new(gen::TuringQuantizer::new()),
        ModuleKind::SampleAndHoldSlew => alloc::boxed::Box::new(gen::SampleAndHoldSlew::new()),
        ModuleKind::MatrixMixer => alloc::boxed::Box::new(routing::MatrixMixer::new()),
        ModuleKind::StereoDelay => alloc::boxed::Box::new(routing::StereoDelay::new()),
        ModuleKind::BidulePluginHost => alloc::boxed::Box::new(host::BidulePluginHost::new()),
    }
}

