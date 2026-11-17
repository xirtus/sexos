//! Safe pure-Rust LV2 Turtle (`.ttl`) manifest/plugin parser and narrow C ABI
//! `LV2_Descriptor` (`lv2_descriptor(index)`) offline/block processor adapter.
//!
//! Scanning LV2 bundles (`Lv2Manifest::parse_ttl` / `Lv2Manifest::parse_bundle`) never loads
//! or executes any native shared-object code. Execution via [`Lv2Processor`] matches `clap.rs`
//! and `vst.rs` confinement invariants: pre-allocated stereo `f32` ports, zero heap allocation
//! inside `run()`, and strict `NaN`/`inf`/`[-16.0, 16.0]` output clamping.

use crate::job::{buffer, BLOCK, MAX_FRAMES, MAX_PARAMS, RATE};
use alloc::{
    boxed::Box,
    format,
    string::String,
    vec,
    vec::Vec,
};
use core::{
    ffi::{c_char, c_void, CStr},
    ptr,
};

pub const MAX_TTL_BYTES: usize = 1024 * 1024;
pub const MAX_TURTLE_TOKENS: usize = 65_536;
pub const MAX_TURTLE_DEPTH: usize = 16;
pub const MAX_PLUGINS: usize = 128;
pub const MAX_PORTS: usize = 256;
pub const MAX_PRESETS: usize = 512;
pub const MAX_URI_LEN: usize = 512;
pub const MAX_DESCRIPTORS: u32 = 64;

