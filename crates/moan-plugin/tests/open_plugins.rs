//! Open LADSPA and LV2 plugins, built from their unmodified upstream sources by
//! scripts/build_moan_open_plugins.sh, checked two ways:
//!
//! * the SexOS build is admitted by the helper's ELF rules, imports only the
//!   helper's C surface, and its LV2 bundles describe correctly once packed;
//! * the host build runs through the same LADSPA/LV2 host code the helper uses,
//!   and its audio output is checked (gain, low-pass and high-pass response).
//!
//! Tools missing on a machine (clang/ld.lld or cc) skip that half with a note.
use moan_plugin::{
    elf::{Image, Reloc},
    imports,
    job::{Parameter, RATE},
    ladspa, lv2,
    native::Kind,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn build(mode: &str) -> Option<PathBuf> {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("open-plugins-{mode}"));
    let status = Command::new("sh")
        .arg(root().join("scripts/build_moan_open_plugins.sh"))
        .arg(mode)
        .arg(&out)
        .status()
        .ok()?;
    if !status.success() {
        if std::env::var_os("MOAN_OPEN_PLUGINS_REQUIRED").is_some() {
            panic!("{mode} build of the open plugins failed");
        }
        eprintln!("skipping {mode} open-plugin checks: build tools unavailable");
        return None;
    }
    Some(out)
}

fn sexos() -> Option<&'static PathBuf> {
    static OUT: OnceLock<Option<PathBuf>> = OnceLock::new();
    OUT.get_or_init(|| build("sexos")).as_ref()
}
fn host() -> Option<&'static PathBuf> {
    static OUT: OnceLock<Option<PathBuf>> = OnceLock::new();
    OUT.get_or_init(|| build("host")).as_ref()
}

fn pack(dir: &Path) -> Vec<u8> {
    let manifest = std::fs::read(dir.join("manifest.ttl")).unwrap();
    let names = lv2::manifest_files(&manifest).unwrap();
    let mut files = vec![(b"manifest.ttl".to_vec(), manifest)];
    for n in names {
        files.push((n.clone().into_bytes(), std::fs::read(dir.join(&n)).unwrap()));
    }
    let refs: Vec<(&[u8], &[u8])> = files.iter().map(|(n, d)| (&n[..], &d[..])).collect();
    lv2::encode(&refs).unwrap()
}

/// What the helper's loader would check before mapping a library.
fn admit(bytes: &[u8], entry: &[u8], constructors: bool) {
    let image = Image::parse(bytes).expect("helper ELF admission");
    image.export(entry).expect("entry point exported");
    let mut imported = Vec::new();
    image
        .relocations(|_, _, r| {
            if let Reloc::Symbol {
                name, value: None, ..
            } = r
            {
                imported.push(String::from_utf8_lossy(name).into_owned());
            }
            Ok(())
        })
        .unwrap();
    for name in &imported {
        assert!(
            imports::allowed(name.as_bytes()),
            "import {name} is outside the plugin C surface"
        );
    }
    assert_eq!(image.init_array.is_some(), constructors);
}

#[test]
fn sexos_builds_are_admitted_by_the_helper() {
    let Some(out) = sexos() else { return };
    for name in ["t5_so_lowpass.so", "t5_so_highpass.so"] {
        admit(&std::fs::read(out.join(name)).unwrap(), b"ladspa_descriptor", true);
    }
    for (dir, uri, ports) in [
        ("eg-amp.lv2", "http://lv2plug.in/plugins/eg-amp", 3),
        ("blop-lp4pole.lv2", "http://drobilla.net/plugins/blop/lp4pole", 4),
    ] {
        let packed = pack(&out.join(dir));
        let bundle = lv2::Bundle::decode(&packed).unwrap();
        let plugin = lv2::describe(&bundle).unwrap();
        assert_eq!(&plugin.uri[..plugin.uri.len() - 1], uri.as_bytes());
        assert_eq!(plugin.ports.len(), ports);
        admit(
            bundle.get(plugin.binary.as_bytes()).unwrap(),
            b"lv2_descriptor",
            false,
        );
    }
}

// ------------------------------------------------------------------ host runs

struct Library(*mut libc::c_void);
impl Library {
    fn open(path: &Path) -> Self {
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        assert!(!h.is_null(), "dlopen {}", path.display());
        Self(h)
    }
    fn symbol(&self, name: &str) -> *mut libc::c_void {
        let c = std::ffi::CString::new(name).unwrap();
        let s = unsafe { libc::dlsym(self.0, c.as_ptr()) };
        assert!(!s.is_null(), "{name}");
        s
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe { libc::dlclose(self.0) };
    }
}

fn sine(hz: f32, frames: usize) -> Vec<[f32; 2]> {
    (0..frames)
        .map(|i| {
            let v = 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin();
            [v, -v]
        })
        .collect()
}
/// RMS of each channel, skipping the filter's settling time.
fn rms(x: &[[f32; 2]]) -> [f32; 2] {
    let tail = &x[x.len() / 2..];
    let mut s = [0.0f64; 2];
    for f in tail {
        s[0] += (f[0] as f64).powi(2);
        s[1] += (f[1] as f64).powi(2);
    }
    let n = tail.len() as f64;
    [(s[0] / n).sqrt() as f32, (s[1] / n).sqrt() as f32]
}

