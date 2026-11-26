# Live Userspace Server Hot-Swap over Tether

Date: 2026-11-26
Branch: `gemini/live-server-hotswap` (based on `b7e9c2d7d`)

## Summary

Added live in-place userspace server hot-swap over the active Wi-Fi tether so freshly compiled server ELFs (`sexdisplay`, `sexdrive`, `silk-shell`, `sexinput`, `sexusb`, `sexbell`, `sexfiles`, `sexnet`, `sexacpi`, `denim`, `cufflinks`, `sexgpu-nv`, `saddle`) can be pushed and reloaded on running hardware in <1s without rebooting or dropping the tether.

## Architecture & Mechanism

1. **Kernel Mechanism (`kernel/src/service_restart.rs`, `kernel/src/crash_log.rs`, `kernel/src/init.rs`)**:
   - Exposed `SYS_CRASH_LOG` op `OP_HOTSWAP` (`8`), capability-gated strictly to Spindle (`SPINDLE_TARGETS`) or Shell (`SHELL_PD`), refusing self-swap and non-restartable domains.
   - Copies user ELF bytes into a temporary kernel buffer (TOCTOU-safe), validates PIE ELF layout/segments/relocations via `sex_pdx::program_image::parse` (moved out of `kernel/src/program_image.rs` to stay strictly within the 24,500 kernel line budget), applies `R_X86_64_RELATIVE` relocations against `code_base(pd)`, and scans every executable page in address order with `sex_pdx::program_code::Scan` to reject WRPKRU/XRSTOR/XRSTORS instructions.
   - Pre-allocates frames, writable pristine snapshots, and page-table leaves in syscall context into `PENDING` (`PendingSwap`), then calls `request_end(target_pd)` so `poll_one()` atomically applies the swap once the target is fully quiesced (`Phase::Dead`, DMA quiesced, IOMMU revoked, workers dead) with zero allocation in timer interrupt context, flushes TLB across all cores, updates `s.entry` and `s.pages` pristine copies, and rearms the server (sending `PEER_RESTARTED` to in-flight callers).
   - Expanded `service_restart::SERVICES` from 10 to 13 and captured `sexacpi` (50), `cufflinks` (52), and `sexgpu-nv` (53) at boot.
   - Emits `[service.hotswap.ok] pd={} entry={:#x} bytes={}` or `[service.hotswap.err] pd={} reason=...`.

2. **Tether & Spindle Wire Protocol (`crates/sex-pdx/src/crash_log.rs`, `apps/spindle/src/tether_command.rs`, `apps/spindle/src/main.rs`, `scripts/tether_host.py`)**:
   - Framed binary upload over active TCP tether: `SEXOS-HOTSWAP <server_name> <size> <fnv64_hex>\n` + raw ELF bytes.
   - Spindle allocates a temporary heap `Vec<u8>` only during upload (`HOTSWAP_IDLE` has 0 static buffer bytes), verifies FNV-1a checksum, invokes `sex_pdx::crash_log::hotswap_server(pd, &buf)`, immediately drops the staging `Vec<u8>`, and reports `hotswap: ok ...` or `hotswap: err ...` back over tether. Even on unknown server or OOM, Spindle drains `size` bytes so the tether stream never desyncs.
   - Added `hotswap <server_name>` command in Spindle and `push-server <server_name> <elf_path>` subcommand in `scripts/tether_host.py` (staged via `@HOTSWAP <file>` in `inbox.txt` so raw binary never corrupts line-oriented `inbox.txt`).

## Verification

- `tests/server_hotswap.rs` (Rust host unit tests: server name mapping, header parsing, FNV-1a mismatch, bad magic, non-restartable PD, WRPKRU rejection, full staged swap + relocation + rearm).
- `scripts/tests/test_tether_hotswap.py` (Python host unit tests: FNV-1a vector, framing roundtrip, bad-input rejection).
- `cargo check --target x86_64-sex.json -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem` for `sex-kernel`, `crates/sex-pdx`, and `apps/spindle`.
- `scripts/check_architectural_truth.py`, `scripts/tests/test_kernel_design_guardrails.py`, `scripts/service_peer_host_gate.sh`, `scripts/fault_recovery_host_gate.sh`, and `scripts/program_image_host_gate.sh`.
