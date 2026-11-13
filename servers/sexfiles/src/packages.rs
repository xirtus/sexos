// SPDX-License-Identifier: GPL-3.0-only
//! Packages (`sex_pkg`): Files checks a package's signature against the keys this system
//! trusts, then installs its program into `/bin` and its data into the program's own
//! folder, in one filesystem transaction: a failure part way leaves nothing behind.
//!
//! Trusted keys: the release key this system was built with, and the keys the person
//! added with `pkg trust` (`/system/packages/trusted`). Records of what each package
//! installed, and the person's answers to its grants, live in `/system/packages/PROGRAM/`.
//! Only Spindle, the person's terminal, reaches these operations.
use crate::fs_service::Disk;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_pdx::fs as w;
use sex_pdx::package as p;
use sex_pkg::{Manifest, Target};
use sexfs::{Error, Fs, Kind};

type F = Fs<'static, Disk>;
const CHUNK: usize = 64 * 1024;

fn read_all(fs: &mut F, path: &[u8], max: usize) -> Result<Vec<u8>, Error> {
    let st = fs.open(path)?;
    if st.kind == Kind::Dir { return Err(Error::IsDir); }
    if st.size as usize > max { return Err(Error::TooBig); }
    let mut buf = alloc::vec![0u8; st.size as usize];
    read_exact(fs, st.id, 0, &mut buf)?;
    Ok(buf)
}
fn read_exact(fs: &mut F, id: u64, mut off: u64, mut buf: &mut [u8]) -> Result<(), Error> {
    while !buf.is_empty() {
        let n = fs.read(id, off, buf)?;
        if n == 0 { return Err(Error::Corrupt); }
        off += n as u64;
        buf = &mut buf[n..];
    }
    Ok(())
}
fn mkdirs(fs: &mut F, path: &[u8]) -> Result<(), Error> {
    for (i, b) in path.iter().enumerate().skip(1).chain(core::iter::once((path.len(), &b'/'))) {
        if *b != b'/' { continue; }
        match fs.open(&path[..i]) {
            Ok(st) if st.kind == Kind::Dir => {}
            Ok(_) => return Err(Error::NotDir),
            Err(Error::NotFound) => { fs.mkdir(&path[..i], 0)?; }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn write_file(fs: &mut F, path: &[u8], data: &[u8]) -> Result<(), Error> {
    if let Some(slash) = path.iter().rposition(|b| *b == b'/') { if slash > 0 { mkdirs(fs, &path[..slash])?; } }
    let id = match fs.open(path) { Ok(st) if st.kind == Kind::Dir => return Err(Error::IsDir), Ok(st) => { fs.truncate(st.id, 0, 0)?; st.id } Err(Error::NotFound) => fs.create(path, 0)?, Err(e) => return Err(e) };
    let mut at = 0;
    while at < data.len() { at += fs.write(id, at as u64, &data[at..], 0)?; }
    Ok(())
}
fn record(program: &str, leaf: &str) -> Vec<u8> { format!("{}/{}/{}", core::str::from_utf8(p::ROOT).unwrap(), program, leaf).into_bytes() }
fn bin(program: &str) -> Vec<u8> { format!("/bin/{program}").into_bytes() }
fn data_path(program: &str, rel: &str) -> Vec<u8> { format!("/data/{program}/{rel}").into_bytes() }

/// The keys packages may be signed with.
pub fn trusted(fs: &mut F) -> Vec<[u8; 32]> {
    let mut keys = sex_pkg::unhex::<32>(env!("SEXOS_RELEASE_KEY_HEX").as_bytes()).into_iter().collect::<Vec<_>>();
    if let Ok(text) = read_all(fs, p::TRUSTED, 64 * 1024) { keys.extend(sex_pkg::parse_keys(&text)); }
    keys
}
/// What the installed program's package asks for, and what the person said.
pub fn note(fs: &mut F, program: &[u8]) -> Option<p::Note> {
    let program = core::str::from_utf8(program).ok().filter(|s| sex_pkg::valid_name(s))?;
    let text = read_all(fs, &record(program, "manifest"), sex_pkg::MANIFEST_MAX).ok()?;
    let m = Manifest::parse(core::str::from_utf8(&text).ok()?).ok()?;
    let (answered, allowed) = read_all(fs, &record(program, "consent"), 256).map(|c| p::parse_consent(&c)).unwrap_or((0, 0));
    Some(p::Note { asked: m.grants, answered: answered & m.grants, allowed: allowed & answered & m.grants })
}

struct Report<'a> { out: &'a mut [u8], n: usize }
impl Report<'_> {
    fn line(&mut self, s: &str) {
        for b in s.bytes().chain(core::iter::once(b'\n')) { if self.n < self.out.len() { self.out[self.n] = b; self.n += 1; } }
    }
}

/// One package request. Returns the reply status and the report's length.
pub fn handle(fs: &mut F, op: u64, arg: &[u8], length: u64, out: &mut [u8]) -> (u8, u32) {
    let mut r = Report { out, n: 0 };
    let status = match op {
        p::INSTALL | p::CHECK => {
            let st = install(fs, arg, op == p::INSTALL, length & p::ALLOW_DOWNGRADE != 0, &mut r);
            // A download is used once, whatever came of it.
            if arg == p::DOWNLOAD && op == p::INSTALL {
                if st != w::OK && fs.has_changes() { let _ = fs.rollback(); }
                if fs.remove_path(p::DOWNLOAD).is_ok() && st != w::OK { let _ = fs.commit(); }
            }
            st
        }
        p::REMOVE => remove(fs, arg, &mut r),
        p::LIST => list(fs, &mut r),
        p::TRUST => trust(fs, arg, &mut r),
        p::CONSENT => consent(fs, arg, length, &mut r),
        _ => w::INVALID,
    };
    // A change is all or nothing: commit it, or go back to before it.
    if status == w::OK && matches!(op, p::INSTALL | p::REMOVE | p::TRUST | p::CONSENT) {
        if let Err(e) = fs.commit() { let _ = fs.rollback(); r.line(&format!("The disk failed while saving ({e:?}). Nothing changed.")); return (w::IO, r.n as u32); }
    } else if fs.has_changes() { let _ = fs.rollback(); }
    (status, r.n as u32)
}

fn refused(r: &mut Report, why: &str) -> u8 { sex_pdx::serial_record!("[files.pkg.refused] {}", why); r.line(why); w::DENIED }

fn install(fs: &mut F, path: &[u8], go: bool, downgrade: bool, r: &mut Report) -> u8 {
    let st = match fs.open(path) { Ok(st) if st.kind == Kind::File => st, Ok(_) => return refused(r, "That is a folder, not a package."), Err(_) => { r.line("No such package file."); return w::NOT_FOUND } };
    let keys = trusted(fs);
    // The layout first (small reads), then the signature over every byte.
    let layout = match sex_pkg::layout::<Error>(st.size, |off, buf| read_exact(fs, st.id, off, buf)) {
        Ok(l) => l,
        Err(Ok(e)) => return refused(r, e.why()),
        Err(Err(_)) => { r.line("The disk failed while reading the package."); return w::IO }
    };
    let mut v = sex_pkg::Verifier::new();
    let body = st.size - sex_pkg::TRAILER as u64;
    let mut buf = alloc::vec![0u8; CHUNK];
    let mut at = 0u64;
    while at < body {
        let n = ((body - at) as usize).min(CHUNK);
        if read_exact(fs, st.id, at, &mut buf[..n]).is_err() { r.line("The disk failed while reading the package."); return w::IO; }
        v.update(&buf[..n]);
        at += n as u64;
    }
    let mut trailer = [0u8; sex_pkg::TRAILER];
    if read_exact(fs, st.id, body, &mut trailer).is_err() { return w::IO; }
    let key = match v.finish(&trailer, &keys) { Ok(k) => k, Err(e) => return refused(r, e.why()) };
    let m = &layout.manifest;
    let program = m.program.as_str();
    sex_pdx::serial_record!("[files.pkg.verified] name={} version={} program={} grants={} files={} key={}", m.name, m.version, program, m.grant_words(), layout.entries.len(), &sex_pkg::hex(&key)[..16]);
    // What is there now: this package (an upgrade), another one, or a program no package put there.
    let old = read_all(fs, &record(program, "manifest"), sex_pkg::MANIFEST_MAX).ok().and_then(|t| Manifest::parse(core::str::from_utf8(&t).ok()?).ok());
    match &old {
        Some(o) if o.name != m.name => return refused(r, &format!("/bin/{program} belongs to the package {}. Remove it first.", o.name)),
        Some(o) if o.version > m.version && !downgrade => return refused(r, &format!("{} {} is installed, newer than {}. Not downgraded.", o.name, o.version, m.version)),
        None if fs.open(&bin(program)).is_ok() => return refused(r, &format!("/bin/{program} exists and no package installed it. Not replaced.")),
        _ => {}
    }
    let summary = if m.summary.is_empty() { String::new() } else { format!(": {}", m.summary) };
    r.line(&format!("{} {}{} ({} files, publisher {}...)", m.name, m.version, summary, layout.entries.len(), &sex_pkg::hex(&key)[..16]));
    if m.grants != 0 { r.line(&format!("It asks for: {}. You are asked on its first run.", m.grant_words())); }
    if !go { return w::OK; }
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    let result = (|| -> Result<(), Error> {
        let mut files = String::new();
        for e in &layout.entries {
            let dest = match &e.target { Target::Program => bin(program), Target::Data(rel) => { files.push_str(rel); files.push('\n'); data_path(program, rel) } };
            if let Some(slash) = dest.iter().rposition(|b| *b == b'/') { mkdirs(fs, &dest[..slash])?; }
            let id = match fs.open(&dest) { Ok(st) if st.kind == Kind::Dir => return Err(Error::IsDir), Ok(st) => { fs.truncate(st.id, 0, 0)?; st.id } Err(Error::NotFound) => fs.create(&dest, 0)?, Err(e) => return Err(e) };
            let mut done = 0u64;
            while done < e.size {
                let n = ((e.size - done) as usize).min(CHUNK);
                read_exact(fs, st.id, e.offset + done, &mut buf[..n])?;
                let mut put = 0;
                while put < n { put += fs.write(id, done + put as u64, &buf[put..n], 0)?; }
                done += n as u64;
            }
        }
        // Files the old version installed and this one does not.
        if old.is_some() {
            if let Ok(prev) = read_all(fs, &record(program, "files"), 1 << 20) {
                for rel in prev.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    let rel = core::str::from_utf8(rel).unwrap_or("");
                    if sex_pkg::valid_rel(rel) && !files.lines().any(|f| f == rel) { let _ = fs.remove_path(&data_path(program, rel)); }
                }
            }
        }
        write_file(fs, &record(program, "manifest"), m.to_text().as_bytes())?;
        write_file(fs, &record(program, "files"), files.as_bytes())?;
        // An upgrade that asks for more than before is asked about again.
        if let Some(o) = &old { if m.grants & !o.grants != 0 { let _ = fs.remove_path(&record(program, "consent")); } }
        Ok(())
    })();
    match result {
        Ok(()) => {
            sex_pdx::serial_record!("[files.pkg.installed] name={} version={} program={} upgrade={}", m.name, m.version, program, old.is_some() as u8);
            r.line(&match &old { Some(o) => format!("Upgraded {} from {} to {}. Run it with: run {}", m.name, o.version, m.version, program), None => format!("Installed {}. Run it with: run {}", m.name, program) });
            w::OK
        }
        Err(e) => {
            let _ = fs.rollback();
            sex_pdx::serial_record!("[files.pkg.failed] name={} error={:?}", m.name, e);
            r.line(&format!("Not installed: {}. Nothing was changed.", core::str::from_utf8(w::message(crate::fs_service::status(e))).unwrap_or("?")));
            crate::fs_service::status(e)
        }
    }
}

