// SPDX-License-Identifier: GPL-3.0-only
//! Offers as Chrome and Firefox write them, and an answer as an ICE lite SFU writes one:
//! each parses, its WebRTC attributes read back typed, and it writes back byte for byte.

use sex_sdp::*;

const CHROME: &str = include_str!("fixtures/chrome_offer.sdp");
const FIREFOX: &str = include_str!("fixtures/firefox_offer.sdp");
const SFU: &str = include_str!("fixtures/sfu_answer.sdp");

fn crlf(s: &str) -> String { s.replace("\r\n", "\n").replace('\n', "\r\n") }

#[test]
fn round_trips_byte_for_byte() {
    for f in [CHROME, FIREFOX, SFU] {
        let s = Session::parse(f).unwrap();
        assert_eq!(s.to_string(), crlf(f));
        // And from the CRLF form too, as it comes off the wire.
        assert_eq!(Session::parse(&crlf(f)).unwrap(), s);
        // Every typed attribute writes back the text it came from.
        for m in &s.media {
            for a in &m.attributes {
                macro_rules! same { ($($t:ty),*) => {$(
                    if a.name == <$t as Attr>::NAME {
                        let t = <$t as Attr>::parse(a.value.as_deref()).unwrap_or_else(|| panic!("{} {:?}", a.name, a.value));
                        assert_eq!(t.value(), a.value, "{}", a.name);
                    }
                )*} }
                same!(Rtpmap, Fmtp, RtcpFb, Extmap, Ssrc, SsrcGroup, Msid, Simulcast, Rid, Candidate, Fingerprint, Setup, Mid, IceUfrag, IcePwd, IceOptions, SctpPort, MaxMessageSize, Rtcp);
            }
        }
        for a in &s.attributes {
            if a.name == "group" { assert_eq!(Group::parse(a.value.as_deref()).unwrap().value(), a.value); }
        }
    }
}

#[test]
fn chrome_offer() {
    let s = Session::parse(CHROME).unwrap();
    assert_eq!(s.origin.session_id, 4611731400430051336);
    assert_eq!(s.bundle().unwrap().mids, ["0", "1", "2"]);
    assert!(s.has("extmap-allow-mixed"));
    let ms: MsidSemantic = s.get().unwrap();
    assert_eq!(ms.semantics, "WMS");
    assert_eq!(s.media.len(), 3);
    let (audio, video, data) = (&s.media[0], &s.media[1], &s.media[2]);
    assert_eq!((audio.kind.as_str(), audio.port, audio.proto.as_str()), ("audio", 54312, "UDP/TLS/RTP/SAVPF"));
    assert_eq!(audio.mid(), Some("0"));
    assert!(audio.has("rtcp-mux") && audio.has("rtcp-rsize"));
    assert_eq!(audio.direction(), Some(Direction::SendRecv));
    assert_eq!(s.ice_credentials(audio).unwrap(), ("EsAw".into(), "P2uYro0UCOQ4zxjKXaWCBui1".into()));
    assert!(audio.get::<IceOptions>().unwrap().trickle());
    let fp = s.fingerprint(audio).unwrap();
    assert_eq!((fp.algorithm.as_str(), fp.bytes.len(), fp.bytes[0], fp.bytes[31]), ("sha-256", 32, 0xD2, 0x0F));
    assert_eq!(s.setup(audio), Some(Setup::Actpass));
    let opus = &audio.codecs()[0];
    assert_eq!(opus.rtpmap.as_ref().unwrap(), &Rtpmap { pt: 111, encoding: "opus".into(), clock: 48000, channels: Some(2) });
    assert_eq!(opus.feedback[0].kind, "transport-cc");
    let f: Vec<Fmtp> = audio.all();
    assert_eq!(f[0].param("useinbandfec"), Some("1"));
    let ext: Vec<Extmap> = audio.all();
    assert_eq!((ext[3].id, ext[3].uri.as_str()), (4, "urn:ietf:params:rtp-hdrext:sdes:mid"));
    let c: Vec<Candidate> = audio.all();
    assert_eq!(c.len(), 3);
    assert!(c[0].address.ends_with(".local") && c[0].kind == CandidateType::Host);
    assert_eq!(c[0].extensions[0], ("generation".into(), "0".into()));
    assert_eq!((c[1].kind, c[1].related.clone()), (CandidateType::ServerReflexive, Some(("0.0.0.0".into(), 0))));
    assert_eq!((c[2].transport.as_str(), c[2].tcptype.as_deref()), ("tcp", Some("active")));
    let ssrc: Vec<Ssrc> = audio.all();
    assert_eq!((ssrc[0].ssrc, ssrc[0].attribute.as_str(), ssrc[0].value.as_deref()), (2914781345, "cname", Some("4TOk42mSjXCkVIa6")));
    let msid: Msid = audio.get().unwrap();
    assert_eq!(msid.track.as_deref(), Some("7a1f0c2e-0b3d-4c55-8e61-2f9b8d4a1e70"));

    assert_eq!(video.direction(), Some(Direction::SendOnly));
    assert_eq!(video.payload_types().len(), 25);
    let vp8 = &video.codecs()[0];
    assert_eq!(vp8.name(), Some("VP8"));
    assert_eq!(vp8.feedback.iter().map(|f| (f.kind.as_str(), f.param.as_deref())).collect::<Vec<_>>(),
        [("goog-remb", None), ("transport-cc", None), ("ccm", Some("fir")), ("nack", None), ("nack", Some("pli"))]);
    let rtx = &video.codecs()[1];
    assert_eq!((rtx.name(), rtx.fmtp.as_deref()), (Some("rtx"), Some("apt=96")));
    let sim: Simulcast = video.get().unwrap();
    assert_eq!(sim.send.iter().map(|a| a[0].rid.as_str()).collect::<Vec<_>>(), ["q", "h", "f"]);
    let rids: Vec<Rid> = video.all();
    assert!(rids.iter().all(|r| r.send));

    assert_eq!((data.kind.as_str(), data.proto.as_str(), data.formats[0].as_str()), ("application", "UDP/DTLS/SCTP", "webrtc-datachannel"));
    assert_eq!(data.get::<SctpPort>(), Some(SctpPort(5000)));
    assert_eq!(data.get::<MaxMessageSize>(), Some(MaxMessageSize(262144)));
}

