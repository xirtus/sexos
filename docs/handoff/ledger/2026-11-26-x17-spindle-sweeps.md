# 2026-11-26 — Spindle real-hardware sweeps, auto-reconnect tether & AWCC power CLI

Branch: `gemini/x17-spindle-sweeps` (based on PR #827 `b7e9c2d7d`).

## Summary of changes

1. **Auto-reconnecting `tether sweep` & streamed `cat` (`apps/spindle/src/tether_command.rs`, `apps/spindle/src/fs_commands.rs`)**:
   - `tether sweep HOST[:PORT] [ROUNDS] [SHAPE]` (and bare `tether sweep` when an outbound tether is active) now works while a control tether is connected: it saves the active `(listening, host, port)` state, cleanly closes `FLOW` (and `LISTEN_FLOW` if listening), runs all sweep trials, prints the summary to both `/media/Ventoy/wifi-sweep.txt` and Spindle's scrollback, releases the heap-allocated `RESULTS` buffer (`clear` + `shrink_to_fit`), and automatically reconnects the control tether after a 1.5 s settle window.
   - Root cause of `cat /media/Ventoy/x17-dhcp2.txt` truncating at ~32 KiB: `CAT_MAX` in `fs_commands.rs` was hard-capped at `32 * 1024`, and `FS.send()` could also outrun Wi-Fi tether draining of the 1024-line scrollback ring. Raised `CAT_MAX` to `512 * 1024` (zero static memory) and added `TETHER.backpressure(sb)` pacing in `FS.send(sb)` when >= 512 unsent scrollback lines are queued for a live control tether.

2. **Real-hardware microkernel & NVMe sweeps (`apps/spindle/src/sweep_command.rs`, `apps/spindle/src/sweep_math.rs`)**:
   - Zero static buffers: all sample arrays and NVMe read buffers are heap-allocated (`Vec::try_reserve_exact`) for the duration of the command and dropped immediately.
   - `sweep sched [N]`: calibrates TSC kHz against `get_ticks()`, measures null syscall (40), `get_ticks`, and `sys_yield()` p50/p95/p99/max in cycles + ns.
   - `sweep ipc [N]`: measures IPC v2 `SYS_NOTIFY` (106, signal+poll), `SYS_ENDPOINT` (105, stats), and cross-domain `pdx_call_recv` (102, Files `PING`) latency percentiles.
   - `sweep nvme [PATH]`: read-only sequential 4 KiB, 128 KiB, and 1–2 MiB throughput + latency test; auto-picks a readable file on `/media/Ventoy`, `/media/windows`, or `/` when no path is given (works on Live ISO without a native SexFS disk).
   - `sweep all [PATH]`: runs `sched` + `ipc` + `nvme` and prints a compact summary table.

3. **AWCC `power` / `profile` / `fan` CLI (`apps/spindle/src/power_command.rs`, `crates/sex-pdx/src/power.rs`, `kernel/src/hal/acpi.rs`, `kernel/src/init.rs`, `servers/sexacpi/src/{wmi,laptop}.rs`)**:
   - Added `power::PROFILE` right bit (16) and `CTL_THERMAL_PROFILE` (7) with `THERMAL_*` constants in `crates/sex-pdx/src/power.rs`.
   - Granted Spindle incarnations `power::SENSE | power::PROFILE` at BOOT (`kernel/src/init.rs`, +4 lines) and permitted `PROFILE`-right callers to invoke only `CTL_PROFILE` and `CTL_THERMAL_PROFILE` (`kernel/src/hal/acpi.rs`, +2 lines); Spindle still cannot shut down, restart, or sleep the machine.
   - Wired `Awcc::set_kind()` in `servers/sexacpi/src/wmi.rs` and `CTL_THERMAL_PROFILE` in `servers/sexacpi/src/laptop.rs` so `profile quiet|balanced|balanced_performance|performance|cool|low_power|gmode` updates both CPU HWP/CPPC EPP and Alienware Command Center firmware thermal profiles and immediately refreshes AWCC readings.
   - Added `power`, `profile [NAME]`, and `fan` commands in Spindle (`apps/spindle/src/power_command.rs`), plus `[sexacpi.awcc` in `hw_command::TAGS` and Tab completion entries in `complete.rs`.

## Verification

- `python3 scripts/check_architectural_truth.py`: PASS
- `python3 scripts/tests/test_kernel_design_guardrails.py`: PASS
- `rustc --edition=2021 --test tests/spindle_sweeps.rs` + `complete.rs` unit test: PASS
- `cargo check --target x86_64-sex.json -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem` for `apps/spindle`, `servers/sexacpi`, and `kernel`: PASS
- `cargo test -p sex-pdx` and `cargo test -p sex-laptop`: PASS
- Deleted `target/` before commit.
