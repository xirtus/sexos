//! LV2 host: packed bundle admission, Turtle port description, URID map/unmap
//! and options features, and the instantiate/connect/activate/run lifecycle.
//!
//! An LV2 plugin is a bundle directory (manifest.ttl, data .ttl files and a
//! shared library). The sandboxed helper receives exactly one file, so Moan
//! packs the bundle into a [`Bundle`] container (`MOANLV2B`) and the helper
//! reads its Turtle here, in safe Rust, before any native code is mapped.
use crate::job::{Parameter, BLOCK, RATE};
use crate::native::{self, Channels, Error, Kind, Native, Port, MAX_PORTS, RATE_F64};
use crate::ttl::{Graph, Term, RDF_TYPE};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::ffi::{c_char, c_void};

pub const LV2: &str = "http://lv2plug.in/ns/lv2core#";
const RDFS_SEE_ALSO: &str = "http://www.w3.org/2000/01/rdf-schema#seeAlso";
pub const URID_MAP: &str = "http://lv2plug.in/ns/ext/urid#map";
pub const URID_UNMAP: &str = "http://lv2plug.in/ns/ext/urid#unmap";
pub const OPTIONS: &str = "http://lv2plug.in/ns/ext/options#options";
pub const BOUNDED_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#boundedBlockLength";
const SAMPLE_RATE: &[u8] = b"http://lv2plug.in/ns/ext/parameters#sampleRate\0";
const MIN_BLOCK: &[u8] = b"http://lv2plug.in/ns/ext/buf-size#minBlockLength\0";
const MAX_BLOCK: &[u8] = b"http://lv2plug.in/ns/ext/buf-size#maxBlockLength\0";
const NOMINAL_BLOCK: &[u8] = b"http://lv2plug.in/ns/ext/buf-size#nominalBlockLength\0";
const ATOM_FLOAT: &[u8] = b"http://lv2plug.in/ns/ext/atom#Float\0";
const ATOM_INT: &[u8] = b"http://lv2plug.in/ns/ext/atom#Int\0";

/// Features this host provides; any other `lv2:requiredFeature` is refused.
pub const PROVIDED: [&str; 6] = [
    URID_MAP,
    URID_UNMAP,
    OPTIONS,
    BOUNDED_BLOCK,
    "http://lv2plug.in/ns/lv2core#isLive",
    "http://lv2plug.in/ns/lv2core#hardRTCapable",
];

pub const MAGIC: &[u8; 8] = b"MOANLV2B";
pub const MAX_ENTRIES: usize = 64;
const MAX_NAME: usize = 128;

pub type Handle = *mut c_void;

#[repr(C)]
pub struct Feature {
    pub uri: *const c_char,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct Descriptor {
    pub uri: *const c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(*const Descriptor, f64, *const c_char, *const *const Feature) -> Handle,
    >,
    pub connect_port: Option<unsafe extern "C" fn(Handle, u32, *mut c_void)>,
    pub activate: Option<unsafe extern "C" fn(Handle)>,
    pub run: Option<unsafe extern "C" fn(Handle, u32)>,
    pub deactivate: Option<unsafe extern "C" fn(Handle)>,
    pub cleanup: Option<unsafe extern "C" fn(Handle)>,
    pub extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

/// `lv2_descriptor(index)`, the library's required export.
pub type Entry = unsafe extern "C" fn(u32) -> *const Descriptor;

#[repr(C)]
pub struct UridMap {
    pub handle: *mut c_void,
    pub map: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> u32>,
}
#[repr(C)]
pub struct UridUnmap {
    pub handle: *mut c_void,
    pub unmap: Option<unsafe extern "C" fn(*mut c_void, u32) -> *const c_char>,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptionValue {
    pub context: u32,
    pub subject: u32,
    pub key: u32,
    pub size: u32,
    pub kind: u32,
    pub value: *const c_void,
}

// ---------------------------------------------------------------- container

/// A packed LV2 bundle: `MOANLV2B`, u32 count, then per entry
/// u16 name length, u32 data length, name bytes, data bytes. Names are flat
/// bundle-relative file names; nothing else is trusted.
pub struct Bundle<'a> {
    entries: Vec<(&'a [u8], &'a [u8])>,
}

impl<'a> Bundle<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        let bad = Error::Descriptor;
        if bytes.len() < 12 || &bytes[..8] != MAGIC {
            return Err(bad);
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if count == 0 || count > MAX_ENTRIES {
            return Err(bad);
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(count).map_err(|_| Error::Memory)?;
        let mut at = 12;
        for _ in 0..count {
            let head = bytes.get(at..at + 6).ok_or(bad)?;
            let n = u16::from_le_bytes([head[0], head[1]]) as usize;
            let len = u32::from_le_bytes(head[2..6].try_into().unwrap()) as usize;
            at += 6;
            let name = bytes.get(at..at.checked_add(n).ok_or(bad)?).ok_or(bad)?;
            at += n;
            let data = bytes.get(at..at.checked_add(len).ok_or(bad)?).ok_or(bad)?;
            at += len;
            if !valid_name(name) || entries.iter().any(|(e, _)| *e == name) {
                return Err(bad);
            }
            entries.push((name, data));
        }
        if at != bytes.len() {
            return Err(bad);
        }
        Ok(Self { entries })
    }
    pub fn get(&self, name: &[u8]) -> Option<&'a [u8]> {
        let name = name.strip_prefix(b"./").unwrap_or(name);
        self.entries
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, d)| *d)
    }
}

fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with(b".")
        && name
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Pack bundle files for the helper. Used by Moan and by build scripts.
pub fn encode(files: &[(&[u8], &[u8])]) -> Result<Vec<u8>, Error> {
    if files.is_empty() || files.len() > MAX_ENTRIES {
        return Err(Error::Descriptor);
    }
    let mut total = 12usize;
    for (name, data) in files {
        if !valid_name(name) || data.len() > u32::MAX as usize {
            return Err(Error::Descriptor);
        }
        total = total
            .checked_add(6 + name.len() + data.len())
            .ok_or(Error::Memory)?;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| Error::Memory)?;
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, data) in files {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(data);
    }
    Ok(out)
}

// ---------------------------------------------------------------- description

/// What the host needs from a bundle's Turtle before loading native code.
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    /// NUL-terminated plugin URI, compared with each `lv2_descriptor` URI.
    pub uri: Vec<u8>,
    /// Bundle-relative name of the shared library.
    pub binary: String,
    pub ports: Vec<Port>,
    pub required: Vec<String>,
}

fn lv2(name: &str) -> String {
    let mut s = String::from(LV2);
    s.push_str(name);
    s
}

/// Bundle files a packer must include besides manifest.ttl: the first
/// plugin's library and its bundle-local `rdfs:seeAlso` data files.
pub fn manifest_files(manifest: &[u8]) -> Result<Vec<String>, Error> {
    let mut graph = Graph::default();
    graph.parse(manifest).map_err(|_| Error::Descriptor)?;
    let subject = graph
        .instances(&lv2("Plugin"))
        .into_iter()
        .next()
        .ok_or(Error::Descriptor)?;
    let mut out = Vec::new();
    let binary = graph
        .object(&subject, &lv2("binary"))
        .and_then(Term::iri)
        .ok_or(Error::Descriptor)?;
    out.push(String::from(binary.strip_prefix("./").unwrap_or(binary)));
    for name in graph.objects(&subject, RDFS_SEE_ALSO).filter_map(Term::iri) {
        let name = name.strip_prefix("./").unwrap_or(name);
        if !name.contains(':') && !out.iter().any(|o| o == name) {
            out.push(String::from(name));
        }
    }
    if out.iter().any(|n| !valid_name(n.as_bytes())) || out.len() + 1 > MAX_ENTRIES {
        return Err(Error::Descriptor);
    }
    Ok(out)
}

