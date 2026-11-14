//! Round trips, interop with Python's zipfile/tarfile/gzip/lzma, 7z and
//! zstd reference fixtures, traversal rejection, virtual folders and
//! cancellation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use sex_archive::{
    no_progress, sanitize_path, Archive, ArchiveBuilder, EntryKind, Error, ExtractVisitor, Flow, Format, IoError,
    OutputFormat, Progress,
};

fn data(seed: u32, len: usize) -> Vec<u8> {
    // Half text-like (compressible), half pseudo-random.
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..len)
        .map(|i| {
            if (i / 4096) % 2 == 0 {
                b"lorem ipsum dolor sit amet "[i % 27]
            } else {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            }
        })
        .collect()
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sex-archive-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn python() -> Option<&'static str> {
    let ok = Command::new("python3").arg("-c").arg("import zipfile, tarfile, lzma, gzip").output().ok()?.status.success();
    if ok {
        Some("python3")
    } else {
        eprintln!("python3 not available: skipping interop test");
        None
    }
}

fn run_py(script: &str, args: &[&Path]) {
    let mut c = Command::new("python3");
    c.arg("-I").arg("-c").arg(script);
    for a in args {
        c.arg(a);
    }
    let out = c.output().unwrap();
    assert!(out.status.success(), "python failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn extract_vec<S: sex_archive::ByteSource>(a: &Archive<S>, i: usize) -> Vec<u8> {
    let mut v = Vec::new();
    a.extract(i, &mut v, &mut no_progress).unwrap_or_else(|e| panic!("extract {}: {e}", a.entries()[i].path));
    v
}

/// path -> contents of every regular file.
fn read_all<S: sex_archive::ByteSource>(a: &Archive<S>) -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    for (i, e) in a.entries().iter().enumerate() {
        if e.kind == EntryKind::File {
            m.insert(e.path.clone(), extract_vec(a, i));
        }
    }
    m
}

fn sample_items() -> Vec<(String, Vec<u8>)> {
    let long = format!("{}/{}", "d".repeat(120), "f".repeat(90)); // needs ustar prefix
    let very_long = format!("{}{}.txt", "segment/".repeat(40), "x".repeat(150)); // needs pax
    vec![
        ("hello.txt".into(), b"hello world\n".to_vec()),
        ("empty".into(), Vec::new()),
        ("dir/big.bin".into(), data(1, 300_000)),
        ("dir/sub/ünïcödé ☃.txt".into(), "snow ☃\n".as_bytes().to_vec()),
        (long, data(2, 5000)),
        (very_long, data(3, 70_000)),
    ]
}

fn build(fmt: OutputFormat, items: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut b = ArchiveBuilder::new(fmt);
    b.add_dir("dir", 1_700_000_000).unwrap();
    for (p, d) in items {
        b.add_file(p, d.len() as u64, 1_700_000_000, &d[..]).unwrap();
    }
    let mut out = Vec::new();
    let mut last = Progress { done: 0, total: 0 };
    let n = b
        .write(&mut out, &mut |p| {
            assert!(p.done >= last.done && p.done <= p.total);
            last = p;
            Flow::Continue
        })
        .unwrap();
    assert_eq!(n as usize, out.len());
    assert_eq!(last.done, last.total);
    out
}

fn check_round_trip(fmt: OutputFormat, expect: Format) {
    let items = sample_items();
    let bytes = build(fmt, &items);
    let a = Archive::open(bytes).unwrap();
    assert_eq!(a.format(), expect);
    let got = read_all(&a);
    let want: BTreeMap<String, Vec<u8>> = items.iter().cloned().collect();
    assert_eq!(got.len(), want.len());
    for (k, v) in &want {
        assert_eq!(got.get(k).map(|g| crc32fast::hash(g)), Some(crc32fast::hash(v)), "{k}");
        assert_eq!(got[k], *v);
    }
    let d = &a.entries()[a.find("dir").unwrap()];
    assert!(d.is_dir());
    assert_eq!(d.mtime, Some(1_700_000_000));
    let f = &a.entries()[a.find("hello.txt").unwrap()];
    assert_eq!((f.size, f.mtime, f.mode), (12, Some(1_700_000_000), Some(0o644)));
}