/// The program a name means: a program with a record, or the program of the package by that name.
fn resolve(fs: &mut F, name: &str) -> Option<(String, Manifest)> {
    let parse = |t: Vec<u8>| Manifest::parse(core::str::from_utf8(&t).ok()?).ok();
    if sex_pkg::valid_name(name) {
        if let Some(m) = read_all(fs, &record(name, "manifest"), sex_pkg::MANIFEST_MAX).ok().and_then(parse) { return Some((name.into(), m)); }
    }
    installed(fs).into_iter().find(|(_, m)| m.name == name)
}
fn installed(fs: &mut F) -> Vec<(String, Manifest)> {
    let mut out = Vec::new();
    let Ok(root) = fs.open(p::ROOT) else { return out };
    let (mut cursor, mut name) = (0, [0u8; sexfs::NAME_MAX]);
    let mut names = Vec::new();
    while let Ok(Some((len, kind, _, next))) = fs.read_dir(root.id, cursor, &mut name) {
        if kind == Kind::Dir { if let Ok(s) = core::str::from_utf8(&name[..len]) { names.push(String::from(s)); } }
        cursor = next;
    }
    for program in names {
        if let Some(m) = read_all(fs, &record(&program, "manifest"), sex_pkg::MANIFEST_MAX).ok().and_then(|t| Manifest::parse(core::str::from_utf8(&t).ok()?).ok()) { out.push((program, m)); }
    }
    out
}

