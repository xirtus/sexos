// Web Audio for pages: AudioContext and its nodes over op::AUDIO, the graph in
// sex-webaudio (rendered here by the test as the browser would).
use sex_js::{Document, Limits, Script};
use sex_webaudio::{Hub, QUANTUM};
use std::cell::RefCell;
use std::rc::Rc;

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(hub: &Rc<RefCell<Hub>>, capture: bool) -> Script<Document> {
    let dom = sex_html::parse::parse("<audio id=a></audio>");
    let mut d = Document::new(dom, "https://studio.example/", (410, 330), clock);
    d.audio = Some(hub.clone());
    if capture {
        let mut c = sex_js::Capture::default();
        c.devices.push(sex_js::capture::Device { kind: "audioinput", id: String::from("mic0"), label: String::from("Built-in microphone") });
        d.capture = Some(Rc::new(RefCell::new(c)));
    }
    Script::new(d, Limits::default()).expect("engine")
}

fn logs(s: &mut Script<Document>) -> Vec<String> { s.page_mut().log.drain(..).map(|(_, t)| t).collect() }

fn render(hub: &Rc<RefCell<Hub>>, n: usize) -> Vec<[i32; 2]> {
    let mut all = Vec::new();
    for _ in 0..n { let mut q = [[0; 2]; QUANTUM]; hub.borrow_mut().render(&mut q); all.extend_from_slice(&q); }
    all
}

/// Events the browser hands the page (as apps/web does).
fn deliver(hub: &Rc<RefCell<Hub>>, s: &mut Script<Document>) {
    let ev: Vec<String> = hub.borrow_mut().events.drain(..).map(|(_, e)| e).collect();
    for e in ev { s.dispatch(&e).unwrap(); }
}

fn tick(s: &mut Script<Document>) { s.dispatch("tick").unwrap(); }

#[test]
fn oscillator_plays_and_ends() {
    let hub = Rc::new(RefCell::new(Hub::new()));
    let mut s = page(&hub, false);
    s.eval(r#"
        console.log(typeof AudioContext, webkitAudioContext === AudioContext, typeof OfflineAudioContext);
        const ctx = new AudioContext();
        window.ctx = ctx;
        console.log(ctx.state, ctx.sampleRate, ctx.currentTime, ctx.destination.maxChannelCount, ctx.destination.numberOfInputs);
        const o = ctx.createOscillator(), g = new GainNode(ctx, { gain: 0.5 });
        console.log(o.type, o.frequency.value, o.frequency.maxValue, g.gain.value, o.connect(g) === g);
        g.connect(ctx.destination);
        o.onended = () => console.log('ended', ctx.currentTime > 0.09);
        o.start();
        o.stop(0.1);
        try { o.start(); } catch (e) { console.log(e.name); }
        try { ctx.createAnalyser().fftSize = 1000; } catch (e) { console.log(e.name); }
        try { g.gain.exponentialRampToValueAtTime(0, 1); } catch (e) { console.log(e.name); }
        try { o.connect({}); } catch (e) { console.log(e.name); }
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["function true function", "running 48000 0 2 1", "sine 440 24000 0.5 true", "InvalidStateError", "IndexSizeError", "RangeError", "TypeError"]);
    let out = render(&hub, 40);
    let peak = out[..4800].iter().map(|f| f[0].abs()).max().unwrap();
    assert!((16000..=16400).contains(&peak), "{peak}");
    assert!(out[4800..].iter().all(|f| *f == [0, 0]), "stopped at 0.1 s");
    deliver(&hub, &mut s);
    assert_eq!(logs(&mut s), ["ended true"]);
    s.eval("console.log(ctx.currentTime.toFixed(4))", "t.js").unwrap();
    assert_eq!(logs(&mut s), [format!("{:.4}", 40.0 * 128.0 / 48000.0)]);
    // Suspend stops the clock.
    s.eval("ctx.suspend().then(() => console.log('state', ctx.state))", "t.js").unwrap();
    tick(&mut s);
    std::thread::sleep(std::time::Duration::from_millis(5));
    tick(&mut s);
    assert_eq!(logs(&mut s), ["state suspended"]);
    assert!(!hub.borrow().running());
}