#[test]
fn zip_round_trip() {
    check_round_trip(OutputFormat::Zip, Format::Zip);
}

#[test]
fn tar_round_trip() {
    check_round_trip(OutputFormat::Tar, Format::Tar);
}

#[test]
fn tar_gz_round_trip() {
    check_round_trip(OutputFormat::TarGz, Format::TarGz);
}

#[test]
fn tar_zst_round_trip() {
    check_round_trip(OutputFormat::TarZst, Format::TarZst);
}

#[test]
fn zip_forced_zip64_round_trip() {
    let items = sample_items();
    let mut b = ArchiveBuilder::new(OutputFormat::Zip);
    b.force_zip64(true);
    for (p, d) in &items {
        b.add_file(p, d.len() as u64, 0, &d[..]).unwrap();
    }
    let mut out = Vec::new();
    b.write(&mut out, &mut no_progress).unwrap();
    let a = Archive::open(out).unwrap();
    assert_eq!(read_all(&a), items.into_iter().collect());
}

#[test]
fn size_mismatch_is_reported() {
    for fmt in [OutputFormat::Zip, OutputFormat::Tar, OutputFormat::TarZst] {
        let mut b = ArchiveBuilder::new(fmt);
        b.add_file("a", 10, 0, &b"short"[..]).unwrap();
        assert_eq!(b.write(&mut Vec::new(), &mut no_progress), Err(Error::SizeMismatch), "{fmt:?}");
        let mut b = ArchiveBuilder::new(fmt);
        b.add_file("a", 2, 0, &b"too long"[..]).unwrap();
        assert_eq!(b.write(&mut Vec::new(), &mut no_progress), Err(Error::SizeMismatch), "{fmt:?}");
    }
}

#[test]
fn builder_rejects_unsafe_names() {
    let mut b = ArchiveBuilder::new(OutputFormat::Zip);
    assert!(matches!(b.add_file("../x", 0, 0, &b""[..]), Err(Error::UnsafePath(_))));
    assert!(matches!(b.add_file("/etc/x", 0, 0, &b""[..]), Err(Error::UnsafePath(_))));
    assert!(matches!(b.add_dir("./", 0), Err(Error::InvalidName(_))));
}

#[test]
fn sanitize() {
    assert_eq!(sanitize_path("./a//b/./c").unwrap(), "a/b/c");
    assert_eq!(sanitize_path("a/b/").unwrap(), "a/b");
    for bad in ["../a", "a/../../b", "/abs", "a/..", "C:/win", "c:evil", "a\0b"] {
        assert!(matches!(sanitize_path(bad), Err(Error::UnsafePath(_))), "{bad}");
    }
}

// ---------------------------------------------------------------- traversal

fn tar_header(name: &str, size: usize, typeflag: u8) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..107].copy_from_slice(b"0000644");
    h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    h[136..147].copy_from_slice(b"00000000000");
    h[156] = typeflag;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    h
}

fn raw_tar(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut t = Vec::new();
    for (n, d) in members {
        t.extend(tar_header(n, d.len(), b'0'));
        t.extend_from_slice(d);
        t.resize(t.len().div_ceil(512) * 512, 0);
    }
    t.resize(t.len() + 1024, 0);
    t
}

struct Collect {
    files: BTreeMap<String, Vec<u8>>,
    cur: Option<String>,
    rejected: Vec<String>,
}

impl Collect {
    fn new() -> Self {
        Self { files: BTreeMap::new(), cur: None, rejected: Vec::new() }
    }
}

