//! LADSPA 1.1 host: descriptor admission, port hints to defaults, and the
//! instantiate/connect/activate/run/deactivate/cleanup lifecycle.
//!
//! The C ABI below is written from the LADSPA 1.1 specification (ladspa.h).
//! As with CLAP, native calls belong only in a disposable helper domain.
use crate::job::Parameter;
use crate::native::{self, Channels, Error, Kind, Native, Port, MAX_PORTS, RATE_F64};
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_ulong, c_void};

pub type Handle = *mut c_void;
pub type Data = f32;

pub const PROPERTY_REALTIME: c_int = 0x1;
pub const PROPERTY_INPLACE_BROKEN: c_int = 0x2;
pub const PROPERTY_HARD_RT_CAPABLE: c_int = 0x4;

pub const PORT_INPUT: c_int = 0x1;
pub const PORT_OUTPUT: c_int = 0x2;
pub const PORT_CONTROL: c_int = 0x4;
pub const PORT_AUDIO: c_int = 0x8;

pub const HINT_BOUNDED_BELOW: c_int = 0x1;
pub const HINT_BOUNDED_ABOVE: c_int = 0x2;
pub const HINT_TOGGLED: c_int = 0x4;
pub const HINT_SAMPLE_RATE: c_int = 0x8;
pub const HINT_LOGARITHMIC: c_int = 0x10;
pub const HINT_INTEGER: c_int = 0x20;
pub const HINT_DEFAULT_MASK: c_int = 0x3C0;
pub const HINT_DEFAULT_NONE: c_int = 0x0;
pub const HINT_DEFAULT_MINIMUM: c_int = 0x40;
pub const HINT_DEFAULT_LOW: c_int = 0x80;
pub const HINT_DEFAULT_MIDDLE: c_int = 0xC0;
pub const HINT_DEFAULT_HIGH: c_int = 0x100;
pub const HINT_DEFAULT_MAXIMUM: c_int = 0x140;
pub const HINT_DEFAULT_0: c_int = 0x200;
pub const HINT_DEFAULT_1: c_int = 0x240;
pub const HINT_DEFAULT_100: c_int = 0x280;
pub const HINT_DEFAULT_440: c_int = 0x2C0;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RangeHint {
    pub hint: c_int,
    pub lower: Data,
    pub upper: Data,
}

#[repr(C)]
pub struct Descriptor {
    pub unique_id: c_ulong,
    pub label: *const c_char,
    pub properties: c_int,
    pub name: *const c_char,
    pub maker: *const c_char,
    pub copyright: *const c_char,
    pub port_count: c_ulong,
    pub port_descriptors: *const c_int,
    pub port_names: *const *const c_char,
    pub port_range_hints: *const RangeHint,
    pub implementation_data: *mut c_void,
    pub instantiate: Option<unsafe extern "C" fn(*const Descriptor, c_ulong) -> Handle>,
    pub connect_port: Option<unsafe extern "C" fn(Handle, c_ulong, *mut Data)>,
    pub activate: Option<unsafe extern "C" fn(Handle)>,
    pub run: Option<unsafe extern "C" fn(Handle, c_ulong)>,
    pub run_adding: Option<unsafe extern "C" fn(Handle, c_ulong)>,
    pub set_run_adding_gain: Option<unsafe extern "C" fn(Handle, Data)>,
    pub deactivate: Option<unsafe extern "C" fn(Handle)>,
    pub cleanup: Option<unsafe extern "C" fn(Handle)>,
}

/// `ladspa_descriptor(index)`, the library's only required export.
pub type Entry = unsafe extern "C" fn(c_ulong) -> *const Descriptor;

