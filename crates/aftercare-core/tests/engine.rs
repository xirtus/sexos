// SPDX-License-Identifier: GPL-3.0-only
//! End-to-end tests over an in-memory store and an in-memory file tree.

use aftercare_core::*;
use rand_chacha::rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::collections::{BTreeMap, BTreeSet};

const PASSWORD: &[u8] = b"correct horse battery staple";

fn opts() -> InitOptions {
    InitOptions {
        chunker: ChunkerParams { min: 16 * 1024, avg: 64 * 1024, max: 256 * 1024 },
        pack_size: 1024 * 1024,
        compression: true,
        // Tiny Argon2 cost so tests stay fast; real repos use the 64 MiB default.
        kdf: KdfParams { m_cost_kib: 64, t_cost: 1, p_cost: 1 },
    }
}

#[derive(Clone, Debug, PartialEq)]
enum F {
    File(Vec<u8>, i64),
    Dir,
    Link(String),
}

/// A file tree in memory: absolute path -> node.
#[derive(Clone, Default)]
struct Tree {
    nodes: BTreeMap<String, F>,
}

fn parent(p: &str) -> &str {
    match p.rfind('/') {
        Some(0) => "/",
        Some(i) => &p[..i],
        None => "/",
    }
}

impl Tree {
    fn file(&mut self, p: &str, data: Vec<u8>) {
        self.mkdirs(parent(p));
        self.nodes.insert(p.into(), F::File(data, 1_700_000_000));
    }
    fn mkdirs(&mut self, p: &str) {
        if p == "/" {
            return;
        }
        self.mkdirs(parent(p));
        self.nodes.entry(p.into()).or_insert(F::Dir);
    }
    fn touch(&mut self, p: &str, data: Vec<u8>, mtime: i64) {
        self.nodes.insert(p.into(), F::File(data, mtime));
    }
    fn entry(&self, p: &str) -> Option<Entry> {
        let name = p.rsplit('/').next().unwrap().to_string();
        if p == "/" {
            return Some(Entry { name, kind: NodeKind::Dir, size: 0, mtime: Timestamp::default(), mode: 0o755 });
        }
        Some(match self.nodes.get(p)? {
            F::File(d, m) => {
                Entry { name, kind: NodeKind::File, size: d.len() as u64, mtime: Timestamp::from_secs(*m), mode: 0o644 }
            }
            F::Dir => Entry { name, kind: NodeKind::Dir, size: 0, mtime: Timestamp::from_secs(5), mode: 0o755 },
            F::Link(_) => Entry { name, kind: NodeKind::Symlink, size: 0, mtime: Timestamp::from_secs(5), mode: 0o777 },
        })
    }
    /// Files under `root`, relative to it.
    fn under(&self, root: &str) -> BTreeMap<String, F> {
        let pre = if root == "/" { "/".to_string() } else { format!("{root}/") };
        self.nodes
            .iter()
            .filter(|(p, _)| p.starts_with(&pre))
            .map(|(p, f)| {
                let f = match f {
                    F::File(d, _) => F::File(d.clone(), 0),
                    o => o.clone(),
                };
                (p[pre.len()..].to_string(), f)
            })
            .collect()
    }
}

/// A [`Source`] over a [`Tree`] that counts bytes read.
struct Src<'a> {
    t: &'a Tree,
    reads: u64,
}

impl Source for Src<'_> {
    fn stat(&mut self, path: &str) -> Result<Entry, Error> {
        self.t.entry(path).ok_or_else(|| Error::Source(format!("{path}: no such file")))
    }
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Error> {
        let pre = if path == "/" { "/".to_string() } else { format!("{path}/") };
        Ok(self
            .t
            .nodes
            .range(pre.clone()..)
            .take_while(|(p, _)| p.starts_with(&pre))
            .filter(|(p, _)| !p[pre.len()..].contains('/'))
            .map(|(p, _)| self.t.entry(p).unwrap())
            .collect())
    }
    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        match self.t.nodes.get(path) {
            Some(F::File(d, _)) => {
                let o = (offset as usize).min(d.len());
                let n = buf.len().min(d.len() - o);
                buf[..n].copy_from_slice(&d[o..o + n]);
                self.reads += n as u64;
                Ok(n)
            }
            _ => Err(Error::Source(format!("{path}: not a file"))),
        }
    }
    fn read_link(&mut self, path: &str) -> Result<String, Error> {
        match self.t.nodes.get(path) {
            Some(F::Link(t)) => Ok(t.clone()),
            _ => Err(Error::Source("not a link".into())),
        }
    }
}

