extern crate alloc;

use alloc::boxed::Box;
use core::{
    ffi::{c_char, c_void},
    ptr,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use moan_plugin::job::{Format, Parameter, BLOCK};
use moan_plugin::lv2::{
    enumerate_descriptors, Error, Lv2Manifest, Lv2PortKind, Lv2Processor, LV2_Descriptor,
    LV2_Feature, LV2_Handle,
};

const MANIFEST_TTL: &str = r#"
@prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix pset: <http://lv2plug.in/ns/ext/presets#> .

<http://omaaudio.org/plugins/voice-polish>
    a lv2:Plugin , lv2:CompressorPlugin ;
    lv2:binary <omaaudio_voice.so> ;
    rdfs:seeAlso <voice_polish.ttl> , <presets.ttl> .

<http://omaaudio.org/plugins/voice-polish#bank-studio>
    a pset:Bank ;
    rdfs:label "Studio Presets" .

<http://omaaudio.org/plugins/voice-polish#preset-podcast>
    a pset:Preset ;
    lv2:appliesTo <http://omaaudio.org/plugins/voice-polish> ;
    pset:bank <http://omaaudio.org/plugins/voice-polish#bank-studio> ;
    rdfs:label "Podcast Clean" ;
    rdfs:seeAlso <presets.ttl> .
"#;

const PLUGIN_TTL: &str = r#"
@prefix doap: <http://usefulinc.com/ns/doap#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix pprops: <http://lv2plug.in/ns/ext/port-props#> .
@prefix atom: <http://lv2plug.in/ns/ext/atom#> .

<http://omaaudio.org/plugins/voice-polish>
    a lv2:Plugin , lv2:DynamicsPlugin ;
    doap:name "OmaAudio Voice Polish" ;
    doap:maintainer [
        foaf:name "OmaAudio"
    ] ;
    lv2:port [
        a lv2:AudioPort , lv2:InputPort ;
        lv2:index 0 ;
        lv2:symbol "in_l" ;
        lv2:name "Audio In L" ;
    ] , [
        a lv2:AudioPort , lv2:InputPort ;
        lv2:index 1 ;
        lv2:symbol "in_r" ;
        lv2:name "Audio In R" ;
    ] , [
        a lv2:AudioPort , lv2:OutputPort ;
        lv2:index 2 ;
        lv2:symbol "out_l" ;
        lv2:name "Audio Out L" ;
    ] , [
        a lv2:AudioPort , lv2:OutputPort ;
        lv2:index 3 ;
        lv2:symbol "out_r" ;
        lv2:name "Audio Out R" ;
    ] , [
        a atom:AtomPort , lv2:InputPort ;
        lv2:index 4 ;
        lv2:symbol "midi_in" ;
        lv2:name "Events" ;
    ] , [
        a lv2:ControlPort , lv2:InputPort ;
        lv2:index 5 ;
        lv2:symbol "gain" ;
        lv2:name "Output Gain" ;
        lv2:default 1.0 ;
        lv2:minimum 0.0 ;
        lv2:maximum 4.0 ;
        lv2:portProperty pprops:logarithmic ;
        lv2:scalePoint [
            rdfs:label "Unity" ;
            rdf:value 1.0
        ] , [
            rdfs:label "Boost +6dB" ;
            rdf:value 2.0
        ] ;
    ] , [
        a lv2:ControlPort , lv2:InputPort ;
        lv2:index 6 ;
        lv2:symbol "bypass" ;
        lv2:name "Bypass" ;
        lv2:default 0 ;
        lv2:minimum 0 ;
        lv2:maximum 1 ;
        lv2:portProperty lv2:integer , lv2:toggled ;
    ] , [
        a lv2:ControlPort , lv2:OutputPort ;
        lv2:index 7 ;
        lv2:symbol "peak_out" ;
        lv2:name "Peak Meter" ;
        lv2:default 0.0 ;
        lv2:minimum 0.0 ;
        lv2:maximum 16.0 ;
    ] .
"#;

const PRESETS_TTL: &str = r#"
@prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
@prefix pset: <http://lv2plug.in/ns/ext/presets#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

<http://omaaudio.org/plugins/voice-polish#preset-podcast>
    a pset:Preset ;
    lv2:appliesTo <http://omaaudio.org/plugins/voice-polish> ;
    pset:bank <http://omaaudio.org/plugins/voice-polish#bank-studio> ;
    rdfs:label "Podcast Clean" ;
    lv2:port [
        lv2:symbol "gain" ;
        pset:value 2.0
    ] , [
        lv2:symbol "bypass" ;
        pset:value 0
    ] .
"#;

#[test]
fn ttl_parser_scans_bundle_ports_presets_and_banks_without_native_exec() {
    let manifest = Lv2Manifest::parse_bundle(MANIFEST_TTL, &[PLUGIN_TTL, PRESETS_TTL]).unwrap();
    assert_eq!(manifest.plugins.len(), 1);
    let plug = &manifest.plugins[0];
    assert_eq!(plug.uri, "http://omaaudio.org/plugins/voice-polish");
    assert_eq!(plug.name, "OmaAudio Voice Polish");
    assert_eq!(plug.binary, "omaaudio_voice.so");
    assert!(plug.is_stereo_capable());

    // AtomPort at index 4 was safely skipped; remaining 7 ports preserve exact LV2 indices.
    assert_eq!(plug.ports.len(), 7);
    assert_eq!(plug.audio_inputs().count(), 2);
    assert_eq!(plug.audio_outputs().count(), 2);
    assert_eq!(plug.control_inputs().count(), 2);
    assert_eq!(plug.control_outputs().count(), 1);

    let gain = plug
        .ports
        .iter()
        .find(|p| p.symbol == "gain")
        .expect("gain port");
    assert_eq!(gain.index, 5);
    assert_eq!(gain.kind, Lv2PortKind::ControlIn);
    assert_eq!(gain.default_value, 1.0);
    assert_eq!(gain.min_value, Some(0.0));
    assert_eq!(gain.max_value, Some(4.0));
    assert!(gain.is_logarithmic);
    assert_eq!(gain.scale_points.len(), 2);

    let bypass = plug.ports.iter().find(|p| p.symbol == "bypass").unwrap();
    assert!(bypass.is_integer);
    assert!(bypass.is_toggled);

    assert_eq!(plug.preset_banks.len(), 1);
    assert_eq!(plug.preset_banks[0].label, "Studio Presets");
    assert_eq!(plug.preset_banks[0].presets.len(), 1);
    assert_eq!(plug.preset_banks[0].presets[0].label, "Podcast Clean");
    assert_eq!(plug.preset_banks[0].presets[0].values[0].value, 2.0);
}

#[test]
fn ttl_parser_rejects_malformed_and_invalid_ranges() {
    assert_eq!(
        Lv2Manifest::parse_ttl("<http://example.org/p> a \"unterminated").err(),
        Some(Error::Manifest)
    );
    let bad_range = r#"
        @prefix lv2: <http://lv2plug.in/ns/lv2core#> .
        <http://example.org/bad> a lv2:Plugin ;
            lv2:port [
                a lv2:ControlPort , lv2:InputPort ;
                lv2:index 0 ;
                lv2:symbol "x" ;
                lv2:minimum 10.0 ;
                lv2:maximum 2.0 ;
            ] .
    "#;
    assert_eq!(Lv2Manifest::parse_ttl(bad_range).err(), Some(Error::Manifest));

    let dup_index = r#"
        @prefix lv2: <http://lv2plug.in/ns/lv2core#> .
        <http://example.org/dup> a lv2:Plugin ;
            lv2:port [ a lv2:AudioPort , lv2:InputPort ; lv2:index 0 ; lv2:symbol "a" ] ,
                     [ a lv2:AudioPort , lv2:OutputPort ; lv2:index 0 ; lv2:symbol "b" ] .
    "#;
    assert_eq!(Lv2Manifest::parse_ttl(dup_index).err(), Some(Error::Manifest));
}

// --- Mock C-ABI LV2 plugin for Lv2Processor lifecycle & clamping tests ---

struct MockLv2State {
    in_l: *const f32,
    in_r: *const f32,
    out_l: *mut f32,
    out_r: *mut f32,
    gain: *const f32,
    bypass: *const f32,
    peak_out: *mut f32,
    poison_mode: AtomicUsize,
}

static ACTIVATED: AtomicBool = AtomicBool::new(false);
static DEACTIVATED: AtomicBool = AtomicBool::new(false);
static CLEANED_UP: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn mock_instantiate(
    _descriptor: *const LV2_Descriptor,
    _sample_rate: f64,
    _bundle_path: *const c_char,
    _features: *const *const LV2_Feature,
) -> LV2_Handle {
    let state = Box::new(MockLv2State {
        in_l: ptr::null(),
        in_r: ptr::null(),
        out_l: ptr::null_mut(),
        out_r: ptr::null_mut(),
        gain: ptr::null(),
        bypass: ptr::null(),
        peak_out: ptr::null_mut(),
        poison_mode: AtomicUsize::new(0),
    });
    Box::into_raw(state).cast::<c_void>()
}

unsafe extern "C" fn mock_connect_port(
    instance: LV2_Handle,
    port: u32,
    data_location: *mut c_void,
) {
    let st = &mut *instance.cast::<MockLv2State>();
    match port {
        0 => st.in_l = data_location.cast::<f32>(),
        1 => st.in_r = data_location.cast::<f32>(),
        2 => st.out_l = data_location.cast::<f32>(),
        3 => st.out_r = data_location.cast::<f32>(),
        5 => st.gain = data_location.cast::<f32>(),
        6 => st.bypass = data_location.cast::<f32>(),
        7 => st.peak_out = data_location.cast::<f32>(),
        _ => {}
    }
}

unsafe extern "C" fn mock_activate(_instance: LV2_Handle) {
    ACTIVATED.store(true, Ordering::Release);
}

unsafe extern "C" fn mock_run(instance: LV2_Handle, n_samples: u32) {
    let st = &*instance.cast::<MockLv2State>();
    let gain = if st.gain.is_null() { 1.0 } else { *st.gain };
    let bypass = !st.bypass.is_null() && *st.bypass >= 0.5;
    let mode = st.poison_mode.load(Ordering::Relaxed);

    let mut max_p = 0.0f32;
    for i in 0..n_samples as usize {
        let l = *st.in_l.add(i);
        let r = *st.in_r.add(i);
        let (ol, or) = if mode == 1 && i == 0 {
            (f32::NAN, f32::INFINITY)
        } else if mode == 1 && i == 1 {
            (42.0, -99.0)
        } else if bypass {
            (l, r)
        } else {
            (l * gain, r * gain)
        };
        *st.out_l.add(i) = ol;
        *st.out_r.add(i) = or;
        if ol.is_finite() && ol.abs() > max_p {
            max_p = ol.abs();
        }
    }
    if !st.peak_out.is_null() {
        *st.peak_out = max_p;
    }
}

unsafe extern "C" fn mock_deactivate(_instance: LV2_Handle) {
    DEACTIVATED.store(true, Ordering::Release);
}

unsafe extern "C" fn mock_cleanup(instance: LV2_Handle) {
    CLEANED_UP.store(true, Ordering::Release);
    drop(Box::from_raw(instance.cast::<MockLv2State>()));
}

#[repr(transparent)]
struct SyncDesc(LV2_Descriptor);
unsafe impl Sync for SyncDesc {}

static MOCK_DESC: SyncDesc = SyncDesc(LV2_Descriptor {
    uri: c"http://omaaudio.org/plugins/voice-polish".as_ptr(),
    instantiate: Some(mock_instantiate),
    connect_port: Some(mock_connect_port),
    activate: Some(mock_activate),
    run: Some(mock_run),
    deactivate: Some(mock_deactivate),
    cleanup: Some(mock_cleanup),
    extension_data: None,
});

unsafe extern "C" fn mock_lv2_descriptor(index: u32) -> *const LV2_Descriptor {
    if index == 0 {
        &MOCK_DESC.0
    } else {
        ptr::null()
    }
}

#[test]
fn lv2_processor_lifecycle_presets_run_block_and_output_clamping() {
    assert_eq!(Format::Lv2.token(), b"lv2");

    ACTIVATED.store(false, Ordering::Relaxed);
    DEACTIVATED.store(false, Ordering::Relaxed);
    CLEANED_UP.store(false, Ordering::Relaxed);

    let metas = unsafe { enumerate_descriptors(mock_lv2_descriptor) }.unwrap();
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].uri, "http://omaaudio.org/plugins/voice-polish");

    let manifest = Lv2Manifest::parse_bundle(MANIFEST_TTL, &[PLUGIN_TTL, PRESETS_TTL]).unwrap();
    let info = &manifest.plugins[0];

    {
        let mut proc = unsafe {
            Lv2Processor::open(mock_lv2_descriptor, 0, info, c"/plugins/omaaudio.lv2")
        }
        .unwrap();
        assert!(ACTIVATED.load(Ordering::Acquire));

        // Apply Podcast Clean preset (gain = 2.0)
        proc.apply_preset(&info.presets[0], info).unwrap();

        let input = alloc::vec![[0.25f32, -0.5f32]; BLOCK + 12];
        let out = proc.render(&input).unwrap();
        assert_eq!(out.len(), BLOCK + 12);
        assert_eq!(out[0], [0.5, -1.0]);
        assert_eq!(out[BLOCK + 11], [0.5, -1.0]);
        assert_eq!(proc.control_output(7), Some(0.5));

        // Parameter override: gain = 3.0 on port 5
        let out2 = proc
            .render_with_parameters(&[[0.25, -0.25]; 8], &[Parameter { id: 5, value: 3.0 }])
            .unwrap();
        assert_eq!(out2, alloc::vec![[0.75, -0.75]; 8]);

        // Zero-alloc run_block rejects invalid audio
        assert_eq!(
            proc.run_block(&[[f32::NAN, 0.0]], &mut [[0.0; 2]]).err(),
            Some(Error::Audio)
        );
        assert_eq!(
            proc.run_block(&[[17.0, 0.0]], &mut [[0.0; 2]]).err(),
            Some(Error::Audio)
        );
    }

    assert!(DEACTIVATED.load(Ordering::Acquire));
    assert!(CLEANED_UP.load(Ordering::Acquire));
}

