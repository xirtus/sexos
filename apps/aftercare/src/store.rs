// SPDX-License-Identifier: GPL-3.0-only
//! The engine's three doors onto folders, through `std::fs` (Files underneath):
//!
//! * [`DirStore`]: a repository in a folder (a USB disk under `/media`, or any folder
//!   the person granted this run);
//! * [`DirSource`]: a granted folder as the backup source;
//! * [`DirSink`]: a folder that restored files are written into, hashing what it writes.
//!
//! A folder the person granted is `@/absolute/path` to this run. Inside a snapshot the
//! same folder is `/absolute/path`, so `@` + snapshot path is where a file came from.
//!
//! The engine's own `FsBackend`/`FsSource`/`FsSink` lean on `fs::rename`,
//! `File::sync_all` and `File::set_modified`, which std's Hermit port (this target)
//! refuses; these use `sex_std::fs` for the first two and leave modified times as
//! Files sets them.
use aftercare_core::{Backend, Entry, Error, FileKind, Node, NodeKind, Sink, Source, Timestamp};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::time::UNIX_EPOCH;

#[cfg(target_os = "hermit")]
mod os {
    use sex_std::fs::FileExt;
    pub fn rename(a: &str, b: &str) -> std::io::Result<()> { sex_std::fs::rename(a, b) }
    pub fn sync(f: &std::fs::File) -> std::io::Result<()> { f.fsync() }
    pub fn read_dir(p: &str) -> std::io::Result<Vec<std::fs::DirEntry>> { sex_std::fs::read_dir(p) }
}
#[cfg(not(target_os = "hermit"))]
mod os {
    pub fn rename(a: &str, b: &str) -> std::io::Result<()> { std::fs::rename(a, b) }
    pub fn sync(f: &std::fs::File) -> std::io::Result<()> { f.sync_all() }
    pub fn read_dir(p: &str) -> std::io::Result<Vec<std::fs::DirEntry>> { std::fs::read_dir(p)?.collect() }
}

/// One entry of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: Option<Timestamp>,
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// The folder holding `path` (`@/a/b` -> `@/a`, `/a` -> `/`).
pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) => "/".into(),
        Some(i) if path[..i].ends_with('@') => format!("{}/", &path[..i]),
        Some(i) => path[..i].into(),
        None => path.into(),
    }
}

/// A folder's entries, sorted by name. An empty folder is an empty list.
pub fn list(path: &str) -> io::Result<Vec<Listed>> {
    let mut out = Vec::new();
    for e in os::read_dir(path)? {
        let name = e.file_name().to_string_lossy().into_owned();
        let m = fs::metadata(join(path, &name))?;
        let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() });
        out.push(Listed { name, dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, mtime });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// SHA-256 of a whole file, as lowercase hex.
#[cfg(test)]
pub fn sha256_file(path: &str) -> io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match f.read(&mut buf)? {
            0 => break,
            n => h.update(&buf[..n]),
        }
    }
    Ok(hex(&h.finalize()))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Makes `rel`'s folders (`a/b/c`) inside `root`, which must exist.
fn make_dirs(root: &str, rel: &str, made: &mut BTreeSet<String>) -> io::Result<()> {
    let mut at = root.to_string();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        at = join(&at, part);
        if made.contains(&at) {
            continue;
        }
        match fs::create_dir(&at) {
            Ok(()) => {}
            Err(_) if fs::metadata(&at).is_ok_and(|m| m.is_dir()) => {}
            Err(e) => return Err(e),
        }
        made.insert(at.clone());
    }
    Ok(())
}

// ---- repository ----------------------------------------------------------------------

/// A repository in a folder, laid out as the engine's README describes.
pub struct DirStore {
    root: String,
    made: BTreeSet<String>,
}

fn be(e: io::Error, what: &str) -> Error {
    if e.kind() == io::ErrorKind::NotFound { Error::NotFound(what.into()) } else { Error::Backend(format!("{what}: {e}")) }
}

