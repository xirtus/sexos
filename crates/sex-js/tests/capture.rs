// Camera and microphone for pages: navigator.mediaDevices over op::CAPTURE.
use sex_js::{Capture, Decision, Document, Limits, Script};
use std::cell::RefCell;
use std::rc::Rc;

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(url: &str, html: &str, capture: Option<Rc<RefCell<Capture>>>) -> Script<Document> {
    let dom = sex_html::parse::parse(html);
    let mut d = Document::new(dom, url, (410, 330), clock);
    d.capture = capture;
    Script::new(d, Limits::default()).expect("engine")
}

fn logs(s: &mut Script<Document>) -> Vec<String> {
    s.page_mut().log.drain(..).map(|(l, t)| format!("{l}:{t}")).collect()
}

fn shared() -> Rc<RefCell<Capture>> {
    let mut c = Capture::default();
    c.devices.push(sex_js::capture::Device { kind: "audioinput", id: String::from("mic0"), label: String::from("Built-in microphone") });
    c.devices.push(sex_js::capture::Device { kind: "videoinput", id: String::from("cam0"), label: String::from("Integrated webcam") });
    Rc::new(RefCell::new(c))
}

#[test]
fn get_user_media_resolves_with_tracks_and_plays_in_video() {
    let cap = shared();
    let mut s = page("https://studio.example/room", "<video id=v autoplay muted></video>", Some(cap.clone()));
    s.eval(r#"
        console.log(typeof navigator.mediaDevices.getUserMedia, typeof MediaStream, typeof MediaStreamTrack);
        navigator.mediaDevices.getUserMedia({ audio: true, video: { width: 1280, deviceId: { exact: 'cam0' } } }).then(st => {
          window.st = st;
          console.log('got', st.active, st.getTracks().map(t => t.kind + ':' + t.label + ':' + t.readyState).join(','));
          const v = st.getVideoTracks()[0].getSettings();
          console.log('settings', v.width, v.height, v.frameRate, v.deviceId, st.getVideoTracks()[0].getConstraints().width);
          document.getElementById('v').srcObject = st;
        }, e => console.log('failed', e.name));
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:function function function"]);
    let asks = std::mem::take(&mut s.page_mut().capture_asks);
    assert_eq!(asks.len(), 1);
    assert_eq!((asks[0].origin.as_str(), asks[0].audio, asks[0].video, asks[0].video_device.as_str()), ("https://studio.example", true, true, "cam0"));
    s.dispatch(&format!("capture\t{}\tok\tBuilt-in microphone\tmic0\tIntegrated webcam\tcam0\t1280\t720\t30", asks[0].id)).unwrap();
    assert_eq!(logs(&mut s), ["0:got true audio:Built-in microphone:live,video:Integrated webcam:live", "0:settings 1280 720 30 cam0 1280"]);
    assert_eq!(s.page().capture_use, Some((true, true)));
    // The video element is told to show the camera.
    let media: Vec<_> = s.page_mut().media.drain(..).map(|(_, v, a)| format!("{v}={a}")).collect();
    assert!(media.contains(&String::from("live=video")), "{media:?}");
    // Stopping the camera track lets the camera go and blanks the element.
    s.eval("st.getVideoTracks()[0].stop(); console.log(st.active, st.getVideoTracks()[0].readyState)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:true ended"]);
    assert_eq!(s.page().capture_use, Some((true, false)));
    let media: Vec<_> = s.page_mut().media.drain(..).map(|(_, v, a)| format!("{v}={a}")).collect();
    assert_eq!(media, ["live="]);
    // The browser ending the microphone fires `ended`.
    s.eval("st.getAudioTracks()[0].onended = () => console.log('mic ended'); st.getAudioTracks()[0].addEventListener('mute', () => console.log('muted'))", "t.js").unwrap();
    s.dispatch("capturemute\taudio\t1").unwrap();
    s.dispatch("capturegone\taudio").unwrap();
    s.eval("console.log(st.active)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:muted", "0:mic ended", "0:false"]);
}

