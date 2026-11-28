// SPDX-License-Identifier: GPL-3.0-only
// MediaRecorder and canvas.captureStream for pages, over op::RECORDER: what the page
// asks the browser, and the pieces the browser hands back as Blobs.
use sex_js::recorder::{self, Ask, OPUS, VP8, VP9};
use sex_js::{Capture, Document, Limits, Script};
use std::cell::RefCell;
use std::rc::Rc;

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(html: &str, codecs: u8) -> Script<Document> {
    let dom = sex_html::parse::parse(html);
    let mut d = Document::new(dom, "https://studio.example/room", (410, 330), clock);
    let mut c = Capture::default();
    c.devices.push(sex_js::capture::Device { kind: "audioinput", id: String::from("mic0"), label: String::from("Built-in microphone") });
    c.devices.push(sex_js::capture::Device { kind: "videoinput", id: String::from("cam0"), label: String::from("Integrated webcam") });
    d.capture = Some(Rc::new(RefCell::new(c)));
    d.recorders.codecs = codecs;
    Script::new(d, Limits::default()).expect("engine")
}

fn logs(s: &mut Script<Document>) -> Vec<String> {
    s.page_mut().log.drain(..).map(|(l, t)| format!("{l}:{t}")).collect()
}

fn asks(s: &mut Script<Document>) -> Vec<Ask> { std::mem::take(&mut s.page_mut().recorders.asks) }

/// The page's camera and microphone stream, as getUserMedia gives it.
fn camera(s: &mut Script<Document>) {
    s.eval("navigator.mediaDevices.getUserMedia({ audio: true, video: true }).then(st => { window.st = st; })", "t.js").unwrap();
    let id = std::mem::take(&mut s.page_mut().capture_asks)[0].id;
    s.dispatch(&format!("capture\t{id}\tok\tBuilt-in microphone\tmic0\tIntegrated webcam\tcam0\t640\t480\t30")).unwrap();
}

const ALL: u8 = VP8 | VP9 | OPUS;