impl DirStore {
    /// Uses the folder `root` (which must exist).
    pub fn new(root: &str) -> DirStore {
        DirStore { root: root.trim_end_matches('/').to_string(), made: BTreeSet::new() }
    }
    /// Whether `root` already holds a repository.
    pub fn exists(root: &str) -> bool {
        fs::metadata(join(root, "config")).is_ok_and(|m| m.is_file())
    }
    fn path(&self, kind: FileKind, name: &str) -> String {
        join(&self.root, &kind.path(name))
    }
}

impl Backend for DirStore {
    fn put(&mut self, kind: FileKind, name: &str, data: &[u8]) -> Result<(), Error> {
        let rel = kind.path(name);
        if let Some((dir, _)) = rel.rsplit_once('/') {
            make_dirs(&self.root, dir, &mut self.made).map_err(|e| be(e, &join(&self.root, dir)))?;
        }
        let p = self.path(kind, name);
        let tmp = format!("{p}.tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| be(e, &tmp))?;
        f.write_all(data).and_then(|_| os::sync(&f)).map_err(|e| be(e, &tmp))?;
        drop(f);
        os::rename(&tmp, &p).map_err(|e| be(e, &p))
    }
    fn get(&mut self, kind: FileKind, name: &str) -> Result<Vec<u8>, Error> {
        let p = self.path(kind, name);
        fs::read(&p).map_err(|e| be(e, &p))
    }
    fn get_range(&mut self, kind: FileKind, name: &str, offset: u64, len: u32) -> Result<Vec<u8>, Error> {
        let p = self.path(kind, name);
        let mut f = fs::File::open(&p).map_err(|e| be(e, &p))?;
        let mut buf = vec![0; len as usize];
        f.seek(SeekFrom::Start(offset)).and_then(|_| f.read_exact(&mut buf)).map_err(|e| be(e, &p))?;
        Ok(buf)
    }
    fn size(&mut self, kind: FileKind, name: &str) -> Result<u64, Error> {
        let p = self.path(kind, name);
        fs::metadata(&p).map(|m| m.len()).map_err(|e| be(e, &p))
    }
    fn list(&mut self, kind: FileKind) -> Result<Vec<String>, Error> {
        let files = |d: &str| -> Result<Vec<String>, Error> {
            match list(d) {
                Ok(v) => Ok(v.into_iter().filter(|e| !e.dir && !e.name.ends_with(".tmp")).map(|e| e.name).collect()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(e) => Err(be(e, d)),
            }
        };
        match kind {
            FileKind::Config => Ok(if DirStore::exists(&self.root) { vec!["config".into()] } else { vec![] }),
            FileKind::Pack => {
                let data = join(&self.root, "data");
                let mut out = Vec::new();
                match list(&data) {
                    Ok(v) => {
                        for d in v.into_iter().filter(|e| e.dir) {
                            out.extend(files(&join(&data, &d.name))?);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(be(e, &data)),
                }
                Ok(out)
            }
            k => files(&join(&self.root, k.dir())),
        }
    }
    fn delete(&mut self, kind: FileKind, name: &str) -> Result<(), Error> {
        let p = self.path(kind, name);
        match fs::remove_file(&p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(be(e, &p)),
            _ => Ok(()),
        }
    }
}

// ---- source --------------------------------------------------------------------------

/// Files to back up: snapshot path `/x/y` is `prefix` + `/x/y` (`@/x/y` on SexOS).
pub struct DirSource {
    prefix: String,
    /// A modified time for files whose volume keeps none: a value no earlier snapshot
    /// holds, so such files are always read again (deduplication still stores nothing
    /// new for unchanged ones).
    unknown_mtime: Timestamp,
}

fn se(e: io::Error, p: &str) -> Error {
    Error::Source(format!("{p}: {e}"))
}

impl DirSource {
    pub fn new(prefix: &str, nonce: u32) -> DirSource {
        DirSource { prefix: prefix.to_string(), unknown_mtime: Timestamp { secs: -1, nanos: nonce % 1_000_000_000 } }
    }
    fn host(&self, path: &str) -> String {
        format!("{}{}", self.prefix, path)
    }
    fn entry(&self, l: Listed) -> Entry {
        let kind = if l.dir { NodeKind::Dir } else { NodeKind::File };
        Entry { name: l.name, kind, size: l.size, mtime: l.mtime.unwrap_or(self.unknown_mtime), mode: 0o644 }
    }
}

impl Source for DirSource {
    fn stat(&mut self, path: &str) -> Result<Entry, Error> {
        let p = self.host(path);
        let m = fs::metadata(&p).map_err(|e| se(e, &p))?;
        let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() });
        let name = path.rsplit('/').next().unwrap_or("").to_string();
        Ok(self.entry(Listed { name, dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, mtime }))
    }
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Error> {
        let p = self.host(path);
        Ok(list(&p).map_err(|e| se(e, &p))?.into_iter().map(|l| self.entry(l)).collect())
    }
    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        let p = self.host(path);
        let mut f = fs::File::open(&p).map_err(|e| se(e, &p))?;
        f.seek(SeekFrom::Start(offset)).map_err(|e| se(e, &p))?;
        let mut n = 0;
        while n < buf.len() {
            match f.read(&mut buf[n..]).map_err(|e| se(e, &p))? {
                0 => break,
                k => n += k,
            }
        }
        Ok(n)
    }
}

// ---- restore target ------------------------------------------------------------------

/// What a restore wrote: each file's path, length and SHA-256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Writes restored files into the folder `root`. A file already there is replaced.
pub struct DirSink {
    root: String,
    made: BTreeSet<String>,
    file: Option<(fs::File, Sha256, u64)>,
    pub written: Vec<Written>,
}

fn ke(e: io::Error, p: &str) -> Error {
    Error::Sink(format!("{p}: {e}"))
}

impl DirSink {
    pub fn new(root: &str) -> DirSink {
        DirSink { root: root.trim_end_matches('/').to_string(), made: BTreeSet::new(), file: None, written: Vec::new() }
    }
    fn host(&self, rel: &str) -> String {
        join(&self.root, rel)
    }
}

impl Sink for DirSink {
    fn create_dir(&mut self, path: &str, _node: &Node) -> Result<(), Error> {
        make_dirs(&self.root, path, &mut self.made).map_err(|e| ke(e, &self.host(path)))
    }
    fn begin_file(&mut self, path: &str, _node: &Node) -> Result<(), Error> {
        if let Some((dir, _)) = path.rsplit_once('/') {
            make_dirs(&self.root, dir, &mut self.made).map_err(|e| ke(e, &self.host(dir)))?;
        }
        let p = self.host(path);
        if fs::metadata(&p).is_ok_and(|m| m.is_file()) {
            fs::remove_file(&p).map_err(|e| ke(e, &p))?;
        }
        self.file = Some((fs::File::create(&p).map_err(|e| ke(e, &p))?, Sha256::new(), 0));
        Ok(())
    }
    fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        let (f, h, n) = self.file.as_mut().ok_or_else(|| Error::Sink("write without a file".into()))?;
        f.write_all(data).map_err(|e| Error::Sink(e.to_string()))?;
        h.update(data);
        *n += data.len() as u64;
        Ok(())
    }
    fn end_file(&mut self, path: &str, _node: &Node) -> Result<(), Error> {
        let (f, h, n) = self.file.take().ok_or_else(|| Error::Sink("end without a file".into()))?;
        let p = self.host(path);
        os::sync(&f).map_err(|e| ke(e, &p))?;
        self.written.push(Written { path: p, bytes: n, sha256: hex(&h.finalize()) });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aftercare_core::{BackupOptions, InitOptions, KdfParams, ParentChoice, Quiet, Repository};
    use rand_chacha::rand_core::SeedableRng;

    fn temp(tag: &str) -> String {
        let d = std::env::temp_dir().join(format!("aftercare-app-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    #[test]
    fn paths() {
        assert_eq!(parent("@/data/src/a.txt"), "@/data/src");
        assert_eq!(parent("@/data"), "@/");
        assert_eq!(parent("/data"), "/");
        assert_eq!(join("@/", "data"), "@/data");
    }

    /// Back up a folder, delete a file, restore it in place and to another folder:
    /// the hashes match, the wrong password is refused, and a second backup reuses data.
    #[test]
    fn backup_delete_restore() {
        let base = temp("rt");
        let (src, repo_dir, other) = (join(&base, "src"), join(&base, "repo"), join(&base, "other"));
        for d in [&src, &repo_dir, &other] {
            fs::create_dir_all(d).unwrap();
        }
        fs::create_dir_all(join(&src, "docs")).unwrap();
        let big: Vec<u8> = (0..1_500_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        fs::write(join(&src, "docs/big.bin"), &big).unwrap();
        fs::write(join(&src, "notes.txt"), b"remember the milk\n").unwrap();
        fs::create_dir_all(join(&src, "empty")).unwrap();
        let want = sha256_file(&join(&src, "docs/big.bin")).unwrap();

        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(7);
        let opts = InitOptions { kdf: KdfParams { m_cost_kib: 64, t_cost: 1, p_cost: 1 }, ..Default::default() };
        assert!(!DirStore::exists(&repo_dir));
        let mut repo = Repository::init(DirStore::new(&repo_dir), b"pw", &mut rng, &opts).unwrap();
        assert!(DirStore::exists(&repo_dir));
        // Snapshot paths are the source folder's absolute path; the prefix maps them to disk.
        let bo = BackupOptions { host: "sexos".into(), time: Timestamp::from_secs(1000), parent: ParentChoice::Latest, ..Default::default() };
        let (id, snap) = repo.backup(&mut DirSource::new("", 1), &[src.as_str()], &bo, &mut rng, &mut Quiet).unwrap();
        assert_eq!(snap.summary.files_new, 2);

        fs::remove_file(join(&src, "docs/big.bin")).unwrap();
        assert!(matches!(Repository::open(DirStore::new(&repo_dir), b"nope"), Err(Error::WrongPassword)));
        let mut repo = Repository::open(DirStore::new(&repo_dir), b"pw").unwrap();
        let mut sink = DirSink::new(&join(&src, "docs"));
        repo.restore(&id, &format!("{src}/docs/big.bin"), &mut sink, &mut Quiet).unwrap();
        assert_eq!(sink.written.len(), 1);
        assert_eq!(sink.written[0].sha256, want);
        assert_eq!(sha256_file(&join(&src, "docs/big.bin")).unwrap(), want);

        let mut sink = DirSink::new(&other);
        repo.restore(&id, &src, &mut sink, &mut Quiet).unwrap();
        assert_eq!(fs::read(join(&other, "src/notes.txt")).unwrap(), b"remember the milk\n");
        assert_eq!(sha256_file(&join(&other, "src/docs/big.bin")).unwrap(), want);
        assert!(fs::metadata(join(&other, "src/empty")).unwrap().is_dir());

        let bo2 = BackupOptions { time: Timestamp::from_secs(2000), ..bo };
        let (_, s2) = repo.backup(&mut DirSource::new("", 2), &[src.as_str()], &bo2, &mut rng, &mut Quiet).unwrap();
        // The restored file has a new modified time, so it is read again, but its data
        // is already stored: only the folder records change.
        assert_eq!((s2.summary.files_changed, s2.summary.bytes_added), (1, 0), "{:?}", s2.summary);
        assert!(repo.check(true).unwrap().is_ok());
        assert_eq!(repo.snapshots().unwrap().len(), 2);
        fs::remove_dir_all(&base).unwrap();
    }
}
