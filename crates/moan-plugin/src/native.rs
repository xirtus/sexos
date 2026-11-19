//! Port model and block runner shared by the LADSPA and LV2 adapters.
//!
//! Both formats describe a plugin as numbered ports (audio, control, CV) and
//! run it with `connect_port` + `run(frames)`. The format modules turn their own
//! descriptions into [`Port`]s; this module decides how a stereo offline region
//! maps onto them (one stereo instance, or two mono instances), owns every
//! buffer a plugin can see, and checks every sample the plugin writes.
use crate::job::{Parameter, BLOCK, MAX_FRAMES, RATE};
use alloc::{boxed::Box, vec::Vec};

/// Ports beyond this are refused; real effects use a handful.
pub const MAX_PORTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Memory,
    Descriptor,
    Ports,
    Feature,
    Parameter,
    Instantiate,
    Frames,
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    AudioIn,
    AudioOut,
    ControlIn,
    ControlOut,
    /// Audio-rate control signal. Held at the port's default for offline renders.
    CvIn,
    CvOut,
    /// A port this host does not drive (LV2 atom/event ports marked
    /// `lv2:connectionOptional`). Connected to null as the spec allows.
    Unused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Port {
    pub index: u32,
    pub kind: Kind,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub integer: bool,
    pub toggled: bool,
}

impl Port {
    pub fn new(index: u32, kind: Kind) -> Self {
        Self {
            index,
            kind,
            default: 0.0,
            min: f32::NEG_INFINITY,
            max: f32::INFINITY,
            integer: false,
            toggled: false,
        }
    }
    /// Clamp/round a requested control value to what the plugin declared.
    pub fn admit(&self, value: f32) -> f32 {
        let mut v = value;
        if v < self.min {
            v = self.min;
        }
        if v > self.max {
            v = self.max;
        }
        if self.toggled {
            v = if v > 0.0 { 1.0 } else { 0.0 };
        } else if self.integer {
            v = libm::roundf(v);
        }
        v
    }
}

/// How the two channels of an offline region reach the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    /// One instance with two audio inputs and two audio outputs.
    Stereo { inputs: [u32; 2], outputs: [u32; 2] },
    /// Two instances (left, right) of a one-in/one-out plugin.
    Mono { input: u32, output: u32 },
}

impl Channels {
    pub fn instances(self) -> usize {
        match self {
            Self::Stereo { .. } => 1,
            Self::Mono { .. } => 2,
        }
    }
}

/// Check the port list and decide the channel plan. Ports must be numbered
/// 0..n exactly once, because both formats address them by position.
pub fn plan(ports: &[Port]) -> Result<Channels, Error> {
    if ports.is_empty() || ports.len() > MAX_PORTS {
        return Err(Error::Ports);
    }
    for (i, p) in ports.iter().enumerate() {
        if p.index as usize != i {
            return Err(Error::Ports);
        }
        if !p.default.is_finite() || p.min > p.max || p.min.is_nan() || p.max.is_nan() {
            return Err(Error::Ports);
        }
    }
    let of = |k: Kind| -> Vec<u32> {
        ports
            .iter()
            .filter(|p| p.kind == k)
            .map(|p| p.index)
            .collect()
    };
    let (ins, outs) = (of(Kind::AudioIn), of(Kind::AudioOut));
    match (ins.len(), outs.len()) {
        (1, 1) => Ok(Channels::Mono {
            input: ins[0],
            output: outs[0],
        }),
        (2, 2) => Ok(Channels::Stereo {
            inputs: [ins[0], ins[1]],
            outputs: [outs[0], outs[1]],
        }),
        // Generators, sidechains and surround layouts are not offline effects.
        _ => Err(Error::Ports),
    }
}