#[test]
fn types_follow_what_the_browser_records() {
    let mut s = page("", ALL);
    s.eval(r#"
        const t = ['video/webm', 'video/webm;codecs=vp8', 'video/webm;codecs="vp8,opus"', 'video/webm;codecs=vp9,opus', 'video/webm;codecs=vp09.00.10.08',
          'audio/webm', 'audio/webm;codecs=opus', 'video/webm;codecs=opus', '', 'video/mp4', 'video/webm;codecs=h264', 'audio/ogg', 'audio/webm;codecs=vp8', 'video/webm;codecs=vp8,vp9'];
        console.log(typeof MediaRecorder, typeof BlobEvent, t.map(x => MediaRecorder.isTypeSupported(x) ? 1 : 0).join(''));
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:function function 11111111100000"]);
    // A build without the encoders records nothing (an empty type is still "supported").
    let mut s = page("", 0);
    s.eval("console.log(MediaRecorder.isTypeSupported('video/webm'), MediaRecorder.isTypeSupported('audio/webm;codecs=opus'), MediaRecorder.isTypeSupported(''))", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:false false true"]);
    // VP8 only: VP9 is not offered.
    assert!(recorder::kind(VP8 | OPUS, "video/webm;codecs=vp9").is_none());
    assert_eq!(recorder::choose(VP8 | OPUS, "", true, true).map(|c| c.0), Some(String::from("video/webm;codecs=vp8,opus")));
    assert_eq!(recorder::choose(ALL, "", false, true).map(|c| c.0), Some(String::from("audio/webm;codecs=opus")));
    assert_eq!(recorder::choose(ALL, "video/webm;codecs=opus", true, true).map(|c| (c.0, c.1, c.2)), Some((String::from("video/webm;codecs=opus"), 0, true)));
    assert_eq!(recorder::choose(ALL, "audio/webm", true, false), None);
}

#[test]
fn records_the_camera_and_microphone_in_pieces() {
    let mut s = page("", ALL);
    camera(&mut s);
    s.eval(r#"
        const r = new MediaRecorder(st, { mimeType: 'video/webm;codecs=vp9,opus', videoBitsPerSecond: 1000000 });
        window.r = r; window.got = [];
        console.log(r.state, r.mimeType, r.videoBitsPerSecond, r.audioBitsPerSecond, r.stream === st);
        r.onstart = () => console.log('start', r.state);
        r.ondataavailable = e => { got.push(e.data); console.log('data', e instanceof BlobEvent, e.data.size, e.data.type); };
        r.addEventListener('stop', () => console.log('stop', r.state, got.length));
        r.start(1000);
        console.log(r.state, r.mimeType);
        try { r.start(); } catch (e) { console.log(e.name); }
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:inactive video/webm;codecs=vp9,opus 1000000 128000 true", "0:recording video/webm;codecs=vp9,opus", "0:InvalidStateError"]);
    let a = asks(&mut s);
    assert_eq!(a, [Ask::Start { id: 1, video: "camera".into(), audio: "mic".into(), mime: "video/webm;codecs=vp9,opus".into(), codec: VP9, video_bps: 1_000_000, audio_bps: 128_000, timeslice_ms: 1000, fps: 30 }]);
    // The browser opened its encoder and hands over a piece.
    s.dispatch("recorder\t1\tstart").unwrap();
    assert!(s.page_mut().recorders.give(1, vec![0x1A, 0x45, 0xDF, 0xA3, 1, 2, 3]));
    s.dispatch("recorder\t1\tdata\t7").unwrap();
    assert_eq!(logs(&mut s), ["0:start recording", "0:data true 7 video/webm;codecs=vp9,opus"]);
    assert!(s.page().recorders.pieces.is_empty(), "the piece was taken");
    // Pause, resume and requestData go to the browser; their events come after.
    s.eval("r.pause(); console.log(r.state); r.onpause = () => console.log('paused'); r.onresume = () => console.log('resumed')", "t.js").unwrap();
    s.dispatch("tick").unwrap();
    s.eval("r.resume(); r.requestData()", "t.js").unwrap();
    s.dispatch("tick").unwrap();
    assert_eq!(logs(&mut s), ["0:paused", "0:paused", "0:resumed"]);
    assert_eq!(asks(&mut s), [Ask::Pause(1), Ask::Resume(1), Ask::Data(1)]);
    // Disabling the camera track tells the browser (black pictures from then on).
    s.eval("st.getVideoTracks()[0].enabled = false", "t.js").unwrap();
    assert_eq!(asks(&mut s), [Ask::Sources { id: 1, video: "-camera".into(), audio: "mic".into() }]);
    // stop: inactive at once; the last piece, then `stop`.
    s.eval("r.stop(); console.log(r.state); r.stop(); try { r.pause() } catch (e) { console.log(e.name) }", "t.js").unwrap();
    assert_eq!(asks(&mut s), [Ask::Stop(1)]);
    assert!(s.page_mut().recorders.give(1, vec![9; 100]));
    s.dispatch("recorder\t1\tdata\t100").unwrap();
    s.dispatch("recorder\t1\tstop").unwrap();
    s.eval("new Blob(got).arrayBuffer().then(b => console.log('whole', b.byteLength, new Uint8Array(b)[0]))", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:inactive", "0:InvalidStateError", "0:data true 100 video/webm;codecs=vp9,opus", "0:stop inactive 2", "0:whole 107 26"]);
}

#[test]
fn canvas_capture_stream_records_the_canvas() {
    let mut s = page("<canvas id=c width=320 height=180></canvas><video id=v></video>", ALL);
    s.eval(r#"
        const c = document.getElementById('c');
        const cs = c.captureStream(25);
        const [t] = cs.getVideoTracks();
        console.log(cs instanceof MediaStream, t instanceof CanvasCaptureMediaStreamTrack, t.kind, t.canvas === c, t.getSettings().width, t.getSettings().height, typeof t.requestFrame);
        const r = new MediaRecorder(cs);
        r.start();
        console.log(r.mimeType);
        document.getElementById('v').srcObject = cs;
        window.cs = cs; window.r = r;
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:true true video true 320 180 function", "0:video/webm;codecs=vp8"]);
    s.eval("console.log(document.getElementById('c')._id)", "t.js").unwrap();
    let node = logs(&mut s)[0].trim_start_matches("0:").to_string();
    let a = asks(&mut s);
    assert_eq!(a, [Ask::Start { id: 1, video: format!("canvas:{node}"), audio: String::new(), mime: "video/webm;codecs=vp8".into(), codec: VP8, video_bps: 2_500_000, audio_bps: 128_000, timeslice_ms: 0, fps: 25 }]);
    // A video element shows the canvas's stream, never the camera; the canvas holds no device.
    let media: Vec<_> = s.page_mut().media.drain(..).map(|(_, v, a)| format!("{v}={a}")).collect();
    assert!(media.contains(&format!("live=canvas:{node}")), "{media:?}");
    assert_eq!(s.page().capture_use, None);
    // A camera track and a canvas track mixed: the camera's sound and the canvas's pictures.
    camera(&mut s);
    s.eval(r#"
        const mixed = new MediaStream([cs.getVideoTracks()[0], st.getAudioTracks()[0]]);
        const m = new MediaRecorder(mixed, { mimeType: 'video/webm;codecs=vp8,opus' });
        m.start(500);
        // The camera's own video track stops: the camera goes, the recording goes on.
        st.getVideoTracks()[0].stop();
        window.m = m;
    "#, "t.js").unwrap();
    let a = asks(&mut s);
    assert!(matches!(&a[0], Ask::Start { id: 2, video, audio, timeslice_ms: 500, .. } if *video == format!("canvas:{node}") && audio == "mic"), "{a:?}");
    assert_eq!(s.page().capture_use, Some((true, false)));
    // Every track ends: the recorder stops by itself.
    s.eval("m.onstop = () => console.log('m stopped'); cs.getVideoTracks()[0].stop(); st.getAudioTracks()[0].stop(); console.log(m.state, r.state)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:inactive inactive"]);
    assert_eq!(asks(&mut s), [Ask::Stop(1), Ask::Sources { id: 2, video: String::new(), audio: "mic".into() }, Ask::Stop(2)]);
}

#[test]
fn refusals() {
    let mut s = page("<canvas id=c></canvas>", ALL);
    s.eval(r#"
        try { new MediaRecorder({}); } catch (e) { console.log(e.name); }
        const st = document.getElementById('c').captureStream();
        try { new MediaRecorder(st, { mimeType: 'video/mp4' }); } catch (e) { console.log(e.name); }
        // audio/webm of a stream with only pictures has nothing to record.
        const r = new MediaRecorder(st, { mimeType: 'audio/webm' });
        try { r.start(); } catch (e) { console.log(e.name, r.state); }
        st.getTracks()[0].stop();
        try { new MediaRecorder(st).start(); } catch (e) { console.log(e.name); }
        try { new BlobEvent('dataavailable', {}); } catch (e) { console.log(e.name); }
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:TypeError", "0:NotSupportedError", "0:NotSupportedError inactive", "0:NotSupportedError", "0:TypeError"]);
    // A build without encoders: start fails, construction with a type fails.
    let mut s = page("<canvas id=c></canvas>", 0);
    s.eval("const st = document.getElementById('c').captureStream(); const r = new MediaRecorder(st); try { r.start() } catch (e) { console.log(e.name) }", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:NotSupportedError"]);
    // The browser failing a recorder: `error`, then `stop`.
    let mut s = page("<canvas id=c></canvas>", ALL);
    s.eval("const r = new MediaRecorder(document.getElementById('c').captureStream()); r.onerror = e => console.log('error', e.error.name, r.state); r.onstop = () => console.log('stop'); r.start()", "t.js").unwrap();
    s.dispatch("recorder\t1\terror\tNotSupportedError").unwrap();
    s.dispatch("recorder\t1\tstop").unwrap();
    assert_eq!(logs(&mut s), ["0:error NotSupportedError inactive", "0:stop"]);
}

#[test]
fn a_recorded_blob_plays_in_a_video_element() {
    let mut s = page("<video id=v></video>", ALL);
    s.eval(r#"
        const b = new Blob([new Uint8Array([0x1A, 0x45, 0xDF, 0xA3, 5])], { type: 'video/webm' });
        const v = document.getElementById('v');
        v.src = URL.createObjectURL(b);
    "#, "t.js").unwrap();
    let media: Vec<_> = s.page_mut().media.drain(..).map(|(_, v, a)| format!("{v}={a}")).collect();
    assert!(media.contains(&String::from("blob=")), "{media:?}");
    assert!(!media.iter().any(|m| m.starts_with("load=")), "{media:?}");
    let bytes = std::mem::take(&mut s.page_mut().media_bytes);
    assert_eq!(bytes.len(), 1);
    assert_eq!((bytes[0].1, bytes[0].2.as_slice()), (sex_js::document::MEDIA_FILE, &[0x1A, 0x45, 0xDF, 0xA3, 5][..]));
}

#[test]
fn pieces_waiting_are_bounded() {
    let mut r = recorder::Recorders { codecs: ALL, ..Default::default() };
    assert!(r.give(1, vec![0; recorder::MAX_WAITING - 10]));
    assert!(!r.give(1, vec![0; 11]));
    assert!(r.take(2).is_none());
    assert_eq!(r.take(1).map(|b| b.len()), Some(recorder::MAX_WAITING - 10));
    for i in 0..recorder::MAX_RECORDERS as u32 { assert!(r.call(&["start", &i.to_string(), "camera", "", "", "", "", "", ""]).is_some()); }
    assert!(r.call(&["start", "99", "camera", "", "", "", "", "", ""]).is_none(), "too many recorders");
    r.ended(3);
    assert!(r.call(&["start", "99", "camera", "", "", "", "", "", ""]).is_some());
    assert!(r.call(&["pause", "1000"]).is_none() && !r.asks.iter().any(|a| a.id() == 1000), "unknown recorders are ignored");
}
