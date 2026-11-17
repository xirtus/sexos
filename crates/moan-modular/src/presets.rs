// SPDX-License-Identifier: GPL-3.0-only
//! Factory presets for EuropaRack.



use crate::graph::{Error, PatchGraph};
use crate::modules::ModuleKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactoryPatch {
    ClassicSubtractiveLead,
    GenerativeTuringChoir,
    EuclideanAcidBassline,
    WestCoastWavefoldPluck,
    BiduleParallelFxSplitter,
    PolyphonicLushPad,
}

impl FactoryPatch {
    pub const ALL: [FactoryPatch; 6] = [
        FactoryPatch::ClassicSubtractiveLead,
        FactoryPatch::GenerativeTuringChoir,
        FactoryPatch::EuclideanAcidBassline,
        FactoryPatch::WestCoastWavefoldPluck,
        FactoryPatch::BiduleParallelFxSplitter,
        FactoryPatch::PolyphonicLushPad,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::ClassicSubtractiveLead => "Classic Subtractive Lead",
            Self::GenerativeTuringChoir => "Generative Turing Choir (Ariel CV)",
            Self::EuclideanAcidBassline => "Euclidean Acid Bassline",
            Self::WestCoastWavefoldPluck => "West-Coast Wavefold Pluck",
            Self::BiduleParallelFxSplitter => "Bidule Parallel FX Splitter",
            Self::PolyphonicLushPad => "Polyphonic Lush Pad",
        }
    }

    pub fn build(self) -> Result<PatchGraph, Error> {
        match self {
            Self::ClassicSubtractiveLead => build_subtractive_lead(),
            Self::GenerativeTuringChoir => build_turing_choir(),
            Self::EuclideanAcidBassline => build_euclidean_acid(),
            Self::WestCoastWavefoldPluck => build_wavefold_pluck(),
            Self::BiduleParallelFxSplitter => build_bidule_splitter(),
            Self::PolyphonicLushPad => build_lush_pad(),
        }
    }
}

fn build_subtractive_lead() -> Result<PatchGraph, Error> {
    let mut g = PatchGraph::new();

    let midi = g.add_module(ModuleKind::PolyMidiToCv)?;
    let vco = g.add_module(ModuleKind::AnalogVco)?;
    let vcf = g.add_module(ModuleKind::LadderVcf)?;
    let adsr = g.add_module(ModuleKind::PolyAdsr)?;
    let vca = g.add_module(ModuleKind::PolyVca)?;
    let out = g.add_module(ModuleKind::MasterOut)?;

    // Configure modules
    if let Some(m) = g.find_node_mut(vcf) {
        m.module.set_param(0, 1200.0); // Cutoff 1.2 kHz
        m.module.set_param(1, 0.5);    // Resonance
        m.module.set_param(2, 1.5);    // Drive
    }
    if let Some(m) = g.find_node_mut(adsr) {
        m.module.set_param(0, 5.0);   // Attack 5ms
        m.module.set_param(1, 150.0); // Decay 150ms
        m.module.set_param(2, 0.4);   // Sustain 0.4
        m.module.set_param(3, 200.0); // Release 200ms
    }

    // Connect cables
    g.connect(midi, 0, vco, 0, None, None)?;  // Pitch CV -> VCO
    g.connect(vco, 0, vcf, 0, None, None)?;   // Saw -> VCF Audio In
    g.connect(midi, 1, adsr, 0, None, None)?; // Gate -> ADSR
    g.connect(adsr, 0, vcf, 1, None, Some(0.8))?; // Env -> VCF Cutoff CV
    g.connect(vcf, 0, vca, 0, None, None)?;   // VCF -> VCA Audio In
    g.connect(adsr, 0, vca, 1, None, None)?;  // Env -> VCA Control CV
    g.connect(vca, 0, out, 0, None, None)?;   // VCA -> Master Out

    g.compile()?;
    Ok(g)
}

