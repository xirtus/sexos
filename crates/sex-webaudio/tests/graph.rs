// SPDX-License-Identifier: GPL-3.0-only
// The graph on the host: mixing, params, the analyser's FFT, sources and taps.
use sex_webaudio::{Hub, QUANTUM};
use std::rc::Rc;

const OWNER: u64 = 7;

struct T { hub: Hub, alive: Rc<()> }

impl T {
    fn new() -> T { T { hub: Hub::new(), alive: Rc::new(()) } }
    fn call(&mut self, a: &[&str]) -> Option<String> { self.hub.call(OWNER, &self.alive, a) }
    fn ctx(&mut self) -> String { self.call(&["new", "0"]).unwrap().split('\t').next().unwrap().to_string() }
    fn offline(&mut self, ch: usize, len: u64, rate: u32) -> String {
        self.call(&["new", "1", &ch.to_string(), &len.to_string(), &rate.to_string()]).unwrap().split('\t').next().unwrap().to_string()
    }
    fn node(&mut self, c: &str, kind: &str, opt: &str) -> String { self.call(&["node", c, kind, opt]).unwrap().split('\t').next().unwrap().to_string() }
    fn connect(&mut self, c: &str, from: &str, to: &str) { assert_eq!(self.call(&["connect", c, from, "0", to, "0"]).as_deref(), Some("1")); }
    fn param(&mut self, c: &str, n: &str, p: &str, verb: &str, args: &[&str]) {
        let mut a = vec!["param", c, n, p, verb];
        a.extend_from_slice(args);
        assert_eq!(self.call(&a).as_deref(), Some("1"), "{a:?}");
    }
    fn quanta(&mut self, n: usize) -> Vec<[i32; 2]> {
        let mut all = Vec::new();
        for _ in 0..n { let mut q = [[0; 2]; QUANTUM]; self.hub.render(&mut q); all.extend_from_slice(&q); }
        all
    }
    fn render(&mut self, c: &str) -> Vec<f32> {
        let b = self.hub.call_bytes(OWNER, &[b"render", c.as_bytes()]).unwrap();
        b.chunks_exact(4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).collect()
    }
    fn bytes(&mut self, c: &str, n: &str, kind: &str) -> Vec<u8> { self.hub.call_bytes(OWNER, &[b"analyser", c.as_bytes(), n.as_bytes(), kind.as_bytes()]).unwrap() }
}

fn peak(v: &[u8]) -> usize { v.iter().enumerate().rev().max_by_key(|(_, x)| **x).unwrap().0 }

/// The loudest bin of the float spectrum.
fn loudest(t: &mut T, c: &str, a: &str) -> (usize, f32) {
    let f = t.hub.call_bytes(OWNER, &[b"analyser", c.as_bytes(), a.as_bytes(), b"ff"]).unwrap();
    let db: Vec<f32> = f.chunks_exact(4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).collect();
    let i = db.iter().enumerate().max_by(|x, y| x.1.partial_cmp(y.1).unwrap()).unwrap().0;
    (i, db[i])
}

/// The fake microphone's tone (apps/web capture.rs): a 440 Hz triangle.
fn tone(i: u32) -> i16 {
    let period = 48_000 / 440;
    let p = (i % period) as i32;
    let half = period as i32 / 2;
    let tri = if p < half { p * 2 - half } else { (period as i32 - p) * 2 - half };
    (tri * 8000 / half) as i16
}

#[test]
fn analyser_finds_a_440_hz_oscillator() {
    let mut t = T::new();
    let c = t.ctx();
    let o = t.node(&c, "oscillator", "");
    let a = t.node(&c, "analyser", "");
    t.connect(&c, &o, &a);
    t.call(&["start", &c, &o, "0"]).unwrap();
    t.quanta(40);
    let bins = t.bytes(&c, &a, "bf");
    assert_eq!(bins.len(), 1024);
    // 440 / (48000 / 2048) = 18.8
    assert_eq!(peak(&bins), 19, "{:?}", &bins[..40]);
    assert!(bins[19] > 200 && bins[200] < 40, "{} {}", bins[19], bins[200]);
    // Float dB: a full-scale sine with the Blackman window is about -14 dB at its bin,
    // once the smoothing (from silence) has caught up.
    for _ in 0..40 { t.quanta(1); loudest(&mut t, &c, &a); }
    let (i, db) = loudest(&mut t, &c, &a);
    assert_eq!(i, 19);
    assert!((-15.0..-13.0).contains(&db), "{db}");
    // The analyser is not connected to the destination: nothing is heard.
    assert!(t.quanta(1).iter().all(|f| *f == [0, 0]));
    // Time domain: a sine around 128.
    let td = t.bytes(&c, &a, "bt");
    assert_eq!(td.len(), 2048);
    assert!(td.iter().max().unwrap() > &240 && td.iter().min().unwrap() < &16);
    // Another fftSize moves the bin.
    assert_eq!(t.call(&["set", &c, &a, "fftSize", "8192"]).as_deref(), Some("1"));
    assert_eq!(t.call(&["set", &c, &a, "fftSize", "1000"]).as_deref(), Some("0"));
    t.quanta(80);
    assert_eq!(peak(&t.bytes(&c, &a, "bf")), 75); // 440 * 8192 / 48000 = 75.1
}

