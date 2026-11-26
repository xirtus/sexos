# 2026-11-26: Multi-Flow `sexnet` / Spindle Tether Concurrency & RTL8125 (`10ec:3000`) Link-Down Fix

## Root Causes Observed on Alienware x17 R1 (Boot `c28152a0`)

1. **Multi-Flow `sexnet` & Spindle Tether Concurrency**:
   - `servers/sexnet/src/web.rs` (`Service`) already supports per-`(Identity, u8)` flow slots with lazy `Tls` session allocation (`Slot.tls = None` until `w::TLS_START`), but three bottlenecks still broke concurrent Wi-Fi tether + `fetch` / `Tailor` / `Kaleidoscope`:
     - `servers/sexnet/src/main.rs` kept a single `web_reply: Option<(u32, u64)>` deferred reply slot; a second PD's `w::OP` arriving before the first reply drained could overwrite and drop the first PD's reply.
     - `servers/sexnet/src/web.rs` checked the single-flight `self.resolving` lock before checking `engine.open_cache_lookup()`, returning `w::BUSY` even for cached DNS names, and `fetch_command.rs`, `tether_command.rs`, and `apps/kaleidoscope/src/web_page.rs` immediately failed on `w::BUSY` during `ResolveAsk`.
     - `apps/spindle/src/main.rs` hard-blocked `fetch`/`update`/`pkg` whenever `TETHER.active()` was true (even though tether uses `FLOW = 1` / `LISTEN_FLOW = 2` and `fetch` uses flow `0`), and `fetch` and `tether` shared overlapping `seq` numbers (`0x00_0001..=0x7F_FFFF`) and unarbitrated access to Spindle's single 64 KiB `w::CLIENT_SLOT` region.

2. **RTL8125 (`3d:00.0` `10ec:3000`, Killer E3000) Unplugged Quarantine**:
   - `servers/sexnet/src/realtek.rs` defined `PHY_STATUS_8125 = 0x80` instead of `0x6c`. On Realtek r8169/r8125/r8126 (including Killer E3000 `10ec:3000`), `PHYstatus` is at MMIO offset `0x6c` (bit 1 `LinkStatus = 0x02`), whereas `0x80` is `EPHYAR` (Extended PHY Access Register). Reading `0x80` on an unplugged RTL8125 saw stale bit 1 set, falsely reporting `link_up() == Some(true)` → `Phase::Running` → TX descriptor submission → `Failure::Deadline` → `Phase::Quarantined`.
   - Additionally, `servers/sexnet/src/ethernet.rs` enforced `WAIT_TICKS` (5 s) on `Phase::Link`, so even with accurate `Some(false)` link-down reporting, an unplugged NIC would quarantine 5 s after boot and never recover when a cable was plugged in later.

## Fixes

- **`servers/sexnet/src/main.rs`**: Replaced single-slot `web_reply` with a zero-allocation 16-slot `web_replies` queue; `w::OP` replies immediately via `pdx_reply()` and only queues on deferral, purging retired/denied PDs cleanly.
- **`servers/sexnet/src/web.rs`**: Serve open DNS cache hits (`engine.open_cache_lookup`) before checking `self.resolving` so concurrent PDs resolve cached names with zero contention.
- **`apps/spindle/src/{main.rs,tether_command.rs,fetch_command.rs}`**:
  - Assigned tether disjoint sequence numbers (`0x80_0000 | ...`) from `fetch` (`0x00_0001..=0x7F_FFFF`).
  - Arbitrated Spindle's shared `w::CLIENT_SLOT` region turn-by-turn via `TETHER.in_call()` and `FETCH.region_in_use()`.
  - Allowed `fetch`, `update`, and `pkg` concurrently with live `tether` (blocking only during a Wi-Fi benchmark `tether sweep`).
  - Added `w::BUSY` retry on `Phase::ResolveAsk` in `fetch_command.rs`, `tether_command.rs`, and `apps/kaleidoscope/src/web_page.rs`.
- **`servers/sexnet/src/realtek.rs` & `servers/sexnet/src/ethernet.rs`**:
  - Fixed `PHY_STATUS_8125` from `0x80` to `0x6c`.
  - Exempted `Phase::Link` from the initialization `WAIT_TICKS`/`WAIT_TURNS` quarantine deadline (hardware disappearance `None` still quarantines with `Failure::Registers`; hardware init phases `Check..=Queues` keep strict deadlines).
- **Zero kernel changes** (`kernel/` untouched, `servers/sexnet/src/wifi.rs` and `crates/sex-wifi` untouched).

## Verification

- `bash scripts/tcp_modern_host_gate.sh` (including new `concurrent_open_flows_keep_tether_alive_while_second_client_transfers` test)
- `bash scripts/web_protocols_host_gate.sh`
- `tests/sexnet_realtek.rs` (including new `rtl8125_unplugged_ignores_ephyar_0x80_and_never_quarantines` test)
- `python3 scripts/check_architectural_truth.py`
- Bare-metal `cargo check` for `servers/sexnet`, `apps/spindle`, and `apps/kaleidoscope` targeting `x86_64-sex.json`