impl ExtractVisitor for Collect {
    fn begin(&mut self, _e: &sex_archive::Entry, path: &str) -> bool {
        self.cur = Some(path.to_string());
        self.files.insert(path.to_string(), Vec::new());
        true
    }
    fn data(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        self.files.get_mut(self.cur.as_ref().unwrap()).unwrap().extend_from_slice(bytes);
        Ok(())
    }
    fn end(&mut self, _e: &sex_archive::Entry) -> Result<(), IoError> {
        self.cur = None;
        Ok(())
    }
    fn rejected(&mut self, e: &sex_archive::Entry) {
        self.rejected.push(e.raw_path.clone());
    }
}

#[test]
fn traversal_is_rejected() {
    let t = raw_tar(&[("../evil.txt", b"x"), ("/etc/passwd", b"y"), ("ok/../../z", b"z"), ("./good/./f.txt", b"g")]);
    let a = Archive::open(t).unwrap();
    assert_eq!(a.format(), Format::Tar);
    assert_eq!(a.entries().len(), 4);
    for i in 0..3 {
        let e = &a.entries()[i];
        assert!(!e.safe);
        let mut sink = Vec::new();
        assert_eq!(a.extract(i, &mut sink, &mut no_progress), Err(Error::UnsafePath(e.raw_path.clone())));
        assert!(sink.is_empty());
    }
    let mut v = Vec::new();
    let r = a.extract(3, &mut v, &mut no_progress).unwrap();
    assert_eq!((r.path.as_str(), v.as_slice()), ("good/f.txt", &b"g"[..]));
    // Unsafe members are hidden from the folder view.
    let root: Vec<_> = a.list_dir("").unwrap().into_iter().map(|d| d.name).collect();
    assert_eq!(root, ["good"]);
    let mut c = Collect::new();
    a.extract_all(&mut c, &mut no_progress).unwrap();
    assert_eq!(c.files.keys().collect::<Vec<_>>(), ["good/f.txt"]);
    assert_eq!(c.rejected, ["../evil.txt", "/etc/passwd", "ok/../../z"]);
}

#[test]
fn zip_backslash_traversal_is_rejected() {
    let Some(_) = python() else { return };
    let d = tmpdir("zipslip");
    let z = d.join("slip.zip");
    run_py(
        "import zipfile,sys\nwith zipfile.ZipFile(sys.argv[1],'w') as z:\n z.writestr(zipfile.ZipInfo('..\\\\..\\\\evil.txt'),'x')\n z.writestr(zipfile.ZipInfo('/abs.txt'),'y')\n z.writestr(zipfile.ZipInfo('fine\\\\name.txt'),'z')\n",
        &[&z],
    );
    let a = Archive::open(std::fs::read(&z).unwrap()).unwrap();
    let e: Vec<_> = a.entries().iter().map(|e| (e.path.clone(), e.safe)).collect();
    assert_eq!(e[2], ("fine/name.txt".to_string(), true));
    assert!(!e[0].1 && !e[1].1);
    assert!(matches!(a.extract(0, &mut Vec::new(), &mut no_progress), Err(Error::UnsafePath(_))));
    assert!(matches!(a.extract(1, &mut Vec::new(), &mut no_progress), Err(Error::UnsafePath(_))));
    assert_eq!(a.extract(2, &mut Vec::new(), &mut no_progress).unwrap().path, "fine/name.txt");
}

// ---------------------------------------------------------------- virtual dirs