#[test]
fn microphone_tap_reaches_the_analyser() {
    let mut t = T::new();
    let c = t.ctx();
    let s = t.node(&c, "streamsource", "0");
    let a = t.node(&c, "analyser", "");
    t.connect(&c, &s, &a);
    let mut i = 0u32;
    for _ in 0..30 {
        let frames: Vec<[i16; 2]> = (0..QUANTUM).map(|_| { let v = tone(i); i += 1; [v, v] }).collect();
        t.hub.feed(&frames);
        t.quanta(1);
    }
    let bins = t.bytes(&c, &a, "bf");
    assert_eq!(peak(&bins), 19);
    // Without sound the bins fall away (smoothing 0.8).
    t.quanta(200);
    for _ in 0..40 { t.bytes(&c, &a, "bf"); t.quanta(1); }
    assert!(t.bytes(&c, &a, "bf")[19] < 60);
}

#[test]
fn gains_mix_into_the_destination() {
    let mut t = T::new();
    let c = t.ctx();
    let (k1, k2, g) = (t.node(&c, "constant", ""), t.node(&c, "constant", ""), t.node(&c, "gain", ""));
    t.param(&c, &k1, "0", "value", &["0.25"]);
    t.param(&c, &k2, "0", "value", &["0.5"]);
    t.param(&c, &g, "0", "value", &["0.5"]);
    t.connect(&c, &k1, &g);
    t.connect(&c, &k2, &g);
    t.connect(&c, &g, "0");
    for n in [&k1, &k2] { t.call(&["start", &c, n, "0"]).unwrap(); }
    let out = t.quanta(1);
    // (0.25 + 0.5) * 0.5 = 0.375, both channels (mono up-mixed).
    assert!(out.iter().all(|f| (f[0] - 12288).abs() <= 2 && f[0] == f[1]), "{:?}", out[0]);
    // Disconnecting one input.
    t.call(&["disconnect", &c, &k2, "", &g]);
    let out = t.quanta(1);
    assert!((out[5][0] - 4096).abs() <= 2, "{:?}", out[5]);
    // A gain modulated by a constant through its param: 0.5 + 0.25.
    let m = t.node(&c, "constant", "");
    t.param(&c, &m, "0", "value", &["0.25"]);
    t.call(&["start", &c, &m, "0"]).unwrap();
    assert_eq!(t.call(&["connectparam", &c, &m, "0", &g, "0"]).as_deref(), Some("1"));
    let out = t.quanta(1);
    assert!((out[5][0] - 6144).abs() <= 2, "{:?}", out[5]);
}

#[test]
fn param_ramps_and_targets() {
    let mut t = T::new();
    let c = t.offline(1, 48_000, 48_000);
    let (k, g) = (t.node(&c, "constant", ""), t.node(&c, "gain", ""));
    t.connect(&c, &k, &g);
    t.connect(&c, &g, "0");
    t.param(&c, &g, "0", "set", &["0", "0"]);
    t.param(&c, &g, "0", "linear", &["1", "0.5"]);
    t.param(&c, &g, "0", "exp", &["0.01", "0.75"]);
    t.param(&c, &g, "0", "target", &["0", "0.8", "0.05"]);
    t.call(&["start", &c, &k, "0"]).unwrap();
    let s = t.render(&c);
    assert_eq!(s.len(), 48_000);
    let at = |sec: f64| s[(sec * 48_000.0) as usize];
    assert!(at(0.0).abs() < 0.01);
    assert!((at(0.25) - 0.5).abs() < 0.01, "{}", at(0.25));
    assert!((at(0.5) - 1.0).abs() < 0.01);
    // Halfway through an exponential ramp from 1 to 0.01: 0.1.
    assert!((at(0.625) - 0.1).abs() < 0.01, "{}", at(0.625));
    // One time constant after the target starts: 0.01 * e^-1.
    assert!((at(0.85) - 0.01 * 0.3679).abs() < 0.002, "{}", at(0.85));
    assert!(at(0.99).abs() < 0.001);
}