fn remove(fs: &mut F, arg: &[u8], r: &mut Report) -> u8 {
    let name = core::str::from_utf8(arg).unwrap_or("");
    let Some((program, m)) = resolve(fs, name) else { r.line(&format!("No installed package {name}.")); return w::NOT_FOUND };
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    let files = read_all(fs, &record(&program, "files"), 1 << 20).unwrap_or_default();
    for rel in files.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        if let Ok(rel) = core::str::from_utf8(rel) { if sex_pkg::valid_rel(rel) { let _ = fs.remove_path(&data_path(&program, rel)); } }
    }
    let _ = fs.remove_path(&bin(&program));
    for leaf in ["manifest", "files", "consent"] { let _ = fs.remove_path(&record(&program, leaf)); }
    let dir = format!("{}/{}", core::str::from_utf8(p::ROOT).unwrap(), program);
    let _ = fs.remove_path(dir.as_bytes());
    sex_pdx::serial_record!("[files.pkg.removed] name={} program={}", m.name, program);
    r.line(&format!("Removed {} {}. Its folder /data/{} keeps what the program saved there.", m.name, m.version, program));
    w::OK
}

fn list(fs: &mut F, r: &mut Report) -> u8 {
    let all = installed(fs);
    if all.is_empty() { r.line("No packages installed."); }
    for (program, m) in &all {
        let note = note(fs, program.as_bytes()).unwrap_or_default();
        let mut grants = String::new();
        for (bit, word) in sex_pkg::GRANTS {
            if m.grants & bit == 0 { continue; }
            let state = if note.answered & bit == 0 { "ask" } else if note.allowed & bit != 0 { "yes" } else { "no" };
            if !grants.is_empty() { grants.push(' '); }
            grants.push_str(&format!("{word}={state}"));
        }
        r.line(&format!("{} {} {}  {}", program, m.name, m.version, if grants.is_empty() { "-".into() } else { grants }));
    }
    sex_pdx::serial_record!("[files.pkg.list] count={}", all.len());
    w::OK
}