fn build_turing_choir() -> Result<PatchGraph, Error> {
    let mut g = PatchGraph::new();

    let clock = g.add_module(ModuleKind::ClockDivider)?;
    let turing = g.add_module(ModuleKind::TuringQuantizer)?;
    let ariel = g.add_module(ModuleKind::ArielFormantCv)?;
    let lfo = g.add_module(ModuleKind::TempoLfo)?;
    let delay = g.add_module(ModuleKind::StereoDelay)?;
    let out = g.add_module(ModuleKind::MasterOut)?;

    if let Some(m) = g.find_node_mut(clock) {
        m.module.set_param(0, 108.0); // 108 BPM
    }
    if let Some(m) = g.find_node_mut(turing) {
        m.module.set_param(0, 0.85); // 85% Lock
        m.module.set_param(1, 2.0);  // Minor scale
    }
    if let Some(m) = g.find_node_mut(ariel) {
        m.module.set_param(2, 0.7); // Effort
        m.module.set_param(3, 0.2); // Breath
    }
    if let Some(m) = g.find_node_mut(delay) {
        m.module.set_param(0, 280.0); // 280ms
        m.module.set_param(1, 0.55);  // Feedback
        m.module.set_param(2, 0.4);   // Dry/Wet
    }

    g.connect(clock, 0, turing, 0, None, None)?; // Clock -> Turing Clock In
    g.connect(clock, 2, ariel, 5, None, None)?;  // Div4 -> Ariel Phoneme Step Trig
    g.connect(turing, 0, ariel, 0, None, None)?; // Turing Pitch CV -> Ariel Pitch CV
    g.connect(lfo, 0, ariel, 1, None, Some(0.4))?; // LFO Sine -> Vowel X
    g.connect(lfo, 1, ariel, 2, None, Some(0.4))?; // LFO Tri -> Vowel Y
    g.connect(ariel, 0, delay, 0, None, None)?;  // Ariel Audio -> Delay In
    g.connect(delay, 0, out, 0, None, None)?;    // Delay -> Master Out

    g.compile()?;
    Ok(g)
}

fn build_euclidean_acid() -> Result<PatchGraph, Error> {
    let mut g = PatchGraph::new();

    let clock = g.add_module(ModuleKind::ClockDivider)?;
    let euclid = g.add_module(ModuleKind::EuclideanSeq)?;
    let vco = g.add_module(ModuleKind::AnalogVco)?;
    let svf = g.add_module(ModuleKind::StateVariableVcf)?;
    let adsr = g.add_module(ModuleKind::PolyAdsr)?;
    let vca = g.add_module(ModuleKind::PolyVca)?;
    let out = g.add_module(ModuleKind::MasterOut)?;

    if let Some(m) = g.find_node_mut(clock) {
        m.module.set_param(0, 134.0); // 134 BPM Acid
    }
    if let Some(m) = g.find_node_mut(euclid) {
        m.module.set_param(0, 16.0); // 16 steps
        m.module.set_param(1, 7.0);  // 7 pulses (complex groove)
    }
    if let Some(m) = g.find_node_mut(svf) {
        m.module.set_param(0, 800.0); // Cutoff 800 Hz
        m.module.set_param(1, 6.0);   // High Q resonance
    }
    if let Some(m) = g.find_node_mut(adsr) {
        m.module.set_param(0, 2.0);   // Attack 2ms
        m.module.set_param(1, 90.0);  // Decay 90ms
        m.module.set_param(2, 0.1);   // Sustain 0.1
        m.module.set_param(3, 80.0);  // Release 80ms
    }

    g.connect(clock, 0, euclid, 0, None, None)?; // Clock -> Euclid Clock
    g.connect(euclid, 0, adsr, 0, None, None)?;  // Gate -> ADSR
    g.connect(euclid, 1, vco, 0, None, None)?;   // Pitch CV -> VCO
    g.connect(vco, 1, svf, 0, None, None)?;      // Pulse Out -> SVF
    g.connect(adsr, 0, svf, 1, None, Some(1.2))?; // Env -> SVF Cutoff CV
    g.connect(svf, 0, vca, 0, None, None)?;      // Lowpass Out -> VCA
    g.connect(adsr, 0, vca, 1, None, None)?;     // Env -> VCA Control CV
    g.connect(vca, 0, out, 0, None, None)?;      // VCA -> Master Out

    g.compile()?;
    Ok(g)
}

