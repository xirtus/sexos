// SPDX-License-Identifier: GPL-3.0-only
use super::plugins::{self, Action, Media, Share};
use super::*;

#[test]
fn packet_round_trip_and_payload() {
    let p = plugins::share_file("photo.jpg", 1234, 1739, 42);
    let line = p.to_line();
    assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
    let q = Packet::parse(line.as_bytes()).unwrap();
    assert_eq!(q, p);
    assert_eq!(q.payload_size, Some(1234));
    assert_eq!(q.payload_port, Some(1739));
    assert_eq!(plugins::share(&q), Some(Share::File { name: "photo.jpg".into(), size: 1234, port: Some(1739) }));
}

#[test]
fn parses_peer_packets() {
    // A string id, CRLF ending, unknown fields.
    let p = Packet::parse(br#"{"id":"1700000000000","type":"kdeconnect.ping","body":{},"extra":1}"#).unwrap();
    assert_eq!((p.id, p.kind.as_str()), (1_700_000_000_000, PING));
    assert_eq!(Packet::parse(b"[1,2]\r\n"), Err(Error::Malformed));
    assert_eq!(Packet::parse(br#"{"id":1,"body":{}}"#), Err(Error::Malformed));
    assert_eq!(Packet::parse(br#"{"id":1,"type":"x"}"#).unwrap().body.len(), 0);
}

#[test]
fn identity_round_trip_and_checks() {
    let me = Identity::hookup("0123456789abcdef0123456789abcdef", "SexOS", Some(1716));
    let p = Packet::parse(me.packet(7).to_line().as_bytes()).unwrap();
    let back = Identity::from_packet(&p).unwrap();
    assert_eq!(back, me);
    assert!(back.incoming.iter().any(|c| c == plugins::SHARE));
    assert!(back.outgoing.iter().any(|c| c == plugins::MPRIS_REQUEST));
    let bad = Packet::new(IDENTITY, json!({"deviceId": "../../x", "deviceName": "n"}), 1);
    assert_eq!(Identity::from_packet(&bad), Err(Error::BadDeviceId));
    assert_eq!(Identity::from_packet(&pair_packet(true, 1)), Err(Error::NotIdentity));
    // Control characters in names are dropped; a v8 phone with a UUID id is fine.
    let phone = Packet::new(IDENTITY, json!({"deviceId": "a1b2c3d4-e5f6-4711-8899-aabbccddeeff", "deviceName": "Pho\u{7}ne", "protocolVersion": 8, "tcpPort": 1716}), 1);
    let phone = Identity::from_packet(&phone).unwrap();
    assert_eq!(phone.device_name, "Phone");
    assert_eq!(discovered("me000000000000000000", &phone, false), Discovery::Connect(1716));
    assert_eq!(discovered(&phone.device_id, &phone, false), Discovery::Ourselves);
    assert_eq!(discovered("me000000000000000000", &phone, true), Discovery::AlreadyLinked);
}

#[test]
fn framer_splits_and_bounds_lines() {
    let mut f = Framer::new();
    assert!(f.push(b"{\"a\":1}\n{\"b\"").unwrap().len() == 1);
    let lines = f.push(b":2}\n\n").unwrap();
    assert_eq!(lines, alloc::vec![b"{\"b\":2}".to_vec()]);
    let big = alloc::vec![b'x'; MAX_LINE + 1];
    assert_eq!(f.push(&big), Err(Error::TooLong));
}

#[test]
fn pairing_both_ways() {
    // We ask, they accept.
    let mut p = Pairing::new(false);
    assert_eq!(p.request(0), alloc::vec![PairOutcome::Send(true)]);
    assert_eq!(p.received(true, 10), alloc::vec![PairOutcome::NowPaired]);
    assert!(p.paired());
    // Re-asked while paired: say yes again.
    assert_eq!(p.received(true, 20), alloc::vec![PairOutcome::Send(true)]);
    // They unpair.
    assert_eq!(p.received(false, 30), alloc::vec![PairOutcome::NowUnpaired("unpaired by the device")]);
    // They ask, the person accepts.
    assert_eq!(p.received(true, 40), alloc::vec![PairOutcome::Ask]);
    assert_eq!(p.accept(), alloc::vec![PairOutcome::Send(true), PairOutcome::NowPaired]);
    assert_eq!(p.unpair(), alloc::vec![PairOutcome::Send(false), PairOutcome::NowUnpaired("unpaired")]);
    // They ask, the person refuses.
    p.received(true, 50);
    assert_eq!(p.reject(), alloc::vec![PairOutcome::Send(false), PairOutcome::NowUnpaired("refused")]);
    // Refused by them.
    p.request(60);
    assert_eq!(p.received(false, 70), alloc::vec![PairOutcome::NowUnpaired("refused by the device")]);
    // Accept without a request does nothing.
    assert!(p.accept().is_empty());
}

#[test]
fn pairing_times_out() {
    let mut p = Pairing::new(false);
    p.request(1000);
    assert!(p.tick(1000 + PAIR_TIMEOUT_MS - 1).is_empty());
    assert_eq!(p.tick(1000 + PAIR_TIMEOUT_MS), alloc::vec![PairOutcome::NowUnpaired("timed out")]);
    p.received(true, 0);
    assert_eq!(p.tick(PAIR_TIMEOUT_MS + 5), alloc::vec![PairOutcome::NowUnpaired("timed out")]);
    assert_eq!(p.state, PairState::NotPaired);
}

#[test]
fn plugins_parse() {
    let n = Packet::new(plugins::NOTIFICATION, json!({"id": "0|com.chat|7", "appName": "Chat", "title": "Ana", "text": "See you at 8", "isClearable": true}), 1);
    let n = plugins::notification(&n).unwrap();
    assert_eq!((n.app.as_str(), n.title.as_str(), n.text.as_str(), n.cancel), ("Chat", "Ana", "See you at 8", false));
    let old = Packet::new(plugins::NOTIFICATION, json!({"id": 5, "appName": "Mail", "ticker": "Mail: 3 new"}), 1);
    let old = plugins::notification(&old).unwrap();
    assert_eq!((old.id.as_str(), old.title.as_str(), old.text.as_str()), ("5", "Mail", "Mail: 3 new"));
    let c = Packet::new(plugins::CLIPBOARD_CONNECT, json!({"content": "copied", "timestamp": 5}), 1);
    assert_eq!(plugins::clipboard(&c).as_deref(), Some("copied"));
    let m = Packet::new(plugins::MPRIS, json!({"playerList": ["Spotify", "VLC"]}), 1);
    assert_eq!(plugins::media(&m), Some(Media::Players(alloc::vec!["Spotify".into(), "VLC".into()])));
    let m = Packet::new(plugins::MPRIS, json!({"player": "VLC", "nowPlaying": "Artist - Song", "isPlaying": true, "pos": 1000, "length": 9000}), 1);
    let Some(Media::Playing(now)) = plugins::media(&m) else { panic!() };
    assert_eq!((now.artist.as_str(), now.title.as_str(), now.playing, now.length), ("Artist", "Song", true, Some(9000)));
    let a = plugins::media_action("VLC", Action::Next, 3);
    assert_eq!(a.str("action"), Some("Next"));
    assert_eq!(plugins::share(&plugins::share_text("hi", 1)), Some(Share::Text("hi".into())));
}

#[test]
fn trusted_file_and_names() {
    let mut t = Trusted::parse("0123456789abcdef0123\tPhone\nbad id\tx\n");
    assert_eq!(t.devices.len(), 1);
    t.add("fedcba9876543210fedc", "Tab\tlet");
    t.add("0123456789abcdef0123", "Renamed");
    let back = Trusted::parse(&t.serialize());
    assert_eq!(back.devices, alloc::vec![("0123456789abcdef0123".into(), "Renamed".into()), ("fedcba9876543210fedc".into(), "Tab let".into())]);
    t.remove("0123456789abcdef0123");
    assert!(!t.contains("0123456789abcdef0123"));
    assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_file_name("C:\\x\\..hidden.txt"), "hidden.txt");
    assert_eq!(safe_file_name("..."), "received");
    assert_eq!(device_id_from(&[0xAB; 16]).len(), 32);
}
