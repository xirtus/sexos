// SPDX-License-Identifier: GPL-3.0-only
//! std::fs implementations: a repository in a directory, a directory as a backup source,
//! and a directory as a restore target.

use crate::backend::{Backend, FileKind};
use crate::model::{Entry, Node, NodeKind, Sink, Source, Timestamp};
use crate::Error;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn be(e: std::io::Error, what: &Path) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::NotFound(what.display().to_string())
    } else {
        Error::Backend(format!("{}: {e}", what.display()))
    }
}

/// A repository in a host directory (a mounted USB disk, for example).
pub struct FsBackend {
    root: PathBuf,
}

impl FsBackend {
    /// Uses (and creates) `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| be(e, &root))?;
        Ok(FsBackend { root })
    }
    fn path(&self, kind: FileKind, name: &str) -> PathBuf {
        self.root.join(kind.path(name))
    }
}

impl Backend for FsBackend {
    fn put(&mut self, kind: FileKind, name: &str, data: &[u8]) -> Result<(), Error> {
        let p = self.path(kind, name);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|e| be(e, d))?;
        }
        let tmp = p.with_extension("tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| be(e, &tmp))?;
        f.write_all(data).and_then(|_| f.sync_all()).map_err(|e| be(e, &tmp))?;
        fs::rename(&tmp, &p).map_err(|e| be(e, &p))
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
        let mut dirs = Vec::new();
        match kind {
            FileKind::Config => {
                return Ok(if self.root.join("config").exists() { vec!["config".into()] } else { vec![] })
            }
            FileKind::Pack => {
                let d = self.root.join("data");
                if let Ok(rd) = fs::read_dir(&d) {
                    for e in rd.flatten() {
                        dirs.push(e.path());
                    }
                }
            }
            k => dirs.push(self.root.join(k.dir())),
        }
        let mut out = Vec::new();
        for d in dirs {
            let Ok(rd) = fs::read_dir(&d) else { continue };
            for e in rd {
                let e = e.map_err(|e| be(e, &d))?;
                let n = e.file_name().to_string_lossy().into_owned();
                if !n.ends_with(".tmp") && e.path().is_file() {
                    out.push(n);
                }
            }
        }
        Ok(out)
    }
    fn delete(&mut self, kind: FileKind, name: &str) -> Result<(), Error> {
        let p = self.path(kind, name);
        match fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(be(e, &p)),
            _ => Ok(()),
        }
    }
}

fn ts(t: std::io::Result<SystemTime>) -> Timestamp {
    match t.ok().map(|t| t.duration_since(UNIX_EPOCH)) {
        Some(Ok(d)) => Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() },
        Some(Err(e)) => Timestamp { secs: -(e.duration().as_secs() as i64), nanos: 0 },
        None => Timestamp::default(),
    }
}

fn se(e: std::io::Error, p: &Path) -> Error {
    Error::Source(format!("{}: {e}", p.display()))
}

/// A host directory as a backup source: source path `/x/y` is `root/x/y`.
pub struct FsSource {
    root: PathBuf,
}

impl FsSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsSource { root: root.into() }
    }
    fn host(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
    fn entry(name: String, m: &fs::Metadata) -> Entry {
        let kind = if m.file_type().is_symlink() {
            NodeKind::Symlink
        } else if m.is_dir() {
            NodeKind::Dir
        } else {
            NodeKind::File
        };
        #[cfg(unix)]
        let mode = std::os::unix::fs::PermissionsExt::mode(&m.permissions());
        #[cfg(not(unix))]
        let mode = if m.permissions().readonly() { 0o444 } else { 0o644 };
        Entry { name, kind, size: if kind == NodeKind::File { m.len() } else { 0 }, mtime: ts(m.modified()), mode }
    }
}