#[test]
fn virtual_dirs() {
    let t = raw_tar(&[("a/b/c.txt", b"c"), ("x.txt", b"x"), ("a/b/d/e.txt", b"e"), ("a/z.txt", b"z")]);
    let a = Archive::open(t).unwrap();
    let names = |p: &str| -> Vec<(String, bool, bool)> {
        a.list_dir(p).unwrap().into_iter().map(|d| (d.name.clone(), d.is_dir(), d.entry.is_some())).collect()
    };
    assert_eq!(names(""), [("a".into(), true, false), ("x.txt".into(), false, true)]);
    assert_eq!(names("/"), names(""));
    assert_eq!(names("a"), [("b".into(), true, false), ("z.txt".into(), false, true)]);
    assert_eq!(names("a/b/"), [("d".into(), true, false), ("c.txt".into(), false, true)]);
    let item = &a.list_dir("a/b").unwrap()[0];
    assert_eq!(item.path, "a/b/d");
    assert_eq!(a.list_dir("x.txt"), Err(Error::NotADirectory));
    assert_eq!(a.list_dir("nope"), Err(Error::NoSuchEntry));
    assert_eq!(a.list_dir("a/b/c"), Err(Error::NoSuchEntry));

    // Explicit folder entries are used when present.
    let mut b = ArchiveBuilder::new(OutputFormat::Zip);
    b.add_dir("only/empty", 5).unwrap();
    let mut out = Vec::new();
    b.write(&mut out, &mut no_progress).unwrap();
    let z = Archive::open(out).unwrap();
    let only = z.list_dir("only").unwrap();
    assert_eq!(only.len(), 1);
    assert!(only[0].is_dir() && only[0].entry.is_some() && only[0].mtime == Some(5));
    assert!(z.list_dir("only/empty").unwrap().is_empty());
    assert_eq!(z.extract(0, &mut Vec::new(), &mut no_progress), Err(Error::NotAFile));
}

// ---------------------------------------------------------------- cancellation

#[test]
fn cancellation() {
    let items = sample_items();
    for fmt in [OutputFormat::Zip, OutputFormat::TarGz, OutputFormat::TarZst] {
        let bytes = build(fmt, &items);
        let a = Archive::open(bytes).unwrap();
        let i = a.find("dir/big.bin").unwrap();
        let mut calls = 0;
        let mut sink = Vec::new();
        let r = a.extract(i, &mut sink, &mut |p| {
            calls += 1;
            if p.done > 0 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        });
        assert_eq!(r, Err(Error::Cancelled), "{fmt:?}");
        assert!(sink.len() < 300_000);
        assert!(calls >= 2);

        let mut c = Collect::new();
        assert_eq!(a.extract_all(&mut c, &mut |_| Flow::Cancel), Err(Error::Cancelled));

        let mut b = ArchiveBuilder::new(fmt);
        b.add_file("big", 300_000, 0, &items[2].1[..]).unwrap();
        let r = b.write(&mut Vec::new(), &mut |p| if p.done >= 65536 { Flow::Cancel } else { Flow::Continue });
        assert_eq!(r, Err(Error::Cancelled), "{fmt:?}");
    }
}

#[test]
fn corruption_is_detected() {
    let items = sample_items();
    let mut bytes = build(OutputFormat::Zip, &items);
    // Flip a byte inside the stored-ish text of hello.txt's deflate data.
    let a = Archive::open(bytes.clone()).unwrap();
    let i = a.find("dir/big.bin").unwrap();
    let pos = bytes.len() / 3;
    bytes[pos] ^= 0x55;
    let a2 = Archive::open(bytes).unwrap();
    let r = a2.extract(i, &mut Vec::new(), &mut no_progress);
    assert!(matches!(r, Err(Error::Checksum) | Err(Error::Corrupt(_))), "{r:?}");
    drop(a);
    assert_eq!(Archive::open(b"just some text, not an archive".to_vec()).err(), Some(Error::UnknownFormat));
}

#[test]
fn extract_all_one_pass() {
    let items = sample_items();
    for fmt in [OutputFormat::Zip, OutputFormat::Tar, OutputFormat::TarGz, OutputFormat::TarZst] {
        let a = Archive::open(build(fmt, &items)).unwrap();
        let mut c = Collect::new();
        let mut last = Progress { done: 0, total: 0 };
        a.extract_all(&mut c, &mut |p| {
            last = p;
            Flow::Continue
        })
        .unwrap();
        assert_eq!(c.files, items.iter().cloned().collect::<BTreeMap<_, _>>(), "{fmt:?}");
        assert_eq!(last.done, last.total);
    }
}