/// Read manifest.ttl and the files it names, and describe the first plugin.
pub fn describe(bundle: &Bundle) -> Result<Plugin, Error> {
    let mut graph = Graph::default();
    let manifest = bundle.get(b"manifest.ttl").ok_or(Error::Descriptor)?;
    graph.parse(manifest).map_err(|_| Error::Descriptor)?;
    let plugins = graph.instances(&lv2("Plugin"));
    // Explicitly the first plugin of the bundle, as for CLAP and LADSPA.
    let subject = plugins.first().ok_or(Error::Descriptor)?.clone();
    let uri = subject.iri().ok_or(Error::Descriptor)?;
    let binary = graph
        .object(&subject, &lv2("binary"))
        .and_then(Term::iri)
        .ok_or(Error::Descriptor)?;
    let binary = String::from(binary.strip_prefix("./").unwrap_or(binary));
    let mut extra: Vec<String> = graph
        .objects(&subject, RDFS_SEE_ALSO)
        .filter_map(Term::iri)
        .map(String::from)
        .collect();
    extra.dedup();
    for name in &extra {
        // seeAlso files outside the bundle (absolute IRIs) are never fetched.
        if let Some(text) = bundle.get(name.as_bytes()) {
            graph.parse(text).map_err(|_| Error::Descriptor)?;
        } else if !name.contains(':') {
            return Err(Error::Descriptor);
        }
    }
    let required: Vec<String> = graph
        .objects(&subject, &lv2("requiredFeature"))
        .filter_map(Term::iri)
        .map(String::from)
        .collect();
    let mut ports: Vec<Option<Port>> = Vec::new();
    for node in graph.objects(&subject, &lv2("port")) {
        let index = graph
            .object(node, &lv2("index"))
            .and_then(Term::number)
            .filter(|n| *n >= 0.0 && *n < MAX_PORTS as f64 && *n == libm::floor(*n))
            .ok_or(Error::Ports)? as usize;
        let is = |class: &str| graph.has(node, RDF_TYPE, &lv2(class));
        let input = is("InputPort");
        if input == is("OutputPort") {
            return Err(Error::Ports);
        }
        let optional = graph.has(node, &lv2("portProperty"), &lv2("connectionOptional"));
        let kind = if is("AudioPort") {
            if input {
                Kind::AudioIn
            } else {
                Kind::AudioOut
            }
        } else if is("ControlPort") {
            if input {
                Kind::ControlIn
            } else {
                Kind::ControlOut
            }
        } else if is("CVPort") {
            if input {
                Kind::CvIn
            } else {
                Kind::CvOut
            }
        } else if optional {
            Kind::Unused
        } else {
            // Atom/event ports (MIDI, patch messages) are not driven yet.
            return Err(Error::Ports);
        };
        let mut p = Port::new(index as u32, kind);
        let num = |name: &str| {
            graph
                .object(node, &lv2(name))
                .and_then(Term::number)
                .map(|v| v as f32)
                .filter(|v| v.is_finite())
        };
        if let Some(v) = num("minimum") {
            p.min = v;
        }
        if let Some(v) = num("maximum") {
            p.max = v;
        }
        if p.min > p.max {
            return Err(Error::Ports);
        }
        let sample_rate = graph.has(node, &lv2("portProperty"), &lv2("sampleRate"));
        if sample_rate {
            p.min *= RATE as f32;
            p.max *= RATE as f32;
        }
        p.toggled = graph.has(node, &lv2("portProperty"), &lv2("toggled"));
        // lv2:enumeration scale points may be fractional, so it does not imply rounding.
        p.integer = graph.has(node, &lv2("portProperty"), &lv2("integer"));
        p.default = match num("default") {
            Some(v) if sample_rate => v * RATE as f32,
            Some(v) => v,
            None if p.min.is_finite() => p.min,
            None => 0.0,
        };
        if ports.len() <= index {
            ports.resize(index + 1, None);
        }
        if ports[index].replace(p).is_some() {
            return Err(Error::Ports);
        }
    }
    let ports: Vec<Port> = ports
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::Ports)?;
    let mut c = Vec::from(uri.as_bytes());
    if c.contains(&0) {
        return Err(Error::Descriptor);
    }
    c.push(0);
    Ok(Plugin {
        uri: c,
        binary,
        ports,
        required,
    })
}

// ---------------------------------------------------------------- features

const MAX_URIDS: usize = 4096;

/// URID table owned by the host. IDs start at 1; 0 means "not mapped".
struct Urids {
    strings: Vec<Box<[u8]>>,
}

unsafe extern "C" fn map(handle: *mut c_void, uri: *const c_char) -> u32 {
    if handle.is_null() || uri.is_null() {
        return 0;
    }
    let table = &mut *(handle as *mut Urids);
    let s = core::ffi::CStr::from_ptr(uri).to_bytes_with_nul();
    if s.len() > 4096 {
        return 0;
    }
    if let Some(i) = table.strings.iter().position(|e| &e[..] == s) {
        return i as u32 + 1;
    }
    if table.strings.len() >= MAX_URIDS || table.strings.try_reserve(1).is_err() {
        return 0;
    }
    table.strings.push(Box::from(s));
    table.strings.len() as u32
}

unsafe extern "C" fn unmap(handle: *mut c_void, id: u32) -> *const c_char {
    if handle.is_null() || id == 0 {
        return core::ptr::null();
    }
    let table = &*(handle as *const Urids);
    table
        .strings
        .get(id as usize - 1)
        .map_or(core::ptr::null(), |s| s.as_ptr().cast())
}

