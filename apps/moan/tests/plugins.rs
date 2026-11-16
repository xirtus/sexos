#[path = "../src/plugins.rs"]
mod plugins;
use moan_plugin::job::Format;
use plugins::*;
#[test]
fn explicit_tokens_and_wire_budget() {
    let p = Paths::new(b"@/plugins/gain.vst3", b"@/render", [1; 16], Format::Vst3).unwrap();
    assert!(p.input.starts_with(b"@/render/moan-"));
    assert!(p.output.ends_with(b".out"));
    assert_ne!(p.input, p.output);
    assert_eq!(p.plugin, b"@/plugins/gain.vst3");
    for bad in [
        b"@/".as_slice(),
        b"@/a/../b",
        b"@/a//b",
        b"@/space here",
        b"relative",
    ] {
        assert!(Paths::new(b"@/p", bad, [1; 16], Format::Clap).is_err());
    }
    assert!(Paths::new(b"@/p", b"@/render", [0; 16], Format::Clap).is_err());
    let long = [b"@/".as_slice(), &[b'a'; 180]].concat();
    assert!(Paths::new(&long, &long, [1; 16], Format::Clap).is_err());
}
#[test]
fn exact_pcm_roundtrip_and_visible_clipping() {
    let input = [[-32768, 32767], [-1, 0], [1024, -2048]];
    let floats = float_input(&input).unwrap();
    let result = quantize(&floats, Format::Clap).unwrap();
    assert_eq!(result.frames, input);
    assert_eq!(result.clipped_samples, 0);
    let clipped = quantize(&[[2.0, -2.0]], Format::Vst3).unwrap();
    assert_eq!(clipped.frames, [[32767, -32768]]);
    assert_eq!(clipped.clipped_samples, 2);
    assert_eq!(clipped.format, Format::Vst3);
}
#[test]
fn invalid_audio_never_imported() {
    assert_eq!(float_input(&[]).err(), Some(Error::Frames));
    for value in [f32::NAN, f32::INFINITY, 17.0] {
        assert_eq!(
            quantize(&[[value, 0.0]], Format::Clap).err(),
            Some(Error::Audio)
        );
    }
}