/// A [`Sink`] collecting restored files.
#[derive(Default)]
struct Sk {
    out: BTreeMap<String, F>,
    cur: Option<(String, Vec<u8>)>,
}

impl Sink for Sk {
    fn create_dir(&mut self, path: &str, _: &Node) -> Result<(), Error> {
        self.out.insert(path.into(), F::Dir);
        Ok(())
    }
    fn begin_file(&mut self, path: &str, _: &Node) -> Result<(), Error> {
        self.cur = Some((path.into(), Vec::new()));
        Ok(())
    }
    fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        self.cur.as_mut().unwrap().1.extend_from_slice(data);
        Ok(())
    }
    fn end_file(&mut self, path: &str, _: &Node) -> Result<(), Error> {
        let (p, d) = self.cur.take().unwrap();
        assert_eq!(p, path);
        self.out.insert(p, F::File(d, 0));
        Ok(())
    }
    fn symlink(&mut self, path: &str, node: &Node) -> Result<(), Error> {
        self.out.insert(path.into(), F::Link(node.target.clone()));
        Ok(())
    }
}

fn noise(rng: &mut ChaCha20Rng, n: usize) -> Vec<u8> {
    let mut v = vec![0; n];
    rng.fill_bytes(&mut v);
    v
}

/// ~3000 small files in 50 directories plus three multi-MB files sharing most content.
fn synthetic(rng: &mut ChaCha20Rng) -> Tree {
    let mut t = Tree::default();
    let words: Vec<&str> = "lorem ipsum dolor sit amet aftercare backup restore chunk pack index tree"
        .split(' ')
        .collect();
    for d in 0..50 {
        for f in 0..60 {
            let n = (rng.next_u32() % 4000) as usize;
            let data: Vec<u8> = if f % 3 == 0 {
                // Text-like (compressible).
                (0..n).map(|i| words[(i * 7 + d) % words.len()].as_bytes()[0] ^ (i as u8 & 1)).collect()
            } else {
                noise(rng, n)
            };
            t.file(&format!("/home/ann/dir{d:02}/file{f:03}.dat"), data);
        }
    }
    let a = noise(rng, 5 << 20);
    let mut b = a[..2 << 20].to_vec();
    b.extend_from_slice(&noise(rng, 100_000));
    b.extend_from_slice(&a[2 << 20..]);
    let mut c = a.clone();
    c[3 << 20] ^= 0xff;
    t.file("/home/ann/big/a.bin", a);
    t.file("/home/ann/big/b.bin", b);
    t.file("/home/ann/big/c.bin", c);
    t.file("/home/ann/empty.txt", Vec::new());
    t.nodes.insert("/home/ann/link".into(), F::Link("big/a.bin".into()));
    t.file("/etc/hostname", b"sexos\n".to_vec());
    t
}

fn bopts(secs: i64) -> BackupOptions {
    BackupOptions {
        host: "testbox".into(),
        tags: vec![],
        time: Timestamp::from_secs(secs),
        parent: ParentChoice::Latest,
    }
}

fn restore_all<B: Backend>(repo: &mut Repository<B>, snap: &Id, path: &str) -> BTreeMap<String, F> {
    let mut sk = Sk::default();
    repo.restore(snap, path, &mut sk, &mut Quiet).unwrap();
    sk.out
}

