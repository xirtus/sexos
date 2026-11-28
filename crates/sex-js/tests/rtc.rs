// RTCPeerConnection over op::RTC, with the browser's engine (sex-rtc) behind it: two
// peer connections in one page call each other, the way the QEMU gate's page does.
use sex_js::{Capture, Document, Limits, Script};
use std::cell::RefCell;
use std::rc::Rc;

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn logs(s: &mut Script<Document>) -> Vec<String> {
    s.page_mut().log.drain(..).map(|(_, t)| t).collect()
}

const PAGE: &str = r#"
  window.log = [];
  const say = (...a) => console.log(a.join(' '));
  const pc1 = new RTCPeerConnection({ iceServers: [{ urls: 'stun:stun.example.org:3478' }], bundlePolicy: 'max-bundle' });
  const pc2 = new RTCPeerConnection();
  window.pc1 = pc1; window.pc2 = pc2;
  pc1.onicecandidate = e => { if (e.candidate) pc2.addIceCandidate(e.candidate); else say('pc1 gathered'); };
  pc2.onicecandidate = e => { if (e.candidate) pc1.addIceCandidate(e.candidate.toJSON()); };
  pc1.onnegotiationneeded = () => say('negotiationneeded');
  pc1.onsignalingstatechange = () => say('pc1 signaling', pc1.signalingState);
  pc1.onconnectionstatechange = () => say('pc1 connection', pc1.connectionState);
  pc2.addEventListener('connectionstatechange', () => say('pc2 connection', pc2.connectionState));
  pc2.ontrack = e => {
    say('track', e.track.kind, e.track.muted, e.streams.length, e.streams[0] && e.streams[0].id === window.camId, e.transceiver.mid);
    if (e.track.kind === 'video') { document.getElementById('remote').srcObject = e.streams[0]; e.track.onunmute = () => say('remote video unmuted'); }
  };
  navigator.mediaDevices.getUserMedia({ audio: true, video: true }).then(async st => {
    window.camId = st.id;
    for (const t of st.getTracks()) pc1.addTrack(t, st);
    const sender = pc1.getSenders().find(s => s.track.kind === 'video');
    const p = sender.getParameters();
    p.encodings[0].maxBitrate = 500000;
    await sender.setParameters(p);
    say('caps', RTCRtpSender.getCapabilities('video').codecs.map(c => c.mimeType).join(','));
    const offer = await pc1.createOffer();
    window.offer = offer.sdp;
    say('offer', offer.type, /a=fingerprint:sha-256 ([0-9A-F]{2}:){31}[0-9A-F]{2}/.test(offer.sdp), /a=ice-ufrag:\S+/.test(offer.sdp),
      /a=group:BUNDLE 0 1/.test(offer.sdp), /a=rtpmap:\d+ VP8\/90000/.test(offer.sdp), /a=rtpmap:\d+ opus\/48000\/2/.test(offer.sdp));
    await pc1.setLocalDescription(offer);
    await pc2.setRemoteDescription(pc1.localDescription);
    say('pc2 state', pc2.signalingState, pc2.getTransceivers().map(t => t.mid + ':' + t.direction).join(','));
    await pc2.setLocalDescription();
    say('answer', pc2.localDescription.type, /a=setup:active/.test(pc2.localDescription.sdp), pc2.localDescription.sdp.includes('a=recvonly'));
    await pc1.setRemoteDescription(pc2.localDescription);
    say('pc1 transceivers', pc1.getTransceivers().map(t => t.mid + ':' + t.currentDirection).join(','));
    try { await pc1.setRemoteDescription({ type: 'answer', sdp: 'nonsense' }); } catch (e) { say('bad sdp', e.name); }
    try { new RTCIceCandidate({ candidate: 'candidate:1 1 udp 1 1.2.3.4 5 typ host' }); } catch (e) { say('bad candidate', e.name); }
    const dc = pc1.createDataChannel('chat');
    say('datachannel', dc.label, dc.readyState);
  }, e => say('gum failed', e.name));
"#;

