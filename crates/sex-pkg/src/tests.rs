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
    let item = index::Item { name: "lua".into(), version: Version::parse("5.4.7").unwrap(), file: "lua-5.4.7.sexpkg".into(), bytes: pkg.len() as u64, sha256: sha2::Sha256::digest(&pkg).into(), grants: GRANT_NETWORK, summary: "The Lua language".into(), ..Default::default() };
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

#[test]
fn old_catalog_line_unchanged() {
    // A catalog an older `sexpkg index` wrote parses, and writes back byte for byte.
    let old = format!("name=fd version=10.2 file=fd.sexpkg bytes=5 sha256={} grants=network summary=Find files fast\n", hex(&[3; 32]));
    let items = index::parse(&old).unwrap();
    assert_eq!(items[0].category(), "other");
    assert!(!items[0].featured && items[0].icon.is_none() && items[0].screenshots.is_empty());
    assert_eq!(index::line(&items[0]), old);
}

#[test]
fn catalog_pages() {
    let h = hex(&[5; 32]);
    let text = format!("name=pics version=2 file=pics.sexpkg bytes=9 sha256={h} grants=none category=graphics featured=1 icon=icons/pics.png:{h} screenshot=s/1.png:{h} screenshot=s/2.jpg:{h} future=x description=Line%20one%0ALine%3D2%25 summary=Pictures and more\n");
    let items = index::parse(&text).unwrap();
    let i = &items[0];
    assert_eq!((i.category(), i.featured, i.screenshots.len()), ("graphics", true, 2));
    assert_eq!(i.icon.as_ref().unwrap().file, "icons/pics.png");
    assert_eq!(i.description, "Line one\nLine=2%");
    assert_eq!(i.summary, "Pictures and more");
    // Written and read again: the same entry (the unknown field is dropped).
    assert_eq!(&index::parse(&index::line(i)).unwrap()[0], i);
    assert_eq!(index::decode(&index::encode("a b\nc%=\u{e9}")), "a b\nc%=\u{e9}");
    // Bad pictures are refused: not a picture, a path that climbs, a short hash.
    for bad in [format!("icon=x.exe:{h}"), format!("icon=../x.png:{h}"), "screenshot=a.png:12".into()] {
        let t = format!("name=a version=1 file=a.sexpkg bytes=1 sha256={h} {bad} summary=x");
        assert!(index::parse(&t).is_err(), "{bad}");
    }
    // Searching by category and featured.
    let two = format!("{}name=b version=1 file=b.sexpkg bytes=1 sha256={h} category=games summary=Bee\n", text);
    let items = index::parse(&two).unwrap();
    assert_eq!(index::search(&items, "category:games").len(), 1);
    assert_eq!(index::search(&items, "is:featured")[0].name, "pics");
    assert_eq!(index::search(&items, "line one").len(), 1);
    assert_eq!(index::categories(&items), vec![("graphics".into(), 1), ("games".into(), 1)]);
    // The store's listing line round-trips.
    let l = index::Listing { repo: "extra".into(), base: "http://h/x/".into(), installed: Version::parse("1"), previous: None, newer: true, item: items[0].clone() };
    let back = index::Listing::parse(&l.line()).unwrap();
    assert_eq!(back, l);
    assert_eq!(back.full_name(), "extra/pics");
}

#[test]
fn repositories() {
    use repo::*;
    let (k1, k2) = (public_key([1; 32]), public_key([2; 32]));
    let r = parse_add(&format!("extra http://10.0.2.2:80/extra {}", hex(&k2))).unwrap();
    assert_eq!(r.url, "http://10.0.2.2:80/extra/");
    assert!(parse_add("main http://a/ x").is_err());
    assert!(parse_add(&format!("Bad http://a/ {}", hex(&k2))).is_err());
    assert!(parse_add(&format!("ok ftp://a/ {}", hex(&k2))).is_err());
    assert!(parse_add("ok http://a/ 1234").is_err());
    let list = add(&[], r.clone()).unwrap();
    assert!(add(&list, r.clone()).is_err());
    let mut same_url = r.clone(); same_url.name = "other".into();
    assert!(add(&list, same_url).is_err());
    let text = text(&list);
    assert_eq!(parse(text.as_bytes()), list);
    assert_eq!(parse(b"# c\nbroken line\nmain http://a/ 00\n"), vec![]);
    assert!(remove(&list, "nope").is_none());
    assert_eq!(remove(&list, "extra").unwrap(), vec![]);
    // Keys: the main store uses the trusted keys, a repository only its own.
    assert_eq!(keys_for(MAIN, &list, &[k1]), vec![k1]);
    assert_eq!(keys_for("extra", &list, &[k1]), vec![k2]);
    assert_eq!(all_keys(&list, &[k1]), vec![k1, k2]);
    assert_eq!(origin(&k1, &list, &[k1]).as_deref(), Some(MAIN));
    assert_eq!(origin(&k2, &list, &[k1]).as_deref(), Some("extra"));
    assert_eq!(origin(&[9; 32], &list, &[k1]), None);
    assert!(may_replace("extra", "extra") && !may_replace(MAIN, "extra"));
}