#[test]
fn backup_restore_roundtrip_with_dedup() {
    let mut rng = ChaCha20Rng::seed_from_u64(1);
    let tree = synthetic(&mut rng);
    let total: u64 = tree.nodes.values().map(|f| if let F::File(d, _) = f { d.len() as u64 } else { 0 }).sum();
    let mut repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let mut src = Src { t: &tree, reads: 0 };
    let (id, snap) = repo.backup(&mut src, &["/home/ann", "/etc"], &bopts(1_700_000_000), &mut rng, &mut Quiet).unwrap();
    let s = &snap.summary;
    let stored = repo.backend().bytes(FileKind::Pack);
    println!("first backup: {total} file bytes, {} new plaintext, {stored} stored in packs; {s:?}", s.bytes_added);
    assert_eq!(s.files_new, 3000 + 3 + 2);
    assert_eq!(s.bytes_total, total);
    // b.bin and c.bin share all but ~100 KB and one chunk with a.bin.
    assert!(s.bytes_added < total - 9 * (1 << 20), "dedup inside the first backup: {}", s.bytes_added);
    assert!(stored < total * 6 / 10, "{stored}");
    assert!(repo.backend().files.keys().all(|(k, _)| *k != FileKind::Lock), "lock released");

    // Restore everything; every byte equal.
    let out = restore_all(&mut repo, &id, "/");
    let mut expect = tree.under("/");
    expect.insert("home".into(), F::Dir);
    expect.insert("etc".into(), F::Dir);
    assert_eq!(out.len(), expect.len());
    assert!(out == expect, "restored tree differs");

    // A subtree, after reopening the repository from the store.
    let mut repo = Repository::open(repo.into_backend(), PASSWORD).unwrap();
    let out = restore_all(&mut repo, &id, "/home/ann/dir07");
    let mut sub: BTreeMap<String, F> =
        tree.under("/home/ann/dir07").into_iter().map(|(p, f)| (format!("dir07/{p}"), f)).collect();
    sub.insert("dir07".into(), F::Dir);
    assert_eq!(out, sub);

    // Browse.
    let names: Vec<String> = repo.ls(&id, "/home/ann").unwrap().into_iter().map(|n| n.name).collect();
    assert_eq!(names.len(), 50 + 3);
    assert!(names.contains(&"big".to_string()) && names.contains(&"link".to_string()));
    let n = repo.node(&id, "/home/ann/big/b.bin").unwrap();
    assert_eq!(n.size, (5 << 20) + 100_000);
    assert!(matches!(repo.ls(&id, "/nope"), Err(Error::NoSuchPath(_))));
    assert_eq!(repo.find_snapshot(&id.to_hex()[..10]).unwrap(), id);
    assert_eq!(repo.find_snapshot("latest").unwrap(), id);

    let rep = repo.check(true).unwrap();
    assert!(rep.is_ok(), "{:?}", rep.errors);
    assert!(rep.warnings.is_empty(), "{:?}", rep.warnings);
    assert!(rep.blobs_verified > 3000);
}

#[test]
fn second_backup_stores_little() {
    let mut rng = ChaCha20Rng::seed_from_u64(2);
    let mut tree = synthetic(&mut rng);
    let mut repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let (first, _) =
        repo.backup(&mut Src { t: &tree, reads: 0 }, &["/home"], &bopts(1000), &mut rng, &mut Quiet).unwrap();
    let before = repo.backend().total_bytes();
    let orig = tree.clone();

    // Small changes: one small file edited, 10 KB appended to a big file, one new file.
    tree.touch("/home/ann/dir03/file010.dat", b"edited".to_vec(), 1_800_000_000);
    if let Some(F::File(d, m)) = tree.nodes.get_mut("/home/ann/big/b.bin") {
        d.extend_from_slice(&[7u8; 10_000]);
        *m += 60;
    }
    tree.file("/home/ann/new.txt", b"hello".to_vec());

    let mut src = Src { t: &tree, reads: 0 };
    let (second, snap) = repo.backup(&mut src, &["/home"], &bopts(2000), &mut rng, &mut Quiet).unwrap();
    let after = repo.backend().total_bytes();
    let s = &snap.summary;
    println!(
        "second backup: {} files unchanged, {} changed, {} new; read {} bytes; {} new plaintext; store grew {} bytes (of {before})",
        s.files_unchanged,
        s.files_changed,
        s.files_new,
        src.reads,
        s.bytes_added,
        after - before
    );
    assert_eq!(snap.parent, Some(first));
    assert_eq!(s.files_unchanged, 3000 + 4 - 2);
    assert_eq!((s.files_changed, s.files_new), (2, 1));
    // Only the two changed files are read: the small one and b.bin.
    assert!(src.reads < (6 << 20), "{}", src.reads);
    // New data: the last chunk(s) of b.bin plus tiny files; trees for the changed dirs.
    assert!(s.bytes_added < 600_000, "{}", s.bytes_added);
    assert!(after - before < 700_000, "store grew {}", after - before);
    assert!(after - before < before / 20);

    let out = restore_all(&mut repo, &second, "/home");
    let mut expect: BTreeMap<String, F> = tree.under("/home").into_iter().map(|(p, f)| (format!("home/{p}"), f)).collect();
    expect.insert("home".into(), F::Dir);
    assert!(out == expect);
    let out = restore_all(&mut repo, &first, "/home/ann/big");
    assert_eq!(out.get("big/b.bin"), orig.under("/home/ann/big").get("b.bin"));
}

