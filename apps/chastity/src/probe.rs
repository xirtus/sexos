// SPDX-License-Identifier: GPL-3.0-only
//! `chastity probe`: the vault's code checked on the machine it runs on.
use chastity_core::generator::{self, Options};
use chastity_core::totp::{Algorithm, Totp};
use chastity_core::vault::{Kdf, NewEntry, OpenError, Vault};
use std::time::Instant;
use zeroize::Zeroizing;

pub fn run(record: &mut dyn FnMut(&str)) -> (usize, usize) {
    let mut checks: Vec<(&str, bool)> = Vec::new();
    let mut check = |name: &'static str, ok: bool, record: &mut dyn FnMut(&str)| {
        if !ok { println!("FAIL {name}"); record(&format!("[chastity.probe.fail] check={name}")); }
        checks.push((name, ok));
    };

    let totp = Totp::new(b"12345678901234567890".to_vec(), Algorithm::Sha1, 8, 30).unwrap();
    check("totp_rfc6238", totp.code(59) == "94287082" && totp.code(1111111109) == "07081804", record);
    let a = generator::generate(&Options::default());
    let b = generator::generate(&Options::default());
    check("generator_entropy", matches!((&a, &b), (Ok(a), Ok(b)) if a.len() == 20 && a != b), record);

    // The real cost new vaults use, so the record says how long an unlock takes here.
    let started = Instant::now();
    let mut v = Vault::create("probe", &mut "probe password".to_string(), Kdf::STANDARD);
    v.add(NewEntry { title: "Probe".into(), username: "me".into(), password: Zeroizing::new("pw".into()), url: "https://example.org".into(), ..Default::default() });
    let saved = v.save();
    let save_ms = started.elapsed().as_millis();
    check("kdbx4_save", saved.is_ok(), record);
    let bytes = saved.unwrap_or_default();
    let started = Instant::now();
    let reopened = Vault::open(&bytes, &mut "probe password".to_string());
    let open_ms = started.elapsed().as_millis();
    check("kdbx4_reopen", reopened.as_ref().is_ok_and(|r| r.entries().len() == 1 && r.entries()[0].title == "Probe"), record);
    check("kdbx4_wrong_password", matches!(Vault::open(&bytes, &mut "probe passworD".to_string()), Err(OpenError::WrongPassword)), record);
    record(&format!("[chastity.probe.kdf] memory_mib={} iterations={} save_ms={save_ms} open_ms={open_ms} bytes={}", Kdf::STANDARD.memory_bytes >> 20, Kdf::STANDARD.iterations, bytes.len()));

    // The program's folder takes a file and gives it back (no vault is touched).
    let file_ok = std::fs::write("probe.kdbx", &bytes).is_ok() && std::fs::read("probe.kdbx").is_ok_and(|b| b == bytes);
    let _ = std::fs::remove_file("probe.kdbx");
    check("folder_round_trip", file_ok, record);

    (checks.iter().filter(|c| c.1).count(), checks.len())
}