/// Default value for a control port from its range hint, per ladspa.h.
pub fn default_for(h: RangeHint) -> f32 {
    let scale = if h.hint & HINT_SAMPLE_RATE != 0 {
        RATE_F64 as f32
    } else {
        1.0
    };
    let (lo, hi) = (h.lower * scale, h.upper * scale);
    let log = h.hint & HINT_LOGARITHMIC != 0 && lo > 0.0 && hi > 0.0;
    let mix = |w: f32| {
        if log {
            libm::expf(libm::logf(lo) * (1.0 - w) + libm::logf(hi) * w)
        } else {
            lo * (1.0 - w) + hi * w
        }
    };
    match h.hint & HINT_DEFAULT_MASK {
        HINT_DEFAULT_MINIMUM => lo,
        HINT_DEFAULT_LOW => mix(0.25),
        HINT_DEFAULT_MIDDLE => mix(0.5),
        HINT_DEFAULT_HIGH => mix(0.75),
        HINT_DEFAULT_MAXIMUM => hi,
        HINT_DEFAULT_0 => 0.0,
        HINT_DEFAULT_1 => 1.0,
        HINT_DEFAULT_100 => 100.0,
        HINT_DEFAULT_440 => 440.0,
        // No default given: the lower bound if there is one, else zero.
        _ if h.hint & HINT_BOUNDED_BELOW != 0 => lo,
        _ if h.hint & HINT_BOUNDED_ABOVE != 0 && hi < 0.0 => hi,
        _ => 0.0,
    }
}

/// Turn a descriptor's port arrays into the shared port model.
///
/// # Safety
/// `d` must point to a descriptor whose arrays hold `port_count` entries.
pub unsafe fn ports(d: *const Descriptor) -> Result<Vec<Port>, Error> {
    let d = &*d;
    let n = d.port_count as usize;
    if n == 0 || n > MAX_PORTS || d.port_descriptors.is_null() || d.port_range_hints.is_null() {
        return Err(Error::Descriptor);
    }
    let kinds = core::slice::from_raw_parts(d.port_descriptors, n);
    let hints = core::slice::from_raw_parts(d.port_range_hints, n);
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| Error::Memory)?;
    for (i, (&k, &h)) in kinds.iter().zip(hints).enumerate() {
        let input = k & PORT_INPUT != 0;
        if input == (k & PORT_OUTPUT != 0) || (k & PORT_AUDIO != 0) == (k & PORT_CONTROL != 0) {
            return Err(Error::Ports);
        }
        let kind = match (k & PORT_AUDIO != 0, input) {
            (true, true) => Kind::AudioIn,
            (true, false) => Kind::AudioOut,
            (false, true) => Kind::ControlIn,
            (false, false) => Kind::ControlOut,
        };
        let mut p = Port::new(i as u32, kind);
        if matches!(kind, Kind::ControlIn | Kind::ControlOut) {
            let scale = if h.hint & HINT_SAMPLE_RATE != 0 {
                RATE_F64 as f32
            } else {
                1.0
            };
            if h.hint & HINT_BOUNDED_BELOW != 0 && h.lower.is_finite() {
                p.min = h.lower * scale;
            }
            if h.hint & HINT_BOUNDED_ABOVE != 0 && h.upper.is_finite() {
                p.max = h.upper * scale;
            }
            if p.min > p.max {
                return Err(Error::Ports);
            }
            p.toggled = h.hint & HINT_TOGGLED != 0;
            p.integer = h.hint & HINT_INTEGER != 0;
            p.default = default_for(h);
            if !p.default.is_finite() {
                p.default = 0.0;
            }
        }
        out.push(p);
    }
    Ok(out)
}

pub struct Processor {
    desc: *const Descriptor,
    handles: [Handle; 2],
    instances: usize,
    ports: Vec<Port>,
    channels: Channels,
    values: Vec<f32>,
    active: bool,
}

impl Processor {
    /// Admit descriptor 0 of a LADSPA library and create its instances.
    ///
    /// # Safety
    /// `entry` must be the relocated `ladspa_descriptor` of a genuine LADSPA
    /// library whose mapping outlives this object. Invoke only in a child domain.
    pub unsafe fn open(entry: Entry, parameters: &[Parameter]) -> Result<Self, Error> {
        // Explicitly the first descriptor; a library index is never user-controlled.
        let desc = entry(0);
        Self::open_descriptor(desc, parameters)
    }

