// SPDX-License-Identifier: GPL-3.0-only
//! Packages (`sex_pkg`): Files checks a package's signature against the keys this system
//! trusts, then installs its program into `/bin` and its data into the program's own
//! folder, in one filesystem transaction: a failure part way leaves nothing behind.
//!
//! Trusted keys: the release key this system was built with, and the keys the person
//! added with `pkg trust` (`/system/packages/trusted`). Third-party repositories the person
//! added with `pkg repo add` (`/system/packages/repos.txt`) each bring their own key: it
//! signs that repository's catalog, and its packages, and nothing else's catalog. A
//! package remembers the repository it came from (by the key that signed it), and an
//! upgrade from another repository is refused. Records of what each package installed,
//! the person's answers to its grants and the version an upgrade replaced (for rollback)
//! live in `/system/packages/PROGRAM/`. Spindle, the person's terminal, reaches these
//! operations, and programs the person started with `run +pkg` reach the ones
//! `fs_service::program_package` lets through.
use crate::fs_service::Disk;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sex_pdx::fs as w;
use sex_pdx::package as p;
use sex_pkg::repo::{self, Have, Repo, Source, MAIN};
use sex_pkg::{rollback, Manifest, Target};
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
fn parent_dirs(fs: &mut F, path: &[u8]) -> Result<(), Error> {
    if let Some(slash) = path.iter().rposition(|b| *b == b'/') { if slash > 0 { mkdirs(fs, &path[..slash])?; } }
    Ok(())
}
fn write_file(fs: &mut F, path: &[u8], data: &[u8]) -> Result<(), Error> {
    parent_dirs(fs, path)?;
    let id = match fs.open(path) { Ok(st) if st.kind == Kind::Dir => return Err(Error::IsDir), Ok(st) => { fs.truncate(st.id, 0, 0)?; st.id } Err(Error::NotFound) => fs.create(path, 0)?, Err(e) => return Err(e) };
    let mut at = 0;
    while at < data.len() { at += fs.write(id, at as u64, &data[at..], 0)?; }
    Ok(())
}
/// Removes a folder and everything in it (a package's kept version: no person's files).
fn remove_tree(fs: &mut F, path: &[u8]) -> Result<(), Error> {
    let st = match fs.open(path) { Ok(st) => st, Err(Error::NotFound) => return Ok(()), Err(e) => return Err(e) };
    if st.kind == Kind::Dir {
        let (mut cursor, mut name) = (0, [0u8; sexfs::NAME_MAX]);
        let mut children = Vec::new();
        while let Some((len, _, _, next)) = fs.read_dir(st.id, cursor, &mut name)? {
            let mut child = path.to_vec();
            child.push(b'/');
            child.extend_from_slice(&name[..len]);
            children.push(child);
            cursor = next;
        }
        for c in children { remove_tree(fs, &c)?; }
    }
    fs.remove_path(path)
}
/// Carries out renames planned by `sex_pkg::rollback`.
fn apply_moves(fs: &mut F, moves: &[rollback::Move]) -> Result<(), Error> {
    for m in moves {
        match fs.open(m.from.as_bytes()) {
            Ok(_) => {}
            Err(Error::NotFound) if m.optional => continue,
            Err(e) => return Err(e),
        }
        parent_dirs(fs, m.to.as_bytes())?;
        fs.rename(m.from.as_bytes(), m.to.as_bytes())?;
    }
    Ok(())
}
fn root() -> &'static str { core::str::from_utf8(p::ROOT).unwrap() }
fn record(program: &str, leaf: &str) -> Vec<u8> { format!("{}/{}/{}", root(), program, leaf).into_bytes() }
fn bin(program: &str) -> Vec<u8> { format!("/bin/{program}").into_bytes() }
fn data_path(program: &str, rel: &str) -> Vec<u8> { format!("/data/{program}/{rel}").into_bytes() }
fn manifest_at(fs: &mut F, path: &[u8]) -> Option<Manifest> {
    let t = read_all(fs, path, sex_pkg::MANIFEST_MAX).ok()?;
    Manifest::parse(core::str::from_utf8(&t).ok()?).ok()
}
/// The repository an installed program's package came from.
fn origin_of(fs: &mut F, program: &str, leaf: &str) -> String {
    read_all(fs, &record(program, leaf), 64).ok().and_then(|t| String::from_utf8(t).ok()).map(|s| s.trim().to_string())
        .filter(|s| sex_pkg::valid_name(s)).unwrap_or_else(|| MAIN.into())
}