#[test]
fn wrong_password_and_extra_keys() {
    let mut rng = ChaCha20Rng::seed_from_u64(3);
    let repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let be = repo.into_backend();
    assert_eq!(Repository::open(be.clone(), b"hunter2").err(), Some(Error::WrongPassword));
    let mut repo = Repository::open(be, PASSWORD).unwrap();
    let k2 = repo.add_password(b"second", KdfParams { m_cost_kib: 64, t_cost: 1, p_cost: 1 }, &mut rng).unwrap();
    let be = repo.into_backend();
    let mut repo = Repository::open(be.clone(), b"second").unwrap();
    assert!(repo.remove_password(&k2).is_err(), "cannot remove the key in use");
    let mut repo = Repository::open(repo.into_backend(), PASSWORD).unwrap();
    repo.remove_password(&k2).unwrap();
    assert_eq!(Repository::open(repo.into_backend(), b"second").err(), Some(Error::WrongPassword));
    assert_eq!(Repository::init(be, b"x", &mut rng, &opts()).err(), Some(Error::AlreadyInitialized));
    assert!(matches!(Repository::open(MemBackend::new(), PASSWORD), Err(Error::NotFound(_))));
}

#[test]
fn tampered_pack_is_detected() {
    let mut rng = ChaCha20Rng::seed_from_u64(4);
    let mut tree = Tree::default();
    tree.file("/d/a.bin", noise(&mut rng, 900_000));
    tree.file("/d/b.txt", b"small".to_vec());
    let mut repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let (id, _) = repo.backup(&mut Src { t: &tree, reads: 0 }, &["/d"], &bopts(1), &mut rng, &mut Quiet).unwrap();
    assert!(repo.check(true).unwrap().is_ok());

    // Flip one byte in the middle of the biggest pack (a data blob).
    let key = repo
        .backend()
        .files
        .iter()
        .filter(|((k, _), _)| *k == FileKind::Pack)
        .max_by_key(|(_, v)| v.len())
        .map(|(k, _)| k.clone())
        .unwrap();
    let pack = repo.backend_mut().files.get_mut(&key).unwrap();
    let mid = pack.len() / 3;
    pack[mid] ^= 0x01;

    let rep = repo.check(true).unwrap();
    println!("check after tamper: {:?}", rep.errors);
    assert!(!rep.is_ok());
    assert!(rep.errors.iter().any(|e| e.contains("SHA-256")));
    assert!(rep.errors.iter().any(|e| e.contains("authentication failed")));
    let mut sk = Sk::default();
    assert!(matches!(repo.restore(&id, "/d/a.bin", &mut sk, &mut Quiet), Err(Error::Corrupt(_))));

    // Tampering with a pack header is caught even without reading data.
    let pack = repo.backend_mut().files.get_mut(&key).unwrap();
    pack[mid] ^= 0x01;
    let n = pack.len();
    pack[n - 10] ^= 0x80;
    assert!(!repo.check(false).unwrap().is_ok());
}

