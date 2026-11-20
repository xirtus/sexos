// SPDX-License-Identifier: GPL-3.0-only
//! The Updates and Storage pages, and About's version and image hash: Files' `key=value`
//! lines (`sex_pdx::facts`) as text for people. What Files left out is said to be unknown,
//! never guessed. Host-tested (`scripts/settings_host_gate.sh`).

/// The value after `key=` on its first line, if Files sent one.
pub fn field<'a>(data: &'a [u8], key: &'a str) -> Option<&'a [u8]> { all(data, key).next() }
/// Every line's value for `key` (`volume=`, `package=`).
pub fn all<'a>(data: &'a [u8], key: &'a str) -> impl Iterator<Item = &'a [u8]> + 'a {
    data.split(|b| *b == b'\n').filter_map(move |l| l.strip_prefix(key.as_bytes())?.strip_prefix(b"="))
}
fn num(v: &[u8]) -> Option<u64> { core::str::from_utf8(v).ok()?.parse().ok() }
fn text(v: &[u8]) -> &str { core::str::from_utf8(v).unwrap_or("?") }

struct Out<'a> { buf: &'a mut [u8], at: usize }
impl Out<'_> {
    fn put(&mut self, s: &str) { for b in s.bytes() { if self.at < self.buf.len() { self.buf[self.at] = b; self.at += 1; } } }
    fn num(&mut self, mut v: u64) {
        let mut d = [0u8; 20];
        let mut i = d.len();
        loop { i -= 1; d[i] = b'0' + (v % 10) as u8; v /= 10; if v == 0 { break; } }
        self.put(core::str::from_utf8(&d[i..]).unwrap_or("?"));
    }
    /// Bytes as MiB, or GiB with one decimal from 10 GiB up.
    fn size(&mut self, bytes: u64) {
        let mib = bytes >> 20;
        if mib < 10 * 1024 { self.num(mib); self.put(" MiB"); return; }
        let tenths = mib * 10 / 1024;
        self.num(tenths / 10); self.put("."); self.num(tenths % 10); self.put(" GiB");
    }
}

/// The Updates page's lines; returns their length.
pub fn updates(data: &[u8], out: &mut [u8]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    match field(data, "slot") {
        None => w.put("  Files did not say which system slot is running.\n"),
        Some(b"none") => w.put("  This system started from a USB stick or CD, not from an installed SexOS: it has no system slots.\n"),
        Some(s) => {
            w.put("  Running from system slot "); w.put(if s == b"a" { "A" } else { "B" }); w.put(".\n");
            if let Some(v) = field(data, "version").and_then(num) {
                w.put("  Version: "); w.num(v);
                w.put(if v == 0 { " (as installed; no update has passed its trial yet)\n" } else { " (the last update that passed its trial)\n" });
            }
            let candidate = field(data, "candidate").and_then(num).unwrap_or(0);
            let on_trial = |w: &mut Out, what: &str| { w.put("  Version "); w.num(candidate); w.put(what); };
            match field(data, "trial") {
                Some(b"pending") => on_trial(&mut w, " waits for its try: the next start runs it.\n"),
                Some(b"started") => on_trial(&mut w, " is on trial: it stays once the desktop answers; a failed start goes back to the previous system.\n"),
                Some(b"failed") => on_trial(&mut w, " did not reach the desktop: the previous system stayed the default.\n"),
                _ => w.put("  No update on trial.\n"),
            }
            w.put(if field(data, "other") == Some(b"1") {
                "  The other slot holds the previous system: pick \"SexOS previous system\" in the boot menu to start it once.\n"
            } else { "  The other slot holds no system yet.\n" });
        }
    }
    w.put("  Channel: none. A release is signed files: Spindle's update command checks them and writes the other slot.\n");
    if field(data, "release_key") == Some(b"0") { w.put("  This build has no release key, so it takes no system updates.\n"); }
    match field(data, "packages").and_then(num) {
        None => w.put("  Packages: no store catalog yet (pkg refresh in Spindle gets one).\n"),
        Some(0) => w.put("  Packages: up to date.\n"),
        Some(n) => {
            w.put("  Packages: "); w.num(n); w.put(if n == 1 { " update" } else { " updates" }); w.put(" in the store (pkg upgrade in Spindle):\n");
            for p in all(data, "package") { w.put("    "); w.put(text(p)); w.put("\n"); }
        }
    }
    w.at
}