fn build_wavefold_pluck() -> Result<PatchGraph, Error> {
    let mut g = PatchGraph::new();

    let clock = g.add_module(ModuleKind::ClockDivider)?;
    let sh = g.add_module(ModuleKind::SampleAndHoldSlew)?;
    let vco = g.add_module(ModuleKind::AnalogVco)?;
    let folder = g.add_module(ModuleKind::Wavefolder)?;
    let adsr = g.add_module(ModuleKind::PolyAdsr)?;
    let vca = g.add_module(ModuleKind::PolyVca)?;
    let out = g.add_module(ModuleKind::MasterOut)?;

    if let Some(m) = g.find_node_mut(folder) {
        m.module.set_param(0, 4.0); // Fold amount
        m.module.set_param(1, 0.2); // Bias
    }

    g.connect(clock, 0, sh, 1, None, None)?;     // Clock -> S&H Trigger
    g.connect(clock, 1, adsr, 0, None, None)?;   // Div2 -> ADSR Gate
    g.connect(sh, 2, vco, 0, None, Some(0.3))?;  // Slewed Out -> VCO Pitch CV
    g.connect(vco, 2, folder, 0, None, None)?;   // Triangle Out -> Wavefolder In
    g.connect(adsr, 0, folder, 1, None, Some(0.5))?; // Env -> Fold CV
    g.connect(folder, 0, vca, 0, None, None)?;   // Wavefolder -> VCA In
    g.connect(adsr, 0, vca, 1, None, None)?;     // Env -> VCA Control CV
    g.connect(vca, 0, out, 0, None, None)?;      // VCA -> Master Out

    g.compile()?;
    Ok(g)
}

fn build_bidule_splitter() -> Result<PatchGraph, Error> {
    let mut g = PatchGraph::new();

    let track_in = g.add_module(ModuleKind::TrackIn)?;
    let matrix = g.add_module(ModuleKind::MatrixMixer)?;
    let delay = g.add_module(ModuleKind::StereoDelay)?;
    let host = g.add_module(ModuleKind::BidulePluginHost)?;
    let out = g.add_module(ModuleKind::MasterOut)?;

    // Route matrix mixer
    g.connect(track_in, 0, matrix, 0, None, None)?; // TrackIn -> Matrix In 1
    g.connect(matrix, 0, delay, 0, None, None)?;    // Matrix Out 1 -> Delay In
    g.connect(matrix, 1, host, 0, None, None)?;     // Matrix Out 2 -> Host In
    g.connect(delay, 0, out, 0, None, Some(0.7))?;  // Delay -> Master Out
    g.connect(host, 0, out, 0, None, Some(0.7))?;   // Host -> Master Out

    g.compile()?;
    Ok(g)
}

fn build_lush_pad() -> Result<PatchGraph, Error> {
    let mut g = PatchGraph::new();

    let midi = g.add_module(ModuleKind::PolyMidiToCv)?;
    let fm = g.add_module(ModuleKind::Fm4OpMacro)?;
    let vco = g.add_module(ModuleKind::AnalogVco)?;
    let svf = g.add_module(ModuleKind::StateVariableVcf)?;
    let lfo = g.add_module(ModuleKind::TempoLfo)?;
    let adsr = g.add_module(ModuleKind::PolyAdsr)?;
    let vca = g.add_module(ModuleKind::PolyVca)?;
    let delay = g.add_module(ModuleKind::StereoDelay)?;
    let out = g.add_module(ModuleKind::MasterOut)?;

    if let Some(m) = g.find_node_mut(adsr) {
        m.module.set_param(0, 300.0);  // Attack 300ms
        m.module.set_param(1, 800.0);  // Decay 800ms
        m.module.set_param(2, 0.7);    // Sustain 0.7
        m.module.set_param(3, 1000.0); // Release 1000ms
    }
    if let Some(m) = g.find_node_mut(svf) {
        m.module.set_param(0, 1600.0);
        m.module.set_param(1, 2.0);
    }

    g.connect(midi, 0, fm, 0, None, None)?;      // Pitch CV -> FM
    g.connect(midi, 0, vco, 0, None, None)?;     // Pitch CV -> VCO
    g.connect(midi, 1, adsr, 0, None, None)?;    // Gate -> ADSR
    g.connect(fm, 0, svf, 0, None, Some(0.6))?;  // FM -> SVF
    g.connect(vco, 0, svf, 0, None, Some(0.5))?; // VCO Saw -> SVF
    g.connect(lfo, 0, svf, 1, None, Some(0.2))?; // LFO -> SVF Cutoff
    g.connect(svf, 0, vca, 0, None, None)?;      // SVF LP -> VCA In
    g.connect(adsr, 0, vca, 1, None, None)?;     // Env -> VCA Control
    g.connect(vca, 0, delay, 0, None, None)?;    // VCA -> Delay In
    g.connect(delay, 0, out, 0, None, None)?;    // Delay -> Master Out

    g.compile()?;
    Ok(g)
}