/// The keys packages may be signed with.
pub fn trusted(fs: &mut F) -> Vec<[u8; 32]> {
    let mut keys = sex_pkg::unhex::<32>(env!("SEXOS_RELEASE_KEY_HEX").as_bytes()).into_iter().collect::<Vec<_>>();
    if let Ok(text) = read_all(fs, p::TRUSTED, 64 * 1024) { keys.extend(sex_pkg::parse_keys(&text)); }
    keys
}
fn repos(fs: &mut F) -> Vec<Repo> { read_all(fs, p::REPOS_FILE, 64 * 1024).map(|t| repo::parse(&t)).unwrap_or_default() }
/// What the installed program's package asks for, and what the person said.
pub fn note(fs: &mut F, program: &[u8]) -> Option<p::Note> {
    let program = core::str::from_utf8(program).ok().filter(|s| sex_pkg::valid_name(s))?;
    let m = manifest_at(fs, &record(program, "manifest"))?;
    let (answered, allowed) = read_all(fs, &record(program, "consent"), 256).map(|c| p::parse_consent(&c)).unwrap_or((0, 0));
    Some(p::Note { asked: m.grants, answered: answered & m.grants, allowed: allowed & answered & m.grants })
}

struct Report<'a> { out: &'a mut [u8], n: usize }
impl Report<'_> {
    fn machine(&mut self, s: &str) {
        if self.n < self.out.len() { self.out[self.n] = p::MACHINE; self.n += 1; }
        self.line(s);
    }
    fn line(&mut self, s: &str) {
        for b in s.bytes().chain(core::iter::once(b'\n')) { if self.n < self.out.len() { self.out[self.n] = b; self.n += 1; } }
    }
}

/// A download's path: at the root for Spindle, in the program's own folder for a program.
fn downloaded(folder: Option<&[u8]>, leaf: &[u8]) -> Vec<u8> {
    let mut path = folder.unwrap_or(b"").to_vec();
    path.extend_from_slice(leaf);
    path
}

/// One package request. Returns the reply status and the report's length. `folder` is the
/// asking program's own folder (`/data/NAME`), or None for Spindle; the caller has
/// already checked the program may ask this and confined its paths.
pub fn handle(fs: &mut F, op: u64, arg: &[u8], length: u64, out: &mut [u8], folder: Option<&[u8]>) -> (u8, u32) {
    let mut r = Report { out, n: 0 };
    let status = match op {
        p::INSTALL | p::CHECK => {
            let st = install(fs, arg, op == p::INSTALL, length & p::ALLOW_DOWNGRADE != 0, &mut r);
            // A download is used once, whatever came of it.
            let download = downloaded(folder, p::DOWNLOAD);
            if arg == &download[..] && op == p::INSTALL {
                if st != w::OK && fs.has_changes() { let _ = fs.rollback(); }
                if fs.remove_path(&download).is_ok() && st != w::OK { let _ = fs.commit(); }
            }
            st
        }
        p::REMOVE => remove(fs, arg, &mut r),
        p::LIST => list(fs, &mut r),
        p::TRUST => trust(fs, arg, &mut r),
        p::CONSENT => consent(fs, arg, length, &mut r),
        p::CATALOG => catalog(fs, arg, folder, &mut r),
        p::SEARCH => search(fs, arg, &mut r),
        p::LOCATE => locate(fs, arg, &mut r),
        p::SOURCE => source(fs, &mut r),
        p::REPO_ADD => repo_add(fs, arg, &mut r),
        p::REPO_REMOVE => repo_remove(fs, arg, &mut r),
        p::REPOS => repo_list(fs, &mut r),
        p::ROLLBACK => roll_back(fs, arg, &mut r),
        p::DETAILS => details(fs, arg, &mut r),
        _ => w::INVALID,
    };
    // A change is all or nothing: commit it, or go back to before it.
    if status == w::OK && matches!(op, p::INSTALL | p::REMOVE | p::TRUST | p::CONSENT | p::CATALOG | p::REPO_ADD | p::REPO_REMOVE | p::ROLLBACK) {
        if let Err(e) = fs.commit() { let _ = fs.rollback(); r.line(&format!("The disk failed while saving ({e:?}). Nothing changed.")); return (w::IO, r.n as u32); }
    } else if fs.has_changes() { let _ = fs.rollback(); }
    (status, r.n as u32)
}