#[test]
fn forget_and_prune_keep_referenced_data() {
    let mut rng = ChaCha20Rng::seed_from_u64(5);
    let mut tree = Tree::default();
    let mut states = Vec::new();
    let mut ids = Vec::new();
    let mut repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let day = 86_400;
    // Five daily backups; each replaces one big file and adds small ones.
    for i in 0..5i64 {
        let p = format!("/data/gen{}.bin", i % 2);
        tree.file(&p, Vec::new());
        tree.touch(&p, noise(&mut rng, 700_000), 1_700_000_000 + i);
        for j in 0..20 {
            tree.file(&format!("/data/s{i}/{j}.txt"), noise(&mut rng, 3000));
        }
        tree.nodes.retain(|p, _| !p.starts_with(&format!("/data/s{}", i - 2)));
        let (id, _) = repo
            .backup(&mut Src { t: &tree, reads: 0 }, &["/data"], &bopts(1_700_000_000 + i * day), &mut rng, &mut Quiet)
            .unwrap();
        ids.push(id);
        states.push(tree.clone());
    }
    let policy = KeepPolicy { last: 2, ..Default::default() };
    let plan = repo.forget(&policy, Timestamp::from_secs(1_800_000_000), &mut rng).unwrap();
    assert_eq!(plan.keep, vec![ids[4], ids[3]]);
    assert_eq!(plan.remove.len(), 3);
    assert_eq!(repo.snapshots().unwrap().len(), 2);

    let before = repo.backend().bytes(FileKind::Pack);
    let sum = repo.prune(&PruneOptions { max_unused_percent: 5, ..Default::default() }, &mut rng).unwrap();
    println!("prune: {sum:?}");
    assert!(sum.packs_deleted + sum.packs_repacked > 0);
    assert!(sum.bytes_after < before);
    assert_eq!(sum.bytes_after, repo.backend().bytes(FileKind::Pack));
    assert_eq!(repo.backend().files.keys().filter(|(k, _)| *k == FileKind::Index).count(), 1);

    let rep = repo.check(true).unwrap();
    assert!(rep.is_ok() && rep.warnings.is_empty(), "{rep:?}");
    for i in [3, 4] {
        let mut repo2 = Repository::open(repo.backend().clone(), PASSWORD).unwrap();
        let out = restore_all(&mut repo2, &ids[i], "/");
        let mut expect = states[i].under("/");
        expect.insert("data".into(), F::Dir);
        let ok: Vec<_> = out.keys().filter(|k| out.get(*k) != expect.get(*k)).collect();
        let missing: Vec<_> = expect.keys().filter(|k| !out.contains_key(*k)).collect();
        assert!(out == expect, "snapshot {i} restores after prune: differ {ok:?} missing {missing:?}");
    }
    // A second prune finds nothing more to do.
    let again = repo.prune(&PruneOptions::default(), &mut rng).unwrap();
    assert_eq!((again.packs_deleted, again.packs_repacked), (0, 0));
    // Empty policy is refused.
    assert!(matches!(repo.plan_forget(&KeepPolicy::default()), Err(Error::Invalid(_))));
}

#[test]
fn keep_policy_calendar() {
    // Snapshots every 6 hours for 120 days from 2024-01-01T00:00Z (a Monday).
    let start = 1_704_067_200i64;
    let mk = |i: i64| {
        let mut id = Id::default();
        id.0[..8].copy_from_slice(&i.to_le_bytes());
        (
            id,
            Snapshot {
                time: Timestamp::from_secs(start + i * 6 * 3600),
                tree: Id::default(),
                parent: None,
                host: "h".into(),
                paths: vec!["/".into()],
                tags: if i == 7 { vec!["keep".into()] } else { vec![] },
                summary: Default::default(),
            },
        )
    };
    let snaps: Vec<_> = (0..480).map(mk).collect();
    let p = KeepPolicy { daily: 7, weekly: 4, monthly: 3, tags: vec!["keep".into()], ..Default::default() };
    let plan = apply_keep_policy(&snaps, &p);
    let kept: BTreeSet<i64> = plan.keep.iter().map(|id| i64::from_le_bytes(id.0[..8].try_into().unwrap())).collect();
    // Newest is index 479 (2024-04-29 18:00, a Monday). Daily: last snapshot of each of 7 days.
    let days: Vec<i64> = (0..7).map(|d| 479 - 4 * d).collect();
    for d in &days {
        assert!(kept.contains(d), "daily {d}");
    }
    // Weekly: newest of the weeks of Mon 04-29, 04-22, 04-15, 04-08: 479, then Sundays 18:00.
    for w in [479, 475, 447, 419] {
        assert!(kept.contains(&w), "weekly {w}");
    }
    // Monthly: April (479), March 31 18:00 (index 4*90+3=363), Feb 29 18:00 (4*59+3=239).
    for m in [479, 363, 239] {
        assert!(kept.contains(&m), "monthly {m}");
    }
    assert!(kept.contains(&7), "tagged");
    // 7 daily, weekly adds 447 and 419, monthly adds 363 and 239, plus the tagged one.
    assert_eq!(kept.len(), 7 + 2 + 2 + 1, "{kept:?}");
    assert_eq!(plan.keep.len() + plan.remove.len(), 480);
}

