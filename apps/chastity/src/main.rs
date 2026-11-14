// SPDX-License-Identifier: GPL-3.0-only
//! Chastity, the SexOS password vault.
//!
//! `run chastity` opens the vault in the program's folder (`/data/chastity/vault.kdbx`),
//! or makes one. The file is KeePass KDBX 4, so the same vault opens in KeePassXC and
//! KeePassXC's open here (with a master password; key files are not offered yet).
//!
//! Unlocked, the vault (entries and master password) lives only in this run's memory,
//! behind `chastity_core::keeper` (the seam for Shibari, the RAM-only secret keeper to
//! come). Locking (`l`), five idle minutes, or closing the window zeroizes it.
//!
//! `run chastity probe` checks the vault code on this machine (a KDBX4 file written,
//! read back and refused with the wrong password; TOTP; the generator; entropy) and
//! prints `[chastity.probe.ok] passed=N/N`.
//!
//! Keys: type and Enter; in the list, Up/Down, Enter opens, `a` adds, `/` searches,
//! `c` copies the password, `u` the user name, Delete deletes, `l` locks; in an entry,
//! `r` reveals the password and `t` copies the TOTP code.
mod probe;
mod ui;

#[cfg(target_os = "hermit")]
mod sexos;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("probe") {
        let (passed, total) = probe::run(&mut |line| record(line));
        println!("Chastity probe: {passed} of {total} checks passed.");
        record(&format!("[chastity.probe.{}] passed={passed}/{total}", if passed == total { "ok" } else { "fail" }));
        std::process::exit(if passed == total { 0 } else { 1 });
    }
    #[cfg(target_os = "hermit")]
    sexos::main();
    #[cfg(not(target_os = "hermit"))]
    eprintln!("Chastity runs on SexOS; on this host use `cargo test -p chastity` or `chastity probe`.");
}

fn record(line: &str) {
    #[cfg(target_os = "hermit")]
    sex_std::record(line);
    #[cfg(not(target_os = "hermit"))]
    eprintln!("{line}");
}