fn refused(r: &mut Report, why: &str) -> u8 { sex_pdx::serial_record!("[files.pkg.refused] {}", why); r.line(why); w::DENIED }

fn install(fs: &mut F, path: &[u8], go: bool, downgrade: bool, r: &mut Report) -> u8 {
    let st = match fs.open(path) { Ok(st) if st.kind == Kind::File => st, Ok(_) => return refused(r, "That is a folder, not a package."), Err(_) => { r.line("No such package file."); return w::NOT_FOUND } };
    let trusted = trusted(fs);
    let repos = repos(fs);
    let keys = repo::all_keys(&repos, &trusted);
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
    let from = repo::origin(&key, &repos, &trusted).unwrap_or_else(|| MAIN.into());
    let m = &layout.manifest;
    let program = m.program.as_str();
    sex_pdx::serial_record!("[files.pkg.verified] name={} version={} program={} grants={} files={} key={} repo={}", m.name, m.version, program, m.grant_words(), layout.entries.len(), &sex_pkg::hex(&key)[..16], from);
    // What is there now: this package (an upgrade), another one, or a program no package put there.
    let old = manifest_at(fs, &record(program, "manifest"));
    let old_repo = origin_of(fs, program, "repo");
    match &old {
        Some(o) if o.name != m.name => return refused(r, &format!("/bin/{program} belongs to the package {}. Remove it first.", o.name)),
        Some(o) if !repo::may_replace(&old_repo, &from) => return refused(r, &format!("{} came from the repository {}; this one is from {}. Remove it first.", o.name, old_repo, from)),
        Some(o) if o.version > m.version && !downgrade => return refused(r, &format!("{} {} is installed, newer than {}. Not downgraded.", o.name, o.version, m.version)),
        None if fs.open(&bin(program)).is_ok() => return refused(r, &format!("/bin/{program} exists and no package installed it. Not replaced.")),
        _ => {}
    }
    let summary = if m.summary.is_empty() { String::new() } else { format!(": {}", m.summary) };
    let shown_from = if from == MAIN { String::new() } else { format!(", from {from}") };
    r.line(&format!("{} {}{} ({} files, publisher {}...{})", m.name, m.version, summary, layout.entries.len(), &sex_pkg::hex(&key)[..16], shown_from));
    if m.grants != 0 { r.line(&format!("It asks for: {}. You are asked on its first run.", m.grant_words())); }
    if !go { return w::OK; }
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    let keep = old.as_ref().is_some_and(|o| rollback::keeps_previous(o.version, m.version));
    let result = (|| -> Result<(), Error> {
        let prev_files = read_all(fs, &record(program, "files"), 1 << 20).unwrap_or_default();
        if keep {
            // The version being replaced is kept whole for `pkg rollback`; the one kept before goes.
            remove_tree(fs, &record(program, rollback::PREV))?;
            apply_moves(fs, &rollback::keep(root(), program, &prev_files))?;
        }
        let mut files = String::new();
        for e in &layout.entries {
            let dest = match &e.target { Target::Program => bin(program), Target::Data(rel) => { files.push_str(rel); files.push('\n'); data_path(program, rel) } };
            parent_dirs(fs, &dest)?;
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
        // A reinstall of the same version: files it no longer has go (an upgrade moved them).
        if old.is_some() && !keep {
            for rel in rollback::data_files(&prev_files) {
                if !files.lines().any(|f| f == rel) { let _ = fs.remove_path(&data_path(program, rel)); }
            }
        }
        write_file(fs, &record(program, "manifest"), m.to_text().as_bytes())?;
        write_file(fs, &record(program, "files"), files.as_bytes())?;
        if from == MAIN { let _ = fs.remove_path(&record(program, "repo")); } else { write_file(fs, &record(program, "repo"), from.as_bytes())?; }
        // An upgrade that asks for more than before is asked about again.
        if let Some(o) = &old { if !rollback::consent_kept(o.grants, m.grants) { let _ = fs.remove_path(&record(program, "consent")); } }
        Ok(())
    })();
    match result {
        Ok(()) => {
            sex_pdx::serial_record!("[files.pkg.installed] name={} version={} program={} upgrade={} repo={} kept={}", m.name, m.version, program, old.is_some() as u8, from, old.as_ref().filter(|_| keep).map(|o| o.version.to_string()).unwrap_or_else(|| "-".into()));
            r.line(&match &old { Some(o) => format!("Upgraded {} from {} to {}. Run it with: run {}", m.name, o.version, m.version, program), None => format!("Installed {}. Run it with: run {}", m.name, program) });
            if let Some(o) = old.as_ref().filter(|_| keep) { r.line(&format!("{} {} is kept: pkg rollback {} puts it back.", o.name, o.version, m.name)); }
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
    if sex_pkg::valid_name(name) {
        if let Some(m) = manifest_at(fs, &record(name, "manifest")) { return Some((name.into(), m)); }
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
        if let Some(m) = manifest_at(fs, &record(&program, "manifest")) { out.push((program, m)); }
    }
    out
}
/// Installed packages with the repository each came from.
fn have(fs: &mut F) -> Vec<(String, Have)> {
    installed(fs).into_iter().map(|(program, m)| { let repo = origin_of(fs, &program, "repo"); (program, Have { name: m.name, version: m.version, repo }) }).collect()
}

fn remove(fs: &mut F, arg: &[u8], r: &mut Report) -> u8 {
    let name = core::str::from_utf8(arg).unwrap_or("");
    let Some((program, m)) = resolve(fs, name) else { r.line(&format!("No installed package {name}.")); return w::NOT_FOUND };
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    let files = read_all(fs, &record(&program, "files"), 1 << 20).unwrap_or_default();
    for rel in rollback::data_files(&files) { let _ = fs.remove_path(&data_path(&program, rel)); }
    let _ = fs.remove_path(&bin(&program));
    let _ = remove_tree(fs, &record(&program, rollback::PREV));
    let _ = remove_tree(fs, &record(&program, rollback::SWAP));
    for leaf in ["manifest", "files", "consent", "repo"] { let _ = fs.remove_path(&record(&program, leaf)); }
    let dir = format!("{}/{}", root(), program);
    let _ = fs.remove_path(dir.as_bytes());
    sex_pdx::serial_record!("[files.pkg.removed] name={} program={}", m.name, program);
    r.line(&format!("Removed {} {}. Its folder /data/{} keeps what the program saved there.", m.name, m.version, program));
    w::OK
}

/// `pkg rollback NAME`: the kept version goes back in, the current one is kept instead.
fn roll_back(fs: &mut F, arg: &[u8], r: &mut Report) -> u8 {
    let name = core::str::from_utf8(arg).unwrap_or("").trim();
    let Some((program, cur)) = resolve(fs, name) else { r.line(&format!("No installed package {name}.")); return w::NOT_FOUND };
    let prev_dir = format!("{}/{}/{}/", root(), program, rollback::PREV);
    let Some(prev) = manifest_at(fs, format!("{prev_dir}manifest").as_bytes()) else { r.line(&format!("No earlier version of {} is kept. One is kept when it is upgraded.", cur.name)); return w::NOT_FOUND };
    if prev.name != cur.name || prev.program != cur.program { return refused(r, "The kept version is another package's. Not rolled back."); }
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    let result = (|| -> Result<(), Error> {
        let files = read_all(fs, &record(&program, "files"), 1 << 20).unwrap_or_default();
        let prev_files = read_all(fs, format!("{prev_dir}files").as_bytes(), 1 << 20).unwrap_or_default();
        remove_tree(fs, &record(&program, rollback::SWAP))?;
        apply_moves(fs, &rollback::swap(root(), &program, &files, &prev_files))?;
        remove_tree(fs, &record(&program, rollback::SWAP))?;
        if !rollback::consent_kept(cur.grants, prev.grants) { let _ = fs.remove_path(&record(&program, "consent")); }
        Ok(())
    })();
    match result {
        Ok(()) => {
            let repo = origin_of(fs, &program, "repo");
            sex_pdx::serial_record!("[files.pkg.rollback] name={} program={} from={} to={} repo={}", cur.name, program, cur.version, prev.version, repo);
            r.line(&format!("Rolled {} back from {} to {}. {} is kept: pkg rollback {} again goes forward.", cur.name, cur.version, prev.version, cur.version, cur.name));
            w::OK
        }
        Err(e) => {
            let _ = fs.rollback();
            sex_pdx::serial_record!("[files.pkg.failed] name={} rollback error={:?}", cur.name, e);
            r.line("Not rolled back: the disk refused a step. Nothing was changed.");
            crate::fs_service::status(e)
        }
    }
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
        let repo = origin_of(fs, program, "repo");
        let prev = manifest_at(fs, &record(program, "prev/manifest")).map(|p| p.version.to_string());
        let mut extra = String::new();
        if repo != MAIN { extra.push_str(&format!("  from {repo}")); }
        if let Some(v) = &prev { extra.push_str(&format!("  (rollback to {v})")); }
        r.line(&format!("{} {} {}  {}{}", program, m.name, m.version, if grants.is_empty() { "-".into() } else { grants }, extra));
        sex_pdx::serial_record!("[files.pkg.listed] program={} name={} version={} repo={} previous={}", program, m.name, m.version, repo, prev.as_deref().unwrap_or("-"));
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

fn store_file(leaf: &str) -> Vec<u8> { format!("{}/{}", root(), leaf).into_bytes() }
/// Where a repository's checked catalog is kept: `catalog.txt` for the main store (as
/// before repositories), `catalog.NAME.txt` for the others.
fn catalog_file(repo: &str) -> Vec<u8> { if repo == MAIN { store_file("catalog.txt") } else { store_file(&format!("catalog.{repo}.txt")) } }
fn kept_catalog(fs: &mut F, repo: &str) -> Option<Vec<sex_pkg::index::Item>> {
    let text = read_all(fs, &catalog_file(repo), sex_pkg::index::MAX_BYTES).ok()?;
    sex_pkg::index::parse(core::str::from_utf8(&text).ok()?).ok()
}
fn main_source(fs: &mut F) -> Option<String> { read_all(fs, &store_file("source"), 1024).ok().and_then(|u| String::from_utf8(u).ok()) }
/// Every kept catalog: the main store's first, then each repository's in list order.
fn sources(fs: &mut F) -> Vec<Source> {
    let mut out = Vec::new();
    if let (Some(url), Some(items)) = (main_source(fs), kept_catalog(fs, MAIN)) { out.push(Source { repo: MAIN.into(), url, items }); }
    for rp in repos(fs) {
        if let Some(items) = kept_catalog(fs, &rp.name) { out.push(Source { repo: rp.name, url: rp.url, items }); }
    }
    out
}
fn updates(fs: &mut F) -> usize {
    let s = sources(fs);
    let h: Vec<Have> = have(fs).into_iter().map(|x| x.1).collect();
    repo::upgrades(&s, &h).len()
}

fn source(fs: &mut F, r: &mut Report) -> u8 {
    let main = main_source(fs);
    let list = repos(fs);
    if main.is_none() && list.is_empty() { r.line("No store set. Use: pkg refresh URL"); return w::NOT_FOUND; }
    if let Some(u) = main { r.machine(&u); }
    for rp in &list { r.machine(&format!("{} {}", rp.url, rp.name)); }
    w::OK
}

/// A downloaded catalog. `arg` is the main store's folder URL, or `URL NAME` for a repository.
fn catalog(fs: &mut F, arg: &[u8], folder: Option<&[u8]>, r: &mut Report) -> u8 {
    let (text_at, sig_at) = (downloaded(folder, p::CATALOG_DOWNLOAD), downloaded(folder, p::SIG_DOWNLOAD));
    let text = read_all(fs, &text_at, sex_pkg::index::MAX_BYTES);
    let sig = read_all(fs, &sig_at, 512);
    let _ = fs.remove_path(&text_at);
    let _ = fs.remove_path(&sig_at);
    let (Ok(text), Ok(sig)) = (text, sig) else { r.line("The catalog did not download."); let _ = fs.commit(); return w::NOT_FOUND };
    let arg = core::str::from_utf8(arg).unwrap_or("");
    let (url, name) = match arg.split_once(' ') { Some((u, n)) => (u, n.trim()), None => (arg, MAIN) };
    let list = repos(fs);
    let trusted = trusted(fs);
    if name != MAIN && !list.iter().any(|rp| rp.name == name && rp.url == url) { let _ = fs.commit(); return refused(r, "No repository by that name and URL. Add it first with: pkg repo add NAME URL KEY"); }
    // The main store's catalog is checked against the trusted keys, a repository's only against its own.
    let keys = repo::keys_for(name, &list, &trusted);
    let items = match sex_pkg::index::verify(&text, sig.trim_ascii(), &keys) { Ok(i) => i, Err(e) => { let _ = fs.commit(); return refused(r, e.why()) } };
    if !repo::valid_url(url) { return refused(r, "A store is a folder URL ending in /."); }
    if write_file(fs, &catalog_file(name), &text).is_err() { r.line("The disk failed."); return w::IO; }
    if name == MAIN {
        if write_file(fs, &store_file("source"), url.as_bytes()).is_err() { r.line("The disk failed."); return w::IO; }
        sex_pdx::serial_record!("[files.pkg.catalog] packages={}", items.len());
        r.line(&format!("Store catalog: {} packages. Find some with: pkg search WORDS", items.len()));
    } else {
        sex_pdx::serial_record!("[files.pkg.repo.catalog] repo={} packages={}", name, items.len());
        r.line(&format!("Repository {}: {} packages.", name, items.len()));
    }
    let n = updates(fs);
    sex_pdx::serial_record!("[files.pkg.updates] count={}", n);
    if n > 0 { r.line(&format!("{n} updates available. Get them with: pkg upgrade")); }
    w::OK
}

fn search(fs: &mut F, query: &[u8], r: &mut Report) -> u8 {
    let all = sources(fs);
    if all.is_empty() { r.line("No store catalog yet. Get one with: pkg refresh URL"); return w::NOT_FOUND; }
    let have = have(fs);
    let q = core::str::from_utf8(query).unwrap_or("");
    let mut hits = 0;
    for s in &all {
        for i in sex_pkg::index::search(&s.items, q) {
            let ours = have.iter().find(|(_, h)| h.name == i.name && h.repo == s.repo).map(|(_, h)| h.version);
            let mark = match ours { Some(v) if v < i.version => format!("  (installed {}, newer here)", v), Some(_) => "  (installed)".into(), None => String::new() };
            let grants = if i.grants == 0 { String::new() } else { format!("  asks for: {}", sex_pkg::index::grant_words(i.grants)) };
            let name = if s.repo == MAIN { i.name.clone() } else { format!("{}/{}", s.repo, i.name) };
            r.line(&format!("{} {}  {}{}{}", name, i.version, i.summary, grants, mark));
            hits += 1;
        }
    }
    if hits == 0 { r.line("Nothing matches."); }
    sex_pdx::serial_record!("[files.pkg.search] hits={}", hits);
    w::OK
}

/// What to download: one package (`NAME` or `REPO/NAME`), or every update (empty).
fn locate(fs: &mut F, name: &[u8], r: &mut Report) -> u8 {
    let all = sources(fs);
    if all.is_empty() { r.line("No store catalog yet. Get one with: pkg refresh URL"); return w::NOT_FOUND; }
    let name = core::str::from_utf8(name).unwrap_or("").trim();
    let have: Vec<Have> = have(fs).into_iter().map(|x| x.1).collect();
    let picks = if name.is_empty() { repo::upgrades(&all, &have) } else { repo::select(&all, &have, name).into_iter().collect() };
    for pk in &picks {
        let from = if pk.repo == MAIN { String::new() } else { format!(" from {}", pk.repo) };
        r.line(&format!("Getting {} {}{} ({} KiB)...", pk.item.name, pk.item.version, from, pk.item.bytes.div_ceil(1024)));
        r.machine(&format!("{}{}", pk.url, pk.item.file));
    }
    sex_pdx::serial_record!("[files.pkg.locate] name={} found={}", name, picks.len());
    if picks.is_empty() { r.line(if name.is_empty() { "Everything is up to date." } else { "The store has no package by that name." }); return if name.is_empty() { w::OK } else { w::NOT_FOUND }; }
    w::OK
}

fn repo_add(fs: &mut F, arg: &[u8], r: &mut Report) -> u8 {
    let new = match repo::parse_add(core::str::from_utf8(arg).unwrap_or("")) { Ok(rp) => rp, Err(why) => return refused(r, why) };
    let list = match repo::add(&repos(fs), new.clone()) { Ok(l) => l, Err(why) => return refused(r, why) };
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    if write_file(fs, p::REPOS_FILE, repo::text(&list).as_bytes()).is_err() { r.line("The disk failed."); return w::IO; }
    sex_pdx::serial_record!("[files.pkg.repo] added name={} key={}", new.name, &sex_pkg::hex(&new.key)[..16]);
    r.line(&format!("Repository {} added: its catalog and packages count when signed by {}... Get its catalog with: pkg refresh", new.name, &sex_pkg::hex(&new.key)[..16]));
    w::OK
}
fn repo_remove(fs: &mut F, arg: &[u8], r: &mut Report) -> u8 {
    let name = core::str::from_utf8(arg).unwrap_or("").trim();
    let Some(list) = repo::remove(&repos(fs), name) else { r.line(&format!("No repository {name}.")); return w::NOT_FOUND };
    if crate::fs_service::writes_pending() { if fs.commit().is_err() { let _ = fs.rollback(); return w::IO; } }
    if write_file(fs, p::REPOS_FILE, repo::text(&list).as_bytes()).is_err() { r.line("The disk failed."); return w::IO; }
    let _ = fs.remove_path(&catalog_file(name));
    sex_pdx::serial_record!("[files.pkg.repo] removed name={}", name);
    r.line(&format!("Repository {name} removed. Packages installed from it stay, without updates."));
    w::OK
}
fn repo_list(fs: &mut F, r: &mut Report) -> u8 {
    let main = main_source(fs);
    let main_n = kept_catalog(fs, MAIN).map_or(0, |i| i.len());
    match &main { Some(u) => r.line(&format!("{MAIN} {u} trusted-keys packages={main_n}")), None => r.line("main (none: pkg refresh URL sets it)") }
    let list = repos(fs);
    for rp in &list {
        let n = kept_catalog(fs, &rp.name).map_or(0, |i| i.len());
        r.line(&format!("{} {} {}... packages={}", rp.name, rp.url, &sex_pkg::hex(&rp.key)[..16], n));
        sex_pdx::serial_record!("[files.pkg.repo.listed] name={} packages={}", rp.name, n);
    }
    let n = updates(fs);
    r.line(&format!("{n} updates available."));
    sex_pdx::serial_record!("[files.pkg.repos] count={} updates={}", list.len(), n);
    w::OK
}
/// The store window's view of every matching entry, one Listing line each.
fn details(fs: &mut F, query: &[u8], r: &mut Report) -> u8 {
    let all = sources(fs);
    if all.is_empty() { r.line("No store catalog yet. Get one with: pkg refresh URL"); return w::NOT_FOUND; }
    let have = have(fs);
    let q = core::str::from_utf8(query).unwrap_or("");
    let mut n = 0;
    for s in &all {
        for i in sex_pkg::index::search(&s.items, q) {
            let ours = have.iter().find(|(_, h)| h.name == i.name && h.repo == s.repo);
            let previous = ours.and_then(|(program, _)| manifest_at(fs, &record(program, "prev/manifest"))).map(|m| m.version);
            let installed = ours.map(|(_, h)| h.version);
            let l = sex_pkg::index::Listing { repo: s.repo.clone(), base: s.url.clone(), installed, previous, newer: installed.is_some_and(|v| v < i.version), item: i.clone() };
            r.machine(&l.line());
            n += 1;
        }
    }
    let up = updates(fs);
    sex_pdx::serial_record!("[files.pkg.details] count={} updates={}", n, up);
    w::OK
}