// ---------------------------------------------------------------- fixtures

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn fixture_files() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert("docs/readme.txt".to_string(), b"hello from 7z\n".repeat(50));
    m.insert("docs/deep/nested/data.bin".to_string(), (0..=255u8).collect::<Vec<_>>().repeat(40));
    m.insert("empty.txt".to_string(), Vec::new());
    m.insert("unicode-éè.txt".to_string(), "café ☃\n".as_bytes().to_vec());
    m
}

#[test]
fn sevenz_fixtures() {
    // Made by py7zr 1.1.4 with tests/fixtures/gen_fixtures.py.
    for name in ["lzma2.7z", "lzma.7z", "copy.7z"] {
        let a = Archive::open(fixture(name)).unwrap();
        assert_eq!(a.format(), Format::SevenZ);
        assert_eq!(read_all(&a), fixture_files(), "{name}");
        let mut c = Collect::new();
        a.extract_all(&mut c, &mut no_progress).unwrap();
        assert_eq!(c.files, fixture_files(), "{name} extract_all");
        let root: Vec<_> = a.list_dir("").unwrap().into_iter().map(|d| d.name).collect();
        assert_eq!(root, ["docs", "empty.txt", "unicode-éè.txt"]);
        assert!(a.entries().iter().all(|e| e.mtime.is_some()));
    }
    let a = Archive::open(fixture("bcj.7z")).unwrap();
    assert_eq!(a.entries().len(), 4);
    let i = a.find("docs/readme.txt").unwrap();
    assert!(matches!(a.extract(i, &mut Vec::new(), &mut no_progress), Err(Error::Unsupported(_))));
}

#[test]
fn zstd_reference_fixture() {
    // Two concatenated frames from the reference zstd library, GNU tar.
    let a = Archive::open(fixture("ref.tar.zst")).unwrap();
    assert_eq!(a.format(), Format::TarZst);
    assert_eq!(read_all(&a), fixture_files());
    assert_eq!(a.entries()[0].mtime, Some(1_700_000_000));
}

// ---------------------------------------------------------------- python interop

const PY_MAKE: &str = r##"
import zipfile, tarfile, gzip, lzma, io, sys, os
out = sys.argv[1]
def data(i, n):
    return bytes(((j * 31 + i * 7) ^ (j >> 5)) & 0xff for j in range(n))
files = {
    "a.txt": b"alpha\n" * 1000,
    "dir/b.bin": data(1, 200000),
    "dir/sub/c.txt": "unicod\u00e9 \u2603".encode(),
    "empty": b"",
    ("L" * 150) + "/" + ("n" * 120) + ".txt": data(2, 3000),
}
exp = os.path.join(out, "expected")
for k, v in files.items():
    p = os.path.join(exp, k)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "wb").write(v)
with zipfile.ZipFile(os.path.join(out, "deflate.zip"), "w", zipfile.ZIP_DEFLATED) as z:
    for k, v in files.items():
        z.writestr(k, v)
with zipfile.ZipFile(os.path.join(out, "stored.zip"), "w", zipfile.ZIP_STORED) as z:
    for k, v in files.items():
        z.writestr(k, v)
with zipfile.ZipFile(os.path.join(out, "local64.zip"), "w", zipfile.ZIP_DEFLATED) as z:
    for k, v in files.items():
        with z.open(k, "w", force_zip64=True) as f:
            f.write(v)
# More than 65535 members forces the zip64 end-of-central-directory records.
with zipfile.ZipFile(os.path.join(out, "many.zip"), "w", zipfile.ZIP_STORED) as z:
    for k, v in files.items():
        z.writestr(k, v)
    for i in range(65600):
        z.writestr("m/%d" % i, b"")