#[test]
fn offline_oscillator_and_filter() {
    let mut t = T::new();
    let c = t.offline(2, 48_000, 48_000);
    let o = t.node(&c, "oscillator", "");
    t.connect(&c, &o, "0");
    t.call(&["start", &c, &o, "0"]).unwrap();
    t.call(&["stop", &c, &o, "0.5"]).unwrap();
    let s = t.render(&c);
    assert_eq!(s.len(), 96_000);
    let half = &s[..24_000];
    let rms = (half.iter().map(|v| v * v).sum::<f32>() / half.len() as f32).sqrt();
    assert!((rms - 0.707).abs() < 0.01, "{rms}");
    let crossings = half.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    assert!((219..=221).contains(&crossings), "{crossings}");
    assert!(s[30_000..48_000].iter().all(|v| *v == 0.0), "stopped");
    assert_eq!(s[48_000..72_000], s[..24_000], "both channels");
    assert!(t.hub.events.iter().any(|(_, e)| e == &format!("audio\tended\t{c}\t{o}")));

    // An 8 kHz square through a 500 Hz lowpass loses most of its level.
    let level = |filtered: bool| {
        let mut t = T::new();
        let c = t.offline(1, 9600, 48_000);
        let o = t.node(&c, "oscillator", "");
        t.call(&["set", &c, &o, "type", "square"]).unwrap();
        t.param(&c, &o, "0", "value", &["8000"]);
        let last = if filtered {
            let f = t.node(&c, "biquad", "");
            t.param(&c, &f, "0", "value", &["500"]);
            t.connect(&c, &o, &f);
            f
        } else { o.clone() };
        t.connect(&c, &last, "0");
        t.call(&["start", &c, &o, "0"]).unwrap();
        let s = t.render(&c);
        (s[4800..].iter().map(|v| v * v).sum::<f32>() / 4800.0).sqrt()
    };
    let (dry, wet) = (level(false), level(true));
    assert!(dry > 0.9 && wet < 0.02, "{dry} {wet}");
}

#[test]
fn buffer_source_delay_panner_and_stream_tap() {
    let mut t = T::new();
    let c = t.ctx();
    let b = t.node(&c, "buffersource", "");
    // A short buffer of 0.5 in one channel.
    let data: Vec<u8> = (0..1000).flat_map(|_| 0.5f32.to_le_bytes()).collect();
    t.hub.call_bytes(OWNER, &[b"buffer", c.as_bytes(), b.as_bytes(), b"1", b"48000", &data]).unwrap();
    let d = t.node(&c, "delay", "1");
    t.param(&c, &d, "0", "value", &["0.01"]);
    let p = t.node(&c, "panner", "");
    t.param(&c, &p, "0", "value", &["-1"]);
    let (sd, tap) = {
        let r = t.call(&["node", &c, "streamdestination"]).unwrap();
        let (a, b) = r.split_once('\t').unwrap();
        (a.to_string(), b.parse::<u32>().unwrap())
    };
    t.connect(&c, &b, &d);
    t.connect(&c, &d, &p);
    t.connect(&c, &p, "0");
    t.connect(&c, &p, &sd);
    t.call(&["start", &c, &b, "0"]).unwrap();
    let out = t.quanta(16);
    // 10 ms of delay: 480 frames of silence first, then the buffer, all on the left.
    assert!(out[..479].iter().all(|f| *f == [0, 0]), "{:?}", out.iter().position(|f| *f != [0, 0]));
    assert!((out[600][0] - 16384).abs() < 4 && out[600][1].abs() < 4, "{:?}", out[600]);
    assert!(out[1600] == [0, 0]);
    assert!(t.hub.events.iter().any(|(_, e)| e == &format!("audio\tended\t{c}\t{b}")));
    // The stream destination's tap holds the same sound for other readers.
    let mut cursor = 0;
    let mut f = vec![[0i16; 2]; 2048];
    let n = t.hub.read_tap(tap, &mut cursor, &mut f).unwrap();
    assert_eq!(n, 2048);
    assert!((f[600][0] - 16384).abs() < 4 && f[600][1].abs() < 4);
    // A context in another page cannot be reached.
    assert!(t.hub.call(OWNER + 1, &t.alive.clone(), &["time", &c]).is_none());
    // Suspend stops the clock; the page going away drops the context and its tap.
    let before = t.call(&["time", &c]).unwrap();
    assert_eq!(t.call(&["state", &c, "suspend"]).as_deref(), Some("suspended"));
    t.quanta(4);
    assert_eq!(t.call(&["time", &c]).unwrap(), before);
    t.alive = Rc::new(());
    t.hub.prune();
    assert!(t.hub.contexts.is_empty() && t.hub.tap(tap).is_none());
}