    /// # Safety
    /// As [`Processor::open`], for an already selected descriptor.
    pub unsafe fn open_descriptor(
        desc: *const Descriptor,
        parameters: &[Parameter],
    ) -> Result<Self, Error> {
        if desc.is_null() {
            return Err(Error::Descriptor);
        }
        let d = &*desc;
        if d.instantiate.is_none() || d.connect_port.is_none() || d.run.is_none() {
            return Err(Error::Descriptor);
        }
        let ports = ports(desc)?;
        let channels = native::plan(&ports)?;
        let values = native::controls(&ports, parameters)?;
        let mut this = Self {
            desc,
            handles: [core::ptr::null_mut(); 2],
            instances: 0,
            ports,
            channels,
            values,
            active: false,
        };
        for i in 0..channels.instances() {
            let h = (d.instantiate.unwrap())(desc, crate::job::RATE as c_ulong);
            if h.is_null() {
                return Err(Error::Instantiate);
            }
            this.handles[i] = h;
            this.instances = i + 1;
        }
        Ok(this)
    }

    pub fn ports(&self) -> &[Port] {
        &self.ports
    }

    /// Offline effect render. LADSPA requires every port connected before
    /// `activate`, so activation happens on the first render.
    pub fn render(&mut self, input: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, Error> {
        let ports = core::mem::take(&mut self.ports);
        let values = core::mem::take(&mut self.values);
        let channels = self.channels;
        let result = native::render(self, &ports, channels, &values, input);
        self.ports = ports;
        self.values = values;
        result
    }
}

impl Native for Processor {
    unsafe fn connect(&mut self, instance: usize, port: u32, data: *mut f32) {
        ((*self.desc).connect_port.unwrap())(self.handles[instance], port as c_ulong, data)
    }
    unsafe fn run(&mut self, instance: usize, frames: u32) {
        if !self.active {
            if let Some(f) = (*self.desc).activate {
                for h in &self.handles[..self.instances] {
                    f(*h)
                }
            }
            self.active = true;
        }
        ((*self.desc).run.unwrap())(self.handles[instance], frames as c_ulong)
    }
}

impl Drop for Processor {
    fn drop(&mut self) {
        unsafe {
            let d = &*self.desc;
            for h in &self.handles[..self.instances] {
                if self.active {
                    if let Some(f) = d.deactivate {
                        f(*h)
                    }
                }
                if let Some(f) = d.cleanup {
                    f(*h)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::BLOCK;

    #[repr(C)]
    struct Amp {
        gain: *mut f32,
        input: *mut f32,
        output: *mut f32,
        activated: bool,
    }
    unsafe extern "C" fn instantiate(_: *const Descriptor, rate: c_ulong) -> Handle {
        assert_eq!(rate, 48_000);
        alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Amp {
            gain: core::ptr::null_mut(),
            input: core::ptr::null_mut(),
            output: core::ptr::null_mut(),
            activated: false,
        }))
        .cast()
    }
    unsafe extern "C" fn connect(h: Handle, port: c_ulong, data: *mut f32) {
        let a = &mut *(h as *mut Amp);
        match port {
            0 => a.gain = data,
            1 => a.input = data,
            _ => a.output = data,
        }
    }
    unsafe extern "C" fn activate(h: Handle) {
        (*(h as *mut Amp)).activated = true;
    }
    unsafe extern "C" fn run(h: Handle, n: c_ulong) {
        let a = &*(h as *mut Amp);
        assert!(a.activated);
        for i in 0..n as usize {
            *a.output.add(i) = *a.input.add(i) * *a.gain;
        }
    }
    unsafe extern "C" fn cleanup(h: Handle) {
        drop(alloc::boxed::Box::from_raw(h as *mut Amp));
    }
    const KINDS: [c_int; 3] = [
        PORT_INPUT | PORT_CONTROL,
        PORT_INPUT | PORT_AUDIO,
        PORT_OUTPUT | PORT_AUDIO,
    ];
    const HINTS: [RangeHint; 3] = [
        RangeHint {
            hint: HINT_BOUNDED_BELOW | HINT_BOUNDED_ABOVE | HINT_DEFAULT_1,
            lower: 0.0,
            upper: 4.0,
        },
        RangeHint {
            hint: 0,
            lower: 0.0,
            upper: 0.0,
        },
        RangeHint {
            hint: 0,
            lower: 0.0,
            upper: 0.0,
        },
    ];
    fn descriptor() -> Descriptor {
        Descriptor {
            unique_id: 1,
            label: c"amp".as_ptr(),
            properties: PROPERTY_HARD_RT_CAPABLE,
            name: c"Amp".as_ptr(),
            maker: c"test".as_ptr(),
            copyright: c"None".as_ptr(),
            port_count: 3,
            port_descriptors: KINDS.as_ptr(),
            port_names: core::ptr::null(),
            port_range_hints: HINTS.as_ptr(),
            implementation_data: core::ptr::null_mut(),
            instantiate: Some(instantiate),
            connect_port: Some(connect),
            activate: Some(activate),
            run: Some(run),
            run_adding: None,
            set_run_adding_gain: None,
            deactivate: None,
            cleanup: Some(cleanup),
        }
    }

    #[test]
    fn mono_descriptor_renders_both_channels() {
        let d = descriptor();
        let mut p = unsafe {
            Processor::open_descriptor(&d, &[Parameter { id: 0, value: 2.0 }]).unwrap()
        };
        let out = p.render(&alloc::vec![[0.25, -0.125]; BLOCK * 2 + 1]).unwrap();
        assert!(out.iter().all(|f| *f == [0.5, -0.25]));
    }
    #[test]
    fn defaults_follow_hints() {
        let d = descriptor();
        let mut p = unsafe { Processor::open_descriptor(&d, &[]).unwrap() };
        assert_eq!(p.render(&[[0.5, 0.5]]).unwrap(), alloc::vec![[0.5, 0.5]]);
        let h = |hint, lower, upper| RangeHint { hint, lower, upper };
        let b = HINT_BOUNDED_BELOW | HINT_BOUNDED_ABOVE;
        assert_eq!(default_for(h(b | HINT_DEFAULT_MIDDLE, 0.0, 10.0)), 5.0);
        assert_eq!(default_for(h(b | HINT_DEFAULT_LOW, 0.0, 8.0)), 2.0);
        let log = default_for(h(b | HINT_DEFAULT_MIDDLE | HINT_LOGARITHMIC, 10.0, 1000.0));
        assert!((log - 100.0).abs() < 0.01, "{log}");
        assert_eq!(
            default_for(h(b | HINT_DEFAULT_MAXIMUM | HINT_SAMPLE_RATE, 0.0, 0.5)),
            24_000.0
        );
        assert_eq!(default_for(h(b | HINT_DEFAULT_440, 0.0, 0.5)), 440.0);
    }
    #[test]
    fn malformed_descriptors_are_refused() {
        let mut d = descriptor();
        d.run = None;
        assert!(matches!(
            unsafe { Processor::open_descriptor(&d, &[]) },
            Err(Error::Descriptor)
        ));
        let bad = [PORT_INPUT | PORT_OUTPUT | PORT_AUDIO, KINDS[1], KINDS[2]];
        let mut d = descriptor();
        d.port_descriptors = bad.as_ptr();
        assert!(matches!(
            unsafe { Processor::open_descriptor(&d, &[]) },
            Err(Error::Ports)
        ));
        assert!(matches!(
            unsafe { Processor::open_descriptor(core::ptr::null(), &[]) },
            Err(Error::Descriptor)
        ));
    }
}