#[test]
fn firefox_offer() {
    let s = Session::parse(FIREFOX).unwrap();
    assert!(s.origin.username.starts_with("mozilla"));
    // Firefox writes the fingerprint and direction at session level.
    assert_eq!(s.direction(), Some(Direction::SendRecv));
    let audio = &s.media[0];
    assert_eq!(s.fingerprint(audio).unwrap().bytes[0], 0x3B);
    assert_eq!(s.ice_credentials(audio).unwrap().0, "6f1a2b3c");
    let ext: Vec<Extmap> = audio.all();
    assert_eq!(ext[1].direction, Some(Direction::RecvOnly));
    let te: Vec<Fmtp> = audio.all();
    assert_eq!(te[1].pairs().next(), Some(("0-15", "")));
    let g722 = audio.codecs().into_iter().find(|c| c.pt == 9).unwrap();
    assert_eq!(g722.rtpmap.unwrap().channels, Some(1));
    let video = &s.media[1];
    let group: SsrcGroup = video.get().unwrap();
    assert_eq!((group.semantics.as_str(), group.ssrcs.as_slice()), ("FID", &[3302124877, 2178309466][..]));
    let sim: Simulcast = video.get().unwrap();
    assert_eq!(sim.send.len(), 3);
    assert!(video.codecs().iter().find(|c| c.pt == 126).unwrap().fmtp.as_deref().unwrap().contains("packetization-mode=1"));
    assert_eq!(s.media[2].get::<MaxMessageSize>(), Some(MaxMessageSize(1073741823)));
}

#[test]
fn sfu_answer() {
    let s = Session::parse(SFU).unwrap();
    assert!(s.ice_lite());
    let audio = &s.media[0];
    assert_eq!(audio.bandwidths[0], Bandwidth { kind: "AS".into(), value: 128 });
    assert_eq!(audio.connections[0].address, "198.51.100.20");
    assert_eq!(s.setup(audio), Some(Setup::Passive));
    assert!(audio.has("end-of-candidates"));
    let c: Vec<Candidate> = audio.all();
    assert_eq!(c[1].address, "2001:db8:5::20");
    assert_eq!(c[2].tcptype.as_deref(), Some("passive"));
    let video = &s.media[1];
    assert_eq!(video.bandwidths.len(), 2);
    let sim: Simulcast = video.get().unwrap();
    assert!(sim.send.is_empty() && sim.recv.len() == 3);
    assert_eq!(video.direction(), Some(Direction::RecvOnly));
}