#[test]
fn lv2_processor_sanitizes_nan_inf_and_clamps_to_16() {
    unsafe extern "C" fn poison_run(instance: LV2_Handle, n_samples: u32) {
        let st = &*instance.cast::<MockLv2State>();
        for i in 0..n_samples as usize {
            *st.out_l.add(i) = match i {
                0 => f32::NAN,
                1 => f32::INFINITY,
                2 => 42.0,
                _ => 0.25,
            };
            *st.out_r.add(i) = match i {
                0 => f32::NEG_INFINITY,
                1 => -33.5,
                2 => -0.75,
                _ => 0.5,
            };
        }
    }

    static POISON_DESC: SyncDesc = SyncDesc(LV2_Descriptor {
        uri: c"http://omaaudio.org/plugins/voice-polish".as_ptr(),
        instantiate: Some(mock_instantiate),
        connect_port: Some(mock_connect_port),
        activate: Some(mock_activate),
        run: Some(poison_run),
        deactivate: Some(mock_deactivate),
        cleanup: Some(mock_cleanup),
        extension_data: None,
    });

    let manifest = Lv2Manifest::parse_bundle(MANIFEST_TTL, &[PLUGIN_TTL]).unwrap();
    let info = &manifest.plugins[0];
    let mut proc = unsafe {
        Lv2Processor::open_descriptor(&POISON_DESC.0, info, c"/plugins/omaaudio.lv2")
    }
    .unwrap();

    let mut out = [[0.0f32; 2]; 4];
    let sanitized = proc.run_block(&[[0.1, 0.2]; 4], &mut out).unwrap();
    assert_eq!(sanitized, 5);
    assert_eq!(proc.last_sanitized_count(), 5);
    assert_eq!(out[0], [0.0, 0.0]);
    assert_eq!(out[1], [0.0, -16.0]);
    assert_eq!(out[2], [16.0, -0.75]);
    assert_eq!(out[3], [0.25, 0.5]);
}