/// The Storage page's lines; returns their length.
pub fn storage(data: &[u8], out: &mut [u8]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    let words = |key| field(data, key).map(|v| v.split(|b| *b == b' '));
    if let Some(mut d) = words("disk") {
        let sectors = d.next().and_then(num).unwrap_or(0);
        w.put("  System disk: "); w.size(sectors * 512);
        w.put(match d.next() {
            Some(b"whole") => ", SexOS's alone.\n",
            Some(b"store") => ", shared with other systems: SexOS uses its own partition.\n",
            Some(b"foreign") => ", another system's disk: SexOS only reads it.\n",
            Some(b"locked") => ": the installer changed it, so nothing is written until restart.\n",
            _ => ".\n",
        });
    }
    match words("filesystem") {
        Some(mut f) => {
            let _ = f.next();
            let (total, free) = (f.next().and_then(num).unwrap_or(0), f.next().and_then(num).unwrap_or(0));
            w.put("  SexOS filesystem: "); w.size(total * 4096); w.put(", "); w.size(free * 4096); w.put(" free.\n");
        }
        None => w.put("  No SexOS filesystem on this disk.\n"),
    }
    match field(data, "home") {
        Some(b"unlocked") => w.put("  Home folder: encrypted (AES-XTS), unlocked.\n"),
        Some(b"locked") => w.put("  Home folder: encrypted (AES-XTS), locked.\n"),
        Some(_) => w.put("  Home folder: not encrypted.\n"),
        None => {}
    }
    let mut volumes = 0;
    for v in all(data, "volume") {
        if volumes == 0 { w.put("  Volumes under /media:\n"); }
        volumes += 1;
        let mut f = v.splitn(6, |b| *b == b' ');
        let (format, sectors, free, source, ro, name) = (f.next().unwrap_or(b"?"), f.next().and_then(num).unwrap_or(0), f.next().and_then(num), f.next().unwrap_or(b""), f.next(), f.next().unwrap_or(b"?"));
        w.put("    "); w.put(text(name)); w.put(": "); w.put(text(format)); w.put(", "); w.size(sectors * 512);
        if let Some(free) = free { w.put(", "); w.size(free * 4096); w.put(" free"); }
        w.put(match source { b"usb" => ", USB", b"image" => ", disk image", b"disk" => ", on the system disk", _ => "" });
        if ro == Some(b"1") { w.put(", read only"); }
        w.put("\n");
    }
    if volumes == 0 { w.put("  No other volumes mounted.\n"); }
    if field(data, "health") == Some(b"none") || field(data, "health").is_none() {
        w.put("  Disk health (NVMe SMART): no driver reads it yet.\n");
    }
    w.at
}

/// About's lines from Files: the system's version and the boot image's hash.
pub fn about(data: &[u8], out: &mut [u8]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    match (field(data, "slot"), field(data, "version").and_then(num)) {
        (Some(b"none"), _) => w.put("  System version: none (started from a USB stick or CD).\n"),
        (Some(s), Some(v)) => { w.put("  System version: "); w.num(v); w.put(", slot "); w.put(if s == b"a" { "A" } else { "B" }); w.put("\n"); }
        _ => w.put("  System version: Files did not say.\n"),
    }
    match (field(data, "image"), field(data, "image_bytes").and_then(num)) {
        (Some(h), Some(n)) => {
            w.put("  Boot image: "); w.size(n);
            match h.strip_prefix(b"hashing ") {
                Some(done) => { w.put(", SHA-256 being worked out ("); w.put(text(done)); w.put(")\n"); }
                None => { w.put(", SHA-256 "); w.put(text(h)); w.put("\n"); }
            }
        }
        _ => w.put("  Boot image hash: Files did not say.\n"),
    }
    w.at
}
/// Files is still hashing the boot image: ask again soon.
pub fn hashing(data: &[u8]) -> bool { field(data, "image").is_some_and(|v| v.starts_with(b"hashing")) }

