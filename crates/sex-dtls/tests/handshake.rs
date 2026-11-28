// SPDX-License-Identifier: GPL-3.0-only
//! Two sex-dtls endpoints in memory: handshake, keys, data, loss, reordering, small MTU,
//! cookies, a wrong fingerprint, close_notify.
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use sex_dtls::{Certificate, Config, Dtls, Error, Event, Fingerprint, Role, SrtpProfile};

fn cert(seed: u8) -> Certificate {
    Certificate::generate(&mut ChaCha20Rng::from_seed([seed; 32]))
}

pub fn pair(f: impl Fn(&mut Config)) -> (Dtls, Dtls) {
    let (cc, sc) = (cert(1), cert(2));
    let mut c = Config::new(Role::Client, cc.clone(), sc.fingerprint());
    let mut s = Config::new(Role::Server, sc, cc.fingerprint());
    f(&mut c);
    f(&mut s);
    (Dtls::new(c, [3; 32]), Dtls::new(s, [4; 32]))
}

/// Shuttle datagrams; `drop` decides which to lose by running count. Returns rounds.
fn run(c: &mut Dtls, s: &mut Dtls, mut drop: impl FnMut(usize) -> bool, reverse: bool) -> u64 {
    let mut now = 0u64;
    c.start(now);
    let mut n = 0;
    for round in 0..200 {
        let mut moved = false;
        for dir in 0..2 {
            let (a, b) = if dir == 0 { (&mut *c, &mut *s) } else { (&mut *s, &mut *c) };
            let mut q = Vec::new();
            while let Some(d) = a.poll_transmit() {
                q.push(d);
            }
            if reverse {
                q.reverse();
            }
            for d in q {
                n += 1;
                moved = true;
                if !drop(n) {
                    b.handle_datagram(now, &d).unwrap();
                }
            }
        }
        if c.is_connected() && s.is_connected() {
            return round;
        }
        if !moved {
            now = [c.poll_timeout(), s.poll_timeout()].into_iter().flatten().min().unwrap_or(now + 1);
            c.handle_timeout(now).unwrap();
            s.handle_timeout(now).unwrap();
        }
    }
    panic!("no handshake");
}

fn events(d: &mut Dtls) -> Vec<Event> {
    std::iter::from_fn(|| d.poll_event()).collect()
}

#[test]
fn handshake_keys_and_data() {
    let (mut c, mut s) = pair(|_| {});
    run(&mut c, &mut s, |_| false, false);
    assert_eq!(events(&mut c), vec![Event::Connected]);
    assert_eq!(events(&mut s), vec![Event::Connected]);
    assert_eq!(c.srtp_profile(), Some(SrtpProfile::AeadAes128Gcm));
    let (kc, ks) = (c.srtp_keys().unwrap(), s.srtp_keys().unwrap());
    assert_eq!(kc.local_key, ks.remote_key);
    assert_eq!(kc.local_salt, ks.remote_salt);
    assert_eq!(kc.remote_key, ks.local_key);
    assert_eq!(kc.local_salt.len(), 12);
    assert_ne!(kc.local_key, kc.remote_key);
    assert_eq!(c.export_keying_material(b"x", 20), s.export_keying_material(b"x", 20));

    c.send(b"hello sctp").unwrap();
    let d = c.poll_transmit().unwrap();
    s.handle_datagram(0, &d).unwrap();
    assert_eq!(events(&mut s), vec![Event::Data(b"hello sctp".to_vec())]);
    // Replayed record is dropped.
    s.handle_datagram(0, &d).unwrap();
    assert!(events(&mut s).is_empty());

    c.close();
    let d = c.poll_transmit().unwrap();
    s.handle_datagram(0, &d).unwrap();
    assert_eq!(events(&mut s), vec![Event::Closed]);
    assert!(s.is_closed());
    // The server answers close_notify with its own.
    let d = s.poll_transmit().unwrap();
    assert_eq!(d[0], 21);
    assert_eq!(s.send(b"x"), Err(Error::NotConnected));
}

#[test]
fn sha1_80_profile_when_only_offered() {
    let (mut c, mut s) = pair(|cfg| {
        if cfg.role == Role::Client {
            cfg.srtp_profiles = vec![SrtpProfile::Aes128CmHmacSha1_80];
        }
    });
    run(&mut c, &mut s, |_| false, false);
    let k = s.srtp_keys().unwrap();
    assert_eq!(k.profile, SrtpProfile::Aes128CmHmacSha1_80);
    assert_eq!(k.local_salt.len(), 14);
}