impl Source for FsSource {
    fn stat(&mut self, path: &str) -> Result<Entry, Error> {
        let p = self.host(path);
        let m = fs::symlink_metadata(&p).map_err(|e| se(e, &p))?;
        let name = path.rsplit('/').next().unwrap_or("").to_string();
        Ok(Self::entry(name, &m))
    }
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Error> {
        let p = self.host(path);
        let mut out = Vec::new();
        for e in fs::read_dir(&p).map_err(|e| se(e, &p))? {
            let e = e.map_err(|e| se(e, &p))?;
            let m = e.metadata().map_err(|err| se(err, &e.path()))?;
            out.push(Self::entry(e.file_name().to_string_lossy().into_owned(), &m));
        }
        Ok(out)
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
    fn read_link(&mut self, path: &str) -> Result<String, Error> {
        let p = self.host(path);
        fs::read_link(&p).map(|t| t.to_string_lossy().into_owned()).map_err(|e| se(e, &p))
    }
}

/// Restores into a host directory. Symlinks are recreated on Unix hosts only.
pub struct FsSink {
    root: PathBuf,
    file: Option<fs::File>,
}

impl FsSink {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| Error::Sink(format!("{}: {e}", root.display())))?;
        Ok(FsSink { root, file: None })
    }
    fn host(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

fn ke(e: std::io::Error, p: &Path) -> Error {
    Error::Sink(format!("{}: {e}", p.display()))
}

fn systime(t: Timestamp) -> SystemTime {
    if t.secs >= 0 {
        UNIX_EPOCH + Duration::new(t.secs as u64, t.nanos)
    } else {
        UNIX_EPOCH - Duration::from_secs(t.secs.unsigned_abs())
    }
}

impl Sink for FsSink {
    fn create_dir(&mut self, path: &str, _node: &Node) -> Result<(), Error> {
        let p = self.host(path);
        fs::create_dir_all(&p).map_err(|e| ke(e, &p))
    }
    fn finish_dir(&mut self, path: &str, node: &Node) -> Result<(), Error> {
        let p = self.host(path);
        if let Ok(d) = fs::File::open(&p) {
            let _ = d.set_modified(systime(node.mtime));
        }
        Ok(())
    }
    fn begin_file(&mut self, path: &str, _node: &Node) -> Result<(), Error> {
        let p = self.host(path);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|e| ke(e, d))?;
        }
        self.file = Some(fs::File::create(&p).map_err(|e| ke(e, &p))?);
        Ok(())
    }
    fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        let f = self.file.as_mut().ok_or_else(|| Error::Sink("write without a file".into()))?;
        f.write_all(data).map_err(|e| Error::Sink(e.to_string()))
    }
    fn end_file(&mut self, path: &str, node: &Node) -> Result<(), Error> {
        let f = self.file.take().ok_or_else(|| Error::Sink("end without a file".into()))?;
        f.set_modified(systime(node.mtime)).map_err(|e| ke(e, &self.host(path)))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if node.mode & 0o777 != 0 {
                let _ = f.set_permissions(fs::Permissions::from_mode(node.mode & 0o7777));
            }
        }
        Ok(())
    }
    fn symlink(&mut self, path: &str, node: &Node) -> Result<(), Error> {
        #[cfg(unix)]
        {
            let p = self.host(path);
            let _ = fs::remove_file(&p);
            std::os::unix::fs::symlink(&node.target, &p).map_err(|e| ke(e, &p))?;
        }
        #[cfg(not(unix))]
        let _ = (path, node);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackupOptions, InitOptions, KdfParams, ParentChoice, Quiet, Repository};
    use rand_chacha::rand_core::SeedableRng;

    #[test]
    fn host_directory_roundtrip() {
        let base = std::env::temp_dir().join(format!("aftercare-fs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (src, repo_dir, out) = (base.join("src"), base.join("repo"), base.join("out"));
        fs::create_dir_all(src.join("home/ann/docs")).unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        fs::write(src.join("home/ann/docs/big.bin"), &big).unwrap();
        fs::write(src.join("home/ann/note.txt"), b"remember the milk").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("note.txt", src.join("home/ann/link")).unwrap();

        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(42);
        let opts = InitOptions { kdf: KdfParams { m_cost_kib: 64, t_cost: 1, p_cost: 1 }, ..Default::default() };
        let mut repo = Repository::init(FsBackend::new(&repo_dir).unwrap(), b"pw", &mut rng, &opts).unwrap();
        let bo = BackupOptions { host: "h".into(), parent: ParentChoice::Latest, ..Default::default() };
        let (id, _) = repo.backup(&mut FsSource::new(&src), &["/home"], &bo, &mut rng, &mut Quiet).unwrap();
        assert!(repo_dir.join("config").is_file());
        assert!(fs::read_dir(repo_dir.join("data")).unwrap().count() >= 1);

        let mut repo = Repository::open(FsBackend::new(&repo_dir).unwrap(), b"pw").unwrap();
        assert!(repo.check(true).unwrap().is_ok());
        repo.restore(&id, "/home/ann", &mut FsSink::new(&out).unwrap(), &mut Quiet).unwrap();
        assert_eq!(fs::read(out.join("ann/docs/big.bin")).unwrap(), big);
        assert_eq!(fs::read(out.join("ann/note.txt")).unwrap(), b"remember the milk");
        let m1 = fs::metadata(src.join("home/ann/note.txt")).unwrap().modified().unwrap();
        let m2 = fs::metadata(out.join("ann/note.txt")).unwrap().modified().unwrap();
        assert_eq!(m1, m2);
        #[cfg(unix)]
        assert_eq!(fs::read_link(out.join("ann/link")).unwrap(), Path::new("note.txt"));
        fs::remove_dir_all(&base).unwrap();
    }
}