#[test]
fn writes_an_answer() {
    // An answer to Chrome's offer built from parts, then read back.
    let offer = Session::parse(CHROME).unwrap();
    let mut a = Session::new(42);
    a.push(Group { semantics: "BUNDLE".into(), mids: vec!["0".into(), "1".into()] });
    a.push(MsidSemantic { semantics: "WMS".into(), ids: vec!["*".into()] });
    for om in &offer.media[..2] {
        let opus_or_vp8 = om.codecs().into_iter().next().unwrap();
        let mut m = Media::new(&om.kind, &om.proto, &[&opus_or_vp8.pt.to_string()]);
        m.push(IceUfrag("abcd".into()));
        m.push(IcePwd("0123456789abcdefghijkl".into()));
        m.push(IceOptions(vec!["trickle".into()]));
        m.push(Fingerprint { algorithm: "sha-256".into(), bytes: (0..32).collect() });
        m.push(Setup::Active);
        m.push(Mid(om.mid().unwrap().into()));
        m.set_direction(om.direction().unwrap().reverse());
        m.push_flag("rtcp-mux");
        m.push(opus_or_vp8.rtpmap.clone().unwrap());
        for fb in &opus_or_vp8.feedback { m.push(fb.clone()); }
        if let Some(p) = &opus_or_vp8.fmtp { m.push(Fmtp { pt: opus_or_vp8.pt, params: p.clone() }); }
        m.push(Candidate {
            foundation: "1".into(), component: 1, transport: "udp".into(), priority: 2130706431,
            address: "10.0.2.15".into(), port: 40000, kind: CandidateType::Host, related: None, tcptype: None, extensions: vec![],
        });
        a.media.push(m);
    }
    let text = a.to_string();
    assert!(text.starts_with("v=0\r\no=- 42 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0 1\r\na=msid-semantic: WMS *\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\n"), "{}", text);
    assert!(text.contains("a=fingerprint:sha-256 00:01:02:03:"));
    assert!(text.contains("a=candidate:1 1 udp 2130706431 10.0.2.15 40000 typ host\r\n"));
    let back = Session::parse(&text).unwrap();
    assert_eq!(back, a);
    assert_eq!(back.media[1].direction(), Some(Direction::RecvOnly));
    assert_eq!(back.media[1].codecs()[0].feedback.len(), 5);
}

#[test]
fn trickled_candidates() {
    let c = Candidate::parse_line("candidate:3 1 UDP 92216831 192.0.2.9 61234 typ relay raddr 203.0.113.7 rport 54312 generation 0 ufrag EsAw").unwrap();
    assert_eq!((c.kind, c.transport.as_str(), c.port), (CandidateType::Relay, "udp", 61234));
    assert_eq!(c.related, Some(("203.0.113.7".into(), 54312)));
    assert_eq!(c.to_line(), "candidate:3 1 udp 92216831 192.0.2.9 61234 typ relay raddr 203.0.113.7 rport 54312 generation 0 ufrag EsAw");
    assert!(Candidate::parse_line("a=candidate:1 1 udp 1 ::1 9 typ host").is_some());
    assert!(Candidate::parse_line("candidate:1 1 udp 1 ::1 9 type host").is_none());
}

#[test]
fn simulcast_forms() {
    let s = Simulcast::parse(Some("send h,~m;l recv r")).unwrap();
    assert_eq!(s.send[0][1], SimulcastId { rid: "m".into(), paused: true });
    assert_eq!(s.value().as_deref(), Some("send h,~m;l recv r"));
    // Older Firefox.
    let s = Simulcast::parse(Some(" send rid=0;1;2")).unwrap();
    assert_eq!(s.send.len(), 3);
    let r = Rid::parse(Some("h send pt=96,98;max-width=1280;max-height=720")).unwrap();
    assert_eq!((r.payload_types(), r.restriction("max-width")), (vec![96, 98], Some("1280")));
}

#[test]
fn refuses_broken_descriptions() {
    assert_eq!(Session::parse("v=0\r\ns=-\r\n").unwrap_err().what, "v= and o= are required");
    assert_eq!(Session::parse("v=0\r\no=- 1 2 IN IP4\r\n").unwrap_err().line, 2);
    assert_eq!(Session::parse("v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\nm=audio x RTP/AVP 0\r\n").unwrap_err().line, 3);
    assert!(Session::parse("v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\nm=audio 9 RTP/AVP 0\r\nt=0 0\r\n").is_err());
    assert!(Session::parse("garbage").is_err());
}