#[test]
fn two_peer_connections_in_one_page() {
    let rtc = Rc::new(RefCell::new(sex_rtc::Rtc::new([9; 32])));
    let mut cap = Capture::default();
    cap.any_origin = true;
    cap.devices.push(sex_js::capture::Device { kind: "audioinput", id: "mic0".into(), label: "Test microphone".into() });
    cap.devices.push(sex_js::capture::Device { kind: "videoinput", id: "cam0".into(), label: "Test camera".into() });
    let dom = sex_html::parse::parse("<video id=remote autoplay></video>");
    let mut d = Document::new(dom, "http://localhost/call", (640, 480), clock);
    d.capture = Some(Rc::new(RefCell::new(cap)));
    let r2 = rtc.clone();
    d.rtc = Some(Rc::new(move |owner, args: &[&str]| r2.borrow_mut().call(owner, args)));
    let owner = d.owner();
    let mut s = Script::new(d, Limits::default()).expect("engine");
    s.eval(PAGE, "page.js").unwrap();
    let asks = std::mem::take(&mut s.page_mut().capture_asks);
    s.dispatch(&format!("capture\t{}\tok\tTest microphone\tmic0\tTest camera\tcam0\t320\t240\t30", asks[0].id)).unwrap();
    for _ in 0..10 { s.dispatch("tick").unwrap(); }
    let got = logs(&mut s);
    let want = [
        "caps video/VP8,video/VP9,video/H264",
        "offer offer true true true true true",
        "pc1 signaling have-local-offer",
        "track audio true 1 true 0",
        "track video true 1 true 1",
        "pc2 state have-remote-offer 0:recvonly,1:recvonly",
        "answer answer true true",
        "pc1 signaling stable",
        "pc1 transceivers 0:sendonly,1:sendonly",
        "bad sdp OperationError",
        "bad candidate TypeError",
        "datachannel chat connecting",
    ];
    // Negotiation was under way when addTrack's check ran: no event then.
    assert!(!got.iter().any(|g| g == "negotiationneeded"), "{got:#?}");
    for w in want { assert!(got.iter().any(|g| g == w), "missing {w:?} in {got:#?}"); }
    // The remote element shows the remote track's source.
    let media: Vec<String> = s.page_mut().media.drain(..).map(|(_, v, a)| format!("{v}={a}")).collect();
    assert!(media.iter().any(|m| m == "live=rtc:2:1 rtc:2:0"), "{media:?}");

    // The network: both peers' sockets on one address, datagrams delivered in memory.
    let socks: Vec<(u32, std::net::SocketAddr)> = vec![(1, "10.0.2.15:41001".parse().unwrap()), (2, "10.0.2.15:41002".parse().unwrap())];
    let (w, h) = (160, 120);
    let mut frames = 0;
    for ms in 0..3000u64 {
        let mut events = Vec::new();
        {
            let mut r = rtc.borrow_mut();
            r.now = ms;
            for id in r.wants_transport() {
                let a = socks.iter().find(|s| s.0 == id).unwrap().1;
                // The STUN server does not answer here.
                let servers = if id == 1 { vec![Some("192.0.2.1:3478".parse().unwrap())] } else { vec![] };
                r.start_transport(id, &[a], &servers);
            }
            r.handle_timeout();
            if ms % 66 == 0 { let px = vec![0xff80_4020u32 + ms as u32; w * h]; r.send_picture("camera", &px, w, h); }
            for _ in 0..4 {
                let mut moved = false;
                for &(id, _) in &socks {
                    while let Some((route, data)) = r.peer(id).unwrap().poll_transmit() {
                        let sex_ice::Route::Udp { from, to } = route else { continue };
                        if let Some(&(dst, _)) = socks.iter().find(|s| s.1 == to) { r.receive(dst, to, from, &data); moved = true; }
                    }
                }
                if !moved { break; }
            }
            r.collect();
            while let Some((o, key, f)) = r.poll_frame() { assert_eq!(o, owner); assert_eq!(key, "rtc:2:1"); assert_eq!(f.codec, sex_rtc::Codec::Vp8); frames += 1; }
            events.extend(r.events.drain(..).map(|e| e.1));
        }
        for e in events { s.dispatch(&e).unwrap(); }
        if ms % 50 == 0 { s.dispatch("tick").unwrap(); }
    }
    s.eval("pc1.addTransceiver('audio', { direction: 'recvonly' })", "t.js").unwrap();
    s.dispatch("tick").unwrap();
    s.eval(r#"pc1.getStats().then(r => { let b = 0, n = 0; r.forEach(x => { if (x.type === 'outbound-rtp' && x.kind === 'video') b = x.bytesSent; if (x.type === 'candidate-pair') n++; });
        console.log('stats ' + (b > 1000) + ' ' + n + ' ' + (r instanceof Map)); });
        console.log('states ' + pc1.connectionState + ' ' + pc2.iceConnectionState + ' ' + pc2.iceGatheringState + ' ' + pc1.iceGatheringState);
        console.log('local has candidates ' + pc1.localDescription.sdp.includes('a=candidate:'));
        pc1.close(); console.log('closed ' + pc1.signalingState + ' ' + pc1.connectionState);"#, "t.js").unwrap();
    s.dispatch("tick").unwrap();
    let mut got2 = got.clone();
    got2.extend(logs(&mut s));
    let got = got2;
    for w in ["pc1 connection connecting", "pc1 connection connected", "pc2 connection connected", "remote video unmuted",
        "states connected connected complete gathering", "local has candidates true", "closed closed closed", "stats true 1 true", "negotiationneeded"] {
        assert!(got.iter().any(|g| g == w), "missing {w:?} in {got:#?}");
    }
    assert!(frames > 20, "frames {frames}");
}