#[cfg(test)]
mod tests {
    use super::*;
    fn show(f: fn(&[u8], &mut [u8]) -> usize, data: &str) -> String {
        let mut buf = [0u8; 2048];
        let n = f(data.as_bytes(), &mut buf);
        String::from_utf8(buf[..n].to_vec()).unwrap()
    }

    #[test]
    fn updates_show_the_slot_trial_and_packages() {
        let t = show(updates, "slot=b\nversion=7\ncandidate=8\ntrial=started\nother=1\nrelease_key=1\npackages=2\npackage=abacus 1.2.0\npackage=mesh 0.3.1\n");
        assert!(t.contains("system slot B.\n") && t.contains("Version: 7 (the last") && t.contains("Version 8 is on trial"), "{t}");
        assert!(t.contains("SexOS previous system") && t.contains("2 updates in the store") && t.contains("    mesh 0.3.1\n"), "{t}");
        assert!(!t.contains("no release key"), "{t}");
        let t = show(updates, "slot=none\nrelease_key=0\n");
        assert!(t.contains("no system slots") && t.contains("no release key") && t.contains("no store catalog"), "{t}");
        assert_eq!(field(b"slot=a\nversion=0\n", "version"), Some(&b"0"[..]));
        assert_eq!(field(b"versions=3\n", "version"), None);
    }

    #[test]
    fn storage_says_what_files_knows_and_no_more() {
        let t = show(storage, "disk=1000215216 store\nfilesystem=sexfs 2621440 2097152\nhome=locked\nvolume=fat32 4194304 100 usb 0 MY STICK\nvolume=ntfs 209715200 - disk 1 Windows\nhealth=none\n");
        assert!(t.contains("System disk: 476.9 GiB, shared") && t.contains("SexOS filesystem: 10.0 GiB, 8192 MiB free."), "{t}");
        assert!(t.contains("encrypted (AES-XTS), locked") && t.contains("    MY STICK: fat32, 2048 MiB, 0 MiB free, USB\n"), "{t}");
        assert!(t.contains("    Windows: ntfs, 100.0 GiB, on the system disk, read only\n") && t.contains("SMART): no driver"), "{t}");
        let t = show(storage, "disk=0 unknown\nhealth=none\n");
        assert!(t.contains("No SexOS filesystem") && t.contains("No other volumes") && !t.contains("Home folder"), "{t}");
    }

    #[test]
    fn about_has_the_version_and_image_hash() {
        let done = "slot=a\nversion=3\nimage_bytes=33554432\nimage=ab12\n";
        let t = show(about, done);
        assert!(t.contains("System version: 3, slot A\n") && t.contains("Boot image: 32 MiB, SHA-256 ab12\n") && !hashing(done.as_bytes()), "{t}");
        let part = "slot=none\nimage_bytes=33554432\nimage=hashing 37%\n";
        let t = show(about, part);
        assert!(t.contains("version: none") && t.contains("Boot image: 32 MiB, SHA-256 being worked out (37%)\n") && hashing(part.as_bytes()), "{t}");
        let t = show(about, "");
        assert!(t.contains("Files did not say.\n  Boot image hash: Files did not say."), "{t}");
        let mut small = [0u8; 10];
        assert_eq!(about(b"slot=a\nversion=3\n", &mut small), 10, "cut, never overrun");
    }
}
