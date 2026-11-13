extern crate std;
use super::*;
use alloc::{format, vec};
use alloc::string::ToString;

const SEED: [u8; 32] = [7; 32];
fn sample() -> (Manifest, Vec<u8>) {
    let mut m = Manifest::new("lua", "5.4.7");
    m.summary = "The Lua language".into();
    m.grants = GRANT_NETWORK;
    let pkg = build(&m, &[("bin/lua", b"\x7fELF program"), ("data/lib/init.lua", b"print(1)"), ("data/README", b"hi")], SEED).unwrap();
    (m, pkg)
}

#[test]
fn round_trip() {
    let (m, pkg) = sample();
    let (layout, key) = verify(&pkg, &[public_key(SEED)]).unwrap();
    assert_eq!(key, public_key(SEED));
    assert_eq!(layout.manifest, m);
    assert_eq!(layout.entries.len(), 3);
    assert_eq!(layout.entries[0].target, Target::Program);
    let e = &layout.entries[1];
    assert_eq!(e.target, Target::Data("lib/init.lua".into()));
    assert_eq!(&pkg[e.offset as usize..(e.offset + e.size) as usize], b"print(1)");
}

#[test]
fn tampered_refused() {
    let (_, pkg) = sample();
    let k = [public_key(SEED)];
    // Every byte of the body and of the signature counts.
    for at in [3, 20, 40, pkg.len() - 120, pkg.len() - 1] {
        let mut bad = pkg.clone();
        bad[at] ^= 1;
        assert!(verify(&bad, &k).is_err(), "byte {at}");
    }
    // A program byte changed: still parses, signature fails.
    let mut bad = pkg.clone();
    let at = pkg.windows(4).position(|w| w == b"\x7fELF").unwrap() + 5;
    bad[at] ^= 0x20;
    assert_eq!(verify(&bad, &k).unwrap_err(), Error::Signature);
}

#[test]
fn untrusted_and_resigned() {
    let (_, pkg) = sample();
    assert_eq!(verify(&pkg, &[public_key([9; 32])]).unwrap_err(), Error::Untrusted);
    assert_eq!(verify(&pkg, &[]).unwrap_err(), Error::Untrusted);
    // Someone else re-signs the body with their own key: valid, but not trusted.
    let body = pkg[..pkg.len() - TRAILER].to_vec();
    let other = sign(body, [9; 32]).unwrap();
    assert_eq!(verify(&other, &[public_key(SEED)]).unwrap_err(), Error::Untrusted);
}

#[test]
fn manifest_rules() {
    assert!(Manifest::parse("name=Lua\nversion=1").is_err());
    assert!(Manifest::parse("name=lua\nversion=1.x").is_err());
    assert!(Manifest::parse("name=lua\nversion=1\ngrant=root").is_err());
    assert!(Manifest::parse("version=1").is_err());
    let m = Manifest::parse("# c\nname=ripgrep\nprogram=rg\nversion=14.1.0\ngrant=network\ngrant=microphone\nfuture=x\n").unwrap();
    assert_eq!((m.program.as_str(), m.grants), ("rg", GRANT_NETWORK | GRANT_MICROPHONE));
    assert_eq!(Manifest::parse(&m.to_text()).unwrap(), m);
    assert_eq!(m.grant_words(), "network,microphone");
}

#[test]
fn versions() {
    let v = |s| Version::parse(s).unwrap();
    assert!(v("5.4.7") < v("5.10"));
    assert!(v("1") < v("1.0.1"));
    assert_eq!(v("3.12.0").to_string(), "3.12");
    assert_eq!(v("2").to_string(), "2.0");
    assert!(Version::parse("1..2").is_none() && Version::parse("1.2.3.4.5").is_none());
}

#[test]
fn entry_rules() {
    let m = Manifest::new("lua", "1");
    let bad: &[&[(&str, &[u8])]] = &[
        &[("data/x", b"1")],                                   // no program
        &[("bin/lua", b"1"), ("bin/lua", b"2")],               // twice
        &[("bin/other", b"1")],                                // someone else's name
        &[("bin/lua", b"1"), ("data/../etc", b"x")],
        &[("bin/lua", b"1"), ("data//x", b"x")],
        &[("bin/lua", b"1"), ("data/a b", b"x")],
        &[("bin/lua", b"1"), ("etc/passwd", b"x")],
        &[("bin/lua", b"1"), ("data/a", b"x"), ("data/a/b", b"y")],
    ];
    for files in bad {
        let r = build(&m, files, SEED).and_then(|p| verify(&p, &[public_key(SEED)]));
        assert!(r.is_err(), "{files:?}");
    }
}

#[test]
fn truncated_and_padded() {
    let (_, pkg) = sample();
    let k = [public_key(SEED)];
    for n in [0, 10, HEAD + TRAILER - 1, pkg.len() - 1] { assert!(verify(&pkg[..n], &k).is_err()); }
    // Extra bytes inside the signed body that no entry accounts for.
    let mut body = pkg[..pkg.len() - TRAILER].to_vec();
    body.push(0);
    assert!(verify(&sign(body, SEED).unwrap(), &k).is_err());
}

#[test]
fn keys_file() {
    let k = public_key(SEED);
    let text = format!("# trusted\n{}\n\nnot a key\n{}  # second\n", hex(&k), hex(&[1; 32]));
    assert_eq!(parse_keys(text.as_bytes()), vec![k, [1; 32]]);
}

#[test]
fn catalog() {
    let (_, pkg) = sample();
    use sha2::Digest;
    let item = index::Item { name: "lua".into(), version: Version::parse("5.4.7").unwrap(), file: "lua-5.4.7.sexpkg".into(), bytes: pkg.len() as u64, sha256: sha2::Sha256::digest(&pkg).into(), grants: GRANT_NETWORK, summary: "The Lua language".into() };
    let text = format!("# SexOS store\n{}{}", index::line(&item), "name=fd version=10.2 file=fd.sexpkg bytes=5 sha256=".to_string() + &hex(&[0; 32]) + " grants=none summary=Find files fast\n");
    let sig = index::sign(text.as_bytes(), SEED);
    let items = index::verify(text.as_bytes(), &sig, &[public_key(SEED)]).unwrap();
    assert_eq!(items[0], item);
    assert_eq!(items[1].summary, "Find files fast");
    assert_eq!(index::search(&items, "LUA language").len(), 1);
    assert_eq!(index::search(&items, "files").len(), 1);
    let mut bad = text.clone().into_bytes();
    bad[20] ^= 1;
    assert!(index::verify(&bad, &sig, &[public_key(SEED)]).is_err());
    let hexsig = hex(&sig);
    assert!(index::verify(text.as_bytes(), hexsig.as_bytes(), &[public_key(SEED)]).is_ok());
}