/// Everything a plugin's feature pointers refer to. Boxed so the addresses
/// handed to the plugin stay put for the whole instance lifetime.
struct Host {
    urids: Box<Urids>,
    map: Box<UridMap>,
    unmap: Box<UridUnmap>,
    rate: Box<f32>,
    block: Box<[i32; 2]>,
    options: Box<[OptionValue; 5]>,
    features: Box<[Feature; 4]>,
    list: Box<[*const Feature; 5]>,
}

impl Host {
    fn new() -> Self {
        let mut urids = Box::new(Urids {
            strings: Vec::new(),
        });
        let handle = (&mut *urids as *mut Urids).cast();
        let map_feature = Box::new(UridMap {
            handle,
            map: Some(map),
        });
        let unmap_feature = Box::new(UridUnmap {
            handle,
            unmap: Some(unmap),
        });
        let mut this = Self {
            urids,
            map: map_feature,
            unmap: unmap_feature,
            rate: Box::new(RATE as f32),
            block: Box::new([1, BLOCK as i32]),
            options: Box::new(
                [OptionValue {
                    context: 0,
                    subject: 0,
                    key: 0,
                    size: 0,
                    kind: 0,
                    value: core::ptr::null(),
                }; 5],
            ),
            features: Box::new(
                [const {
                    Feature {
                        uri: core::ptr::null(),
                        data: core::ptr::null_mut(),
                    }
                }; 4],
            ),
            list: Box::new([core::ptr::null(); 5]),
        };
        let id = |h: &mut Self, s: &[u8]| unsafe { map(handle_of(h), s.as_ptr().cast()) };
        fn handle_of(h: &mut Host) -> *mut c_void {
            (&mut *h.urids as *mut Urids).cast()
        }
        let (float, int) = (id(&mut this, ATOM_FLOAT), id(&mut this, ATOM_INT));
        let keys = [
            id(&mut this, SAMPLE_RATE),
            id(&mut this, MIN_BLOCK),
            id(&mut this, MAX_BLOCK),
            id(&mut this, NOMINAL_BLOCK),
        ];
        let rate: *const f32 = &*this.rate;
        let block: *const [i32; 2] = &*this.block;
        let values: [(u32, u32, *const c_void); 4] = [
            (keys[0], float, rate.cast()),
            (keys[1], int, block.cast()),
            (keys[2], int, unsafe { (block as *const i32).add(1).cast() }),
            (keys[3], int, unsafe { (block as *const i32).add(1).cast() }),
        ];
        for (i, (key, kind, value)) in values.into_iter().enumerate() {
            // Context 0 = LV2_OPTIONS_INSTANCE; the final entry stays all-zero.
            this.options[i] = OptionValue {
                context: 0,
                subject: 0,
                key,
                size: 4,
                kind,
                value,
            };
        }
        let uris: [&[u8]; 4] = [
            b"http://lv2plug.in/ns/ext/urid#map\0",
            b"http://lv2plug.in/ns/ext/urid#unmap\0",
            b"http://lv2plug.in/ns/ext/options#options\0",
            b"http://lv2plug.in/ns/ext/buf-size#boundedBlockLength\0",
        ];
        let datas: [*mut c_void; 4] = [
            (&mut *this.map as *mut UridMap).cast(),
            (&mut *this.unmap as *mut UridUnmap).cast(),
            this.options.as_mut_ptr().cast(),
            core::ptr::null_mut(),
        ];
        for i in 0..4 {
            this.features[i] = Feature {
                uri: uris[i].as_ptr().cast(),
                data: datas[i],
            };
            this.list[i] = &this.features[i];
        }
        this
    }
}

// ---------------------------------------------------------------- processor

pub struct Processor {
    desc: *const Descriptor,
    handles: [Handle; 2],
    instances: usize,
    ports: Vec<Port>,
    channels: Channels,
    values: Vec<f32>,
    active: bool,
    _host: Host,
}