#[test]
fn microphone_into_analyser() {
    let hub = Rc::new(RefCell::new(Hub::new()));
    let mut s = page(&hub, true);
    s.eval(r#"
        navigator.mediaDevices.getUserMedia({ audio: true }).then(st => {
          const ctx = new AudioContext();
          const src = ctx.createMediaStreamSource(st);
          const an = ctx.createAnalyser();
          an.smoothingTimeConstant = 0;
          src.connect(an);
          window.an = an; window.src = src;
          console.log('mic', src.mediaStream === st, an.frequencyBinCount, an.fftSize);
        });
    "#, "t.js").unwrap();
    let id = std::mem::take(&mut s.page_mut().capture_asks)[0].id;
    s.dispatch(&format!("capture\t{id}\tok\tBuilt-in microphone\tmic0\t\t\t0\t0\t0")).unwrap();
    assert_eq!(logs(&mut s), ["mic true 1024 2048"]);
    // A 1 kHz tone from the microphone.
    for q in 0..24 {
        let f: Vec<[i16; 2]> = (0..QUANTUM).map(|i| { let t = (q * QUANTUM + i) as f64 / 48000.0; let v = ((t * 1000.0 * std::f64::consts::TAU).sin() * 8000.0) as i16; [v, v] }).collect();
        hub.borrow_mut().feed(&f);
        render(&hub, 1);
    }
    s.eval(r#"
        const f = new Float32Array(an.frequencyBinCount), b = new Uint8Array(an.frequencyBinCount), t = new Float32Array(an.fftSize);
        an.getFloatFrequencyData(f); an.getByteFrequencyData(b); an.getFloatTimeDomainData(t);
        let k = 0; for (let i = 1; i < f.length; i++) if (f[i] > f[k]) k = i;
        console.log('peak', k, Math.round(k * ctx_rate() / an.fftSize), b[k] > 200, Math.max(...t) > 0.2 && Math.max(...t) < 0.3);
        function ctx_rate() { return an.context.sampleRate; }
    "#, "t.js").unwrap();
    // 1000 Hz * 2048 / 48000 = 42.7
    assert_eq!(logs(&mut s), ["peak 43 1008 true true"]);
}

#[test]
fn offline_rendering_buffers_and_decoding() {
    let hub = Rc::new(RefCell::new(Hub::new()));
    hub.borrow_mut().decoder = Some(|b: &[u8]| Some(sex_webaudio::Decoded { rate: 48_000, channels: 2, frames: b.iter().map(|v| [*v as i16 * 256, -(*v as i16) * 256]).collect() }));
    let mut s = page(&hub, false);
    s.eval(r#"
        const oc = new OfflineAudioContext(2, 4800, 48000);
        const buf = oc.createBuffer(1, 100, 48000);
        buf.getChannelData(0).fill(0.25);
        const src = oc.createBufferSource();
        src.buffer = buf;
        const p = oc.createStereoPanner(); p.pan.value = 1;
        src.connect(p).connect(oc.destination);
        src.start(0.001);
        src.onended = () => console.log('src ended');
        oc.oncomplete = e => console.log('complete', e.renderedBuffer.length);
        oc.startRendering().then(r => {
          window.done = r;
          const l = r.getChannelData(0), rr = r.getChannelData(1);
          console.log('rendered', r.numberOfChannels, r.length, r.duration, l[60], rr[60].toFixed(3), rr[47], rr[200], oc.state);
        });
        const ac = new AudioContext();
        ac.decodeAudioData(new Uint8Array([10, 20, 30]).buffer, b => console.log('cb', b.length)).then(b => {
          console.log('decoded', b.numberOfChannels, b.sampleRate, b.length, b.getChannelData(0)[1].toFixed(4), b.getChannelData(1)[2].toFixed(4));
        });
        ac.decodeAudioData(new ArrayBuffer(0)).catch(e => console.log('bad', e.name));
        const bq = ac.createBiquadFilter();
        const hz = new Float32Array([100, 350, 10000]), m = new Float32Array(3), ph = new Float32Array(3);
        bq.getFrequencyResponse(hz, m, ph);
        console.log('response', m[0] > 0.98 && m[0] < 1.1, m[2] < 0.01, (m[1]).toFixed(2));
        const osc = ac.createOscillator(); osc.setPeriodicWave(ac.createPeriodicWave(new Float32Array([0, 0]), new Float32Array([0, 1])));
        console.log(osc.type);
        const dst = ac.createMediaStreamDestination();
        const tr = dst.stream.getAudioTracks()[0];
        console.log('stream', tr.kind, tr.readyState, typeof tr._tap, ac.createMediaStreamSource(dst.stream).numberOfOutputs);
        g = ac.createGain(); g.gain.setValueAtTime(0.2, 0); g.gain.linearRampToValueAtTime(1, 1);
        console.log('param', g.gain.value.toFixed(2));
    "#, "t.js").unwrap();
    for _ in 0..3 { tick(&mut s); std::thread::sleep(std::time::Duration::from_millis(2)); }
    let mut l = logs(&mut s);
    l.sort();
    let mut want = vec!["bad EncodingError", "cb 3", "complete 4800", "custom", "decoded 2 48000 3 0.1562 -0.2344", "param 0.20",
        "rendered 2 4800 0.1 0 0.250 0 0 closed", "response true true 1.12", "src ended", "stream audio live number 1"];
    want.sort();
    assert_eq!(l, want);
}