/// Default control values for every port, with the request's overrides applied.
/// A parameter id is a port index and must name a control input.
pub fn controls(ports: &[Port], parameters: &[Parameter]) -> Result<Vec<f32>, Error> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(ports.len())
        .map_err(|_| Error::Memory)?;
    values.extend(ports.iter().map(|p| p.admit(p.default)));
    for p in parameters {
        let port = ports.get(p.id as usize).ok_or(Error::Parameter)?;
        if port.kind != Kind::ControlIn || !p.value.is_finite() {
            return Err(Error::Parameter);
        }
        values[p.id as usize] = port.admit(p.value as f32);
    }
    Ok(values)
}

/// What a format adapter must provide to run its instances.
pub(crate) trait Native {
    /// # Safety
    /// `data` stays valid until the next `connect` of that port or the end of `render`.
    unsafe fn connect(&mut self, instance: usize, port: u32, data: *mut f32);
    /// # Safety
    /// Every port of `instance` was connected to a buffer of at least `frames` floats.
    unsafe fn run(&mut self, instance: usize, frames: u32);
}

/// Buffers a plugin may read or write. Boxed so addresses never move while
/// the plugin holds them.
struct Buffers {
    controls: Vec<Box<[f32]>>,
    cv: Vec<Box<[f32]>>,
    ins: Box<[[f32; BLOCK]; 2]>,
    outs: Box<[[f32; BLOCK]; 2]>,
}

fn boxed(len: usize, fill: f32) -> Result<Box<[f32]>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::Memory)?;
    v.resize(len, fill);
    Ok(v.into_boxed_slice())
}

fn sane(v: f32) -> bool {
    v.is_finite() && v.abs() <= 16.0
}

/// Offline render of a stereo region through `native` according to `channels`.
/// No allocation happens between `run` calls.
pub(crate) fn render(
    native: &mut impl Native,
    ports: &[Port],
    channels: Channels,
    values: &[f32],
    input: &[[f32; 2]],
) -> Result<Vec<[f32; 2]>, Error> {
    if input.is_empty() || input.len() > MAX_FRAMES as usize {
        return Err(Error::Frames);
    }
    if input.iter().flatten().any(|v| !sane(*v)) {
        return Err(Error::Audio);
    }
    if values.len() != ports.len() {
        return Err(Error::Ports);
    }
    let instances = channels.instances();
    let mut buffers = Buffers {
        controls: Vec::new(),
        cv: Vec::new(),
        ins: Box::new([[0.0; BLOCK]; 2]),
        outs: Box::new([[0.0; BLOCK]; 2]),
    };
    let mut result = crate::job::buffer(input.len()).map_err(|_| Error::Memory)?;
    for instance in 0..instances {
        let mut c = boxed(ports.len(), 0.0)?;
        c.copy_from_slice(values);
        buffers.controls.push(c);
        for p in ports {
            if matches!(p.kind, Kind::CvIn | Kind::CvOut) {
                buffers.cv.push(boxed(BLOCK, p.default)?);
            }
        }
        let _ = instance;
    }
    // Connect everything once; buffers are fixed for the whole render.
    let mut cv = 0;
    for instance in 0..instances {
        for p in ports {
            let data: *mut f32 = match p.kind {
                Kind::ControlIn | Kind::ControlOut => {
                    &mut buffers.controls[instance][p.index as usize]
                }
                Kind::CvIn | Kind::CvOut => {
                    cv += 1;
                    buffers.cv[cv - 1].as_mut_ptr()
                }
                Kind::AudioIn => {
                    let ch = match channels {
                        Channels::Mono { .. } => instance,
                        Channels::Stereo { inputs, .. } => (inputs[1] == p.index) as usize,
                    };
                    buffers.ins[ch].as_mut_ptr()
                }
                Kind::AudioOut => {
                    let ch = match channels {
                        Channels::Mono { .. } => instance,
                        Channels::Stereo { outputs, .. } => (outputs[1] == p.index) as usize,
                    };
                    buffers.outs[ch].as_mut_ptr()
                }
                Kind::Unused => core::ptr::null_mut(),
            };
            unsafe { native.connect(instance, p.index, data) };
        }
    }
    for frames in input.chunks(BLOCK) {
        for (i, f) in frames.iter().enumerate() {
            buffers.ins[0][i] = f[0];
            buffers.ins[1][i] = f[1];
            // Unwritten output is caught as NaN below.
            buffers.outs[0][i] = f32::NAN;
            buffers.outs[1][i] = f32::NAN;
        }
        // CV inputs are reset each block in case the plugin scribbled on them.
        let mut at = 0;
        for _ in 0..instances {
            for p in ports {
                if matches!(p.kind, Kind::CvIn | Kind::CvOut) {
                    buffers.cv[at].fill(p.default);
                    at += 1;
                }
            }
        }
        for instance in 0..instances {
            unsafe { native.run(instance, frames.len() as u32) };
        }
        for i in 0..frames.len() {
            let frame = [buffers.outs[0][i], buffers.outs[1][i]];
            if !sane(frame[0]) || !sane(frame[1]) {
                return Err(Error::Audio);
            }
            result.push(frame);
        }
    }
    Ok(result)
}

