// SPDX-License-Identifier: GPL-3.0-only
// Two peer connections of one page call each other through the text protocol the page
// uses (sex-js op::RTC), over an in-memory "loopback" network: offer and answer, trickled
// candidates, ICE, DTLS-SRTP, then VP8 pictures and G.711 sound both ways, a PLI asking
// for a key frame, and getStats counting bytes.
use sex_rtc::Rtc;
use std::net::SocketAddr;

const OWNER: u64 = 7;

fn json(s: &str) -> serde_json::Value { serde_json::from_str(s).unwrap_or_else(|e| panic!("{e}: {s}")) }

fn call(r: &mut Rtc, args: &[&str]) -> serde_json::Value {
    let out = r.call(OWNER, args).expect("answered");
    assert!(!out.starts_with('!'), "{args:?}: {out}");
    json(&out)
}

/// Delivers datagrams between the peers' sockets, and trickles candidates both ways.
fn run(r: &mut Rtc, socks: &[(u32, SocketAddr)], ms: u64) {
    for _ in 0..ms {
        r.now += 1;
        r.handle_timeout();
        for _ in 0..4 {
            let mut moved = false;
            for &(id, _) in socks {
                while let Some((route, data)) = r.peer(id).unwrap().poll_transmit() {
                    let sex_ice::Route::Udp { from, to } = route else { panic!("no TURN here") };
                    if let Some(&(dst, _)) = socks.iter().find(|s| s.1 == to) { r.receive(dst, to, from, &data); moved = true; }
                }
            }
            if !moved { break; }
        }
        r.collect();
        for (_, ev) in std::mem::take(&mut r.events) {
            let f: Vec<&str> = ev.split('\t').collect();
            if f[2] == "candidate" {
                let id: u32 = f[1].parse().unwrap();
                let other = socks.iter().find(|s| s.0 != id).unwrap().0.to_string();
                r.call(OWNER, &["candidate", &other, f[4], f[3]]).unwrap();
            }
        }
        for id in r.wants_transport() {
            let addr = socks.iter().find(|s| s.0 == id).unwrap().1;
            r.start_transport(id, &[addr], &[]);
        }
    }
}