#[test]
fn second_repo_catalog_and_package() {
    // A second repository signs its catalog and package with its own key: both pass with
    // that key, and neither passes with only the main store's keys (nor the other way).
    let (main_seed, extra_seed) = ([1; 32], [2; 32]);
    let m = Manifest::new("hello2", "1.0");
    let pkg = build(&m, &[("bin/hello2", b"\x7fELF")], extra_seed).unwrap();
    use sha2::Digest;
    let item = index::Item { name: "hello2".into(), version: m.version, file: "hello2-1.0.sexpkg".into(), bytes: pkg.len() as u64, sha256: sha2::Sha256::digest(&pkg).into(), ..Default::default() };
    let text = index::line(&item);
    let sig = index::sign(text.as_bytes(), extra_seed);
    let repos = vec![repo::Repo { name: "extra".into(), url: "http://h/extra/".into(), key: public_key(extra_seed) }];
    let trusted = [public_key(main_seed)];
    assert!(index::verify(text.as_bytes(), &sig, &repo::keys_for("extra", &repos, &trusted)).is_ok());
    assert_eq!(index::verify(text.as_bytes(), &sig, &repo::keys_for(repo::MAIN, &repos, &trusted)).unwrap_err(), Error::Untrusted);
    let main_sig = index::sign(text.as_bytes(), main_seed);
    assert!(index::verify(text.as_bytes(), &main_sig, &repo::keys_for("extra", &repos, &trusted)).is_err());
    assert_eq!(verify(&pkg, &trusted).unwrap_err(), Error::Untrusted);
    let (_, key) = verify(&pkg, &repo::all_keys(&repos, &trusted)).unwrap();
    assert_eq!(repo::origin(&key, &repos, &trusted).as_deref(), Some("extra"));
}

#[test]
fn version_selection() {
    use repo::*;
    let it = |n: &str, v: &str| index::Item { name: n.into(), version: Version::parse(v).unwrap(), file: format!("{n}.sexpkg"), ..Default::default() };
    let sources = vec![
        Source { repo: MAIN.into(), url: "http://m/".into(), items: vec![it("lua", "5.4"), it("fd", "10")] },
        Source { repo: "extra".into(), url: "http://e/".into(), items: vec![it("lua", "9.0"), it("hello2", "1.1")] },
    ];
    let pick = |have: &[Have], w: &str| select(&sources, have, w).map(|p| (p.repo, p.item.version.to_string()));
    // Nothing installed: the main store first, then the first repository with it.
    assert_eq!(pick(&[], "lua"), Some((MAIN.into(), "5.4".into())));
    assert_eq!(pick(&[], "hello2"), Some(("extra".into(), "1.1".into())));
    assert_eq!(pick(&[], "extra/lua"), Some(("extra".into(), "9.0".into())));
    assert_eq!(pick(&[], "extra/fd"), None);
    assert_eq!(pick(&[], "nope"), None);
    // Installed from a repository: that repository's entry.
    let have = vec![Have { name: "lua".into(), version: Version::parse("5.0").unwrap(), repo: "extra".into() }, Have { name: "fd".into(), version: Version::parse("10").unwrap(), repo: MAIN.into() }];
    assert_eq!(pick(&have, "lua"), Some(("extra".into(), "9.0".into())));
    // Upgrades come only from each package's own repository.
    let up = upgrades(&sources, &have);
    assert_eq!(up.len(), 1);
    assert_eq!((up[0].repo.as_str(), up[0].url.as_str()), ("extra", "http://e/"));
    let have_main = vec![Have { name: "lua".into(), version: Version::parse("5.4").unwrap(), repo: MAIN.into() }];
    assert!(upgrades(&sources, &have_main).is_empty(), "a newer lua in another repository is not an upgrade");
}

