## Signed packages, install service and first-run grant question (2026-11-13, workstream I1–I3)

- **Format** (`crates/sex-pkg`, GPL-3.0): one file, `SEXPKG01` head, manifest (`name`,
  `version`, `program`, `summary`, `license`, `grant=network|microphone`), entries
  (`bin/PROGRAM`, `data/PATH`), then a 100-byte trailer: publisher key and an Ed25519
  signature over SHA-256 of a domain tag and every byte before it. Checked in one pass,
  so Files never holds a whole package. Host tests: round trip, every tampered byte
  refused, untrusted or re-signed refused, path escapes (`..`, `a` vs `a/b`, foreign
  `bin/` names) refused, truncation and padding refused, catalog signature.
- **Trust**: the release key the system was built with, plus keys the person adds with
  `pkg trust KEY` (`/system/packages/trusted`). Programs never reach `/system` (their
  paths are confined to `/data/NAME`).
- **Install service in Files** (`servers/sexfiles/src/packages.rs`): new Files request
  kind 40 (`sex_pdx::package`, Spindle only): install/check/remove/list/trust/consent.
  An install is one filesystem transaction: refused or failed means nothing changed.
  Upgrades replace the program, drop data files the new version no longer ships, and
  re-ask grants when the new version asks for more. Downgrades need an explicit flag.
  `/bin/NAME` not put there by a package is never replaced.
- **Grant question** (I3): PREPARE of `/bin/PROGRAM` returns a `PKGNOTE1` note (asked,
  answered, allowed). Spindle holds the run and asks "PROGRAM asks to use the network.
  Allow it?"; the answer is saved (`consent`) and the run starts with or without
  `RUN_NETWORK`. `pkg allow|deny PROGRAM network` changes it later. A bare `run NAME`
  now goes through Files too (falling back to the disk program store with no filesystem).
- **Tool**: `tools/sexpkg` (`build`, `public`, `verify`, `index`), with a public test key
  for gates only.
- **Gate**: `scripts/package_gate.py` (CI "Signed packages ..."): untrusted refused, trust,
  tampered refused, install over HTTP, question answered no → chello's own EACCES check
  passes (21/21), allow → `chello net` reaches the echo server without `+net` (23/23),
  upgrade, downgrade refused, list, remove.
- ABI: new Files request kind 40 and `sex_pdx::package` (no syscall). `spec/abi_inputs.sha256` updated.
- Not yet: microphone is asked and recorded but capture does not check it yet; the GUI
  Store and the launcher's own question (the launcher starts catalog apps, not packages).
- **Store (I4, command side)**: `pkg refresh [URL]` downloads `sexos-store.txt` and its
  signature (made by `sexpkg index`), Files checks it against the trusted keys and keeps
  it with the source URL; `pkg search WORDS`, `pkg get NAME`, `pkg upgrade` (every
  installed package the catalog has newer). Each package is still checked on its own
  signature. Gate covers refresh, search, get, re-refresh from the saved source, upgrade
  and an up-to-date second upgrade.