struct CancelAfter {
    n: u32,
    progress: u32,
    warnings: Vec<String>,
}
impl Observer for CancelAfter {
    fn progress(&mut self, _: &Progress) {
        self.progress += 1;
    }
    fn warning(&mut self, m: &str) {
        self.warnings.push(m.into());
    }
    fn cancelled(&mut self) -> bool {
        self.progress >= self.n
    }
}

#[test]
fn cancel_leaves_usable_repo() {
    let mut rng = ChaCha20Rng::seed_from_u64(6);
    let mut tree = Tree::default();
    for i in 0..4 {
        tree.file(&format!("/v/f{i}.bin"), noise(&mut rng, 1_500_000));
    }
    let mut repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let mut obs = CancelAfter { n: 40, progress: 0, warnings: vec![] };
    let r = repo.backup(&mut Src { t: &tree, reads: 0 }, &["/v"], &bopts(1), &mut rng, &mut obs);
    assert_eq!(r.err(), Some(Error::Cancelled));
    assert!(repo.snapshots().unwrap().is_empty());
    assert!(repo.locks().unwrap().is_empty());
    let rep = repo.check(true).unwrap();
    assert!(rep.is_ok(), "{rep:?}");
    let partial = repo.backend().bytes(FileKind::Pack);
    assert!(partial > 1_000_000, "chunks stored before the cancel were kept: {partial}");

    // Reopen and finish: data from the cancelled run is reused.
    let mut repo = Repository::open(repo.into_backend(), PASSWORD).unwrap();
    let (id, snap) =
        repo.backup(&mut Src { t: &tree, reads: 0 }, &["/v"], &bopts(2), &mut rng, &mut Quiet).unwrap();
    println!("after cancel: {partial} bytes kept, resumed run added {}", snap.summary.bytes_added);
    assert!(snap.summary.bytes_added < 6_000_000 - partial / 2);
    let out = restore_all(&mut repo, &id, "/v");
    let mut expect: BTreeMap<String, F> = tree.under("/v").into_iter().map(|(p, f)| (format!("v/{p}"), f)).collect();
    expect.insert("v".into(), F::Dir);
    assert!(out == expect);

    // Cancelling a restore stops it too.
    let mut sk = Sk::default();
    let mut obs = CancelAfter { n: 0, progress: 0, warnings: vec![] };
    assert_eq!(repo.restore(&id, "/", &mut sk, &mut obs).err(), Some(Error::Cancelled));
}

#[test]
fn unreadable_entries_are_skipped_and_locks_conflict() {
    struct Flaky<'a>(Src<'a>);
    impl Source for Flaky<'_> {
        fn stat(&mut self, p: &str) -> Result<Entry, Error> {
            self.0.stat(p)
        }
        fn list(&mut self, p: &str) -> Result<Vec<Entry>, Error> {
            self.0.list(p)
        }
        fn read(&mut self, p: &str, o: u64, b: &mut [u8]) -> Result<usize, Error> {
            if p.ends_with("secret") {
                return Err(Error::Source("permission denied".into()));
            }
            self.0.read(p, o, b)
        }
    }
    let mut rng = ChaCha20Rng::seed_from_u64(7);
    let mut tree = Tree::default();
    tree.file("/x/ok", b"fine".to_vec());
    tree.file("/x/secret", b"nope".to_vec());
    let mut repo = Repository::init(MemBackend::new(), PASSWORD, &mut rng, &opts()).unwrap();
    let mut obs = CancelAfter { n: u32::MAX, progress: 0, warnings: vec![] };
    let (id, snap) =
        repo.backup(&mut Flaky(Src { t: &tree, reads: 0 }), &["/x"], &bopts(1), &mut rng, &mut obs).unwrap();
    assert_eq!(snap.summary.errors, 1);
    assert_eq!(obs.warnings.len(), 1);
    assert_eq!(repo.ls(&id, "/x").unwrap().len(), 1);
    // A missing top-level path is an error, not a silent empty snapshot.
    assert!(repo.backup(&mut Src { t: &tree, reads: 0 }, &["/nope"], &bopts(2), &mut rng, &mut Quiet).is_err());
}