#[test]
fn lv2_mono_topology_and_sparql_turtle_dialects() {
    let x42_style_ttl = r#"
        PREFIX lv2: <http://lv2plug.in/ns/lv2core#>
        PREFIX doap: <http://usefulinc.com/ns/doap#>
        PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
        BASE <http://gareus.org/oss/lv2/meters>

        # Comment line with <ignored> "quotes"
        <#mono_amp>
            a lv2:Plugin , lv2:AmplifierPlugin ;
            doap:name """x42 Mono Gain"""@en ;
            lv2:binary <meters.so> ;
            lv2:port [
                a lv2:AudioPort , lv2:InputPort ;
                lv2:index 0 ;
                lv2:symbol "in" ;
                lv2:name "In"@en
            ] , [
                a lv2:AudioPort , lv2:OutputPort ;
                lv2:index 1 ;
                lv2:symbol "out" ;
                lv2:name "Out"
            ] , [
                a lv2:ControlPort , lv2:InputPort ;
                lv2:index 2 ;
                lv2:symbol "gain" ;
                lv2:name "Gain" ;
                lv2:default "0.5"^^xsd:float ;
                lv2:minimum 0.0 ;
                lv2:maximum 2.0
            ] .
    "#;
    let manifest = Lv2Manifest::parse_ttl(x42_style_ttl).unwrap();
    let info = manifest
        .plugin("http://gareus.org/oss/lv2/meters#mono_amp")
        .expect("BASE + fragment resolution");
    assert_eq!(info.name, "x42 Mono Gain");
    assert!(info.is_stereo_capable());
    assert_eq!(info.default_controls(), alloc::vec![(2, 0.5)]);

    struct MonoState {
        in_p: *const f32,
        out_p: *mut f32,
        gain: *const f32,
    }
    unsafe extern "C" fn mono_inst(
        _: *const LV2_Descriptor,
        _: f64,
        _: *const c_char,
        _: *const *const LV2_Feature,
    ) -> LV2_Handle {
        Box::into_raw(Box::new(MonoState {
            in_p: ptr::null(),
            out_p: ptr::null_mut(),
            gain: ptr::null(),
        }))
        .cast()
    }
    unsafe extern "C" fn mono_conn(h: LV2_Handle, port: u32, loc: *mut c_void) {
        let s = &mut *h.cast::<MonoState>();
        match port {
            0 => s.in_p = loc.cast(),
            1 => s.out_p = loc.cast(),
            2 => s.gain = loc.cast(),
            _ => {}
        }
    }
    unsafe extern "C" fn mono_run(h: LV2_Handle, n: u32) {
        let s = &*h.cast::<MonoState>();
        let g = *s.gain;
        for i in 0..n as usize {
            *s.out_p.add(i) = *s.in_p.add(i) * g;
        }
    }
    unsafe extern "C" fn mono_clean(h: LV2_Handle) {
        drop(Box::from_raw(h.cast::<MonoState>()));
    }
    static MONO_DESC: SyncDesc = SyncDesc(LV2_Descriptor {
        uri: c"http://gareus.org/oss/lv2/meters#mono_amp".as_ptr(),
        instantiate: Some(mono_inst),
        connect_port: Some(mono_conn),
        activate: None,
        run: Some(mono_run),
        deactivate: None,
        cleanup: Some(mono_clean),
        extension_data: None,
    });

    let mut proc = unsafe { Lv2Processor::open_descriptor(&MONO_DESC.0, info, c"/tmp") }.unwrap();
    let out = proc.render(&[[0.4, 0.8]; 4]).unwrap();
    // Mono averages (0.4 + 0.8) * 0.5 = 0.6, multiplies by default 0.5 = 0.3, duplicates to stereo
    for f in out {
        assert!((f[0] - 0.3).abs() < 1e-6);
        assert!((f[1] - 0.3).abs() < 1e-6);
    }
}

