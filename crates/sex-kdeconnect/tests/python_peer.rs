// SPDX-License-Identifier: GPL-3.0-only
//! The device runtime Hookup runs, here on the host against the Python test peer
//! (scripts/hookup_peer.py) over real UDP and TCP: the peer hears the announcement,
//! links, asks to pair and is accepted, sends a notification, clipboard and media
//! player, shares a file that lands in `received/`; then a media key and a file go
//! the other way.
use sex_kdeconnect::net::{Config, Daemon, Event};
use sex_kdeconnect::plugins::{Action, Media};
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::Duration;

fn free_port() -> u16 {
    // A port free for both TCP and UDP.
    loop {
        let t = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = t.local_addr().unwrap().port();
        if std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, p)).is_ok() { return p; }
    }
}

fn next(rx: &Receiver<Event>, what: &str, mut f: impl FnMut(&Event) -> bool) -> Event {
    loop {
        let e = rx.recv_timeout(Duration::from_secs(30)).unwrap_or_else(|_| panic!("no {what}"));
        if f(&e) { return e; }
        assert!(!matches!(e, Event::Problem(_) | Event::ReceiveFailed { .. } | Event::SendFailed { .. }), "{e:?}");
    }
}

fn fnv(data: &[u8]) -> u64 { data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)) }

#[test]
fn pairs_and_moves_files_with_the_python_peer() {
    if Command::new("python3").arg("--version").output().is_err() { eprintln!("no python3: skipped"); return; }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("hookup-peer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let photo: Vec<u8> = (0..150_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    std::fs::write(dir.join("photo.jpg"), &photo).unwrap();
    let note = b"from Hookup to the phone\n".repeat(400);
    std::fs::write(dir.join("note.txt"), &note).unwrap();

    let mut peer = Command::new("python3").arg(root.join("scripts/hookup_peer.py"))
        .args(["--file", dir.join("photo.jpg").to_str().unwrap(), "--wait-media", "--wait-file"])
        .stdout(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(peer.stdout.take().unwrap()).lines();
    let first: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let udp = first["udp"].as_u64().unwrap() as u16;

    let folder = dir.join("hookup");
    std::fs::create_dir_all(&folder).unwrap();
    let config = Config {
        device_id: "hookup0host0test0000000000000001".into(), name: "Host Hookup".into(), port: free_port(),
        announce: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, udp))], broadcast: false, folder: folder.clone(), payload_port: free_port(),
    };
    let (d, rx) = Daemon::start(config).unwrap();
    let Event::Linked { device, incoming, encrypted, paired, .. } = next(&rx, "link", |e| matches!(e, Event::Linked { .. })) else { unreachable!() };
    assert_eq!((device.device_name.as_str(), incoming, encrypted, paired), ("Test Phone", true, false, false));
    let id = device.device_id.clone();
    next(&rx, "pair request", |e| matches!(e, Event::PairAsked { .. }));
    d.accept(&id);
    next(&rx, "paired", |e| matches!(e, Event::Paired { .. }));
    assert!(std::fs::read_to_string(folder.join("trusted.txt")).unwrap().contains(&id));
    let Event::Notification { notification: n, .. } = next(&rx, "notification", |e| matches!(e, Event::Notification { .. })) else { unreachable!() };
    assert_eq!((n.app.as_str(), n.title.as_str(), n.text.as_str()), ("Chat", "Ana", "Meet at the fountain at 8"));
    let Event::Clipboard { text, .. } = next(&rx, "clipboard", |e| matches!(e, Event::Clipboard { .. })) else { unreachable!() };
    assert_eq!(text, "copied on the phone");
    next(&rx, "players", |e| matches!(e, Event::Media { media: Media::Players(_), .. }));
    let Event::Media { media: Media::Playing(now), .. } = next(&rx, "now playing", |e| matches!(e, Event::Media { media: Media::Playing(_), .. })) else { unreachable!() };
    assert_eq!((now.title.as_str(), now.artist.as_str(), now.playing), ("Night Drive", "The Hookups", true));
    let Event::Received { name, path, bytes, .. } = next(&rx, "file", |e| matches!(e, Event::Received { .. })) else { unreachable!() };
    assert_eq!((name.as_str(), bytes), ("photo.jpg", photo.len() as u64));
    assert_eq!(path, folder.join("received/photo.jpg"));
    assert_eq!(std::fs::read(&path).unwrap(), photo);

    d.media(&id, &now.player, Action::PlayPause).unwrap();
    d.send_file(&id, &dir.join("note.txt")).unwrap();
    next(&rx, "sent", |e| matches!(e, Event::Sent { .. }));

    let mut seen = Vec::new();
    for l in lines.by_ref() {
        let v: serde_json::Value = serde_json::from_str(&l.unwrap()).unwrap();
        let done = v["event"] == "done" || v["event"] == "error";
        seen.push(v);
        if done { break; }
    }
    assert!(peer.wait().unwrap().success(), "{seen:#?}");
    let media = seen.iter().find(|v| v["event"] == "media").expect("media request");
    assert_eq!((media["action"].as_str(), media["player"].as_str()), (Some("PlayPause"), Some("Radio")));
    let fetched = seen.iter().find(|v| v["event"] == "fetched").expect("fetched");
    assert_eq!(fetched["name"], "note.txt");
    assert_eq!(fetched["bytes"].as_u64(), Some(note.len() as u64));
    assert_eq!(fetched["fnv"].as_str().unwrap(), format!("{:016x}", fnv(&note)));
    // The peer heard which packets Hookup takes.
    let heard = seen.iter().chain(std::iter::once(&first)).count();
    assert!(heard > 3);
    let _ = std::fs::remove_dir_all(&dir);
}