#[test]
fn refusals_and_errors() {
    let cap = shared();
    let mut s = page("https://meet.example/", "", Some(cap.clone()));
    s.eval(r#"
        const show = p => p.then(() => console.log('ok'), e => console.log(e.name, e.message));
        show(navigator.mediaDevices.getUserMedia({}));
        show(navigator.mediaDevices.getUserMedia({ video: true }));
        show(navigator.mediaDevices.getUserMedia({ audio: true }));
        show(navigator.mediaDevices.getDisplayMedia({ video: true }));
    "#, "t.js").unwrap();
    let asks = std::mem::take(&mut s.page_mut().capture_asks);
    assert_eq!(asks.len(), 3);
    assert!(asks[2].screen && !asks[0].screen, "getDisplayMedia asks for the screen");
    s.dispatch(&format!("capture\t{}\tdenied", asks[0].id)).unwrap();
    s.dispatch(&format!("capture\t{}\tnone", asks[1].id)).unwrap();
    s.dispatch(&format!("capture\t{}\tbusy", asks[2].id)).unwrap();
    let l = logs(&mut s);
    assert!(l.contains(&String::from("0:TypeError Failed to execute 'getUserMedia' on 'MediaDevices': At least one of audio and video must be requested")), "{l:?}");
    assert!(l.contains(&String::from("0:NotAllowedError Permission denied")), "{l:?}");
    assert!(l.contains(&String::from("0:NotFoundError Requested device not found")), "{l:?}");
    assert!(l.contains(&String::from("0:NotReadableError Could not start video source")), "{l:?}");

    // A plain http page may not ask at all; the browser is never bothered.
    let mut s = page("http://insecure.example/", "", Some(cap.clone()));
    s.eval("navigator.mediaDevices.getUserMedia({ video: true }).catch(e => console.log(e.name))", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:NotAllowedError"]);
    assert!(s.page().capture_asks.is_empty());
    // ...unless it is this machine.
    let mut s = page("http://localhost:8080/", "", Some(cap));
    s.eval("navigator.mediaDevices.getUserMedia({ video: true }); console.log('asked')", "t.js").unwrap();
    assert_eq!(s.page().capture_asks.len(), 1);
}

#[test]
fn devices_and_permissions_follow_the_persons_choices() {
    let cap = shared();
    let mut s = page("https://studio.example/", "", Some(cap.clone()));
    let list = r#"
        navigator.mediaDevices.enumerateDevices().then(l => console.log(l.map(d => d.kind + '=' + d.label + '/' + d.deviceId + ' ' + (d instanceof InputDeviceInfo)).join(' ')));
        Promise.all([navigator.permissions.query({ name: 'camera' }), navigator.permissions.query({ name: 'microphone' })]).then(([c, m]) => console.log(c.state, m.state));
    "#;
    s.eval(list, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:audioinput=/ true videoinput=/ true", "0:prompt prompt"]);
    cap.borrow_mut().decide("https://studio.example", true, Decision::Allow);
    cap.borrow_mut().decide("https://studio.example", false, Decision::Block);
    s.eval(list, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:audioinput=/ true videoinput=Integrated webcam/cam0 true", "0:granted denied"]);
    // Another site knows nothing of it.
    let mut other = page("https://other.example/", "", Some(cap.clone()));
    other.eval("navigator.permissions.query({ name: 'camera' }).then(p => console.log(p.state))", "t.js").unwrap();
    assert_eq!(logs(&mut other), ["0:prompt"]);
    // The choices survive a save and load.
    let saved = cap.borrow().save();
    assert_eq!(saved, "https://studio.example\tblock\tallow\n");
    let mut back = Capture::default();
    back.load(&saved);
    assert_eq!(back.decision("https://studio.example", true), Decision::Allow);
    assert_eq!(back.decision("https://studio.example", false), Decision::Block);
    back.decide("https://studio.example", true, Decision::Ask);
    back.decide("https://studio.example", false, Decision::Ask);
    assert!(back.sites.is_empty());
}

#[test]
fn streams_and_tracks_behave_like_other_browsers() {
    let mut s = page("https://studio.example/", "<canvas id=c width=8 height=8></canvas><video id=v></video>", Some(shared()));
    s.eval(r#"
        navigator.mediaDevices.getUserMedia({ video: true }).then(st => {
          const t = st.getVideoTracks()[0];
          const copy = st.clone();
          console.log(copy.id !== st.id, copy.getTracks()[0].id !== t.id, st.getTrackById(t.id) === t);
          const mixed = new MediaStream([t]);
          mixed.removeTrack(t); console.log(mixed.getTracks().length, mixed.active);
          mixed.addTrack(t); console.log(mixed.getTracks().length, mixed.active);
          t.enabled = false;
          copy.getTracks()[0].stop();
          // A video's picture can be drawn on a canvas once it has one.
          document.getElementById('c').getContext('2d').drawImage(document.getElementById('v'), 0, 0);
          console.log('done');
        });
    "#, "t.js").unwrap();
    let id = s.page_mut().capture_asks.remove(0).id;
    s.dispatch(&format!("capture\t{id}\tok\t\t\tCam\tcam0\t640\t480\t30")).unwrap();
    assert_eq!(logs(&mut s), ["0:true true true", "0:0 false", "0:1 true", "0:done"]);
    // Disabled, the only camera track no longer needs the camera.
    assert_eq!(s.page().capture_use, Some((false, false)));
}