# Self-extractor style: junk before the zip.
open(os.path.join(out, "sfx.zip"), "wb").write(b"#!stub\n" * 100 + open(os.path.join(out, "deflate.zip"), "rb").read())
for fmt, name in ((tarfile.GNU_FORMAT, "gnu.tar"), (tarfile.PAX_FORMAT, "pax.tar"), (tarfile.USTAR_FORMAT, "ustar.tar")):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=fmt) as t:
        for k, v in files.items():
            if fmt == tarfile.USTAR_FORMAT and len(k) > 255:
                continue
            ti = tarfile.TarInfo(k); ti.size = len(v); ti.mtime = 1234567890; ti.mode = 0o600
            t.addfile(ti, io.BytesIO(v))
        ti = tarfile.TarInfo("link"); ti.type = tarfile.SYMTYPE; ti.linkname = "a.txt"; t.addfile(ti)
        ti = tarfile.TarInfo("hard"); ti.type = tarfile.LNKTYPE; ti.linkname = "a.txt"; t.addfile(ti)
    open(os.path.join(out, name), "wb").write(buf.getvalue())
raw = open(os.path.join(out, "pax.tar"), "rb").read()
open(os.path.join(out, "pax.tar.gz"), "wb").write(gzip.compress(raw))
open(os.path.join(out, "pax.tgz"), "wb").write(gzip.compress(raw[:5000]) + gzip.compress(raw[5000:]))
open(os.path.join(out, "pax.tar.xz"), "wb").write(lzma.compress(raw, check=lzma.CHECK_CRC64))
with gzip.GzipFile(os.path.join(out, "single.gz"), "wb", mtime=1000000000) as g:
    g.write(files["dir/b.bin"])
open(os.path.join(out, "plain.xz"), "wb").write(lzma.compress(files["dir/b.bin"], check=lzma.CHECK_CRC32))
open(os.path.join(out, "sha.xz"), "wb").write(lzma.compress(files["a.txt"], check=lzma.CHECK_SHA256))
"##;

fn expected(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, p: &Path, m: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap().path();
            if e.is_dir() {
                walk(base, &e, m);
            } else {
                let rel = e.strip_prefix(base).unwrap().to_str().unwrap().replace('\\', "/");
                m.insert(rel, std::fs::read(&e).unwrap());
            }
        }
    }
    let mut m = BTreeMap::new();
    let base = dir.join("expected");
    walk(&base, &base, &mut m);
    m
}

#[test]
fn python_made_archives() {
    let Some(_) = python() else { return };
    let d = tmpdir("pymade");
    run_py(PY_MAKE, &[&d]);
    let want = expected(&d);
    let long = want.keys().find(|k| k.len() > 255).unwrap().clone();

    for (name, fmt) in [
        ("deflate.zip", Format::Zip),
        ("stored.zip", Format::Zip),
        ("local64.zip", Format::Zip),
        ("sfx.zip", Format::Zip),
        ("gnu.tar", Format::Tar),
        ("pax.tar", Format::Tar),
        ("pax.tar.gz", Format::TarGz),
        ("pax.tgz", Format::TarGz),
        ("pax.tar.xz", Format::TarXz),
    ] {
        let a = Archive::open(std::fs::read(d.join(name)).unwrap()).unwrap();
        assert_eq!(a.format(), fmt, "{name}");
        assert_eq!(read_all(&a), want, "{name}");
        assert!(a.find(&long).is_some(), "{name}: long name");
        if name.contains("tar") {
            let link = &a.entries()[a.find("link").unwrap()];
            assert_eq!((link.kind, link.link_target.as_deref()), (EntryKind::Symlink, Some("a.txt")));
            let hard = a.find("hard").unwrap();
            assert_eq!(extract_vec(&a, hard), want["a.txt"]);
            assert_eq!(a.entries()[a.find("a.txt").unwrap()].mtime, Some(1234567890));
            let mut c = Collect::new();
            a.extract_all(&mut c, &mut no_progress).unwrap();
            assert_eq!(c.files["hard"], want["a.txt"], "{name}");
        }
    }
    let a = Archive::open(std::fs::read(d.join("ustar.tar")).unwrap()).unwrap();
    assert_eq!(a.entries()[0].mode, Some(0o600));

    let many = Archive::open(std::fs::read(d.join("many.zip")).unwrap()).unwrap();
    assert_eq!(many.entries().len(), want.len() + 65600);
    assert_eq!(extract_vec(&many, many.find("dir/b.bin").unwrap()), want["dir/b.bin"]);
    assert_eq!(many.list_dir("m").unwrap().len(), 65600);

    let g = Archive::open_named(std::fs::read(d.join("single.gz")).unwrap(), Some("single.gz")).unwrap();
    assert_eq!(g.format(), Format::Gz);
    let e = &g.entries()[0];
    assert_eq!((e.path.as_str(), e.size, e.size_known, e.mtime), ("single", 200000, false, Some(1000000000)));
    assert_eq!(extract_vec(&g, 0), want["dir/b.bin"]);

    let x = Archive::open_named(std::fs::read(d.join("plain.xz")).unwrap(), Some("/x/payload.bin.xz")).unwrap();
    assert_eq!(x.format(), Format::Xz);
    assert_eq!((x.entries()[0].path.as_str(), x.entries()[0].size), ("payload.bin", 200000));
    assert_eq!(extract_vec(&x, 0), want["dir/b.bin"]);
    let x = Archive::open(std::fs::read(d.join("sha.xz")).unwrap()).unwrap();
    assert_eq!(x.entries()[0].path, "data");
    assert_eq!(extract_vec(&x, 0), want["a.txt"]);
}

