# Handoff to Gemini, 2026-11-26

Claude is out of tokens for a while. This file says where things stand and what to do
next. Read `docs/DESIGN_GUARDRAILS.md` first: SexOS is a pure-Rust SASOS microkernel,
not Unix/Linux/POSIX. Kernel line budget, one ledger file per change in
`docs/handoff/ledger/YYYY-MM-DD-topic.md` (never append to the old big ledger).

## Rules that keep master green

- master ruleset: PR required, the `build` check must pass, no force push. Turn on
  auto-merge and let it merge on green. Never merge on red master.
- You develop on an **arm64 Mac**: `#[cfg(target_arch = "x86_64")]` code is compiled
  out of your host runs. Host tests green on your Mac do not mean x86_64 CI is green.
  Check with `cargo check --target x86_64-sex.json -Zbuild-std=core,alloc
  -Zbuild-std-features=compiler-builtins-mem` for no_std crates, and push to let CI run
  the QEMU gates.
- Self-wait pins live in `scripts/self_wait_pins.py` (one tuple per line). If you add
  or move a wait function, add a pin line; don't edit `check_self_wait.py`.
- Andreas's Mac disk rule: delete build output (target/, images, temp worktrees) when
  a task is done.
- **Memory budget**: QEMU gates run near the program-memory limit. A big `static`
  buffer in a program that has many instances (Spindle) costs pages in every instance
  at boot. On 2026-11-25 a 200 KiB static in Spindle cost ~2 MiB and broke the Qupid
  Kodi gate (5th window `no_memory`). Put big buffers on the heap, reserved when used.

## 1. x17 Wi-Fi tether: WORKING as of 2026-11-26 04:31Z

The x17 (Alienware x17 R1, Intel AX210) tethers over Wi-Fi to Andreas's Mac:
on the x17 `tether 192.168.1.177` (Spindle), on the Mac
`cd ~/Projects/Sex/sexwork/tether && python3 tether_host.py --dir logs serve`.
Image on the Ventoy stick: `sexos-x17-9f37ce20.iso` (master after PR #822).
First sustained session: connected 22:31 local, full 535 KB boot log delivered, and
`wifi` sent from the Mac 6 minutes later answered over the tether.

What was wrong (all fixed, all merged in PRs #526 and #822):
1. `link::decap` required RX status bit 11 (DEC_DONE); iwlwifi/iwx need only
   CCM + MIC_OK. macOS's TCP ACKs (TID 6/7) were all dropped.
2. `join::mac_power_awake` set POWER_MANAGEMENT_ENA (bit 1) = 802.11 power save ON
   with zero timeouts. Now flags 0 (constantly awake).
3. `own_rsn` offered MFPC on WPA2, so the WPA2/WPA3 router turned PMF on; its ADDBA
   and deauth frames came encrypted, our plain replies were dropped, and the router
   cut the x17 off after ~19 unicast frames. WPA2 now offers no PMF (like
   wpa_supplicant `pmf=0`). WPA3/SAE still uses MFPC|MFPR.
Also: keep-alives (10 s) only on Spindle-connected flows (`Engine::open_keep_alive`),
tether sends at most 4 KiB per call, `wifi` prints `unicast_in` and per-TID counters.

Diagnostics on the x17: `wifi` (link counters: `[wifi.link.now..now7]`),
`tether sweep HOST [ROUNDS] [SHAPE]` (unattended Wi-Fi test matrix, results to
`/media/Ventoy/wifi-sweep.txt`), `hw all`, `hw save`.
Old sweep reports: `/mnt/project-files/reports/x17-wifi-sweep-*.txt` (Claude's
project files) and `docs/handoff/ledger/2026-11-25-*.md`, `2026-11-26-wifi-wpa2-no-pmf.md`.

How to work on the x17 over the tether: append a command line to
`~/Projects/Sex/sexwork/tether/logs/inbox.txt` on the Mac (or `python3 tether_host.py
--dir logs send "CMD"`); it runs in the x17's Spindle and the output appears in
`logs/current.log`. `logs/state.json` says whether it is connected.

Next on the x17 (in this order):
1. Confirm a long session: leave the tether up 30+ minutes, send `wifi` and `hw all`
   now and then, check nothing stalls. If it drops, get a `wifi` photo/output and
   compare `unicast_in` and `tid_in` before/after.
2. Re-run `tether sweep 192.168.1.177 2` once to measure throughput and check the
   `idle45` shape now passes (it failed 16/16 before keep-alive and the power fix).
3. Block-ack now works: at 04:37Z, 6 minutes into the session, `wifi` over the tether
   showed `ba tid=0 window=64 passed=309 dropped=4`, `unicast_in=315`, nothing
   undecrypted. Watch `dropped` and `in_a_row` in `[wifi.link.now4]` on long runs.
4. Then the x17 hardware checklist: NVIDIA RTX 3070 GSP boot log (Limine entry
   "SexOS with NVIDIA GPU"), trackpad (PR #824), sound, NVMe, power. Use the tether
   to run commands and collect logs instead of photos.

## 2. Gemini's UI, trackpad and speed-ups (thread "Land Gemini's UI, trackpad and speed-ups")

- Candy UI: PR #823 MERGED.
- Trackpad: PR #824 merges by itself on green; if CI goes red, fix it on its branch.
- Your old PR #763: close it once its replacement (#824) is in. (#735 is already merged/closed.)
- PR #821 (your net + microkernel work, re-landed by Claude) MERGED, but these
  speed-ups were taken back out because QEMU boot gates failed. Re-add each one
  separately, only with the gates green:
  - **Drive fast path**: sexdrive `pdx_reply_recv` loop, diskfs `pdx_call_recv`, Files
    trampoline `reply_fast_*` macros. Symptom before: Drive restarted at boot and
    Polyamour save hung.
  - **Open a window in one call**: `Window::open` using OP_WINDOW_OPEN_COMBINED
    (0x10B). Symptom: Jot's first text never committed. Shell/Display already serve 0x10B.
  - **Files page cache**: its fallback ate half of Files' 128 KiB heap. Size it to the
    heap or allocate it lazily.
  - Also out and needing a rethink: sex-app `retry_on_peer_restart` (gave up after 8
    ERR_SERVICE_NOT_READY, which a busy service returns), Files IPC v2 endpoint start,
    and the watchdog widening (hang_limit 60 s restarted a busy Files; programs then
    lost their Files binding).
- Open follow-up: TLS 1.3 resumption test misses resuming the 2nd connection about
  1 in 3 full parallel runs (server does a full handshake). Root cause unknown.

## 3. Other open work

- PR #820 (Codex's native apps, landed by Claude) was still open on 2026-11-26 04:35Z;
  get it green and merged, then
  close the 284 superseded `codex/*` stacked PRs (no comment needed).
- Kernel follow-up: grant Files slot 6 unconditionally (today only recorder images
  get the Files to Shell route; Vellum's image fence falls back without it).
