# SexOS daily driver: the next 108 slices in 12 parallel workstreams (2026-11-13)

Andreas asked for the plan that takes SexOS from a QEMU-qualified alpha to a full-featured
daily driver, split so many threads can work at once. This document is that plan. It is built
from master `a16cd616` (PR #163 merged), the outcome matrix in
[`ledger/2026-11-10-daily-driver-capability-gates.md`](ledger/2026-11-10-daily-driver-capability-gates.md),
the evidence checkpoint [`ledger/2026-11-10-daily-driver-checkpoint-17a.md`](ledger/2026-11-10-daily-driver-checkpoint-17a.md),
the [Linux and desktop parity report](LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md) and the
[daily driver roadmap](DAILY_DRIVER_ROADMAP_2026_11_08.md).

Binding rules for every slice: [`docs/DESIGN_GUARDRAILS.md`](../DESIGN_GUARDRAILS.md) and
[`docs/BRIDGE_LEDGER.md`](../BRIDGE_LEDGER.md). The kernel holds mechanism only and stays inside
its 21,500-line budget. Drivers, filesystems, protocols and UI live in servers or programs.
POSIX and sockets are program libraries. Linux code is a reference or a temporary Tuxedo bridge
with a ledger row, never the design.

## 1. Where we are (audit)

Done since the parity report (merged on master): xHCI with hubs, HID and mass storage; ACPI
power off/restart with AML in the ring-3 `sexacpi` server; on-screen boot log, crash screen and
restartable services; GPT installer beside another OS and signed A/B updates; e1000e/igc/RTL8125,
Intel HDA and AHCI; native resolution and HiDPI; tiling desktop with nine workspaces, four
layouts, scratchpad, launcher search, wallpaper and night light; clipboard in built-in apps;
notifications; TLS 1.2/1.3, IPv6 and SDK sockets; Rust `std`, C and C++ programs; FAT/exFAT
read-write and ext4/NTFS read in Files; PDF viewer; FFmpeg playback, Bell mixer and capture;
progressive media with seek-ahead; MSE; bytecode JIT (opt-in) and wasm2c QuickJS; IOMMU, MSI-X,
driver manifests, VMD/RAID and NVIDIA power-off; slim Geist chrome; desktop screenshots and a
lossless encoder.

Gaps this plan targets, confirmed in source on `a16cd616`:

| Area | Fact found |
| --- | --- |
| Settings | No Settings program; no shared config store with live reload. |
| Keyboard | US layout only; no dead keys, compose, layout switching or text shaping; Geist has no CJK or emoji coverage. |
| Session | No lock screen, idle dim/blank, password or time zone; no NTP; clock is UTC. |
| Browser | One page per window (no tabs); history is an in-memory back stack; no bookmarks; `localStorage` is in-memory only (`prelude.js` `storage()`); no WebSocket; no find in page; no download UI. |
| Terminal | Spindle is a command surface, not a VT/ANSI terminal; CLI ports cannot draw. |
| Packages | `sex-pkg` and `sexshop` are stubs (~300 lines); programs come from the boot catalog or Files. |
| Power | No battery/brightness in the bar, no suspend, no idle power policy, no touchpad gestures. |
| Display | Firmware framebuffer only: no modesetting, no external monitor, no vsync. |
| Reliability | No 24 h soak with fault injection on one candidate image; no system monitor program. |

Already owned elsewhere and **excluded** here: USB Ethernet, Wi-Fi and x17 readiness (thread
"Get networking ready for the x17 test"); stat cache and notifications patches (landed); YouTube
and the JIT path to `/watch` (YouTube thread); V8 (separate session); desktop recorder (#164) and
rounded panels (#158), both Codex drafts.

## 2. How to run these in parallel

Each workstream owns a set of directories (its **write set**). Two workstreams never write the
same source file except the shared files below, which follow the rules beside them.

| Shared file | Rule |
| --- | --- |
| `sexos_build_spec.toml`, `spec/abi_inputs.sha256` | Append-only edits. On conflict take master's side and rerun `scripts/update_abi_hashes.sh`. |
| Boot app catalog / `Makefile` program lists | Append one line per new program; rebase before merge. |
| `servers/silk-shell/src/main.rs` | Owned by workstream C. Others add their own module file in `servers/silk-shell/src/` and a hook of at most ~10 lines in `main.rs`, in a PR of their own. Run the hashed-function gates (`check_architectural_truth --run-host`). |
| `servers/sexnet/src/main.rs` | Owned by the x17 networking thread until it merges; workstream K works in `crates/` first. |
| `crates/sex-js/src/prelude.js`, `apps/web/src/main.rs` | Workstream D, coordinating with the YouTube thread: small PRs, rebase often. |
| `crates/sex-pdx` | New protocol modules only (one file per protocol). Any new syscall number is checked against the taken list and named in the PR. |
| `kernel/` | Only J and L may touch it, and each kernel line is justified against the 21,500 budget. |

Every slice ends in one small PR with a behaviour test (a `scripts/*_gate.py` QEMU gate or a
host test) that fails before and passes after. Threads merge their own PRs when CI is green.
"Gate" below means a new or extended QEMU gate under `scripts/`; "host" means a host-side
`cargo test`/unit test. Hardware-only checks are listed as x17 receipts, not as merge gates.

## 3. Workstreams

### A. Keyboards, languages and text (10 slices)

Goal: anyone can type their language and read any script. Write set: `servers/sexinput/`,
new `crates/sex-keymap/`, `crates/sex-font/`, new `crates/sex-shape/`, `servers/sexdisplay/src/glyphs.rs`.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| A1 | Keymap crate: scancode/HID usage → keysym tables, US as the first table, used by Input | `crates/sex-keymap`, `servers/sexinput` | host: every US key matches the old table; gate: typing in Jot unchanged |
| A2 | Layouts UK, DE, FR, ES, IT, Nordic (SE/NO/DK/FI), Dvorak, Colemak | `crates/sex-keymap/layouts/` | host: golden table per layout; gate: QMP `sendkey` with DE layout yields `ü`, `ß` in Jot |
| A3 | Dead keys and AltGr level 3 | `crates/sex-keymap` | host: `´`+`e` → `é`, AltGr+`q` → `@` on DE |
| A4 | Compose key with an XCompose-style table (Rust data, no X11 code) | `crates/sex-keymap/compose.rs` | host: Compose `o` `"` → `ö`; gate: Compose in Kaleidoscope address bar |
| A5 | Layout switch shortcut (Super+Space) and per-session layout list | `servers/sexinput`, small shell hook | gate: switch US→DE→US, bar shows layout code |
| A6 | Font fallback chain: Geist → Noto Sans (Latin ext, Greek, Cyrillic) → Noto CJK subset → Noto Emoji (monochrome) | `crates/sex-font`, assets | host: glyph lookup for `Ж`, `日`, `😀` hits a font; screenshot gate shows no tofu |
| A7 | Text shaping with `rustybuzz` (MIT) for ligatures, combining marks and Arabic/Devanagari | new `crates/sex-shape` | host: shaped clusters match HarfBuzz reference output for 20 strings |
| A8 | Bidirectional text (`unicode-bidi`) in the shared text renderer | `crates/sex-shape` | host: mixed Hebrew/English line orders correctly; gate: Jot screenshot |
| A9 | Basic input method protocol (pre-edit string + commit) in `sex-pdx`, with a Pinyin table IME program | `crates/sex-pdx/src/ime.rs`, new `apps/ime-pinyin` | gate: typing `nihao` + Space commits `你好` into Jot |
| A10 | Key repeat delay/rate and Caps-as-Ctrl options read from config (needs B1) | `servers/sexinput` | gate: held key repeats at configured rate; option remaps Caps |

### B. Settings, session and time (9 slices)

Goal: one Settings program and a real session: time zone, lock, idle. Write set: new
`apps/settings/`, new `crates/sex-config/`, `crates/sex-time/`, new `servers/silk-shell/src/lock.rs`.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| B1 | `sex-config`: typed TOML config in Files under a grant, change notifications over PDX, live reload | new `crates/sex-config`, `crates/sex-pdx/src/config.rs` | host: write → subscriber sees new value; gate: config survives reboot |
| B2 | Settings program shell: sidebar of pages, search, reads/writes `sex-config` | new `apps/settings` | gate: open Settings, change accent colour, shell repaints |
| B3 | Pages: Display (scale, night light, wallpaper), Keyboard (layouts, repeat), Sound (devices, volumes) | `apps/settings` | gate: change scale 1→2 from Settings, screenshot matches `display_native_gate` |
| B4 | Time zones: compiled tz database (`tzdb`-style Rust data), civil time in bar and programs | `crates/sex-time`, `servers/silkbar` | host: Europe/Berlin DST transitions 2026; gate: bar shows local time |
| B5 | NTP/SNTP client as a program library in `sex-net` user side, setting the system clock through the existing time capability | `crates/sex-time`, new `crates/sex-ntp` | gate: QEMU with skewed RTC converges within 1 s of host |
| B6 | Lock screen: Super+L, password hashed with Argon2id stored by Files, compositor shows only the lock surface | `servers/silk-shell/src/lock.rs`, hook | gate: locked screen ignores app input; wrong password refused; right one unlocks |
| B7 | Idle policy: dim after N min, lock after M, display blank; inhibited while media plays | `lock.rs`, `servers/sexbell` hint | gate: shortened timers dim→lock→blank; playing video inhibits |
| B8 | First-boot setup: language, layout, time zone, user name and password | `apps/settings/src/first_boot.rs` | gate: fresh disk boots into setup, finishes into desktop |
| B9 | Default apps and file associations (open `.pdf` with PDF, `.mp4` with Qupid) | `crates/sex-config`, `apps/files` hook | gate: double-click in Files opens the configured program |

### C. Desktop shell (10 slices)

Goal: a desktop as smooth as Hyprland/KDE. Write set: `servers/silk-shell/` (owner of
`main.rs`), `servers/silkbar/`, `crates/silkbar*`, `servers/sexdisplay/src/scene.rs`.
Excludes rounded panels (#158), notifications and recorder.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| C1 | Window open/close/move and workspace slide animations on the damage path, off switch in config | `wm.rs`, `sexdisplay/scene.rs` | gate: frame timestamps show ≤16 ms steps; animations off → single frame |
| C2 | Alt+Tab switcher with live thumbnails | `servers/silk-shell/src/switcher.rs` | gate: Alt+Tab cycles focus across 3 programs |
| C3 | Overview (Super+Tab): all windows of all workspaces, click to go | `overview.rs` | gate: click target window from overview focuses it |
| C4 | Drag and drop between programs (file and text payloads through revocable regions) | `crates/sex-pdx/src/dnd.rs`, shell | gate: drag a file from Files into Jot inserts its path/contents |
| C5 | Clipboard for every SDK program, images, and history (Super+V) | `crates/sex-pdx/src/clipboard_client.rs`, `clipboard.rs` | gate: copy PNG in View, paste in Jot as image; history lists last 10 |
| C6 | Window rules and keybindings from config (float/tiling per program, custom keys) | `wm.rs`, `desktop_keys.rs` | host: rule parser; gate: rule floats Calculator |
| C7 | Panel tray: battery, network, volume, layout, clock menu with calendar | `servers/silkbar`, `crates/silkbar-model` | gate: tray items render; clicking volume opens slider |
| C8 | Screenshot UI: region, window and full, to file and clipboard (Print, Shift+Print) | `screenshot.rs`, uses #147 capture | gate: region shot PNG equals expected crop |
| C9 | Multiple floating windows per workspace with snap and resize by mouse edges | `wm.rs` | gate: drag edge resizes; snap to half |
| C10 | Session restore: reopen programs and layout after reboot | `session.rs` | gate: reboot restores three windows on their workspaces |

### D. Browser for everyday sites (10 slices)

Goal: the browser handles ordinary daily web use. Write set: `apps/web/`, `crates/sex-html/`,
`crates/sex-web/`, `crates/sex-js/src/prelude.js` (shared with YouTube thread; small PRs).
Excludes JIT and YouTube.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| D1 | Tabs: Ctrl+T/W/Tab, tab strip, one JS realm per tab | `apps/web` | gate: open 3 tabs, close middle, others keep state |
| D2 | Persistent history and bookmarks in Files, Ctrl+D, address-bar suggestions | `apps/web/src/history.rs` | gate: bookmark survives reboot; typed prefix suggests it |
| D3 | Persistent cookies and `localStorage` per origin, with clear-site-data | `prelude.js`, `apps/web/src/storage.rs` | gate: page counter in localStorage increments across reboot |
| D4 | Forms: all input types, `<select>`, checkboxes, GET/POST urlencoded and multipart submit | `crates/sex-html`, `apps/web` | gate: local HTTP server receives the expected POST body |
| D5 | Downloads: save to Downloads via Files grant, progress and open | `apps/web/src/download.rs` | gate: 20 MB download hash matches |
| D6 | Find in page (Ctrl+F) and page zoom (Ctrl +/-) | `apps/web`, `sex-html/layout.rs` | gate: highlights N matches; zoom reflows |
| D7 | WebSocket (RFC 6455) over the SDK socket + TLS | `crates/sex-http`, `prelude.js` | gate: echo server round trip, wss too |
| D8 | CSS: `position: sticky`, transforms, `@font-face` web fonts, `calc()` coverage | `crates/sex-html/style.rs`, `layout.rs` | host: layout golden tests; gate: screenshot of a test page |
| D9 | `IndexedDB` minimal (object stores, get/put/cursor) on Files | `prelude.js`, `apps/web/src/idb.rs` | gate: IDB test page passes 20 operations across reboot |
| D10 | Site suite: Wikipedia, GitHub read-only, Hacker News, a webmail login form, a news site — recorded outcomes | `scripts/browser_sites_gate.py` | gate: each site reaches its expected visible text |

### E. Files and storage (9 slices)

Goal: files are safe and pleasant to manage. Write set: `apps/files/`, `crates/sex-volumes/`,
`crates/sexfs/`, `servers/sexfiles/src/{vfs,volumes,fs_service}.rs` (not the `recording_*` files).

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| E1 | Copy/move/rename/delete with progress and cancel, in the background | `apps/files` | gate: copy 1 GB, cancel half-way leaves no partial file |
| E2 | Trash with restore and empty | `apps/files`, `sexfiles/vfs.rs` | gate: delete → restore → hash equal |
| E3 | Thumbnails for images, PDFs and video (cached) | `apps/files/src/thumbs.rs` | gate: folder of 50 images shows thumbnails within budget |
| E4 | Search by name and content, indexed incrementally | `apps/files/src/search.rs`, `sexfiles` | gate: find a word inside a text file in < 1 s |
| E5 | Power-cut durability gate: kill QEMU during writes 100 times, sexfs and FAT/exFAT verified | `scripts/powercut_gate.py`, `crates/sexfs` | gate: zero corruptions, last fsync'd data present |
| E6 | `sexfs` check-and-repair tool | new `apps/fsck` | host: repairs injected faults in image fixtures |
| E7 | ext4 write (extents, no journal replay first, then journal) | `crates/sex-volumes/src/ext4.rs` | host: Linux `e2fsck -fn` clean on written image |
| E8 | Safe removal: eject flushes and revokes grants; unplug mid-write reported, not corrupting | `sexfiles/volumes.rs`, `apps/files` | gate: eject then unplug, FAT image passes `fsck.vfat -n` |
| E9 | Disk encryption for the home volume (XTS-AES-256, key from password via Argon2id) in a storage server, not the kernel | new `servers/sexcrypt` | gate: unlock at login; wrong password cannot read |

### F. Productivity programs and terminal (9 slices)

Goal: everyday apps people expect. Write set: `apps/jot/`, `servers/quil/`, `apps/pdf/`,
`apps/view/`, `apps/sex-calc/`, new `apps/term/`, new `apps/calendar/`, new `apps/notes/`.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| F1 | Terminal emulator: VT100/xterm subset (`vte` crate, MIT), scrollback, copy/paste, runs Spindle and `std` CLI programs | new `apps/term` | gate: `vttest`-style fixture renders; colored output correct |
| F2 | PTY-like stream pair as a PDX protocol (library, not a kernel API) | `crates/sex-pdx/src/stream.rs`, `crates/sex-std` | host: echo program round trip; gate: program reads lines from Spindle's terminal (Term) |
| F3 | Jot: undo/redo tree, find/replace, multiple tabs, line numbers | `apps/jot`, `servers/quil` | gate: edit, undo 5, redo 2, save, reopen equal |
| F4 | Code editing: syntax highlighting (tree-sitter-free lexers for Rust/C/JS/Markdown/TOML) | `apps/jot/src/highlight.rs` | host: token golden tests; screenshot |
| F5 | Spell check with Hunspell-format dictionaries (Rust `spellbook`, MPL/MIT) | `apps/jot`, assets | gate: misspelled word underlined; suggestion replaces it |
| F6 | PDF: text search, outline, page thumbnails, zoom to fit | `apps/pdf` | gate: search finds term on page 7 and jumps there |
| F7 | View: rotate, zoom, slideshow, GIF/WebP/AVIF animation (via FFmpeg decoders) | `apps/view`, `crates/sex-image` | gate: animated WebP plays N frames |
| F8 | Calendar app with local events and `.ics` import | new `apps/calendar` | gate: import fixture, event shows on its date |
| F9 | Calculator: scientific mode and history | `apps/sex-calc` | host: expression golden tests |

### G. Sound and media players (8 slices)

Goal: music and video feel complete. Write set: `apps/play/`, `servers/sexbell/`,
`crates/sex-media/`, new `apps/music/`. Excludes recorder, seek-ahead (done) and YouTube.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| G1 | Media keys and volume OSD (mute, up/down, play/pause routed to the focused or last player) | `servers/sexbell`, shell hook | gate: volume key changes Bell master; OSD appears |
| G2 | Per-program volume panel in the bar | `servers/silkbar` module, `sexbell` | gate: lower one program, other unchanged in mix output |
| G3 | Qupid library (was Music app): library from Files, albums/artists, queue, gapless | new `apps/music` | gate: two-track queue plays gapless (no silence > 5 ms) |
| G4 | Qupid (was Play): subtitles (SRT/ASS text), audio track choice, playback speed | `apps/play`, `crates/sex-media` | gate: SRT line drawn at its timestamp |
| G5 | Qupid (was Play): playlist, resume position, fullscreen controls overlay | `apps/play` | gate: reopen resumes within 1 s of saved position |
| G6 | USB Audio Class 1/2 output and input in `sexusb` + Bell device | `servers/sexusb`, `servers/sexbell` | gate: QEMU `usb-audio` receives the test tone |
| G7 | HDMI/DP audio on Intel HDA codecs (Bluetooth audio is a later workstream) | `servers/sexbell/src/hda.rs` | x17 receipt; host: codec parse of HDMI pin |
| G8 | A/V sync and latency gate across Qupid and Kaleidoscope | `scripts/av_sync_gate.py` | gate: drift < 40 ms over 5 min |

### H. Laptop power (9 slices)

Goal: a laptop that lasts and sleeps. Write set: `servers/sexacpi/`, new `servers/sexi2c/`,
new `crates/sex-hid-touch/`, `servers/silkbar` battery module. Check with the x17 networking
thread before touching shared x17 test bundles.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| H1 | Battery and AC status (ACPI `_BIF/_BIX/_BST`) to the bar with time remaining | `servers/sexacpi`, silkbar module | gate: QEMU `-device acpi-battery`-style SSDT fixture shows %; x17 receipt |
| H2 | Backlight: ACPI `_BCM/_BQC` first, then Intel PWM register path | `servers/sexacpi`, new `servers/sexbacklight` | gate: brightness keys change `_BQC`; x17 receipt |
| H3 | Lid switch → lock and blank; power button → shutdown dialog | `servers/sexacpi`, shell hook | gate: injected lid event locks (needs B6) |
| H4 | I2C (Intel LPSS DesignWare) bus server, OpenBSD `dwiic` as reference | new `servers/sexi2c` | host: register model tests; x17 receipt |
| H5 | I2C HID touchpad driver: pointer, two-finger scroll, tap-to-click | new `crates/sex-hid-touch`, `sexi2c` | host: HID report fixtures; x17 receipt |
| H6 | Touchpad gestures: three-finger workspace swipe, pinch zoom to programs | `crates/sex-hid-touch`, shell hook | host: gesture recogniser tests |
| H7 | CPU power policy: HWP energy-performance preference per power profile (Performance/Balanced/Saver) | `servers/sexacpi` | gate: profile switch writes expected MSR via power cap |
| H8 | Suspend to idle (s2idle): devices quiesce through power `OP_PREPARE`, CPUs `mwait` deepest C-state, wake on key/lid | `servers/sexacpi`, drivers' prepare hooks | gate: QEMU suspend/resume keeps files and network; x17 receipt |
| H9 | Thermal and fan readings in a System page; critical trip → clean shutdown | `servers/sexacpi` | gate: injected `_TMP` above `_CRT` triggers shutdown |

### I. Programs, packages and SDK (9 slices)

Goal: installing and building software is normal. Write set: `crates/sex-pkg/`,
`servers/sexshop/`, new `apps/store/`, `crates/sex-std/`, `crates/sex-libc/`, `sex-packages/`.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| I1 | Package format: signed archive (ed25519, same trust model as updates) with manifest of grants | `crates/sex-pkg` | host: tampered package refused; manifest parsed |
| I2 | Install/remove/upgrade service in `sexshop`, programs launched outside the boot catalog | `servers/sexshop` | gate: install package from Files, launch it, remove it |
| I3 | Permission prompt on first run from the manifest (network, files, microphone) | `servers/sexshop`, shell hook | gate: denied network grant → program gets refusal |
| I4 | sexshop (was Store) program: catalog from an HTTPS index, search, install, update all | new `apps/store` | gate: local HTTPS index; install + update path |
| I5 | `std`: `std::fs` complete (dirs, metadata, rename), `std::process::Command` as "run program" | `crates/sex-std` | host+gate: std test subset passes |
| I6 | `sex-libc`: `select/poll` on sockets and streams, `getaddrinfo`, `mmap` of files as library emulation | `crates/sex-libc` | gate: C HTTP client + `poll` test |
| I7 | Port a real CLI set: ripgrep, fd, bat, a Lua interpreter | `sex-packages/` recipes | gate: each runs in Spindle's terminal (F1) with expected output |
| I8 | Port SDL2 (video/audio/input onto sex-app) and one SDL game | `sex-packages/sdl2` | gate: SDL test program draws and plays tone |
| I9 | Python (CPython or RustPython) as a package | `sex-packages/python` | gate: `python -c 'print(2**100)'` in Spindle |

### J. Reliability, monitoring and performance (9 slices)

Goal: it never loses work and stays fast. Write set: `scripts/`, `tests/`, `apps/fuzz/`, new
`apps/monitor/`, `apps/bench/`; `kernel/` only with budget justification.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| J1 | System monitor program: per-program CPU, memory, I/O, network, kill/restart | new `apps/monitor`, read-only stats PDX | gate: monitor shows a busy loop at ~100% one core |
| J2 | 24 h soak driver: scripted daily workload (browse, edit, play, copy) with fault injection into every restartable service | `scripts/soak_gate.py` | soak: zero lost saved files, hashes equal, session intact (nightly, not per-PR) |
| J3 | Syscall and PDX fuzzing extended to every protocol in `sex-pdx` | `apps/fuzz` | gate: 1 M calls, no kernel fault, every refusal typed |
| J4 | Out-of-memory policy: pressure → notify → ask programs to trim → kill largest non-system program with a notice | `servers/sext` user side, monitor | gate: allocation storm keeps shell responsive, notice shown |
| J5 | Compressed RAM (zstd-like pages in a ring-3 memory server) | new `servers/sexzram` | gate: 2× working set over RAM completes |
| J6 | Responsiveness gate: input-to-pixel p50/p95/p99 under load | `scripts/latency_gate.py` | gate: p99 < 16 ms with 4 busy programs (QEMU numbers recorded) |
| J7 | Program start time: prelinked images and lazy pages; target < 100 ms for Jot | `servers/sex-ld`, bench | gate: measured start times recorded and under target |
| J8 | NVMe per-core queues and deeper queue depth, after uncached bench shows the bottleneck | `apps/sexdrive`, `servers/sexdrive` | bench: 4K random read at 4 cores vs Linux |
| J9 | Crash reporter UI: crash records listed in Settings with copy-to-file | `apps/settings` page (after B2), `servers/sexfiles/src/crash_keeper.rs` | gate: injected crash appears with backtrace |

### K. Networking beyond the x17 bring-up (8 slices)

Goal: networking as capable as a desktop's. Write set: `crates/sex-net/`, `crates/sex-http/`,
`crates/sex-tls/`, `crates/sex-net-config/`, new `crates/sex-wg/`, new `apps/ssh/`.
`servers/sexnet/src/main.rs` only after the x17 networking thread merges.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| K1 | DHCPv6 and RDNSS for IPv6-only DNS | `crates/sex-net` | gate: IPv6-only QEMU net resolves AAAA |
| K2 | UDP multi-datagram queues and TCP accept backlog > 1 | `crates/sex-net` | gate: 64 concurrent accepts; 1000 UDP datagrams no loss on loopback |
| K3 | HTTP/2 client (ALPN h2) in `sex-http` | `crates/sex-http` | gate: h2 test server, 50 parallel streams |
| K4 | DNS over HTTPS / TLS option with system resolver cache | `crates/sex-net` | gate: resolver uses DoH when configured |
| K5 | Per-program firewall from grants, visible in Settings | `crates/sex-net-config` | gate: blocked program refused, allowed one connects |
| K6 | WireGuard client (native Rust, BSD/MIT reference like boringtun) | new `crates/sex-wg` | gate: tunnel to a QEMU-host WireGuard peer, HTTP through it |
| K7 | SSH client program (`russh`, Apache/MIT) in Spindle | new `apps/ssh` | gate: login to host sshd with key, run `echo` |
| K8 | Network page in Settings: interfaces, IPs, DNS, proxy | `apps/settings` page | gate: static IP set from Settings takes effect |

### L. Native graphics and monitors (8 slices)

Goal: real modesetting and external screens, the base for GPU acceleration. Write set: new
`servers/sexgpu-intel/`, `servers/sexdisplay/src/{scanout,frame_output}.rs`, new `crates/sex-kms/`.
Reverse-engineer from Intel PRMs and MIT i915 docs; no Linux driver bridge.

| # | Slice | Files | Behaviour test |
| --- | --- | --- | --- |
| L1 | `sex-kms` model: connectors, CRTCs, planes, modes, atomic commit protocol over PDX | new `crates/sex-kms` | host: commit validation tests |
| L2 | virtio-gpu KMS backend (QEMU) with page flip and vsync events | `servers/sexdisplay` | gate: flips at 60 Hz, no tearing marker |
| L3 | Intel Gen12 (Tiger Lake) display engine: read EDID over GMBUS, set native mode on eDP | new `servers/sexgpu-intel` | host: register sequence tests; x17 receipt |
| L4 | External HDMI/DP output and hotplug | `sexgpu-intel`, `sex-kms` | x17 receipt; gate: virtio-gpu second head |
| L5 | Multi-monitor desktop: per-output workspaces and scale | `servers/sexdisplay`, shell hook | gate: two virtio heads, window moves between them |
| L6 | Hardware cursor plane and overlay plane for video | `sexgpu-intel`, `sexdisplay` | gate: cursor moves without full-frame damage |
| L7 | Panel self refresh and backlight PWM hand-off with H2 | `sexgpu-intel` | x17 receipt |
| L8 | Plan doc for 3D: command submission, GuC/HuC firmware via firmware store, Mesa-class path | `docs/handoff/` | review only |

## 4. Order and dependencies

Start all twelve at once; within each, slices run in order. Cross-workstream dependencies:

- `sex-config` (B1) first: A10, C6, D-zoom defaults, H7, K5, K8 and J9 read it. Until B1 merges,
  those slices use a hard-coded default and switch to config in a follow-up.
- Spindle's terminal (F1, F2) before I7–I9 and K7.
- Lock screen (B6) before H3. HDA HDMI (G7) and L4 meet on x17.
- I3 permission prompts reuse grants that D5 and E use; keep the grant types in `sex-pdx`.

## 5. Count

A 10, B 9, C 10, D 10, E 9, F 9, G 8, H 9, I 9, J 9, K 8, L 8 = **108 slices**.

## 6. Done means

A slice is done when its PR is merged with its behaviour test in CI. A workstream is done when
all slices are merged and a ledger fragment in `docs/handoff/ledger/` records the candidate SHA,
image hash and gate outputs. "Daily driver" is claimed only after J2's 24 h soak passes on one
candidate image and the x17 receipts for A, H and L exist.