fn ladspa_render(lib: &Library, params: &[Parameter], input: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let entry: ladspa::Entry = unsafe { std::mem::transmute(lib.symbol("ladspa_descriptor")) };
    let mut p = unsafe { ladspa::Processor::open(entry, params).unwrap() };
    p.render(input).unwrap()
}

#[test]
fn t5_ladspa_filters_shape_the_spectrum() {
    let Some(out) = host() else { return };
    let low = Library::open(&out.join("t5_so_lowpass.so"));
    let entry: ladspa::Entry = unsafe { std::mem::transmute(low.symbol("ladspa_descriptor")) };
    let p = unsafe { ladspa::Processor::open(entry, &[]).unwrap() };
    let kinds: Vec<Kind> = p.ports().iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [Kind::AudioIn, Kind::AudioOut, Kind::ControlIn, Kind::ControlIn, Kind::ControlIn]
    );
    assert_eq!(p.ports()[2].default, 440.0, "LADSPA_HINT_DEFAULT_440");
    assert_eq!(p.ports()[2].max, 24_000.0, "sample-rate bound");
    drop(p);
    let cutoff = [Parameter { id: 2, value: 1000.0 }];
    let frames = RATE as usize / 4;
    let pass = rms(&ladspa_render(&low, &cutoff, &sine(100.0, frames)));
    let stop = rms(&ladspa_render(&low, &cutoff, &sine(10_000.0, frames)));
    // 0.5 amplitude sine = 0.3536 RMS. 12 dB/octave: ~-40 dB at 10x cutoff.
    assert!((pass[0] - 0.3536).abs() < 0.01 && (pass[1] - 0.3536).abs() < 0.01, "{pass:?}");
    assert!(stop[0] < 0.01 && stop[1] < 0.01, "{stop:?}");

    let high = Library::open(&out.join("t5_so_highpass.so"));
    let pass = rms(&ladspa_render(&high, &cutoff, &sine(10_000.0, frames)));
    let stop = rms(&ladspa_render(&high, &cutoff, &sine(100.0, frames)));
    assert!((pass[0] - 0.3536).abs() < 0.01, "{pass:?}");
    assert!(stop[0] < 0.01, "{stop:?}");
    // Gain port in dB.
    let louder = rms(&ladspa_render(
        &low,
        &[cutoff[0], Parameter { id: 3, value: 6.0206 }],
        &sine(100.0, frames),
    ));
    assert!((louder[0] - 0.7071).abs() < 0.01, "{louder:?}");
}

fn lv2_render(dir: &Path, params: &[Parameter], input: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let packed = pack(dir);
    let bundle = lv2::Bundle::decode(&packed).unwrap();
    let plugin = lv2::describe(&bundle).unwrap();
    let lib = Library::open(&dir.join(&plugin.binary));
    let entry: lv2::Entry = unsafe { std::mem::transmute(lib.symbol("lv2_descriptor")) };
    let mut p = unsafe { lv2::Processor::open(entry, &plugin, params).unwrap() };
    let out = p.render(input).unwrap();
    drop(p);
    out
}

#[test]
fn lv2_amp_applies_gain_in_db() {
    let Some(out) = host() else { return };
    let dir = out.join("eg-amp.lv2");
    let input = sine(440.0, 4096);
    let unity = lv2_render(&dir, &[], &input);
    assert_eq!(unity, input, "default 0 dB");
    let half = lv2_render(&dir, &[Parameter { id: 0, value: -6.0206 }], &input);
    for (a, b) in half.iter().zip(&input) {
        assert!((a[0] - b[0] * 0.5).abs() < 1e-4 && (a[1] - b[1] * 0.5).abs() < 1e-4);
    }
    // Requests are clamped to the plugin's declared range (max +24 dB).
    let loud = lv2_render(&dir, &[Parameter { id: 0, value: 90.0 }], &sine(440.0, 64));
    let peak = loud.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
    assert!(peak < 0.5 * 15.9 && peak > 0.5 * 15.0, "{peak}");
}

#[test]
fn blop_lp4pole_low_passes_with_urid_and_options() {
    let Some(out) = host() else { return };
    let dir = out.join("blop-lp4pole.lv2");
    let packed = pack(&dir);
    let plugin = lv2::describe(&lv2::Bundle::decode(&packed).unwrap()).unwrap();
    // lv2:sampleRate port property: 0.5 * rate default, 0.00001 * rate minimum.
    assert_eq!(plugin.ports[0].default, 24_000.0);
    assert_eq!(plugin.ports[0].max, 24_000.0);
    let cutoff = [Parameter { id: 0, value: 500.0 }];
    let frames = RATE as usize / 4;
    let pass = rms(&lv2_render(&dir, &cutoff, &sine(50.0, frames)));
    let stop = rms(&lv2_render(&dir, &cutoff, &sine(8_000.0, frames)));
    assert!(pass[0] > 0.25, "{pass:?}");
    assert!(stop[0] < 0.01 && stop[1] < 0.01, "{stop:?}");
}