const LV2_CORE: &str = "http://lv2plug.in/ns/lv2core#";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS_NS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const DOAP_NS: &str = "http://usefulinc.com/ns/doap#";
const PSET_NS: &str = "http://lv2plug.in/ns/ext/presets#";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Memory,
    Bounds,
    Manifest,
    Plugin,
    Ports,
    Lifecycle,
    Frames,
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lv2PortKind {
    AudioIn,
    AudioOut,
    ControlIn,
    ControlOut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2ScalePoint {
    pub label: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2Port {
    pub index: u32,
    pub symbol: String,
    pub name: String,
    pub kind: Lv2PortKind,
    pub default_value: f32,
    pub min_value: Option<f32>,
    pub max_value: Option<f32>,
    pub is_integer: bool,
    pub is_toggled: bool,
    pub is_logarithmic: bool,
    pub is_optional: bool,
    pub scale_points: Vec<Lv2ScalePoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2PresetValue {
    pub symbol: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2Preset {
    pub uri: String,
    pub label: String,
    pub applies_to: String,
    pub bank_uri: Option<String>,
    pub values: Vec<Lv2PresetValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2PresetBank {
    pub uri: String,
    pub label: String,
    pub presets: Vec<Lv2Preset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2PluginEntry {
    pub uri: String,
    pub binary: String,
    pub see_also: Vec<String>,
    pub classes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lv2PluginInfo {
    pub uri: String,
    pub name: String,
    pub binary: String,
    pub classes: Vec<String>,
    pub see_also: Vec<String>,
    pub ports: Vec<Lv2Port>,
    pub presets: Vec<Lv2Preset>,
    pub preset_banks: Vec<Lv2PresetBank>,
}

impl Lv2PluginInfo {
    pub fn audio_inputs(&self) -> impl Iterator<Item = &Lv2Port> {
        self.ports
            .iter()
            .filter(|p| p.kind == Lv2PortKind::AudioIn)
    }

    pub fn audio_outputs(&self) -> impl Iterator<Item = &Lv2Port> {
        self.ports
            .iter()
            .filter(|p| p.kind == Lv2PortKind::AudioOut)
    }

    pub fn control_inputs(&self) -> impl Iterator<Item = &Lv2Port> {
        self.ports
            .iter()
            .filter(|p| p.kind == Lv2PortKind::ControlIn)
    }

    pub fn control_outputs(&self) -> impl Iterator<Item = &Lv2Port> {
        self.ports
            .iter()
            .filter(|p| p.kind == Lv2PortKind::ControlOut)
    }

    pub fn is_stereo_capable(&self) -> bool {
        let ain = self.audio_inputs().count();
        let aout = self.audio_outputs().count();
        (ain == 1 || ain == 2) && (aout == 1 || aout == 2)
    }

    pub fn default_controls(&self) -> Vec<(u32, f32)> {
        self.control_inputs()
            .map(|p| (p.index, p.default_value))
            .collect()
    }
}

/// Pure-Rust scanned LV2 bundle manifest & plugin metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lv2Manifest {
    pub entries: Vec<Lv2PluginEntry>,
    pub plugins: Vec<Lv2PluginInfo>,
    pub presets: Vec<Lv2Preset>,
    pub preset_banks: Vec<Lv2PresetBank>,
}

impl Lv2Manifest {
    /// Parses a single `.ttl` document (such as `manifest.ttl`, a plugin `.ttl`, or combined `.ttl`).
    pub fn parse_ttl(ttl: &str) -> Result<Self, Error> {
        let mut manifest = Self::default();
        manifest.add_ttl(ttl)?;
        Ok(manifest)
    }

    /// Parses `manifest.ttl` together with all referenced `rdfs:seeAlso` plugin/preset `.ttl` files.
    pub fn parse_bundle(manifest_ttl: &str, extra_ttls: &[&str]) -> Result<Self, Error> {
        let mut manifest = Self::parse_ttl(manifest_ttl)?;
        for ttl in extra_ttls {
            manifest.add_ttl(ttl)?;
        }
        Ok(manifest)
    }

    /// Incrementally merges statements from another `.ttl` document into this manifest.
    pub fn add_ttl(&mut self, ttl: &str) -> Result<(), Error> {
        if ttl.len() > MAX_TTL_BYTES {
            return Err(Error::Bounds);
        }
        let doc = parse_turtle(ttl)?;
        self.merge_doc(&doc)
    }

    pub fn plugin(&self, uri: &str) -> Option<&Lv2PluginInfo> {
        self.plugins.iter().find(|p| p.uri == uri)
    }

    fn merge_doc(&mut self, doc: &TurtleDoc) -> Result<(), Error> {
        // 1. Collect all subjects that represent LV2 plugins or banks or presets
        let mut bank_labels: Vec<(String, String)> = Vec::new();
        for s in &doc.subjects {
            if is_bank_subject(doc, s) {
                let label = first_literal(doc, s, &format!("{RDFS_NS}label"))
                    .or_else(|| first_literal(doc, s, &format!("{DOAP_NS}name")))
                    .unwrap_or_else(|| String::from(s));
                bank_labels.push((s.clone(), label.clone()));
                if !self.preset_banks.iter().any(|b| b.uri == *s) {
                    self.preset_banks.push(Lv2PresetBank {
                        uri: s.clone(),
                        label,
                        presets: Vec::new(),
                    });
                }
            }
        }

        // 2. Extract presets
        for s in &doc.subjects {
            if !is_preset_subject(doc, s) {
                continue;
            }
            if self.presets.len() >= MAX_PRESETS {
                return Err(Error::Bounds);
            }
            let applies_to = first_iri(doc, s, &format!("{LV2_CORE}appliesTo")).unwrap_or_default();
            let label = first_literal(doc, s, &format!("{RDFS_NS}label"))
                .or_else(|| first_literal(doc, s, &format!("{DOAP_NS}name")))
                .unwrap_or_else(|| short_fragment(s));
            let bank_uri = first_iri(doc, s, &format!("{PSET_NS}bank"));

            let mut values = Vec::new();
            for port_node in all_objects(doc, s, &format!("{LV2_CORE}port")) {
                if let Some(sym) = first_literal(doc, port_node, &format!("{LV2_CORE}symbol")) {
                    if let Some(val) = first_f32(doc, port_node, &format!("{PSET_NS}value")) {
                        values.push(Lv2PresetValue { symbol: sym, value: val });
                    }
                }
            }

            let preset = Lv2Preset {
                uri: s.clone(),
                label,
                applies_to,
                bank_uri: bank_uri.clone(),
                values,
            };

            if let Some(existing) = self.presets.iter_mut().find(|p| p.uri == preset.uri) {
                if !preset.label.is_empty() {
                    existing.label = preset.label.clone();
                }
                if !preset.applies_to.is_empty() {
                    existing.applies_to = preset.applies_to.clone();
                }
                if preset.bank_uri.is_some() {
                    existing.bank_uri = preset.bank_uri.clone();
                }
                if !preset.values.is_empty() {
                    existing.values = preset.values.clone();
                }
            } else {
                self.presets.push(preset);
            }
        }

        // 3. Extract plugin entries & full plugin descriptions
        for s in &doc.subjects {
            if !is_plugin_subject(doc, s) {
                continue;
            }
            let binary = first_iri(doc, s, &format!("{LV2_CORE}binary"))
                .or_else(|| first_literal(doc, s, &format!("{LV2_CORE}binary")))
                .unwrap_or_default();
            let see_also = all_iris_or_literals(doc, s, &format!("{RDFS_NS}seeAlso"));
            let classes = all_iris_or_literals(doc, s, &format!("{RDF_NS}type"));
            let name = first_literal(doc, s, &format!("{DOAP_NS}name"))
                .or_else(|| first_literal(doc, s, &format!("{RDFS_NS}label")))
                .unwrap_or_else(|| short_fragment(s));

            // Update or insert manifest entry
            if let Some(entry) = self.entries.iter_mut().find(|e| e.uri == *s) {
                if !binary.is_empty() {
                    entry.binary = binary.clone();
                }
                for sa in &see_also {
                    if !entry.see_also.contains(sa) {
                        entry.see_also.push(sa.clone());
                    }
                }
                for cl in &classes {
                    if !entry.classes.contains(cl) {
                        entry.classes.push(cl.clone());
                    }
                }
            } else {
                if self.entries.len() >= MAX_PLUGINS {
                    return Err(Error::Bounds);
                }
                self.entries.push(Lv2PluginEntry {
                    uri: s.clone(),
                    binary: binary.clone(),
                    see_also: see_also.clone(),
                    classes: classes.clone(),
                });
            }

            // Parse ports on this plugin
            let mut parsed_ports = Vec::new();
            for port_node in all_objects(doc, s, &format!("{LV2_CORE}port")) {
                if let Some(port) = extract_port(doc, port_node)? {
                    if parsed_ports.iter().any(|p: &Lv2Port| p.index == port.index) {
                        return Err(Error::Manifest);
                    }
                    if parsed_ports.len() >= MAX_PORTS {
                        return Err(Error::Bounds);
                    }
                    parsed_ports.push(port);
                }
            }
            parsed_ports.sort_by_key(|p| p.index);

            if let Some(info) = self.plugins.iter_mut().find(|p| p.uri == *s) {
                if !binary.is_empty() {
                    info.binary = binary;
                }
                if !name.is_empty() && (info.name.is_empty() || info.name == short_fragment(s)) {
                    info.name = name;
                }
                for sa in see_also {
                    if !info.see_also.contains(&sa) {
                        info.see_also.push(sa);
                    }
                }
                for cl in classes {
                    if !info.classes.contains(&cl) {
                        info.classes.push(cl);
                    }
                }
                if !parsed_ports.is_empty() {
                    info.ports = parsed_ports;
                }
            } else {
                self.plugins.push(Lv2PluginInfo {
                    uri: s.clone(),
                    name,
                    binary,
                    classes,
                    see_also,
                    ports: parsed_ports,
                    presets: Vec::new(),
                    preset_banks: Vec::new(),
                });
            }
        }

        // 4. Rebuild per-bank and per-plugin preset associations
        for bank in &mut self.preset_banks {
            bank.presets.clear();
        }
        for preset in &self.presets {
            if let Some(bank_uri) = &preset.bank_uri {
                if let Some(bank) = self.preset_banks.iter_mut().find(|b| &b.uri == bank_uri) {
                    bank.presets.push(preset.clone());
                } else {
                    let label = bank_labels
                        .iter()
                        .find(|(u, _)| u == bank_uri)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| short_fragment(bank_uri));
                    self.preset_banks.push(Lv2PresetBank {
                        uri: bank_uri.clone(),
                        label,
                        presets: vec![preset.clone()],
                    });
                }
            }
        }
        for plugin in &mut self.plugins {
            plugin.presets = self
                .presets
                .iter()
                .filter(|p| p.applies_to == plugin.uri)
                .cloned()
                .collect();
            let mut banks: Vec<Lv2PresetBank> = Vec::new();
            for p in &plugin.presets {
                let bank_uri = p
                    .bank_uri
                    .clone()
                    .unwrap_or_else(|| format!("{}#default-bank", plugin.uri));
                if let Some(b) = banks.iter_mut().find(|b| b.uri == bank_uri) {
                    b.presets.push(p.clone());
                } else {
                    let label = self
                        .preset_banks
                        .iter()
                        .find(|b| b.uri == bank_uri)
                        .map(|b| b.label.clone())
                        .unwrap_or_else(|| String::from("Default"));
                    banks.push(Lv2PresetBank {
                        uri: bank_uri,
                        label,
                        presets: vec![p.clone()],
                    });
                }
            }
            plugin.preset_banks = banks;
        }

        Ok(())
    }
}

fn short_fragment(uri: &str) -> String {
    if let Some((_, frag)) = uri.rsplit_once('#') {
        if !frag.is_empty() {
            return String::from(frag);
        }
    }
    if let Some((_, tail)) = uri.rsplit_once('/') {
        if !tail.is_empty() {
            return String::from(tail);
        }
    }
    String::from(uri)
}

fn is_plugin_subject(doc: &TurtleDoc, subject: &str) -> bool {
    if subject.starts_with("_:") {
        return false;
    }
    let types = all_iris_or_literals(doc, subject, &format!("{RDF_NS}type"));
    if types
        .iter()
        .any(|t| t == &format!("{PSET_NS}Preset") || t == &format!("{PSET_NS}Bank"))
    {
        return false;
    }
    let has_plugin_type = types.iter().any(|t| {
        t.starts_with(LV2_CORE) && (t.ends_with("Plugin") || t == &format!("{LV2_CORE}Plugin"))
    });
    let has_binary = first_iri(doc, subject, &format!("{LV2_CORE}binary")).is_some();
    if first_iri(doc, subject, &format!("{LV2_CORE}appliesTo")).is_some()
        && !has_plugin_type
        && !has_binary
    {
        return false;
    }
    let has_ports = !all_objects(doc, subject, &format!("{LV2_CORE}port")).is_empty();
    has_plugin_type || has_binary || has_ports
}

fn is_preset_subject(doc: &TurtleDoc, subject: &str) -> bool {
    if subject.starts_with("_:") {
        return false;
    }
    let types = all_iris_or_literals(doc, subject, &format!("{RDF_NS}type"));
    types.iter().any(|t| t == &format!("{PSET_NS}Preset"))
        || (first_iri(doc, subject, &format!("{LV2_CORE}appliesTo")).is_some()
            && !is_plugin_subject(doc, subject))
}

fn is_bank_subject(doc: &TurtleDoc, subject: &str) -> bool {
    if subject.starts_with("_:") {
        return false;
    }
    all_iris_or_literals(doc, subject, &format!("{RDF_NS}type"))
        .iter()
        .any(|t| t == &format!("{PSET_NS}Bank"))
}

fn extract_port(doc: &TurtleDoc, node: &str) -> Result<Option<Lv2Port>, Error> {
    let types = all_iris_or_literals(doc, node, &format!("{RDF_NS}type"));
    let is_audio = types.iter().any(|t| t == &format!("{LV2_CORE}AudioPort"));
    let is_control = types.iter().any(|t| t == &format!("{LV2_CORE}ControlPort"));
    let is_in = types.iter().any(|t| t == &format!("{LV2_CORE}InputPort"));
    let is_out = types.iter().any(|t| t == &format!("{LV2_CORE}OutputPort"));

    // Gracefully skip auxiliary AtomPort / EventPort / CVPort descriptors used by Calf/LSP/x42.
    let kind = if is_audio && is_in {
        Lv2PortKind::AudioIn
    } else if is_audio && is_out {
        Lv2PortKind::AudioOut
    } else if is_control && is_in {
        Lv2PortKind::ControlIn
    } else if is_control && is_out {
        Lv2PortKind::ControlOut
    } else {
        return Ok(None);
    };

    let index_f = first_f32(doc, node, &format!("{LV2_CORE}index")).ok_or(Error::Manifest)?;
    if !index_f.is_finite() || index_f < 0.0 || index_f > (MAX_PORTS as f32) {
        return Err(Error::Manifest);
    }
    let index = index_f as u32;
    let symbol = first_literal(doc, node, &format!("{LV2_CORE}symbol")).ok_or(Error::Manifest)?;
    let name = first_literal(doc, node, &format!("{LV2_CORE}name"))
        .or_else(|| first_literal(doc, node, &format!("{RDFS_NS}label")))
        .unwrap_or_else(|| symbol.clone());

    let min_value = first_f32(doc, node, &format!("{LV2_CORE}minimum"));
    let max_value = first_f32(doc, node, &format!("{LV2_CORE}maximum"));
    if let (Some(mn), Some(mx)) = (min_value, max_value) {
        if mn > mx {
            return Err(Error::Manifest);
        }
    }
    let mut default_value = first_f32(doc, node, &format!("{LV2_CORE}default")).unwrap_or(0.0);
    if let Some(mn) = min_value {
        if default_value < mn {
            default_value = mn;
        }
    }
    if let Some(mx) = max_value {
        if default_value > mx {
            default_value = mx;
        }
    }

    let props = all_iris_or_literals(doc, node, &format!("{LV2_CORE}portProperty"));
    let is_integer = props.iter().any(|p| p == &format!("{LV2_CORE}integer"));
    let is_toggled = props.iter().any(|p| p == &format!("{LV2_CORE}toggled"));
    let is_logarithmic = props
        .iter()
        .any(|p| p == "http://lv2plug.in/ns/ext/port-props#logarithmic");
    let is_optional = props
        .iter()
        .any(|p| p == &format!("{LV2_CORE}connectionOptional"));

    let mut scale_points = Vec::new();
    for sp_node in all_objects(doc, node, &format!("{LV2_CORE}scalePoint")) {
        if let (Some(lbl), Some(val)) = (
            first_literal(doc, sp_node, &format!("{RDFS_NS}label")),
            first_f32(doc, sp_node, &format!("{RDF_NS}value")),
        ) {
            scale_points.push(Lv2ScalePoint {
                label: lbl,
                value: val,
            });
        }
    }

    Ok(Some(Lv2Port {
        index,
        symbol,
        name,
        kind,
        default_value,
        min_value,
        max_value,
        is_integer,
        is_toggled,
        is_logarithmic,
        is_optional,
        scale_points,
    }))
}

// --- Narrow C ABI LV2_Descriptor & Lv2Processor ---

#[allow(non_camel_case_types)]
pub type LV2_Handle = *mut c_void;
pub type Lv2Handle = LV2_Handle;

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LV2_Feature {
    pub uri: *const c_char,
    pub data: *mut c_void,
}
pub type Lv2Feature = LV2_Feature;

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LV2_Descriptor {
    pub uri: *const c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const LV2_Descriptor,
            sample_rate: f64,
            bundle_path: *const c_char,
            features: *const *const LV2_Feature,
        ) -> LV2_Handle,
    >,
    pub connect_port:
        Option<unsafe extern "C" fn(instance: LV2_Handle, port: u32, data_location: *mut c_void)>,
    pub activate: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    pub run: Option<unsafe extern "C" fn(instance: LV2_Handle, n_samples: u32)>,
    pub deactivate: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    pub cleanup: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    pub extension_data: Option<unsafe extern "C" fn(uri: *const c_char) -> *const c_void>,
}
pub type Lv2Descriptor = LV2_Descriptor;
pub type Lv2DescriptorFn = unsafe extern "C" fn(index: u32) -> *const LV2_Descriptor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lv2DescriptorMeta {
    pub index: u32,
    pub uri: String,
}

/// Safely reads a bounded NUL-terminated C string from a plugin descriptor pointer.
unsafe fn bounded_cstr(ptr: *const c_char, max_len: usize) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while len < max_len {
        if *ptr.add(len) == 0 {
            break;
        }
        len += 1;
    }
    if len == max_len {
        return None;
    }
    let bytes = core::slice::from_raw_parts(ptr.cast::<u8>(), len);
    core::str::from_utf8(bytes).ok().map(String::from)
}

/// Enumerates descriptor URIs from an exported `lv2_descriptor` entry point inside a sandboxed helper.
///
/// # Safety
/// `entry` must be a valid `lv2_descriptor` function pointer from a relocated LV2 plugin module.
pub unsafe fn enumerate_descriptors(entry: Lv2DescriptorFn) -> Result<Vec<Lv2DescriptorMeta>, Error> {
    let mut out = Vec::new();
    for index in 0..MAX_DESCRIPTORS {
        let desc = entry(index);
        if desc.is_null() {
            break;
        }
        let uri = bounded_cstr((*desc).uri, MAX_URI_LEN).ok_or(Error::Plugin)?;
        out.push(Lv2DescriptorMeta { index, uri });
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
struct ControlPortSlot {
    port_index: u32,
    buf_slot: usize,
    min: Option<f32>,
    max: Option<f32>,
}

/// Confined offline/block LV2 audio processor matching `clap.rs` & `vst.rs` safety invariants:
/// - Pre-allocated stable stereo `f32` audio and control port buffers (never reallocated after `open`)
/// - Zero heap allocation inside `run_block()`
/// - Input validation (`finite && abs <= 16.0`) and strict output `NaN`/`inf`/`[-16.0, 16.0]` clamping
pub struct Lv2Processor {
    descriptor: *const LV2_Descriptor,
    handle: LV2_Handle,
    // Heap-boxed port buffers keep raw pointers passed to `connect_port` pinned across moves.
    in_bufs: Box<[[f32; BLOCK]; 2]>,
    out_bufs: Box<[[f32; BLOCK]; 2]>,
    audio_in_count: usize,
    audio_out_count: usize,
    control_in_slots: Vec<ControlPortSlot>,
    control_in_vals: Box<[f32]>,
    control_out_slots: Vec<ControlPortSlot>,
    control_out_vals: Box<[f32]>,
    active: bool,
    last_sanitized_count: usize,
}

#[inline]
fn sanitize_output(v: f32, sanitized_count: &mut usize) -> f32 {
    if !v.is_finite() {
        *sanitized_count += 1;
        0.0
    } else if v > 16.0 {
        *sanitized_count += 1;
        16.0
    } else if v < -16.0 {
        *sanitized_count += 1;
        -16.0
    } else {
        v
    }
}

impl Lv2Processor {
    /// Opens an LV2 plugin via `lv2_descriptor(descriptor_index)` and manifest `info`.
    ///
    /// # Safety
    /// `entry` must point to a relocated genuine LV2 `lv2_descriptor` export that outlives
    /// this `Lv2Processor`. Invoke native plugin binaries only inside an isolated helper.
    pub unsafe fn open(
        entry: Lv2DescriptorFn,
        descriptor_index: u32,
        info: &Lv2PluginInfo,
        bundle_path: &CStr,
    ) -> Result<Self, Error> {
        if descriptor_index >= MAX_DESCRIPTORS {
            return Err(Error::Plugin);
        }
        let desc = entry(descriptor_index);
        Self::open_descriptor(desc, info, bundle_path)
    }

    /// Opens an LV2 plugin directly from a validated `LV2_Descriptor` pointer and `Lv2PluginInfo`.
    ///
    /// # Safety
    /// `descriptor` must point to a valid, initialized `LV2_Descriptor` whose functions and static
    /// strings outlive this `Lv2Processor`.
    pub unsafe fn open_descriptor(
        descriptor: *const LV2_Descriptor,
        info: &Lv2PluginInfo,
        bundle_path: &CStr,
    ) -> Result<Self, Error> {
        if descriptor.is_null() {
            return Err(Error::Plugin);
        }
        let d = &*descriptor;
        let uri = bounded_cstr(d.uri, MAX_URI_LEN).ok_or(Error::Plugin)?;
        if !info.uri.is_empty() && uri != info.uri {
            return Err(Error::Plugin);
        }

        let instantiate = d.instantiate.ok_or(Error::Lifecycle)?;
        let connect_port = d.connect_port.ok_or(Error::Ports)?;
        let _run = d.run.ok_or(Error::Lifecycle)?;
        let _cleanup = d.cleanup.ok_or(Error::Lifecycle)?;

        let audio_ins: Vec<&Lv2Port> = info.audio_inputs().collect();
        let audio_outs: Vec<&Lv2Port> = info.audio_outputs().collect();
        if !(1..=2).contains(&audio_ins.len()) || !(1..=2).contains(&audio_outs.len()) {
            return Err(Error::Ports);
        }

        let control_ins: Vec<&Lv2Port> = info.control_inputs().collect();
        let control_outs: Vec<&Lv2Port> = info.control_outputs().collect();
        if control_ins.len() > MAX_PORTS || control_outs.len() > MAX_PORTS {
            return Err(Error::Ports);
        }

        let in_bufs = Box::new([[0.0f32; BLOCK]; 2]);
        let out_bufs = Box::new([[0.0f32; BLOCK]; 2]);

        let mut control_in_slots = Vec::new();
        control_in_slots
            .try_reserve_exact(control_ins.len())
            .map_err(|_| Error::Memory)?;
        let mut cin_vec = Vec::new();
        cin_vec
            .try_reserve_exact(control_ins.len())
            .map_err(|_| Error::Memory)?;
        for (slot_idx, p) in control_ins.iter().enumerate() {
            control_in_slots.push(ControlPortSlot {
                port_index: p.index,
                buf_slot: slot_idx,
                min: p.min_value,
                max: p.max_value,
            });
            cin_vec.push(p.default_value);
        }
        let control_in_vals = cin_vec.into_boxed_slice();

        let mut control_out_slots = Vec::new();
        control_out_slots
            .try_reserve_exact(control_outs.len())
            .map_err(|_| Error::Memory)?;
        let mut cout_vec = Vec::new();
        cout_vec
            .try_reserve_exact(control_outs.len())
            .map_err(|_| Error::Memory)?;
        for (slot_idx, p) in control_outs.iter().enumerate() {
            control_out_slots.push(ControlPortSlot {
                port_index: p.index,
                buf_slot: slot_idx,
                min: p.min_value,
                max: p.max_value,
            });
            cout_vec.push(0.0);
        }
        let control_out_vals = cout_vec.into_boxed_slice();

        let features: [*const LV2_Feature; 1] = [ptr::null()];
        let handle = instantiate(descriptor, RATE as f64, bundle_path.as_ptr(), features.as_ptr());
        if handle.is_null() {
            return Err(Error::Lifecycle);
        }

        let mut this = Self {
            descriptor,
            handle,
            in_bufs,
            out_bufs,
            audio_in_count: audio_ins.len(),
            audio_out_count: audio_outs.len(),
            control_in_slots,
            control_in_vals,
            control_out_slots,
            control_out_vals,
            active: false,
            last_sanitized_count: 0,
        };

        for (ch, port) in audio_ins.iter().enumerate() {
            connect_port(
                this.handle,
                port.index,
                this.in_bufs[ch].as_mut_ptr().cast::<c_void>(),
            );
        }
        for (ch, port) in audio_outs.iter().enumerate() {
            connect_port(
                this.handle,
                port.index,
                this.out_bufs[ch].as_mut_ptr().cast::<c_void>(),
            );
        }
        for slot in &this.control_in_slots {
            connect_port(
                this.handle,
                slot.port_index,
                (&mut this.control_in_vals[slot.buf_slot] as *mut f32).cast::<c_void>(),
            );
        }
        for slot in &this.control_out_slots {
            connect_port(
                this.handle,
                slot.port_index,
                (&mut this.control_out_vals[slot.buf_slot] as *mut f32).cast::<c_void>(),
            );
        }

        if let Some(activate) = d.activate {
            activate(this.handle);
        }
        this.active = true;
        Ok(this)
    }

    pub fn last_sanitized_count(&self) -> usize {
        self.last_sanitized_count
    }

    pub fn set_control(&mut self, port_index: u32, value: f32) -> Result<(), Error> {
        if !value.is_finite() {
            return Err(Error::Audio);
        }
        let slot = self
            .control_in_slots
            .iter()
            .find(|s| s.port_index == port_index)
            .ok_or(Error::Ports)?;
        let mut clamped = value;
        if let Some(mn) = slot.min {
            if clamped < mn {
                clamped = mn;
            }
        }
        if let Some(mx) = slot.max {
            if clamped > mx {
                clamped = mx;
            }
        }
        self.control_in_vals[slot.buf_slot] = clamped;
        Ok(())
    }

    pub fn control_output(&self, port_index: u32) -> Option<f32> {
        let slot = self
            .control_out_slots
            .iter()
            .find(|s| s.port_index == port_index)?;
        let v = self.control_out_vals[slot.buf_slot];
        Some(if v.is_finite() { v } else { 0.0 })
    }

    pub fn apply_preset(&mut self, preset: &Lv2Preset, info: &Lv2PluginInfo) -> Result<(), Error> {
        for pv in &preset.values {
            if let Some(port) = info
                .control_inputs()
                .find(|p| p.symbol == pv.symbol)
            {
                self.set_control(port.index, pv.value)?;
            }
        }
        Ok(())
    }

    /// Zero-allocation block execution using pre-connected stereo `f32` ports.
    /// Rejects empty / `> BLOCK` chunks or out-of-range inputs, and clamps plugin outputs
    /// (`NaN`/`inf` -> `0.0`, finite values clamped to `[-16.0, 16.0]`).
    pub fn run_block(
        &mut self,
        input: &[[f32; 2]],
        output: &mut [[f32; 2]],
    ) -> Result<usize, Error> {
        let n = input.len();
        if n == 0 || n > BLOCK || output.len() != n {
            return Err(Error::Frames);
        }
        if input
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 16.0)
        {
            return Err(Error::Audio);
        }

        if self.audio_in_count == 1 {
            for (i, f) in input.iter().enumerate() {
                self.in_bufs[0][i] = 0.5 * (f[0] + f[1]);
                self.out_bufs[0][i] = 0.0;
                self.out_bufs[1][i] = 0.0;
            }
        } else {
            for (i, f) in input.iter().enumerate() {
                self.in_bufs[0][i] = f[0];
                self.in_bufs[1][i] = f[1];
                self.out_bufs[0][i] = 0.0;
                self.out_bufs[1][i] = 0.0;
            }
        }

        // Zero heap allocations inside run().
        let run_fn = unsafe { (*self.descriptor).run.ok_or(Error::Lifecycle)? };
        unsafe { run_fn(self.handle, n as u32) };

        let mut sanitized = 0usize;
        if self.audio_out_count == 1 {
            for (i, dst) in output.iter_mut().enumerate() {
                let s = sanitize_output(self.out_bufs[0][i], &mut sanitized);
                *dst = [s, s];
            }
        } else {
            for (i, dst) in output.iter_mut().enumerate() {
                dst[0] = sanitize_output(self.out_bufs[0][i], &mut sanitized);
                dst[1] = sanitize_output(self.out_bufs[1][i], &mut sanitized);
            }
        }
        self.last_sanitized_count = sanitized;
        Ok(sanitized)
    }

    /// Offline render matching `clap.rs` / `vst.rs` bounds (`1..=MAX_FRAMES`), allocating the
    /// output buffer once upfront and performing zero heap allocations inside `run()`.
    pub fn render(&mut self, input: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, Error> {
        self.render_with_parameters(input, &[])
    }

    pub fn render_with_parameters(
        &mut self,
        input: &[[f32; 2]],
        parameters: &[crate::job::Parameter],
    ) -> Result<Vec<[f32; 2]>, Error> {
        if input.is_empty() || input.len() > MAX_FRAMES as usize {
            return Err(Error::Frames);
        }
        if parameters.len() > MAX_PARAMS
            || parameters.iter().any(|p| !p.value.is_finite())
        {
            return Err(Error::Audio);
        }
        for p in parameters {
            self.set_control(p.id, p.value as f32)?;
        }

        let mut out = buffer(input.len()).map_err(|_| Error::Memory)?;
        out.resize(input.len(), [0.0, 0.0]);
        let mut total_sanitized = 0usize;

        for (chunk_in, chunk_out) in input.chunks(BLOCK).zip(out.chunks_mut(BLOCK)) {
            total_sanitized += self.run_block(chunk_in, chunk_out)?;
        }
        self.last_sanitized_count = total_sanitized;
        Ok(out)
    }
}

impl Drop for Lv2Processor {
    fn drop(&mut self) {
        unsafe {
            if !self.descriptor.is_null() && !self.handle.is_null() {
                let d = &*self.descriptor;
                if self.active {
                    if let Some(deactivate) = d.deactivate {
                        deactivate(self.handle);
                    }
                }
                if let Some(cleanup) = d.cleanup {
                    cleanup(self.handle);
                }
            }
        }
    }
}

// --- Pure-Rust Bounded Turtle (.ttl) Lexer & Graph Parser ---

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Iri(String),
    Prefixed(String, String),
    Literal(String),
    Number(f32),
    A,
    Dot,
    Semi,
    Comma,
    LBracket,
    RBracket,
    LParen,
    RParen,
    PrefixKw,
    BaseKw,
    ColonLabel(String),
}

#[derive(Clone, Debug)]
struct Triple {
    subj: String,
    pred: String,
    obj: Obj,
}

#[derive(Clone, Debug)]
enum Obj {
    Iri(String),
    Literal(String),
    Number(f32),
}

struct TurtleDoc {
    subjects: Vec<String>,
    triples: Vec<Triple>,
}

impl TurtleDoc {
    fn push_triple(&mut self, subj: String, pred: String, obj: Obj) {
        if !self.subjects.contains(&subj) {
            self.subjects.push(subj.clone());
        }
        self.triples.push(Triple { subj, pred, obj });
    }
}

fn all_objects<'a>(doc: &'a TurtleDoc, subj: &str, pred: &str) -> Vec<&'a str> {
    doc.triples
        .iter()
        .filter(|t| t.subj == subj && t.pred == pred)
        .filter_map(|t| match &t.obj {
            Obj::Iri(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

fn first_iri(doc: &TurtleDoc, subj: &str, pred: &str) -> Option<String> {
    doc.triples
        .iter()
        .find(|t| t.subj == subj && t.pred == pred)
        .and_then(|t| match &t.obj {
            Obj::Iri(s) => Some(s.clone()),
            _ => None,
        })
}

fn first_literal(doc: &TurtleDoc, subj: &str, pred: &str) -> Option<String> {
    doc.triples
        .iter()
        .find(|t| t.subj == subj && t.pred == pred)
        .and_then(|t| match &t.obj {
            Obj::Literal(s) => Some(s.clone()),
            Obj::Iri(s) => Some(s.clone()),
            Obj::Number(n) => Some(format!("{n}")),
        })
}

fn first_f32(doc: &TurtleDoc, subj: &str, pred: &str) -> Option<f32> {
    doc.triples
        .iter()
        .find(|t| t.subj == subj && t.pred == pred)
        .and_then(|t| match &t.obj {
            Obj::Number(n) if n.is_finite() => Some(*n),
            Obj::Literal(s) => s.trim().parse::<f32>().ok().filter(|n| n.is_finite()),
            _ => None,
        })
}

fn all_iris_or_literals(doc: &TurtleDoc, subj: &str, pred: &str) -> Vec<String> {
    doc.triples
        .iter()
        .filter(|t| t.subj == subj && t.pred == pred)
        .map(|t| match &t.obj {
            Obj::Iri(s) | Obj::Literal(s) => s.clone(),
            Obj::Number(n) => format!("{n}"),
        })
        .collect()
}

fn tokenize_turtle(input: &str) -> Result<Vec<Tok>, Error> {
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut toks = Vec::new();

    while i < bytes.len() {
        if toks.len() >= MAX_TURTLE_TOKENS {
            return Err(Error::Bounds);
        }
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        match b {
            b'.' => {
                // Distinguish `.` statement terminator from `.5` decimal number
                if bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                    let (num, next) = read_number(input, i)?;
                    toks.push(Tok::Number(num));
                    i = next;
                } else {
                    toks.push(Tok::Dot);
                    i += 1;
                }
            }
            b';' => {
                toks.push(Tok::Semi);
                i += 1;
            }
            b',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            b'[' => {
                toks.push(Tok::LBracket);
                i += 1;
            }
            b']' => {
                toks.push(Tok::RBracket);
                i += 1;
            }
            b'(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            b')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            b'<' => {
                let (iri, next) = read_iri(input, i)?;
                toks.push(Tok::Iri(iri));
                i = next;
            }
            b'"' | b'\'' => {
                let (lit, mut next) = read_string(input, i)?;
                // Skip optional @lang or ^^datatype suffix on string literals
                if bytes.get(next) == Some(&b'@') {
                    next += 1;
                    while next < bytes.len()
                        && (bytes[next].is_ascii_alphanumeric() || bytes[next] == b'-')
                    {
                        next += 1;
                    }
                } else if bytes.get(next..next + 2) == Some(b"^^") {
                    next += 2;
                    if bytes.get(next) == Some(&b'<') {
                        let (_, after) = read_iri(input, next)?;
                        next = after;
                    } else {
                        while next < bytes.len()
                            && !bytes[next].is_ascii_whitespace()
                            && !matches!(bytes[next], b';' | b',' | b'.' | b']' | b')')
                        {
                            next += 1;
                        }
                    }
                }
                toks.push(Tok::Literal(lit));
                i = next;
            }
            b'@' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let kw = &input[start..i];
                if kw == "@prefix" {
                    toks.push(Tok::PrefixKw);
                } else if kw == "@base" {
                    toks.push(Tok::BaseKw);
                } else {
                    return Err(Error::Manifest);
                }
            }
            b'+' | b'-' | b'0'..=b'9' => {
                let (num, next) = read_number(input, i)?;
                toks.push(Tok::Number(num));
                i = next;
            }
            _ => {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(
                        bytes[i],
                        b';' | b',' | b'[' | b']' | b'(' | b')' | b'<' | b'"' | b'\'' | b'#'
                    )
                {
                    if bytes[i] == b'.' {
                        // Trailing `.` ends the statement unless followed by identifier char
                        if !bytes
                            .get(i + 1)
                            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
                        {
                            break;
                        }
                    }
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                let word = &input[start..i];
                if word == "a" {
                    toks.push(Tok::A);
                } else if word.eq_ignore_ascii_case("PREFIX") {
                    toks.push(Tok::PrefixKw);
                } else if word.eq_ignore_ascii_case("BASE") {
                    toks.push(Tok::BaseKw);
                } else if word == "true" {
                    toks.push(Tok::Number(1.0));
                } else if word == "false" {
                    toks.push(Tok::Number(0.0));
                } else if let Some((pref, local)) = word.split_once(':') {
                    if pref.ends_with(':') {
                        return Err(Error::Manifest);
                    }
                    if local.is_empty()
                        && matches!(toks.last(), Some(Tok::PrefixKw))
                    {
                        toks.push(Tok::ColonLabel(String::from(pref)));
                    } else if pref == "_" {
                        toks.push(Tok::Prefixed(String::from("_"), unescape_local(local)));
                    } else {
                        toks.push(Tok::Prefixed(String::from(pref), unescape_local(local)));
                    }
                } else {
                    return Err(Error::Manifest);
                }
            }
        }
    }

    Ok(toks)
}

fn unescape_local(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn read_iri(input: &str, start: usize) -> Result<(String, usize), Error> {
    let bytes = input.as_bytes();
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'>' {
            return Ok((out, i + 1));
        }
        if b == b'\\' {
            i += 1;
            let esc = *bytes.get(i).ok_or(Error::Manifest)?;
            out.push(esc as char);
            i += 1;
        } else if b < 0x20 {
            return Err(Error::Manifest);
        } else {
            let s = &input[i..];
            let ch = s.chars().next().ok_or(Error::Manifest)?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Err(Error::Manifest)
}

fn read_string(input: &str, start: usize) -> Result<(String, usize), Error> {
    let bytes = input.as_bytes();
    let quote = bytes[start];
    let long = bytes.get(start..start + 3) == Some(&[quote, quote, quote]);
    let mut i = start + if long { 3 } else { 1 };
    let mut out = String::new();

    while i < bytes.len() {
        if long {
            if bytes.get(i..i + 3) == Some(&[quote, quote, quote]) {
                return Ok((out, i + 3));
            }
        } else if bytes[i] == quote {
            return Ok((out, i + 1));
        } else if bytes[i] == b'\n' || bytes[i] == b'\r' {
            return Err(Error::Manifest);
        }

        if bytes[i] == b'\\' {
            i += 1;
            let esc = *bytes.get(i).ok_or(Error::Manifest)?;
            i += 1;
            match esc {
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'b' => out.push('\x08'),
                b'f' => out.push('\x0c'),
                b'"' => out.push('"'),
                b'\'' => out.push('\''),
                b'\\' => out.push('\\'),
                b'u' => {
                    let slice = input.get(i..i + 4).ok_or(Error::Manifest)?;
                    let code = u32::from_str_radix(slice, 16).map_err(|_| Error::Manifest)?;
                    let ch = char::from_u32(code).ok_or(Error::Manifest)?;
                    out.push(ch);
                    i += 4;
                }
                b'U' => {
                    let slice = input.get(i..i + 8).ok_or(Error::Manifest)?;
                    let code = u32::from_str_radix(slice, 16).map_err(|_| Error::Manifest)?;
                    let ch = char::from_u32(code).ok_or(Error::Manifest)?;
                    out.push(ch);
                    i += 8;
                }
                _ => out.push(esc as char),
            }
        } else {
            let s = &input[i..];
            let ch = s.chars().next().ok_or(Error::Manifest)?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Err(Error::Manifest)
}

fn read_number(input: &str, start: usize) -> Result<(f32, usize), Error> {
    let bytes = input.as_bytes();
    let mut i = start;
    if matches!(bytes.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let mut has_digit = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        has_digit = true;
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            has_digit = true;
            i += 1;
        }
    }
    if !has_digit {
        return Err(Error::Manifest);
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return Err(Error::Manifest);
        }
    }
    let num = input[start..i]
        .parse::<f32>()
        .map_err(|_| Error::Manifest)?;
    if !num.is_finite() {
        return Err(Error::Manifest);
    }
    Ok((num, i))
}

struct TurtleParser {
    toks: Vec<Tok>,
    pos: usize,
    prefixes: Vec<(String, String)>,
    base: String,
    next_bnode: usize,
    doc: TurtleDoc,
}

impl TurtleParser {
    fn new(toks: Vec<Tok>) -> Self {
        let prefixes = vec![
            (String::from("lv2"), String::from(LV2_CORE)),
            (String::from("rdf"), String::from(RDF_NS)),
            (String::from("rdfs"), String::from(RDFS_NS)),
            (String::from("doap"), String::from(DOAP_NS)),
            (String::from("pset"), String::from(PSET_NS)),
            (String::from("foaf"), String::from("http://xmlns.com/foaf/0.1/")),
            (String::from("xsd"), String::from("http://www.w3.org/2001/XMLSchema#")),
            (String::from("units"), String::from("http://lv2plug.in/ns/extensions/units#")),
            (String::from("atom"), String::from("http://lv2plug.in/ns/ext/atom#")),
        ];
        Self {
            toks,
            pos: 0,
            prefixes,
            base: String::new(),
            next_bnode: 0,
            doc: TurtleDoc {
                subjects: Vec::new(),
                triples: Vec::new(),
            },
        }
    }

    fn fresh_bnode(&mut self) -> String {
        let id = self.next_bnode;
        self.next_bnode += 1;
        format!("_:b{id}")
    }

    fn set_prefix(&mut self, prefix: String, uri: String) {
        if let Some(p) = self.prefixes.iter_mut().find(|(k, _)| *k == prefix) {
            p.1 = uri;
        } else {
            self.prefixes.push((prefix, uri));
        }
    }

    fn resolve_iri(&self, raw: &str) -> String {
        if self.base.is_empty()
            || raw.contains("://")
            || raw.starts_with("urn:")
            || raw.starts_with("_:")
        {
            String::from(raw)
        } else if raw.starts_with('#') {
            format!("{}{}", self.base.trim_end_matches('#'), raw)
        } else if let Some((prefix_base, _)) = self.base.rsplit_once('/') {
            format!("{prefix_base}/{raw}")
        } else {
            String::from(raw)
        }
    }

    fn resolve_prefixed(&self, pref: &str, local: &str) -> Result<String, Error> {
        if pref == "_" {
            return Ok(format!("_:{local}"));
        }
        let ns = self
            .prefixes
            .iter()
            .find(|(k, _)| k == pref)
            .map(|(_, v)| v.as_str())
            .ok_or(Error::Manifest)?;
        Ok(format!("{ns}{local}"))
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn parse_all(&mut self) -> Result<(), Error> {
        while let Some(tok) = self.peek().cloned() {
            match tok {
                Tok::PrefixKw => {
                    let sparql_style = self.is_sparql_directive();
                    self.pos += 1;
                    let pref = match self.toks.get(self.pos).cloned() {
                        Some(Tok::ColonLabel(p)) => p,
                        Some(Tok::Prefixed(p, l)) if l.is_empty() => p,
                        _ => return Err(Error::Manifest),
                    };
                    self.pos += 1;
                    let uri = match self.toks.get(self.pos).cloned() {
                        Some(Tok::Iri(u)) => u,
                        _ => return Err(Error::Manifest),
                    };
                    self.pos += 1;
                    self.set_prefix(pref, uri);
                    if matches!(self.peek(), Some(Tok::Dot)) {
                        self.pos += 1;
                    } else if !sparql_style {
                        return Err(Error::Manifest);
                    }
                }
                Tok::BaseKw => {
                    let sparql_style = self.is_sparql_directive();
                    self.pos += 1;
                    let uri = match self.toks.get(self.pos).cloned() {
                        Some(Tok::Iri(u)) => u,
                        _ => return Err(Error::Manifest),
                    };
                    self.pos += 1;
                    self.base = uri;
                    if matches!(self.peek(), Some(Tok::Dot)) {
                        self.pos += 1;
                    } else if !sparql_style {
                        return Err(Error::Manifest);
                    }
                }
                _ => {
                    let subj = self.parse_subject(0)?;
                    if matches!(self.peek(), Some(Tok::Dot)) {
                        self.pos += 1;
                        continue;
                    }
                    self.parse_predicate_object_list(&subj, 0)?;
                    match self.peek() {
                        Some(Tok::Dot) => self.pos += 1,
                        _ => return Err(Error::Manifest),
                    }
                }
            }
        }
        Ok(())
    }

    fn is_sparql_directive(&self) -> bool {
        // Both @prefix and PREFIX produce PrefixKw; `.` after IRI is optional in SPARQL mode.
        // We accept optional trailing `.` cleanly while requiring valid syntax structure.
        true
    }

    fn parse_subject(&mut self, depth: usize) -> Result<String, Error> {
        if depth > MAX_TURTLE_DEPTH {
            return Err(Error::Bounds);
        }
        let tok = self.toks.get(self.pos).cloned().ok_or(Error::Manifest)?;
        match tok {
            Tok::Iri(u) => {
                self.pos += 1;
                Ok(self.resolve_iri(&u))
            }
            Tok::Prefixed(p, l) => {
                self.pos += 1;
                self.resolve_prefixed(&p, &l)
            }
            Tok::LBracket => {
                self.pos += 1;
                let id = self.fresh_bnode();
                if !matches!(self.peek(), Some(Tok::RBracket)) {
                    self.parse_predicate_object_list(&id, depth + 1)?;
                }
                if !matches!(self.peek(), Some(Tok::RBracket)) {
                    return Err(Error::Manifest);
                }
                self.pos += 1;
                Ok(id)
            }
            Tok::LParen => self.parse_collection(depth + 1),
            _ => Err(Error::Manifest),
        }
    }

    fn parse_predicate_object_list(&mut self, subj: &str, depth: usize) -> Result<(), Error> {
        if depth > MAX_TURTLE_DEPTH {
            return Err(Error::Bounds);
        }
        loop {
            let pred = match self.toks.get(self.pos).cloned() {
                Some(Tok::A) => {
                    self.pos += 1;
                    format!("{RDF_NS}type")
                }
                Some(Tok::Iri(u)) => {
                    self.pos += 1;
                    self.resolve_iri(&u)
                }
                Some(Tok::Prefixed(p, l)) => {
                    self.pos += 1;
                    self.resolve_prefixed(&p, &l)?
                }
                _ => return Err(Error::Manifest),
            };

            loop {
                let obj = self.parse_object(depth + 1)?;
                self.doc.push_triple(String::from(subj), pred.clone(), obj);
                if matches!(self.peek(), Some(Tok::Comma)) {
                    self.pos += 1;
                } else {
                    break;
                }
            }

            if matches!(self.peek(), Some(Tok::Semi)) {
                while matches!(self.peek(), Some(Tok::Semi)) {
                    self.pos += 1;
                }
                if matches!(
                    self.peek(),
                    None | Some(Tok::Dot) | Some(Tok::RBracket)
                ) {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_object(&mut self, depth: usize) -> Result<Obj, Error> {
        if depth > MAX_TURTLE_DEPTH {
            return Err(Error::Bounds);
        }
        let tok = self.toks.get(self.pos).cloned().ok_or(Error::Manifest)?;
        match tok {
            Tok::Iri(u) => {
                self.pos += 1;
                Ok(Obj::Iri(self.resolve_iri(&u)))
            }
            Tok::Prefixed(p, l) => {
                self.pos += 1;
                Ok(Obj::Iri(self.resolve_prefixed(&p, &l)?))
            }
            Tok::Literal(s) => {
                self.pos += 1;
                Ok(Obj::Literal(s))
            }
            Tok::Number(n) => {
                self.pos += 1;
                Ok(Obj::Number(n))
            }
            Tok::LBracket => {
                self.pos += 1;
                let id = self.fresh_bnode();
                if !matches!(self.peek(), Some(Tok::RBracket)) {
                    self.parse_predicate_object_list(&id, depth + 1)?;
                }
                if !matches!(self.peek(), Some(Tok::RBracket)) {
                    return Err(Error::Manifest);
                }
                self.pos += 1;
                Ok(Obj::Iri(id))
            }
            Tok::LParen => {
                let id = self.parse_collection(depth + 1)?;
                Ok(Obj::Iri(id))
            }
            _ => Err(Error::Manifest),
        }
    }

    fn parse_collection(&mut self, depth: usize) -> Result<String, Error> {
        if depth > MAX_TURTLE_DEPTH {
            return Err(Error::Bounds);
        }
        self.pos += 1; // '('
        let head = self.fresh_bnode();
        let mut items = Vec::new();
        while !matches!(self.peek(), Some(Tok::RParen)) {
            if self.peek().is_none() {
                return Err(Error::Manifest);
            }
            items.push(self.parse_object(depth + 1)?);
        }
        self.pos += 1; // ')'
        if items.is_empty() {
            return Ok(format!("{RDF_NS}nil"));
        }
        let mut cur = head.clone();
        let last_idx = items.len() - 1;
        for (idx, item) in items.into_iter().enumerate() {
            self.doc
                .push_triple(cur.clone(), format!("{RDF_NS}first"), item);
            if self.doc.triples.len() > MAX_TURTLE_TOKENS {
                return Err(Error::Bounds);
            }
            if idx == last_idx {
                self.doc.push_triple(
                    cur.clone(),
                    format!("{RDF_NS}rest"),
                    Obj::Iri(format!("{RDF_NS}nil")),
                );
            } else {
                let rest = self.fresh_bnode();
                self.doc
                    .push_triple(cur.clone(), format!("{RDF_NS}rest"), Obj::Iri(rest.clone()));
                cur = rest;
            }
        }
        Ok(head)
    }
}

fn parse_turtle(input: &str) -> Result<TurtleDoc, Error> {
    let toks = tokenize_turtle(input)?;
    let mut parser = TurtleParser::new(toks);
    parser.parse_all()?;
    Ok(parser.doc)
}