/// A store shared between the repository and a test source, to catch a lock mid-backup.
#[derive(Clone)]
struct Shared(std::rc::Rc<std::cell::RefCell<MemBackend>>);
impl Backend for Shared {
    fn put(&mut self, k: FileKind, n: &str, d: &[u8]) -> Result<(), Error> {
        self.0.borrow_mut().put(k, n, d)
    }
    fn get(&mut self, k: FileKind, n: &str) -> Result<Vec<u8>, Error> {
        self.0.borrow_mut().get(k, n)
    }
    fn list(&mut self, k: FileKind) -> Result<Vec<String>, Error> {
        self.0.borrow_mut().list(k)
    }
    fn delete(&mut self, k: FileKind, n: &str) -> Result<(), Error> {
        self.0.borrow_mut().delete(k, n)
    }
}

#[test]
fn locks_conflict_and_stale_locks_are_removed() {
    struct Spy<'a> {
        src: Src<'a>,
        store: Shared,
        caught: Vec<((FileKind, String), Vec<u8>)>,
    }
    impl Source for Spy<'_> {
        fn stat(&mut self, p: &str) -> Result<Entry, Error> {
            self.src.stat(p)
        }
        fn list(&mut self, p: &str) -> Result<Vec<Entry>, Error> {
            let b = self.store.0.borrow();
            self.caught = b
                .files
                .iter()
                .filter(|((k, _), _)| *k == FileKind::Lock)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            self.src.list(p)
        }
        fn read(&mut self, p: &str, o: u64, b: &mut [u8]) -> Result<usize, Error> {
            self.src.read(p, o, b)
        }
    }
    let mut rng = ChaCha20Rng::seed_from_u64(9);
    let mut tree = Tree::default();
    tree.file("/x/a", b"a".to_vec());
    let store = Shared(Default::default());
    let mut repo = Repository::init(store.clone(), PASSWORD, &mut rng, &opts()).unwrap();
    repo.set_host("desktop");
    let mut spy = Spy { src: Src { t: &tree, reads: 0 }, store: store.clone(), caught: vec![] };
    repo.backup(&mut spy, &["/x"], &bopts(100), &mut rng, &mut Quiet).unwrap();
    assert_eq!(spy.caught.len(), 1, "backup held one lock while running");
    assert!(repo.locks().unwrap().is_empty(), "and released it");

    // Put the lock back, as if that backup had crashed.
    for (k, v) in spy.caught {
        store.0.borrow_mut().files.insert(k, v);
    }
    let locks = repo.locks().unwrap();
    assert_eq!((locks[0].1.host.as_str(), locks[0].1.exclusive), ("desktop", false));
    // Shared with shared is fine; exclusive (prune, forget) is refused.
    repo.backup(&mut Src { t: &tree, reads: 0 }, &["/x"], &bopts(200), &mut rng, &mut Quiet).unwrap();
    assert!(matches!(repo.prune(&PruneOptions::default(), &mut rng), Err(Error::Locked(_))));
    let keep = KeepPolicy { last: 1, ..Default::default() };
    assert!(matches!(repo.forget(&keep, Timestamp::from_secs(300), &mut rng), Err(Error::Locked(_))));
    assert_eq!(repo.remove_locks(Some(Timestamp::from_secs(50))).unwrap(), 0, "not older than 50");
    assert_eq!(repo.remove_locks(Some(Timestamp::from_secs(150))).unwrap(), 1);
    repo.forget(&keep, Timestamp::from_secs(300), &mut rng).unwrap();
    repo.prune(&PruneOptions::default(), &mut rng).unwrap();
    assert!(repo.check(true).unwrap().is_ok());
}

#[test]
fn bad_parameters_are_refused() {
    let mut rng = ChaCha20Rng::seed_from_u64(8);
    let mut o = opts();
    o.chunker = ChunkerParams { min: 100, avg: 50, max: 200 };
    assert!(matches!(Repository::init(MemBackend::new(), b"p", &mut rng, &o), Err(Error::Invalid(_))));
    let mut repo = Repository::init(MemBackend::new(), b"p", &mut rng, &opts()).unwrap();
    let t = Tree::default();
    assert!(matches!(
        repo.backup(&mut Src { t: &t, reads: 0 }, &["relative"], &bopts(1), &mut rng, &mut Quiet),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        repo.backup(&mut Src { t: &t, reads: 0 }, &["/a/../b"], &bopts(1), &mut rng, &mut Quiet),
        Err(Error::Invalid(_))
    ));
    assert!(normalize_path("/a//b/./c/").unwrap() == "/a/b/c");
}