fn trust(fs: &mut F, arg: &[u8], r: &mut Report) -> u8 {
    let Some(key) = sex_pkg::unhex::<32>(arg) else { return refused(r, "A publisher key is 64 hex digits.") };
    let mut text = read_all(fs, p::TRUSTED, 64 * 1024).unwrap_or_default();
    if sex_pkg::parse_keys(&text).contains(&key) { r.line("That publisher is already trusted."); return w::OK; }
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    text.extend_from_slice(sex_pkg::hex(&key).as_bytes());
    text.push(b'\n');
    if write_file(fs, p::TRUSTED, &text).is_err() { r.line("The disk failed."); return w::IO; }
    sex_pdx::serial_record!("[files.pkg.trusted] key={}", &sex_pkg::hex(&key)[..16]);
    r.line(&format!("Packages signed by {}... are trusted from now on.", &sex_pkg::hex(&key)[..16]));
    w::OK
}

fn consent(fs: &mut F, arg: &[u8], length: u64, r: &mut Report) -> u8 {
    let Some(n) = note(fs, arg) else { r.line("No package installed that program."); return w::NOT_FOUND };
    let program = core::str::from_utf8(arg).unwrap_or("");
    let (answered, allowed) = ((length >> 16) as u32 & n.asked, length as u32 & 0xFFFF & n.asked);
    let answered_all = n.answered | answered;
    let allowed_all = (n.allowed & !answered) | (allowed & answered);
    let mut text = [0u8; 64];
    let k = p::consent_text(answered_all, allowed_all, &mut text);
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    if write_file(fs, &record(program, "consent"), &text[..k]).is_err() { return w::IO; }
    sex_pdx::serial_record!("[files.pkg.consent] program={} answered={} allowed={}", program, answered_all, allowed_all);
    w::OK
}
