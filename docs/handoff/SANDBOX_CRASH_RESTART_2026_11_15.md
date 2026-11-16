# Sandbox crash/restart gate (2026-11-15)

Claude took over Codex's restricted-helper sandbox (Moan offline plugin
helper, Shell coordinator, Files immutable jobs) after Codex's own
crash/restart test was stopped. This is ordinary fault-isolation testing of
SexOS's own domains in QEMU.

## Gate

`scripts/sandbox_crash_restart_gate.py --iso ISO --source-disk DISK --out DIR`
needs `target/x86_64-sex-app/release/{moanrack,moanplug}`. Build moanrack with:

```
bash scripts/build_libcxx.sh
sh scripts/build_moan_plugin_fixtures.sh
SEXOS_VST_FIXTURE=$PWD/target/moan-plugin-fixtures/gain.vst3 SEXOS_LIBCXX=$PWD/target/libcxx \
RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked \
  -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
  --manifest-path apps/moanrack/Cargo.toml --features sexos --target x86_64-sex-app.json --release
```

Steps and what each proves:

1. `moanrack --crash-parent` admits a hanging CLAP helper, waits for the
   authenticated Started, stores its admission in its own data folder and
   executes `ud2`. The kernel ends the parent (`cpu.fault.terminal`).
2. Shell retires the parent and force-ends its helper; both reach
   `app.private.retired`. Files records `files.app.cleanup` for the parent.
3. `moanrack --stale-probe` (a new run, which reuses the parent's catalog row
   with a new task) presents the dead parent's exact ticket, nonce and epochs:
   RESULT is refused, CANCEL answers OK0 because the job entry is gone (a
   leaked entry would be denied), and the START produces no Started, no
   `moan.role.reserved` and no `moan.role.started`.
4. A fresh helper job in the reused helper row (new task) renders and verifies.

Evidence: `docs/handoff/assets/sandbox-crash-2026-11-15/result.json`.

## Leak the gate found (fixed)

The helper was mid-bootstrap, parked in a Files call, when its parent died.
Files dropped the pending reply silently. Shell's force-end only clears a
run's pages, so it ends at its next instruction; a task parked in a call never
runs, never faults and never retires, and the helper row stayed taken until
reboot. Files now answers a waiting helper with a bare DENIED (no data) before
forgetting the slot (`release_child` in `servers/sexfiles/src/restricted_io.rs`,
receipt `files.restricted.child.released`). No kernel change.

## Moan offline UI gate

`scripts/moan_offline_ui_gate.py` passes on the restricted path: CLAP render,
crash, hang cancel, swap and VST3 render all import or fail as expected
(`assets/sandbox-crash-2026-11-15/moan-offline-ui-gate.json`). It found that the
Shell coordinator's 5 s admission-to-activation budget was too tight with
Moan's window open (~4.9 s in the policy walk), so it is now 15 s
(`BOOTSTRAP_TICKS`). QEMU here is timing sensitive: running heavy host builds
alongside it starved the guest past even 15 s, so run these gates alone.

## Not covered

A Files or Shell provider restart during a live restricted job needs an image
built with `SEXOS_FILES_CRASH_PROBE=1` (Files) or the crash probe (Shell); the
epoch checks are covered by Codex's host tests, not yet by a QEMU run.