#[test]
fn lv2_multi_plugin_bundle_and_processor_rejection_bounds() {
    let multi_manifest = r#"
        @prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        <http://lsp-plug.in/plugins/lv2/comp_stereo>
            a lv2:Plugin , lv2:CompressorPlugin ;
            lv2:binary <lsp-plugins-lv2.so> ;
            rdfs:seeAlso <comp_stereo.ttl> .
        <http://calf.sourceforge.net/plugins/Equalizer5Band>
            a lv2:Plugin , lv2:EQPlugin ;
            lv2:binary <calf.so> ;
            rdfs:seeAlso <eq5.ttl> .
    "#;
    let eq_ttl = r#"
        @prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
        @prefix doap: <http://usefulinc.com/ns/doap#> .
        <http://calf.sourceforge.net/plugins/Equalizer5Band>
            a lv2:EQPlugin ;
            doap:name "Calf 5 Band EQ" ;
            lv2:port [ a lv2:AudioPort , lv2:InputPort ; lv2:index 0 ; lv2:symbol "in_l" ] ,
                     [ a lv2:AudioPort , lv2:OutputPort ; lv2:index 1 ; lv2:symbol "out_l" ] .
    "#;
    let mut manifest = Lv2Manifest::parse_ttl(multi_manifest).unwrap();
    assert_eq!(manifest.entries.len(), 2);
    manifest.add_ttl(eq_ttl).unwrap();
    let eq = manifest
        .plugin("http://calf.sourceforge.net/plugins/Equalizer5Band")
        .unwrap();
    assert_eq!(eq.name, "Calf 5 Band EQ");
    assert_eq!(eq.binary, "calf.so");
    assert_eq!(eq.ports.len(), 2);

    // Processor rejects descriptor index >= 64, mismatched URI, and zero-port plugins
    assert_eq!(
        unsafe { Lv2Processor::open(mock_lv2_descriptor, 64, eq, c"/tmp") }.err(),
        Some(Error::Plugin)
    );
    assert_eq!(
        unsafe { Lv2Processor::open_descriptor(&MOCK_DESC.0, eq, c"/tmp") }.err(),
        Some(Error::Plugin)
    );

    let no_out_ttl = r#"
        @prefix lv2: <http://lv2plug.in/ns/lv2core#> .
        <http://omaaudio.org/plugins/voice-polish>
            a lv2:Plugin ;
            lv2:port [ a lv2:AudioPort , lv2:InputPort ; lv2:index 0 ; lv2:symbol "in" ] .
    "#;
    let no_out_manifest = Lv2Manifest::parse_ttl(no_out_ttl).unwrap();
    assert_eq!(
        unsafe {
            Lv2Processor::open_descriptor(&MOCK_DESC.0, &no_out_manifest.plugins[0], c"/tmp")
        }
        .err(),
        Some(Error::Ports)
    );

    // Oversized block > BLOCK is refused in zero-alloc run_block
    let full = Lv2Manifest::parse_bundle(MANIFEST_TTL, &[PLUGIN_TTL]).unwrap();
    let mut proc = unsafe {
        Lv2Processor::open(mock_lv2_descriptor, 0, &full.plugins[0], c"/tmp")
    }
    .unwrap();
    let big_in = alloc::vec![[0.0f32; 2]; BLOCK + 1];
    let mut big_out = alloc::vec![[0.0f32; 2]; BLOCK + 1];
    assert_eq!(proc.run_block(&big_in, &mut big_out).err(), Some(Error::Frames));
}