impl Processor {
    /// Find the descriptor whose URI matches `plugin` and create its instances.
    ///
    /// # Safety
    /// `entry` must be the relocated `lv2_descriptor` of the library named by
    /// `plugin.binary`, whose mapping outlives this object. Child domain only.
    pub unsafe fn open(entry: Entry, plugin: &Plugin, parameters: &[Parameter]) -> Result<Self, Error> {
        for f in &plugin.required {
            if !PROVIDED.contains(&f.as_str()) {
                return Err(Error::Feature);
            }
        }
        let mut desc = core::ptr::null();
        for i in 0..64 {
            let d = entry(i);
            if d.is_null() {
                break;
            }
            if !(*d).uri.is_null()
                && core::ffi::CStr::from_ptr((*d).uri).to_bytes_with_nul() == &plugin.uri[..]
            {
                desc = d;
                break;
            }
        }
        Self::open_descriptor(desc, plugin, parameters)
    }

    /// # Safety
    /// As [`Processor::open`], for an already selected descriptor.
    pub unsafe fn open_descriptor(
        desc: *const Descriptor,
        plugin: &Plugin,
        parameters: &[Parameter],
    ) -> Result<Self, Error> {
        if desc.is_null() {
            return Err(Error::Descriptor);
        }
        let d = &*desc;
        if d.instantiate.is_none() || d.connect_port.is_none() || d.run.is_none() {
            return Err(Error::Descriptor);
        }
        let channels = native::plan(&plugin.ports)?;
        let values = native::controls(&plugin.ports, parameters)?;
        let host = Host::new();
        let mut this = Self {
            desc,
            handles: [core::ptr::null_mut(); 2],
            instances: 0,
            ports: plugin.ports.clone(),
            channels,
            values,
            active: false,
            _host: host,
        };
        for i in 0..channels.instances() {
            let h = (d.instantiate.unwrap())(
                desc,
                RATE_F64,
                c"/moan/lv2/".as_ptr(),
                this._host.list.as_ptr(),
            );
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
        ((*self.desc).connect_port.unwrap())(self.handles[instance], port, data.cast())
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
        ((*self.desc).run.unwrap())(self.handles[instance], frames)
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

    const MANIFEST: &[u8] = br#"@prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
<http://lv2plug.in/plugins/eg-amp> a lv2:Plugin ; lv2:binary <amp.so> ; rdfs:seeAlso <amp.ttl> .
"#;
    const DATA: &[u8] = br#"@prefix lv2: <http://lv2plug.in/ns/lv2core#> .
@prefix urid: <http://lv2plug.in/ns/ext/urid#> .
<http://lv2plug.in/plugins/eg-amp> a lv2:Plugin ;
  lv2:requiredFeature urid:map ;
  lv2:port [ a lv2:InputPort , lv2:ControlPort ; lv2:index 0 ; lv2:symbol "gain" ;
             lv2:default 0.0 ; lv2:minimum -90.0 ; lv2:maximum 24.0 ] ,
           [ a lv2:AudioPort , lv2:InputPort ; lv2:index 1 ; lv2:symbol "in" ] ,
           [ a lv2:AudioPort , lv2:OutputPort ; lv2:index 2 ; lv2:symbol "out" ] .
"#;
    fn packed() -> Vec<u8> {
        encode(&[
            (b"manifest.ttl", MANIFEST),
            (b"amp.ttl", DATA),
            (b"amp.so", b"\x7fELF"),
        ])
        .unwrap()
    }

    #[test]
    fn bundle_describes_ports_and_binary() {
        let bytes = packed();
        let b = Bundle::decode(&bytes).unwrap();
        let p = describe(&b).unwrap();
        assert_eq!(p.uri, b"http://lv2plug.in/plugins/eg-amp\0");
        assert_eq!(p.binary, "amp.so");
        assert_eq!(b.get(p.binary.as_bytes()), Some(&b"\x7fELF"[..]));
        assert_eq!(p.required, alloc::vec![String::from(URID_MAP)]);
        assert_eq!(p.ports.len(), 3);
        assert_eq!(p.ports[0].kind, Kind::ControlIn);
        assert_eq!((p.ports[0].min, p.ports[0].max), (-90.0, 24.0));
        assert_eq!(p.ports[2].kind, Kind::AudioOut);
    }
    #[test]
    fn manifest_lists_files_to_pack() {
        assert_eq!(
            manifest_files(MANIFEST).unwrap(),
            alloc::vec![String::from("amp.so"), String::from("amp.ttl")]
        );
        assert!(manifest_files(b"<urn:x> <urn:y> <urn:z> .").is_err());
    }
    #[test]
    fn container_rejects_tricks() {
        let mut bytes = packed();
        bytes.push(0);
        assert!(Bundle::decode(&bytes).is_err(), "trailing data");
        assert!(encode(&[(b"../x.ttl", b"")]).is_err());
        assert!(encode(&[(b"a/b.ttl", b"")]).is_err());
        let dup = encode(&[(b"a.ttl", b""), (b"b.ttl", b"")]).unwrap();
        let mut dup = dup;
        let at = dup.windows(5).rposition(|w| w == b"b.ttl").unwrap();
        dup[at] = b'a';
        assert!(Bundle::decode(&dup).is_err(), "duplicate names");
        let missing = encode(&[(b"manifest.ttl", MANIFEST)]).unwrap();
        assert!(describe(&Bundle::decode(&missing).unwrap()).is_err(), "seeAlso missing");
    }

    #[repr(C)]
    struct Amp {
        gain: *const f32,
        input: *const f32,
        output: *mut f32,
        map: *const UridMap,
    }
    unsafe extern "C" fn instantiate(
        _: *const Descriptor,
        rate: f64,
        _: *const c_char,
        features: *const *const Feature,
    ) -> Handle {
        assert_eq!(rate, 48_000.0);
        let mut map_ptr = core::ptr::null();
        let mut f = features;
        while !(*f).is_null() {
            let uri = core::ffi::CStr::from_ptr((**f).uri).to_bytes();
            if uri == URID_MAP.as_bytes() {
                map_ptr = (**f).data as *const UridMap;
            }
            if uri == OPTIONS.as_bytes() {
                let mut o = (**f).data as *const OptionValue;
                let mut seen = 0;
                while (*o).key != 0 {
                    seen += 1;
                    o = o.add(1);
                }
                assert_eq!(seen, 4);
                let first = &*((**f).data as *const OptionValue);
                assert_eq!(*(first.value as *const f32), 48_000.0);
            }
            f = f.add(1);
        }
        if map_ptr.is_null() {
            return core::ptr::null_mut();
        }
        let m = &*map_ptr;
        let a = (m.map.unwrap())(m.handle, c"urn:test".as_ptr());
        let b = (m.map.unwrap())(m.handle, c"urn:test".as_ptr());
        assert!(a != 0 && a == b);
        Box::into_raw(Box::new(Amp {
            gain: core::ptr::null(),
            input: core::ptr::null(),
            output: core::ptr::null_mut(),
            map: map_ptr,
        }))
        .cast()
    }
    unsafe extern "C" fn connect(h: Handle, port: u32, data: *mut c_void) {
        let a = &mut *(h as *mut Amp);
        match port {
            0 => a.gain = data.cast(),
            1 => a.input = data.cast(),
            _ => a.output = data.cast(),
        }
    }
    unsafe extern "C" fn run(h: Handle, n: u32) {
        let a = &*(h as *mut Amp);
        assert!(!a.map.is_null());
        let coef = libm::powf(10.0, *a.gain * 0.05);
        for i in 0..n as usize {
            *a.output.add(i) = *a.input.add(i) * coef;
        }
    }
    unsafe extern "C" fn cleanup(h: Handle) {
        drop(Box::from_raw(h as *mut Amp));
    }
    static DESC: Descriptor = Descriptor {
        uri: c"http://lv2plug.in/plugins/eg-amp".as_ptr(),
        instantiate: Some(instantiate),
        connect_port: Some(connect),
        activate: None,
        run: Some(run),
        deactivate: None,
        cleanup: Some(cleanup),
        extension_data: None,
    };
    unsafe extern "C" fn entry(i: u32) -> *const Descriptor {
        if i == 0 {
            &DESC
        } else {
            core::ptr::null()
        }
    }
    unsafe impl core::marker::Sync for Descriptor {}

    #[test]
    fn amp_runs_with_features() {
        let bytes = packed();
        let p = describe(&Bundle::decode(&bytes).unwrap()).unwrap();
        let mut amp = unsafe {
            Processor::open(entry, &p, &[Parameter { id: 0, value: -6.0206 }]).unwrap()
        };
        let out = amp.render(&alloc::vec![[0.5, -0.5]; 300]).unwrap();
        assert!(out.iter().all(|f| (f[0] - 0.25).abs() < 1e-4 && (f[1] + 0.25).abs() < 1e-4));
    }
    #[test]
    fn unknown_required_feature_is_refused() {
        let bytes = packed();
        let mut p = describe(&Bundle::decode(&bytes).unwrap()).unwrap();
        p.required.push("http://example.org/worker".into());
        assert!(matches!(
            unsafe { Processor::open(entry, &p, &[]) },
            Err(Error::Feature)
        ));
    }
}