/// Sample rate the offline renderer runs at, as the formats want it.
pub const RATE_F64: f64 = RATE as f64;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::vec;

    /// A Rust stand-in plugin: per-instance gain from control port 0.
    pub struct Gain {
        pub ports: [[*mut f32; 8]; 2],
        pub runs: usize,
    }
    impl Native for Gain {
        unsafe fn connect(&mut self, instance: usize, port: u32, data: *mut f32) {
            self.ports[instance][port as usize] = data;
        }
        unsafe fn run(&mut self, instance: usize, frames: u32) {
            self.runs += 1;
            let p = &self.ports[instance];
            let gain = *p[0];
            for i in 0..frames as usize {
                *p[2].add(i) = *p[1].add(i) * gain;
            }
        }
    }
    pub fn mono_ports() -> Vec<Port> {
        let mut gain = Port::new(0, Kind::ControlIn);
        gain.default = 0.5;
        gain.min = 0.0;
        gain.max = 2.0;
        vec![gain, Port::new(1, Kind::AudioIn), Port::new(2, Kind::AudioOut)]
    }

    #[test]
    fn mono_plugin_runs_twice_per_block() {
        let ports = mono_ports();
        let ch = plan(&ports).unwrap();
        assert_eq!(ch, Channels::Mono { input: 1, output: 2 });
        let values = controls(&ports, &[Parameter { id: 0, value: 4.0 }]).unwrap();
        assert_eq!(values[0], 2.0, "clamped to declared maximum");
        let mut g = Gain {
            ports: [[core::ptr::null_mut(); 8]; 2],
            runs: 0,
        };
        let out = render(&mut g, &ports, ch, &values, &vec![[0.25, -0.5]; BLOCK + 3]).unwrap();
        assert_eq!(g.runs, 4);
        assert!(out.iter().all(|f| *f == [0.5, -1.0]));
    }
    #[test]
    fn unwritten_output_is_refused() {
        struct Lazy;
        impl Native for Lazy {
            unsafe fn connect(&mut self, _: usize, _: u32, _: *mut f32) {}
            unsafe fn run(&mut self, _: usize, _: u32) {}
        }
        let ports = mono_ports();
        let ch = plan(&ports).unwrap();
        let values = controls(&ports, &[]).unwrap();
        assert_eq!(
            render(&mut Lazy, &ports, ch, &values, &[[0.0, 0.0]]),
            Err(Error::Audio)
        );
    }
    #[test]
    fn parameters_must_name_control_inputs() {
        let ports = mono_ports();
        assert_eq!(
            controls(&ports, &[Parameter { id: 1, value: 0.0 }]),
            Err(Error::Parameter)
        );
        assert_eq!(
            controls(&ports, &[Parameter { id: 9, value: 0.0 }]),
            Err(Error::Parameter)
        );
    }
    #[test]
    fn generators_and_gaps_are_refused() {
        assert_eq!(plan(&[Port::new(0, Kind::AudioOut)]), Err(Error::Ports));
        assert_eq!(
            plan(&[Port::new(1, Kind::AudioIn), Port::new(0, Kind::AudioOut)]),
            Err(Error::Ports)
        );
    }
}