const PY_CHECK: &str = r##"
import zipfile, tarfile, sys, zlib
kind, path, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
if kind.endswith("zip"):
    with zipfile.ZipFile(path) as z:
        assert z.testzip() is None
        names = [i.filename for i in z.infolist() if not i.is_dir()]
        assert len(names) == n, names
        for i in z.infolist():
            if i.filename.endswith("big.bin"):
                print(zlib.crc32(z.read(i)))
else:
    with tarfile.open(path) as t:
        names = [m.name for m in t.getmembers() if m.isfile()]
        assert len(names) == n, names
        for m in t.getmembers():
            if m.name.endswith("big.bin"):
                print(zlib.crc32(t.extractfile(m).read()))
"##;

#[test]
fn archives_read_by_python() {
    let Some(_) = python() else { return };
    let d = tmpdir("pyread");
    let items = sample_items();
    let big_crc = crc32fast::hash(&items[2].1);
    let mut z64 = ArchiveBuilder::new(OutputFormat::Zip);
    z64.force_zip64(true);
    for (p, v) in &items {
        z64.add_file(p, v.len() as u64, 0, &v[..]).unwrap();
    }
    let mut z64_bytes = Vec::new();
    z64.write(&mut z64_bytes, &mut no_progress).unwrap();
    for (name, bytes) in [
        ("out.zip", build(OutputFormat::Zip, &items)),
        ("out64.zip", z64_bytes),
        ("out.tar", build(OutputFormat::Tar, &items)),
        ("out.tar.gz", build(OutputFormat::TarGz, &items)),
    ] {
        let p = d.join(name);
        std::fs::write(&p, bytes).unwrap();
        let out = Command::new("python3")
            .arg("-I")
            .arg("-c")
            .arg(PY_CHECK)
            .arg(name)
            .arg(&p)
            .arg(items.len().to_string())
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), big_crc.to_string(), "{name}");
    }
}

#[cfg(feature = "std")]
#[test]
fn std_file_source() {
    let d = tmpdir("stdsrc");
    let p = d.join("t.zip");
    std::fs::write(&p, build(OutputFormat::Zip, &sample_items())).unwrap();
    let src = sex_archive::StdSource::new(std::fs::File::open(&p).unwrap()).unwrap();
    let a = Archive::open(src).unwrap();
    let mut out = sex_archive::StdSink(Vec::new());
    a.extract(a.find("hello.txt").unwrap(), &mut out, &mut no_progress).unwrap();
    assert_eq!(out.0, b"hello world\n");
}
