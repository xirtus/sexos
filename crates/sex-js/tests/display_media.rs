// SPDX-License-Identifier: GPL-3.0-only
// Screen sharing for pages: navigator.mediaDevices.getDisplayMedia over op::CAPTURE.
use sex_js::{Capture, Document, Limits, Script};
use std::cell::RefCell;
use std::rc::Rc;

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(url: &str, html: &str) -> Script<Document> {
    let dom = sex_html::parse::parse(html);
    let mut d = Document::new(dom, url, (410, 330), clock);
    let mut c = Capture::default();
    c.devices.push(sex_js::capture::Device { kind: "videoinput", id: String::from("cam0"), label: String::from("Integrated webcam") });
    d.capture = Some(Rc::new(RefCell::new(c)));
    Script::new(d, Limits::default()).expect("engine")
}

fn logs(s: &mut Script<Document>) -> Vec<String> {
    s.page_mut().log.drain(..).map(|(l, t)| format!("{l}:{t}")).collect()
}

fn media(s: &mut Script<Document>) -> Vec<String> {
    s.page_mut().media.drain(..).map(|(_, v, a)| format!("{v}={a}")).collect()
}

#[test]
fn get_display_media_shares_the_screen_into_video_and_ends_when_stopped() {
    let mut s = page("https://studio.example/broadcast", "<video id=v autoplay></video><video id=c autoplay></video>");
    s.eval(r#"
        navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 15 }, audio: true }).then(st => {
          window.st = st;
          const [t] = st.getVideoTracks();
          const g = t.getSettings();
          console.log('got', st.getTracks().length, st.getAudioTracks().length, t.kind, t.label, t.readyState, t.contentHint);
          console.log('settings', g.displaySurface, g.width, g.height, g.frameRate, g.deviceId);
          t.onended = () => console.log('ended', t.readyState, st.active);
          document.getElementById('v').srcObject = st;
        }, e => console.log('failed', e.name));
    "#, "t.js").unwrap();
    assert!(logs(&mut s).is_empty(), "nothing before the person answers");
    let asks = std::mem::take(&mut s.page_mut().capture_asks);
    assert_eq!(asks.len(), 1);
    assert!(asks[0].screen && asks[0].video && !asks[0].audio, "no sound is shared yet");
    assert_eq!(asks[0].origin, "https://studio.example");
    s.dispatch(&format!("capture\t{}\tok\t\t\tNotes - Quil\tscreen\t864\t540\t15\twindow", asks[0].id)).unwrap();
    assert_eq!(logs(&mut s), ["0:got 1 0 video Notes - Quil live detail", "0:settings window 864 540 15 screen"]);
    // The page uses the screen, not the camera.
    assert_eq!(s.page().capture_use, Some((false, false)));
    assert_eq!(s.page_mut().screen_use.take(), Some(true));
    let m = media(&mut s);
    assert!(m.contains(&String::from("live=screen")), "{m:?}");

    // The camera can be live beside it: each element shows its own source.
    s.eval(r#"
        navigator.mediaDevices.getUserMedia({ video: true }).then(cam => {
          window.cam = cam;
          document.getElementById('c').srcObject = cam;
          console.log('camera', cam.getVideoTracks()[0].label, cam.getVideoTracks()[0].getSettings().displaySurface);
        });
    "#, "t.js").unwrap();
    let asks = std::mem::take(&mut s.page_mut().capture_asks);
    assert!(!asks[0].screen);
    s.dispatch(&format!("capture\t{}\tok\t\t\tIntegrated webcam\tcam0\t640\t480\t30", asks[0].id)).unwrap();
    assert_eq!(logs(&mut s), ["0:camera Integrated webcam undefined"]);
    assert_eq!(s.page().capture_use, Some((false, true)));
    assert_eq!(s.page_mut().screen_use.take(), Some(true));
    let m = media(&mut s);
    assert!(m.contains(&String::from("live=video")), "{m:?}");

    // The person pressed "Stop sharing" in the Shell: only the screen track ends.
    s.dispatch("capturegone\tscreen").unwrap();
    assert_eq!(logs(&mut s), ["0:ended ended false"]);
    s.eval("console.log('camera still', cam.getVideoTracks()[0].readyState)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:camera still live"]);
    let m = media(&mut s);
    assert!(m.contains(&String::from("live=")), "the screen's element blanks: {m:?}");
}

#[test]
fn the_page_stops_sharing_and_refusals() {
    let mut s = page("https://meet.example/", "");
    s.eval(r#"
        navigator.mediaDevices.getDisplayMedia().then(st => { window.st = st; console.log('ok', st.getVideoTracks()[0].getSettings().displaySurface); });
        const show = p => p.then(() => console.log('ok'), e => console.log(e.name, e.message));
        show(navigator.mediaDevices.getDisplayMedia({ video: false }));
        show(navigator.mediaDevices.getDisplayMedia({ video: { width: { exact: 640 } } }));
        show(navigator.mediaDevices.getDisplayMedia(7));
        show(navigator.mediaDevices.getDisplayMedia({ video: true }));
        show(navigator.mediaDevices.getDisplayMedia({ video: true }));
    "#, "t.js").unwrap();
    let l = logs(&mut s);
    assert!(l.contains(&String::from("0:TypeError Failed to execute 'getDisplayMedia' on 'MediaDevices': video must be requested")), "{l:?}");
    assert!(l.contains(&String::from("0:TypeError Failed to execute 'getDisplayMedia' on 'MediaDevices': min and exact are not allowed")), "{l:?}");
    assert_eq!(l.len(), 3, "{l:?}");
    let asks = std::mem::take(&mut s.page_mut().capture_asks);
    assert_eq!(asks.len(), 3);
    s.dispatch(&format!("capture\t{}\tok\t\t\tScreen\tscreen\t960\t540\t15\tmonitor", asks[0].id)).unwrap();
    // The person said no in the Shell's picker; another program was sharing.
    s.dispatch(&format!("capture\t{}\tdenied", asks[1].id)).unwrap();
    s.dispatch(&format!("capture\t{}\tbusy", asks[2].id)).unwrap();
    assert_eq!(logs(&mut s), ["0:ok monitor", "0:NotAllowedError Permission denied", "0:NotReadableError Could not start video source"]);
    assert_eq!(s.page_mut().screen_use.take(), Some(true));
    // The page stops its track: the browser lets the share go (no `ended` event).
    s.eval("st.getVideoTracks()[0].onended = () => console.log('ended'); st.getVideoTracks()[0].stop(); console.log(st.active)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:false"]);
    assert_eq!(s.page_mut().screen_use.take(), Some(false));

    // Plain http pages may not ask; the Shell is never bothered.
    let mut s = page("http://insecure.example/", "");
    s.eval("navigator.mediaDevices.getDisplayMedia({ video: true }).catch(e => console.log(e.name))", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:NotAllowedError"]);
    assert!(s.page().capture_asks.is_empty());
}