#[test]
fn two_peers_call_each_other() {
    let mut r = Rtc::new([5; 32]);
    let a = call(&mut r, &["new", "stun:stun.example.org\nturn:turn.example.org:443?transport=tcp\tuser\tpass", "all"])["id"].as_u64().unwrap() as u32;
    let b = call(&mut r, &["new", "", "all"])["id"].as_u64().unwrap() as u32;
    let (sa, sb) = (a.to_string(), b.to_string());
    assert!(r.call(OWNER, &["new", "http://nope", "all"]).unwrap().starts_with("!SyntaxError"));
    call(&mut r, &["addtrack", &sa, "audio", "sendrecv", "mic", "s1", "tmic", ""]);
    call(&mut r, &["addtrack", &sa, "video", "sendrecv", "camera", "s1", "tcam", "h,1,900000,1,;m,1,300000,2,"]);
    let offer = call(&mut r, &["offer", &sa])["sdp"].as_str().unwrap().to_string();
    for want in ["a=group:BUNDLE 0 1", "a=fingerprint:sha-256 ", "a=ice-ufrag:", "a=rtpmap:96 VP8/90000", "a=rtpmap:111 opus/48000/2", "a=setup:actpass", "a=msid:s1 tcam", "a=rtcp-mux"] {
        assert!(offer.contains(want), "offer lacks {want}:\n{offer}");
    }
    let st = call(&mut r, &["local", &sa, "offer", ""]);
    assert_eq!(st["state"]["sig"], "have-local-offer");
    let st = call(&mut r, &["remote", &sb, "offer", &offer]);
    assert_eq!(st["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(st["state"]["t"][1]["rtrack"], "tcam");
    assert_eq!(st["state"]["t"][1]["rstreams"][0], "s1");
    // B sends its camera back on the video transceiver the offer made.
    let i = call(&mut r, &["addtrack", &sb, "video", "sendrecv", "camera", "s2", "tcam2", ""])["index"].as_u64().unwrap();
    assert_eq!(i, 1, "addTrack reuses the offered transceiver");
    let answer = call(&mut r, &["answer", &sb])["sdp"].as_str().unwrap().to_string();
    assert!(answer.contains("a=setup:active") && answer.contains("a=group:BUNDLE 0 1") && answer.contains("a=recvonly") && answer.contains("a=sendrecv"), "{answer}");
    call(&mut r, &["local", &sb, "answer", &answer]);
    let st = call(&mut r, &["remote", &sa, "answer", &answer]);
    assert_eq!(st["state"]["sig"], "stable");
    assert_eq!(st["state"]["t"][0]["cur"], "sendonly");
    assert_eq!(st["state"]["t"][1]["cur"], "sendrecv");
    // The audio sender uses PCMU (Kaleidoscope encodes G.711), listed first.
    assert_eq!(st["state"]["t"][0]["codecs"][0]["mimeType"], "audio/PCMU");
    assert_eq!(st["tracks"].as_array().unwrap().len(), 1);

    let socks = [(a, "10.0.2.15:40001".parse().unwrap()), (b, "10.0.2.15:40002".parse().unwrap())];
    run(&mut r, &socks, 1500);
    for p in [a, b] {
        let p = r.peer(p).unwrap();
        assert_eq!(p.connection, "connected", "{:?}", p.log);
        assert_eq!(p.gathering, "complete");
    }
    // localDescription carries the candidates.
    let d = call(&mut r, &["desc", &sa]);
    assert!(d["local"]["sdp"].as_str().unwrap().contains("a=candidate:"), "{d}");

    // Pictures and sound for a second and a half.
    let (w, h) = (320, 240);
    let mut got = 0;
    let mut keys = 0;
    let mut audio = 0;
    let mut last_pic = Vec::new();
    for t in 0..90 {
        let mut px = vec![0xff20_4060u32; w * h];
        let x0 = (t * 3) % (w - 20);
        for y in 10..30 { for x in x0..x0 + 20 { px[y * w + x] = 0xffff_ffff; } }
        r.send_picture("camera", &px, w, h);
        r.send_sound("mic", &[[1000i16, 1000]; 960]);
        if t == 45 { r.request_keyframe(&format!("rtc:{a}:1")); }
        run(&mut r, &socks, 17);
        while let Some((owner, key, f)) = r.poll_frame() {
            assert_eq!(owner, OWNER);
            match f.codec {
                sex_rtc::Codec::Vp8 => { got += 1; keys += f.keyframe as usize; if key == format!("rtc:{b}:1") { last_pic = f.data.clone(); } r.decoded(&key, 320, 240); }
                sex_rtc::Codec::Pcmu => { audio += 1; assert_eq!(key, format!("rtc:{b}:0")); assert_eq!(f.data.len(), 160); }
                c => panic!("{c:?}"),
            }
        }
    }
    assert!(got > 36, "video frames {got}");
    assert!(keys >= 3, "key frames {keys}");
    assert!(audio > 40, "audio packets {audio}");
    assert!(!last_pic.is_empty());
    // getStats: bytes rise, and the outbound one names the second encoding's limit.
    let s1 = call(&mut r, &["stats", &sa])["stats"].clone();
    run(&mut r, &socks, 200);
    let mut px = vec![0u32; w * h];
    for _ in 0..5 { px.iter_mut().for_each(|p| *p = p.wrapping_add(0x0101_0101)); r.send_picture("camera", &px, w, h); run(&mut r, &socks, 70); }
    let s2 = call(&mut r, &["stats", &sa])["stats"].clone();
    let sent = |s: &serde_json::Value| s.as_array().unwrap().iter().find(|x| x["type"] == "outbound-rtp" && x["kind"] == "video").unwrap()["bytesSent"].as_u64().unwrap();
    assert!(sent(&s2) > sent(&s1), "{s1} {s2}");
    let inbound = s2.as_array().unwrap().iter().find(|x| x["type"] == "inbound-rtp" && x["kind"] == "video").unwrap();
    assert!(inbound["bytesReceived"].as_u64().unwrap() > 1000);
    assert!(s2.as_array().unwrap().iter().any(|x| x["type"] == "candidate-pair" && x["state"] == "succeeded"));
    let out = s2.as_array().unwrap().iter().find(|x| x["type"] == "outbound-rtp" && x["kind"] == "video").unwrap();
    assert_eq!(out["rid"], "h");
    // A's PLI reached B's sender.
    let sb_stats = call(&mut r, &["stats", &sb])["stats"].clone();
    let bout = sb_stats.as_array().unwrap().iter().find(|x| x["type"] == "outbound-rtp" && x["kind"] == "video").unwrap().clone();
    assert!(bout["pliCount"].as_u64().unwrap() >= 1, "{bout}");
    call(&mut r, &["close", &sa]);
    assert_eq!(r.peer(a).unwrap().connection, "closed");
}