#[test]
fn splitter_merger_and_compressor() {
    let mut t = T::new();
    let c = t.offline(2, 4800, 48_000);
    let (k1, k2) = (t.node(&c, "constant", ""), t.node(&c, "constant", ""));
    t.param(&c, &k1, "0", "value", &["0.2"]);
    t.param(&c, &k2, "0", "value", &["0.4"]);
    let m = t.node(&c, "merger", "2");
    assert_eq!(t.call(&["connect", &c, &k1, "0", &m, "0"]).as_deref(), Some("1"));
    assert_eq!(t.call(&["connect", &c, &k2, "0", &m, "1"]).as_deref(), Some("1"));
    let s = t.node(&c, "splitter", "2");
    t.connect(&c, &m, &s);
    // Swap the channels back through a second merger.
    let m2 = t.node(&c, "merger", "2");
    t.call(&["connect", &c, &s, "0", &m2, "1"]).unwrap();
    t.call(&["connect", &c, &s, "1", &m2, "0"]).unwrap();
    t.connect(&c, &m2, "0");
    for k in [&k1, &k2] { t.call(&["start", &c, k, "0"]).unwrap(); }
    let out = t.render(&c);
    assert!((out[100] - 0.4).abs() < 0.001 && (out[4800 + 100] - 0.2).abs() < 0.001, "{} {}", out[100], out[4900]);

    // A loud tone through the compressor comes out with reduction.
    let mut t = T::new();
    let c = t.ctx();
    let o = t.node(&c, "oscillator", "");
    let k = t.node(&c, "compressor", "");
    t.connect(&c, &o, &k);
    t.connect(&c, &k, "0");
    t.call(&["start", &c, &o, "0"]).unwrap();
    t.quanta(200);
    let r: f32 = t.call(&["reduction", &c, &k]).unwrap().parse().unwrap();
    assert!(r < -10.0, "{r}");
}

#[test]
fn decode_resamples_to_the_context() {
    let mut t = T::new();
    t.hub.decoder = Some(|b: &[u8]| Some(sex_webaudio::Decoded { rate: 48_000, channels: 1, frames: b.iter().map(|v| [*v as i16 * 100, 0]).collect() }));
    let out = t.hub.call_bytes(OWNER, &[b"decode", b"24000", &[1, 2, 3, 4, 5, 6, 7, 8]]).unwrap();
    let w = |i: usize| u32::from_le_bytes(out[i * 4..i * 4 + 4].try_into().unwrap());
    assert_eq!((w(0), w(1), w(2)), (1, 24_000, 4));
    let s: Vec<f32> = out[12..].chunks_exact(4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).collect();
    assert_eq!(s.len(), 4);
    assert!((s[1] - 300.0 / 32768.0).abs() < 1e-4);
}

#[test]
fn periodic_wave_plays_its_harmonic() {
    let mut t = T::new();
    let c = t.ctx();
    let o = t.node(&c, "oscillator", "");
    let a = t.node(&c, "analyser", "");
    // Only the third harmonic: 1320 Hz.
    let re: Vec<u8> = [0f32, 0., 0., 0.].iter().flat_map(|v| v.to_le_bytes()).collect();
    let im: Vec<u8> = [0f32, 0., 0., 1.].iter().flat_map(|v| v.to_le_bytes()).collect();
    t.hub.call_bytes(OWNER, &[b"wave", c.as_bytes(), o.as_bytes(), &re, &im, b"1"]).unwrap();
    t.connect(&c, &o, &a);
    t.call(&["start", &c, &o, "0"]).unwrap();
    t.quanta(40);
    assert_eq!(loudest(&mut t, &c, &a).0, 56); // 1320 * 2048 / 48000 = 56.3
}