#[test]
fn rollback_moves() {
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use rollback::*;
    let root = "/system/packages";
    // A fake disk: path -> contents. Version 1.0 installed with two data files.
    let mut disk: BTreeMap<String, String> = BTreeMap::new();
    let put = |d: &mut BTreeMap<String, String>, p: &str, v: &str| { d.insert(p.into(), v.into()); };
    put(&mut disk, "/bin/hello", "elf 1.0");
    put(&mut disk, "/data/hello/README", "readme 1.0");
    put(&mut disk, "/data/hello/lib/a.lua", "lib 1.0");
    put(&mut disk, "/data/hello/saved.txt", "the person's own file");
    put(&mut disk, "/system/packages/hello/manifest", "version=1.0");
    put(&mut disk, "/system/packages/hello/files", "README\nlib/a.lua\n");
    let apply = |d: &mut BTreeMap<String, String>, ms: &[Move]| for m in ms {
        match d.remove(&m.from) { Some(v) => { d.insert(m.to.clone(), v); } None => assert!(m.optional, "missing {}", m.from) }
    };
    // Upgrade to 1.1: keep 1.0, then write 1.1 (only README now).
    let files_10 = disk["/system/packages/hello/files"].clone();
    apply(&mut disk, &keep(root, "hello", files_10.as_bytes()));
    assert!(!disk.contains_key("/data/hello/lib/a.lua"), "a file 1.1 lacks is not left behind");
    assert_eq!(disk["/data/hello/saved.txt"], "the person's own file");
    put(&mut disk, "/bin/hello", "elf 1.1");
    put(&mut disk, "/data/hello/README", "readme 1.1");
    put(&mut disk, "/system/packages/hello/manifest", "version=1.1");
    put(&mut disk, "/system/packages/hello/files", "README\n");
    put(&mut disk, "/system/packages/hello/repo", "extra");
    assert_eq!(disk["/system/packages/hello/prev/bin"], "elf 1.0");
    // Roll back: 1.0 is installed again, 1.1 kept.
    let (cur, prev) = (disk["/system/packages/hello/files"].clone(), disk["/system/packages/hello/prev/files"].clone());
    apply(&mut disk, &swap(root, "hello", cur.as_bytes(), prev.as_bytes()));
    assert_eq!(disk["/bin/hello"], "elf 1.0");
    assert_eq!(disk["/data/hello/README"], "readme 1.0");
    assert_eq!(disk["/data/hello/lib/a.lua"], "lib 1.0");
    assert_eq!(disk["/system/packages/hello/manifest"], "version=1.0");
    assert!(!disk.contains_key("/system/packages/hello/repo"));
    assert_eq!(disk["/system/packages/hello/prev/bin"], "elf 1.1");
    assert_eq!(disk["/system/packages/hello/prev/repo"], "extra");
    assert!(!disk.keys().any(|k| k.contains("/swap/")));
    // Rolling back again is the upgrade again.
    let (cur, prev) = (disk["/system/packages/hello/files"].clone(), disk["/system/packages/hello/prev/files"].clone());
    apply(&mut disk, &swap(root, "hello", cur.as_bytes(), prev.as_bytes()));
    assert_eq!((disk["/bin/hello"].as_str(), disk["/data/hello/README"].as_str()), ("elf 1.1", "readme 1.1"));
    assert!(!disk.contains_key("/data/hello/lib/a.lua"));
    assert_eq!(disk["/data/hello/saved.txt"], "the person's own file");
    // Bookkeeping rules.
    assert!(keeps_previous(Version::parse("1").unwrap(), Version::parse("1.1").unwrap()));
    assert!(!keeps_previous(Version::parse("1").unwrap(), Version::parse("1.0").unwrap()));
    assert!(consent_kept(GRANT_NETWORK, 0) && consent_kept(GRANT_NETWORK, GRANT_NETWORK) && !consent_kept(0, GRANT_NETWORK));
    assert_eq!(data_files(b"a\n../b\n\nc/d\n"), vec!["a", "c/d"]);
}