#[test]
fn small_mtu_fragments_and_reordering() {
    let (mut c, mut s) = pair(|cfg| cfg.mtu = 256);
    run(&mut c, &mut s, |_| false, true);
    assert_eq!(c.srtp_keys().unwrap().local_key, s.srtp_keys().unwrap().remote_key);
}

#[test]
fn hello_verify_cookie() {
    let (mut c, mut s) = pair(|cfg| cfg.hello_verify = true);
    run(&mut c, &mut s, |_| false, false);
    assert!(c.is_connected());
}

#[test]
fn retransmits_through_loss() {
    for k in 1..10u64 {
        // Lose the first datagram and about a third of the rest, pseudo-randomly.
        let mut x = k;
        let (mut c, mut s) = pair(|cfg| {
            cfg.mtu = 300;
            cfg.hello_verify = k % 2 == 0;
        });
        run(&mut c, &mut s, |n| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            n == 1 || (x >> 33) % 3 == 0
        }, k % 3 == 0);
        assert_eq!(c.srtp_keys().unwrap().local_key, s.srtp_keys().unwrap().remote_key, "k={k}");
    }
}

#[test]
fn lost_last_flight_is_resent_on_duplicate() {
    // Drop the server's ChangeCipherSpec+Finished; the client's timer resends its flight and
    // the server answers the duplicate with its last flight again.
    let (mut c, mut s) = pair(|_| {});
    let mut server_last = None;
    c.start(0);
    let mut now = 0;
    for _ in 0..20 {
        while let Some(d) = c.poll_transmit() {
            s.handle_datagram(now, &d).unwrap();
        }
        while let Some(d) = s.poll_transmit() {
            if s.is_connected() && server_last.is_none() {
                server_last = Some(d);
                continue;
            }
            c.handle_datagram(now, &d).unwrap();
        }
        if c.is_connected() {
            break;
        }
        now = c.poll_timeout().unwrap();
        c.handle_timeout(now).unwrap();
    }
    assert!(server_last.is_some());
    assert!(c.is_connected() && s.is_connected());
}

#[test]
fn wrong_fingerprint_fails_with_alert() {
    let (cc, sc, other) = (cert(1), cert(2), cert(9));
    let mut c = Dtls::new(Config::new(Role::Client, cc.clone(), other.fingerprint()), [3; 32]);
    let mut s = Dtls::new(Config::new(Role::Server, sc, cc.fingerprint()), [4; 32]);
    c.start(0);
    let d = c.poll_transmit().unwrap();
    s.handle_datagram(0, &d).unwrap();
    let mut err = None;
    while let Some(d) = s.poll_transmit() {
        if let Err(e) = c.handle_datagram(0, &d) {
            err = Some(e);
        }
    }
    assert_eq!(err, Some(Error::Fingerprint));
    // The client told the server why.
    let a = c.poll_transmit().unwrap();
    assert_eq!(&a[13..], &[2, 42]);
    assert_eq!(s.handle_datagram(0, &a), Err(Error::Alert(42)));
}

#[test]
fn server_refuses_client_without_srtp() {
    let (mut c, mut s) = pair(|cfg| {
        if cfg.role == Role::Client {
            cfg.srtp_profiles.clear();
        }
    });
    c.start(0);
    let d = c.poll_transmit().unwrap();
    assert_eq!(s.handle_datagram(0, &d), Err(Error::NoSrtpProfile));
}

#[test]
fn gives_up_after_retransmits() {
    let (mut c, _s) = pair(|_| {});
    c.start(0);
    let mut last = Ok(());
    for _ in 0..20 {
        let Some(t) = c.poll_timeout() else { break };
        last = c.handle_timeout(t);
    }
    assert_eq!(last, Err(Error::Timeout));
}

#[test]
fn certificate_roundtrip_and_fingerprint_text() {
    let c = cert(5);
    let back = Certificate::from_parts(c.der.clone(), &c.private_key()).unwrap();
    assert_eq!(back.fingerprint(), c.fingerprint());
    let sdp = c.fingerprint().to_sdp();
    assert!(sdp.starts_with("sha-256 "));
    assert_eq!(sdp.len(), 8 + 32 * 3 - 1);
    assert_eq!(Fingerprint::parse_sdp(&sdp), Some(c.fingerprint()));
    assert_eq!(Fingerprint::parse_sdp(&sdp.to_lowercase()), Some(c.fingerprint()));
    assert_eq!(Fingerprint::parse_sdp("sha-1 AA:BB"), None);
    assert!(sex_dtls::public_key(&c.der).is_some());
}
