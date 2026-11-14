# SexOS apps against the best in their class: gap report and slice plan (2026-11-14)

Andreas asked what else we need to build so SexOS apps reach parity with the best app in each
class. This report compares every SexOS app with its class leader, lists what is missing, and
turns the gaps into slices grouped into workstreams that can run as threads in parallel.

Built from master `cf74e7cf` (PR #208 merged) plus the open PRs that already rewrite apps:
#170 (Settings store and program, B1–B3), #179 (laptop power), #182 (Term, Jot editor, PDF
search, View, calculator, calendar: F1–F9), #183 (media players, G), #204 (screenshot UI) and
the rename PRs #211–#216. App names follow `docs/handoff/APP_NAMES_2026_11_14.md` (PR #211).

**Scope rules**

- Nothing here repeats a slice of `DAILY_DRIVER_NEXT_100_SLICES.md` (workstreams A–L). Where a
  gap is already a slice there, the tables say "owned: X#".
- Qupid is planned in `QUPID_BROOTLITE_PLAN_2026_11_14.md` (PR #215) and is skipped, except
  where Kaleidoscope can share its code (ad blocking).
- Excluded as owned elsewhere: V8/JIT and YouTube, x17 networking, desktop recorder #164,
  rounded panels #158.
- Binding: `docs/DESIGN_GUARDRAILS.md`. Apps are confined programs holding only their grants;
  protocols are libraries over PDX; no new kernel API that exists to look like Linux. Every
  kernel touch is named below with its reason.
- Names: every new app name below is a **proposal** in the themes (fabric, romance,
  psychedelic, puns) and waits for Andreas's OK, like the coined names in PR #211.

## 1. The finding that matters most: apps have no shared toolkit

Every SexOS app draws its own UI from `sex_app::Window` (a pixel surface plus key and pointer
events) by hand. Measured on master:

| Fact | Evidence |
| --- | --- |
| Only 3 programs handle the mouse at all | `Event::Pointer` appears in `apps/web`, `apps/jot`, `apps/shapes` only. Files, Store, Monitor, PDF, View, Talk and Spindle are keyboard-only. |
| No widgets | No button, list, text field, menu, tab bar, scroll bar, tree, table, dialog or tooltip type exists in any crate. Each app re-implements its own list and status line (Files, Store and Monitor each have their own "Up/Down choose" loop). |
| No theme | Colours are constants in each app; there is no dark/light, accent, font size or icon set that apps share. Settings (#170) stores an accent colour that nothing reads yet. |
| No accessibility | No accessibility tree, screen reader, magnifier or keyboard-focus model exists anywhere (`grep -ri accessib` finds nothing in crates, servers or apps). |
| No menus or command palette | Every command is a memorised key listed in a header line (`Jot. Arrows move, Ctrl+S saves…`). |

Every best-in-class app (Firefox, VS Code, Dolphin, KDE System Settings) sits on a toolkit
(GTK, Qt, Electron) that gives it these for free. Without one, each SexOS app reaches maybe
60% of its leader however good its engine is, and every new app costs twice as much.
**Workstream M (the Weft toolkit) is therefore the first thing to start**; the other
workstreams build their UI on it as soon as its first slices land.

Weft design (proposal; the toolkit choice is the one item here worth Andreas's look):

- Retained widget tree in a no_std crate `crates/weft` on `sex-app`, so it works in every
  program, `std` or not.
- Layout with **taffy** (MIT, flexbox and grid). Text with `sex-font` + `sex-shape` (A6–A8),
  so every app gets fallback fonts, shaping and bidi at once. Painting with tiny-skia (already
  used by sex-web's canvas) into the window's pixel region, with damage rectangles.
- Accessibility with **AccessKit** (MIT/Apache, a platform-neutral accessibility tree); we
  write the SexOS adapter, a PDX protocol `sex_pdx::a11y` that the screen reader (S7)
  subscribes to with the program's consent.
- Theme from `sex-config` (B1): light/dark/auto, accent, font scale, density, high contrast.
- egui is already vendored (`crates/egui-hello`) and stays the quick path for tools and
  games; Weft is for the apps people live in.

## 2. Gap report, app by app

Each table: what the app does now, what the class leader has that we lack, and the slice that
closes it. "owned: X#" means an existing slice already covers it.

### Kaleidoscope (browser) vs Firefox and Brave

Now: sex-html layout with flex, grid, floats, tables, sticky, transforms, `@keyframes`,
`@font-face`; QuickJS (wasmi or wasm2c) with DOM, fetch, canvas, WebGL 2, WebGPU, MSE,
WebSocket, IndexedDB; tabs, lasting history/bookmarks/cookies/localStorage, find, zoom,
downloads, multipart forms; mouse; per-window confinement.

| Leader has | SexOS | Slice |
| --- | --- | --- |
| Tracker and ad blocking (Brave Shields, uBlock Origin) | none | N1 |
| Password manager, autofill, passkeys | none | N2 (with Chastity, R7) |
| One process per site, crash of one tab never kills the browser | all tabs in one program | N3 |
| Private windows, container tabs | none | N4 |
| Reader mode | separate Peek program | N5 (folds Peek in) |
| PDF inline | no | N5 (embeds Vellum's renderer) |
| Developer tools: inspector, console, network | serial log only | N6 |
| Extensions (WebExtensions) | none | N7 |
| HTTP cache, WOFF2/Brotli, service workers, installable web apps | none | N8 |
| Print and save as PDF | none | N9 |
| Notifications API, Media Session, picture-in-picture, Gamepad | none | N10 |
| WebRTC (calls, Meet, Discord voice) | none | N11 |
| Spell check in text fields, translate | none | N12 (spell; translate later) |
| V8 speed, YouTube | owned: V8 session, YouTube thread | – |

### Quil (code editor) vs VS Code and Zed

Now: `servers/quil` is the BOOT editor, `no_std` with a dummy allocator and a **12 KiB
fixed buffer** (`QUIL_BUFFER_MAX_LEN = 12288`), a 32-byte find query, autosave and document
exchange. Jot (F3–F5, #182) gets the modern engine (`crates/sex-textedit`: undo tree,
find/replace, tabs, lexers, spell check). Quil is the furthest of any app from its leader.

| Leader has | SexOS | Slice |
| --- | --- | --- |
| Unlimited files, rope buffer, 1 GB files | 12 KiB | O1 (Quil rebuilt as an SDK program on sex-textedit) |
| Multi-cursor, column select, command palette, keybinding maps | none | O2 |
| Project tree, fuzzy file finder, project-wide search/replace | none | O3 |
| Real syntax trees for highlighting, folding, outline | regex-free lexers (F4) | O4 |
| Language servers: completion, go to definition, rename, diagnostics | none | O5 |
| Git: gutter, stage, commit, blame, branches, diff and 3-way merge | none | O6 |
| Integrated terminal panel | none | O7 |
| Debugger (breakpoints, stepping, variables) | none | O8 (needs a kernel debug mechanism) |
| Extensions | none | O9 (Wasm, as Zed does) |
| Remote editing over SSH | none | O10 |
| AI assistant | none | O11 |

### Jot (notepad) vs Kate / GNOME Text Editor

After #182 Jot is close (undo tree, find/replace, tabs, line numbers, highlighting, spelling).
Left: restore unsaved tabs after a crash or reboot, encodings and line endings, word wrap
toggle, Markdown preview, print, large files. → O12. Jot stays a notepad; everything
project-shaped goes to Quil.

### Spindle (terminal) vs WezTerm and kitty

After #182: Term (F1) is a VT100/xterm screen with 24-bit colour, alt screen, scrollback,
selection and copy/paste; programs run inside it through the console relay (F2 streams).
Gaps listed in #182 itself: Ctrl+C reaches a program only at its next read or write, programs
are not told the window size.

| Leader has | SexOS | Slice |
| --- | --- | --- |
| Tabs and split panes | one screen per window | P1 |
| Sessions that survive closing the window (multiplexer, tmux built in) | no | P2 (Yarn sessions) |
| Window size to programs, instant interrupt | gaps in #182 | P3 |
| Ligatures, emoji, GPU-style glyph cache, smooth scrolling | basic renderer | P4 |
| Images in the terminal (kitty graphics, sixel) | no | P5 |
| Hyperlinks (OSC 8), clipboard (OSC 52), notifications (OSC 9/777) | no | P5 |
| Scrollback search, quick-select hints | no | P6 |
| Themes and config file, live reload | constants | P7 |
| Pipelines between programs, job control | Spindle commands only | P8 (structured pipelines, nushell-like, not Unix pipes) |
| SSH domains (remote tabs) | K7 SSH client | P9 |

### Loom (files) vs Dolphin

Now (master): background copy/move with cancel, trash with restore, undo, thumbnails, indexed
name and content search, safe eject, encrypted `/home`, ext4/FAT/exFAT/NTFS, open/save picker
for other programs. Keyboard only.

| Leader has | SexOS | Slice |
| --- | --- | --- |
| Mouse, multi-select, drag and drop, context menus | keyboard only | Q1 (on Weft; DnD protocol is C4) |
| Split view, tabs, places sidebar, breadcrumbs | one pane | Q2 |
| Icons / details / columns views, sort, filter-as-you-type | one list | Q2 |
| Preview pane (pictures, PDFs, video, text) | thumbnails only | Q3 |
| Properties: size, dates, permissions | none | Q4 (shows **grants**: which programs can see this file, revoke) |
| Archives: browse, extract, create (zip, tar, 7z, zstd, xz) | none | Q5 |
| Network places: SFTP, WebDAV, SMB | none | Q6 |
| Batch rename, duplicate finder, disk-usage map (Filelight) | none | Q7 |
| Open with… menu, default apps | owned: B9 | – |
| Snapshots / file versions | none | Q8 (sexfs snapshots, used by Aftercare) |

### Vellum (PDF) vs Okular

After #182: rendering (hayro), search with highlights, outline, thumbnails, fit and zoom.

| Leader has | Slice |
| --- | --- |
| Continuous and two-page scroll, mouse, text selection and copy | Q9 |
| Annotations (highlight, notes, ink) saved into the PDF | Q10 |
| Fill forms (AcroForm), view signatures | Q10 |
| Presentation mode, dark/invert, print | Q11 |
| EPUB, CBZ | in Bodice (U7) |

### Gaze (pictures) vs gThumb / Eye of GNOME

After #182: PNG/JPEG/WebP/GIF with animation, rotate, zoom, pan, folder slideshow.

| Leader has | Slice |
| --- | --- |
| AVIF, HEIC, JPEG XL, camera RAW, SVG | Q12 |
| EXIF view, auto-orientation, map of location | Q12 |
| Library: albums, timeline, tags, search | Q13 |
| Basic edits saved non-destructively (crop, straighten, exposure, filters), set as wallpaper | Q13 |

### Tatami (settings) vs KDE System Settings

After #170: 18 typed settings in `sex-config`, one page per section with search. Owned pages:
Display/Keyboard/Sound (B3), time zones (B4), default apps (B9), network (K8), crash reports
(J9). Missing pages, all owned by R:

| Page | Why it matters | Slice |
| --- | --- | --- |
| **Privacy and grants**: every program's grants (files, network, mic, camera, launch, packages), when each was last used, revoke | This is SexOS's strongest feature (capabilities) and has no UI | R1 |
| Appearance: theme, accent, fonts, icon set, cursor size | needs Weft | R2 |
| Updates: A/B slots, channel, rollback, package updates | updates exist only as Spindle commands | R3 |
| Shortcuts editor, startup programs, notifications per program | config exists (C6) without UI | R3 |
| Storage: volumes, encryption, SMART health | | R3 |
| Accessibility | | S7 |
| About: hardware, versions, image hash | | R3 |

### Mesh (system overview) vs Mission Center / Windows Task Manager

Now: per-domain CPU share and memory from syscall 87 (J1 part 1, #175).

| Leader has | Slice |
| --- | --- |
| History graphs for CPU per core, memory, disk, network, GPU | R4 |
| Per-program disk and network I/O, end/restart program | R4 (needs J1's remaining counters; coordinate with J) |
| **Capability graph**: which domain holds which grant, live (the name "Mesh" is this) | R5 |
| Hardware: devices, drivers, temperatures, fan, battery wear, NVMe SMART | R5 (readings from H9) |
| Boot timeline (Foreplay) and start time per program | R5 |

### sexshop (store) vs Flathub / GNOME Software / Steam

Now (#202/#213): signed catalog, search, install/upgrade/remove, first-run grant question.

| Leader has | Slice |
| --- | --- |
| App pages: icon, screenshots, description, size, grants it asks for | R6 |
| Categories, featured, update badge in the tray, background updates | R6 |
| Third-party repositories with their own keys, rollback of one package | R6 |
| Publish flow for developers: build, sign and upload with the Leather SDK | I-follow-up (packages thread) |

### Flirt (chat) vs Element / Signal / Discord

Now: Flirt is the old Talk, a TCP/UDP test tool (#216). There is no chat program.

→ U3–U4: Flirt becomes a messenger (IRC, Matrix with end-to-end encryption, later XMPP); the
TCP/UDP tool becomes a Spindle command.

### Date (calendar) vs GNOME Calendar / Thunderbird

After #182: local events and `.ics` import. Missing: month/week/day views with mouse,
recurring events (RRULE), reminders through Bell, several calendars, CalDAV sync, invitations.
→ U5. World clock, alarms and timers also land in Date (U5).

### Abacus (calculator) vs Qalculate! / SpeedCrunch

After #182: scientific mode and history. Missing: units and currency, arbitrary precision,
programmer mode (bases, bitwise), variables and functions, graphs, answers inside Voyeur.
→ R8.

### Voyeur (search) vs KRunner / Spotlight / Raycast

Now: launcher search (programs, web) and Files' index. Missing: one search protocol with
providers (apps, files and contents, settings pages, Abacus, windows, browser history and
bookmarks, clipboard history, emoji, contacts, mail), frecency ranking, preview pane, actions
(open with, reveal in Loom, copy). → R9–R10.

### Silk (desktop shell) vs KDE Plasma and Hyprland

Owned by C: animations, Alt+Tab, overview, DnD, clipboard, rules, tray, screenshots, snap,
session restore; lock/idle (B6/B7), multi-monitor (L5), gestures (H6).

| Leader has | Slice |
| --- | --- |
| Window decorations with buttons, window menu, double-click maximise | S1 |
| Tray items from programs (StatusNotifier) and notification actions/replies | S2 |
| Tabbed window groups (Hyprland groups, i3 tabs), dwindle layout, per-workspace layouts | S3 |
| Scripting the desktop (hyprctl / KWin scripts) and Wasm plugins | S4 |
| Desktop widgets (clock, weather, system), hot corners | S5 |
| Theme engine shared with apps (Weft), blur and shadows | S6 (rounded #158 owned) |
| Accessibility: screen reader, magnifier, high contrast, sticky keys, on-screen keyboard | S7–S8 |
| Phone link (KDE Connect) | U8 |

## 3. Missing app classes

Classes a Linux/Windows/macOS desktop has where SexOS has nothing. Names are proposals.

| Class | Leader | Proposed name | Why the name | Slices |
| --- | --- | --- | --- | --- |
| Mail | Thunderbird | **Valentine** | a love letter | U1–U2 |
| Contacts | GNOME Contacts | **Crush** | the people you keep | U6 |
| Word processor | LibreOffice Writer | **Brocade** | rich woven fabric for rich text | T3–T5 |
| Spreadsheet | LibreOffice Calc / Excel | **Gingham** | a checkered fabric: a grid | T1–T2 |
| Presentations | Impress / Keynote | **Burlesque** | a show on a stage | T6 |
| Image editor | GIMP / Krita | **Batik** | painting with dye on cloth | V1–V3 |
| Vector drawing | Inkscape | Gestalt (exists as Shapes) | already Andreas's pick | V4 |
| Video editor | Kdenlive | **Strip** | film strip, and a pun | V5 |
| Audio recorder/editor | Audacity | **Serenade** | | V6 |
| Camera | Cheese | **Flash** | camera flash, and a pun | V7 (needs UVC driver) |
| Password manager | KeePassXC / Bitwarden | **Chastity** | locked away; built on Shibari | R7 |
| Backup and restore | Déjà Dup / Timeshift / Borg | **Aftercare** | what comes after something goes wrong | Q14 |
| Disks and partitions | GNOME Disks / Partition Manager | **Corset** | shapes the disk; the UI of BDSM | Q15 |
| E-books | Foliate / Calibre | **Bodice** | bodice-ripper novels | U7 |
| Feeds and podcasts | Liferea / gPodder | **Gossip** | | U7 |
| Phone link | KDE Connect | **Hookup** | | U8 |
| Maps | GNOME Maps / OsmAnd | **Rendezvous** | where you meet | U9 |
| BitTorrent client | qBittorrent | **Swingers** | everyone swaps with everyone | U10 (Qupid's indexer search, Q11b, hands it magnets) |
| Bluetooth | BlueZ + manager | **Cufflinks** | paired links; goes with Tuxedo | W1–W3 |
| Printing and scanning | CUPS / SANE | **Needle** | a free fabric word | W4–W5 |
| Remote desktop | Remmina / RDP | **Lurk** | a free word | W6 |
| Virtual machines / Linux guest | GNOME Boxes / crosvm | **Drag** | another OS in drag | W7 (major: needs Andreas's OK) |

Not proposed as separate apps: archive manager (in Loom, Q5), Git client and diff tool (in
Quil, O6), emoji picker and font viewer (Voyeur and Tatami), screen recorder (#164), weather
(an S5 widget), games launcher (games names undecided).

## 4. Slices

Every slice ends in one PR with a behaviour test that fails before and passes after: a QEMU
gate (`scripts/*_gate.py`) or a host test. Licences named are those of crates we would vendor;
all are compatible with GPL-3 programs.

### M. Weft toolkit and app foundations (10)

Write set: new `crates/weft`, new `crates/sex-pdx/src/a11y.rs`, `crates/sex-app` (small hooks).

| # | Slice | Test |
| --- | --- | --- |
| M1 | `weft` core: widget tree, taffy layout, damage, focus chain, keyboard navigation, pointer hit-testing | host: layout goldens; focus order; hit-test |
| M2 | Basic widgets: label, button, text field (sex-textedit), check/radio/switch, slider, progress, scroll view | host: event tests; gate: demo program driven by QMP keys and clicks |
| M3 | Lists, trees, tables (virtualised, 1 M rows), tabs, split panes | host: 1 M-row table scrolls under 2 ms/frame on host |
| M4 | Menus, context menus, menu bar, command palette, keyboard shortcut map | gate: right-click menu and Ctrl+Shift+P palette run a command |
| M5 | Dialogs and toasts; file open/save through Loom's picker; colour and font pickers | gate: Save As round trip through the picker |
| M6 | Theme from `sex-config`: light/dark/auto, accent, font scale, density, high contrast; live reload | gate: change theme in Tatami, two Weft apps repaint |
| M7 | Icon set (one SVG icon theme rendered with resvg, MIT) and cursor shapes | host: icons render at 1×/2×; screenshot gate |
| M8 | AccessKit tree from every Weft widget, served over `sex_pdx::a11y` only to a consented reader | host: tree of the demo matches golden |
| M9 | Strings and translation: Fluent (`fluent-rs`, Apache/MIT) catalogues, language from settings, plural rules | host: German catalogue renders the demo |
| M10 | Port the small apps to Weft as references: Abacus, Mesh, sexshop (with their owners' consent; one PR each) | their existing gates still pass, plus a mouse step |

### N. Kaleidoscope to Firefox/Brave class (12)

Write set: `apps/web` (renamed `apps/kaleidoscope` by #212), `crates/sex-html`, `crates/sex-web`,
`crates/sex-js/src/prelude.js` (small PRs, rebase often; YouTube thread shares it), new
`crates/sex-shields`.

| # | Slice | Test |
| --- | --- | --- |
| N1 | Shields: Brave's adblock-rust engine (MPL-2.0) with EasyList/EasyPrivacy, cosmetic filters, per-site toggle, blocked count; `sex-shields` crate shared with Qupid's ad stripping (Q10b) | gate: fixture page loads 0 of 12 tracker requests; host: filter matching goldens |
| N2 | Passwords: save/fill forms, generator, stored in Chastity's vault (R7) through a grant, never in the browser's own files | gate: login fixture filled after reboot; vault locked → nothing filled |
| N3 | Site isolation: each site's tabs in their own confined run of the engine, the browser frame as a separate program; a crashing tab shows a sad page | gate: tab running a fault page dies, other tabs keep state |
| N4 | Private windows (memory-only profile) and container tabs (separate cookie jars per container) | gate: cookie set in container A invisible in B and in private |
| N5 | Reader mode (Peek folded in and retired) and inline PDF via Vellum's renderer | gate: article fixture shows reader view; PDF link renders page 1 |
| N6 | DevTools: element inspector with computed style, JS console, network log; F12 | gate: console shows `console.log` output and a failed request |
| N7 | Extensions: WebExtensions subset (content scripts, storage, tabs, webRequest blocking via Shields), each extension a confined run | gate: sample extension recolours a page and blocks a URL |
| N8 | HTTP cache (Files-backed), Brotli and WOFF2, service workers with offline cache, install a site as an app in the launcher | gate: second load served from cache offline; installed app opens standalone |
| N9 | Print and save as PDF (paged layout, `@page`, `@media print`), through Needle when printing exists | host: PDF output parses in hayro with expected text |
| N10 | Notification API → Bell, Media Session → media keys (G1), picture-in-picture window, Gamepad API (USB HID) | gate: page notification appears in Bell's panel |
| N11 | WebRTC: ICE/STUN/TURN, DTLS-SRTP, Opus and VP8 via FFmpeg, data channels (webrtc-rs, MIT/Apache); mic and camera through grants | gate: loopback call between two tabs, audio frames arrive |
| N12 | Spell check in text fields (spellbook, as Jot) and form autofill of addresses | gate: misspelled word in a textarea is underlined |

### O. Quil workstation and Jot polish (12)

Write set: new `apps/quil` (the SDK program), `servers/quil` (retired from BOOT at O1),
`crates/sex-textedit` (shared with Jot; coordinate with the F thread), new `crates/sex-lsp`,
`crates/sex-git`, `apps/jot`.

| # | Slice | Test |
| --- | --- | --- |
| O1 | Quil rebuilt as an SDK program on Weft + sex-textedit with a rope buffer (ropey, MIT); opens 1 GB files lazily; BOOT Quil retired, its document import kept | gate: open 200 MB file, jump to end, edit, save, hash correct |
| O2 | Multi-cursor, column select, command palette, keymaps (VS Code default, Vim mode, Emacs mode) | host: multi-cursor edit goldens; gate: Vim `dd` deletes a line |
| O3 | Project folder: file tree, fuzzy finder (nucleo, MPL-2.0), project search/replace with ripgrep's `grep` crates | gate: Ctrl+P finds a file; search across 10 k files under 1 s |
| O4 | Tree-sitter parsing (C library through sex-libc, MIT) for highlighting, folding, outline, bracket matching; 20 languages | host: parse goldens; screenshot gate |
| O5 | LSP client (`crates/sex-lsp`) over F2 streams; rust-analyzer, clangd, pyright-free Python LSP packaged in sexshop; completion, hover, go to definition, rename, diagnostics | gate: rust-analyzer completes `Vec::` and reports an error |
| O6 | Git with gitoxide (`gix`, MIT/Apache): gutter, stage hunks, commit, blame, branches, side-by-side diff and 3-way merge view | gate: edit, stage, commit in a fixture repo; `git log` on host agrees |
| O7 | Terminal panel: sex-vt (F1) inside Quil, tasks (build/test) with problem matchers | gate: run `cargo`-free task fixture, error links jump to line |
| O8 | Debugger: debug protocol over PDX plus a **kernel debug mechanism** (read/write memory and registers of a run whose debug grant the person gave, INT3 breakpoints delivered as J4 fault messages). Kernel growth ~150 lines, owned with J. DAP adapter so the same UI drives the Python helper | gate: breakpoint in `stdhello`, step, read a variable |
| O9 | Extensions as Wasm components on sex-wasm (as Zed does): themes, grammars, language-server launchers, commands; installed from sexshop | gate: sample extension adds a command and a theme |
| O10 | Remote editing over SSH (`sex-ssh`, K7): open a folder on another machine, run its language server there | gate: edit a file on the QEMU host over SSH |
| O11 | Assistant panel: chat, inline edit and explain-selection through the Claude API (model configurable, key held by Shibari), off by default | host: request building goldens; gate with a local mock server |
| O12 | Jot polish: restore unsaved tabs after crash, encodings and CRLF/LF, wrap toggle, Markdown preview, print | gate: kill Jot with unsaved text, reopen, text is back |

### P. Spindle terminal and shell (9)

Write set: `apps/term` (from #182; becomes Spindle's terminal window), `crates/sex-vt`,
`crates/sex-pdx/src/stream.rs` (with the F thread), `apps/spindle` command surface.

| # | Slice | Test |
| --- | --- | --- |
| P1 | Tabs and split panes (horizontal/vertical, resize, zoom a pane) | gate: split, run two programs, each pane keeps output |
| P2 | Yarn sessions: a session's streams and scrollback live in a session keeper, so closing the window detaches; `spindle attach` reattaches, also from another window | gate: start a long program, close window, reattach, output continued |
| P3 | Window size reaches attached programs (stream size field read by sex-std and sex-libc `TIOCGWINSZ` emulation); Ctrl+C interrupts a blocked program at once (stream interrupt wakes its wait) | gate: program prints its size, resize changes it; `sleep`-like loop stops within 50 ms |
| P4 | Renderer: ligatures and emoji through sex-shape, glyph atlas cache, smooth scroll, 120 fps on host | host: glyph cache goldens; latency gate under 8 ms per frame on host |
| P5 | kitty graphics protocol and sixel, OSC 8 links, OSC 52 clipboard (through the clipboard broker with consent), OSC 9/777 notifications | gate: fixture prints a PNG via kitty protocol; screenshot matches |
| P6 | Scrollback search (regex), quick-select hints for URLs/paths/hashes, copy mode with keys | gate: search finds 3 matches; hint copies a URL |
| P7 | Themes and config through `sex-config`, live reload, font and padding settings | gate: change theme, terminal repaints without restart |
| P8 | Structured pipelines in Spindle's shell: commands pass typed tables (nushell-like) over F2 streams; byte streams for ported CLIs; background jobs | host: pipeline goldens; gate: `ls | where size > 1mb | sort-by name` |
| P9 | SSH domains: a tab or pane running on another machine via K7 | gate: pane on QEMU host sshd runs `echo` |

### Q. Files, documents and storage apps (15)

Write set: `apps/files` (Loom), `apps/pdf` (Vellum), `apps/view` (Gaze), new `apps/aftercare`,
new `apps/corset`, `crates/sexfs` (snapshots only, with E). Coordinate with E for Files.

| # | Slice | Test |
| --- | --- | --- |
| Q1 | Loom on Weft: mouse, multi-select, rubber band, context menus, drag and drop (C4 protocol) | gate: drag a file onto a folder moves it |
| Q2 | Split view, tabs, places sidebar, breadcrumbs; icons/details/columns views; sort and filter-as-you-type | gate: F3 split, copy across panes |
| Q3 | Preview pane: pictures, PDF first page, video frame, text with highlighting | gate: selecting a PDF shows its page |
| Q4 | Properties with grants: size, dates, type, and which programs hold a grant to it, with revoke | gate: revoke Jot's grant, Jot refused on next read |
| Q5 | Archives: browse zip/tar/7z/zst/xz/gz as folders, extract, create (zip, tar.zst) with progress | gate: create zip, extract, hashes equal |
| Q6 | Network places: SFTP (sex-ssh), WebDAV (sex-http); SMB2/3 later | gate: copy to/from QEMU host SFTP |
| Q7 | Batch rename with preview, duplicate finder, disk-usage map | gate: rename 20 files by pattern |
| Q8 | sexfs snapshots (copy-on-write roots) and "earlier versions" in Properties | gate: overwrite file, restore version, hash equal |
| Q9 | Vellum: continuous and two-page scroll, mouse, text selection and copy | gate: select a line, paste in Jot equals text |
| Q10 | Vellum: annotations (highlight, note, ink) and AcroForm filling saved as incremental PDF updates (lopdf, MIT) | host: saved file reopens with annotations in hayro and on host |
| Q11 | Vellum: presentation mode, dark/invert, print (via Needle) | gate: presentation fullscreen, arrow keys page |
| Q12 | Gaze: AVIF and HEIC (FFmpeg decoders), JPEG XL (jxl-oxide, MIT/Apache), RAW (rawler; licence checked), SVG (resvg); EXIF and auto-orientation | host: decoder goldens; gate: rotated-EXIF JPEG shows upright |
| Q13 | Gaze library: albums, timeline, tags, search; non-destructive edits (crop, straighten, exposure, filters) and set as wallpaper | gate: crop saved as sidecar, original hash unchanged |
| Q14 | **Aftercare**: scheduled, encrypted, deduplicated backups (rustic, MIT/Apache, restic format) to USB or SFTP, browse and restore, system snapshot before updates | gate: back up, delete, restore, hashes equal |
| Q15 | **Corset**: disks and partitions (GPT create/resize/format sexfs/FAT/exFAT/ext4), SMART/NVMe health, encryption setup | gate: partition a blank QEMU disk, format, mount in Loom |

### R. System apps: Tatami, Mesh, sexshop, Voyeur, Abacus, Chastity (10)

Write set: `apps/settings` (Tatami), `apps/monitor` (Mesh), `apps/store` (sexshop), new
`apps/chastity`, `apps/sex-calc` (Abacus), new `crates/sex-pdx/src/search.rs`, launcher search
module in silk-shell (hook only; C owns `main.rs`).

| # | Slice | Test |
| --- | --- | --- |
| R1 | Tatami Privacy and grants page: every program's grants, last use, revoke, and default answers for first-run questions | gate: revoke network from a program, its next connect refused |
| R2 | Tatami Appearance: theme, accent, fonts, icon set, cursor size (drives M6) | gate: dark theme applies to shell and two apps |
| R3 | Tatami pages: Updates (A/B slots, rollback, package updates), Shortcuts editor, Startup programs, Notifications per program, Storage, About | gate: each page round-trips one setting |
| R4 | Mesh graphs: history for CPU per core, memory, disk, network; per-program I/O and network; end and restart (with J1's counters) | gate: busy program shows; end it from Mesh |
| R5 | Mesh capability graph (domains, grants, organs, live), hardware list, temperatures, NVMe SMART, boot timeline | gate: graph lists Jot's file grant after Jot opens a file |
| R6 | sexshop app pages (icon, screenshots, description, grants), categories, update badge, background updates, third-party repos with keys, per-package rollback | gate: second repo's package installs; rollback restores old version |
| R7 | **Chastity** password vault: KDBX4 (keepass-rs, MIT), secrets held by Shibari, TOTP, browser autofill grant for N2 | gate: create vault, add entry, reboot, unlock, entry present |
| R8 | Abacus: units and currency, arbitrary precision (dashu, MIT/Apache), programmer mode, variables, graph plot | host: 200 expression goldens incl. units |
| R9 | Voyeur protocol: search providers are programs answering a query with results only from what they already hold; Shell merges with frecency | host: ranking goldens; gate: two providers answer one query |
| R10 | Voyeur providers: apps, files and contents (E4 index), settings pages, Abacus, windows, browser history/bookmarks, clipboard history, emoji; preview pane and actions | gate: typing `2+2`, a file word and a setting name each return the right first hit |

### S. Silk shell beyond workstream C, and accessibility (8)

Write set: new modules in `servers/silk-shell/src/` (C owns `main.rs`; hooks under 10 lines),
`servers/silkbar`, `servers/sexdisplay/src/scene.rs` (with C), new `apps/reader` (name below).

| # | Slice | Test |
| --- | --- | --- |
| S1 | Window decorations: title bar buttons (close, maximise, float), window menu, double-click maximise, drag to move | gate: click close ends the program |
| S2 | Program tray items (StatusNotifier-like `sex_pdx::tray` items from programs) and notification actions and inline replies | gate: program adds a tray icon with a menu; notification reply reaches it |
| S3 | Tabbed window groups, dwindle layout, per-workspace layout memory | gate: group three windows, cycle tabs |
| S4 | Desktop scripting: a `silk` command in Spindle to list/move/focus windows and subscribe to events; Wasm plugins for layouts | gate: script moves a window to workspace 3 |
| S5 | Desktop widgets (clock, weather from Open-Meteo, system) and hot corners | gate: hot corner opens overview |
| S6 | Shared theme engine with Weft (M6), blur and drop shadows on the damage path | screenshot gate of dark theme with shadows |
| S7 | Screen reader (proposed **Whisper**): reads the AccessKit tree (M8), speech by espeak-ng (GPL-3.0) as a confined program, focus tracking, review keys; Tatami Accessibility page | gate: focusing a button speaks its label (audio buffer contains speech frames; text log equals label) |
| S8 | Magnifier, high contrast, sticky/slow keys, on-screen keyboard | gate: magnifier zooms 2× around pointer |

### T. Office suite: Gingham, Brocade, Burlesque (6)

Write set: new `apps/gingham`, `apps/brocade`, `apps/burlesque`, new `crates/sex-office`.

| # | Slice | Test |
| --- | --- | --- |
| T1 | **Gingham** spreadsheet on IronCalc (MIT/Apache, Rust engine with xlsx): grid on Weft (M3), formulas, formats, multiple sheets, xlsx open/save, CSV | host: IronCalc formula suite; gate: open xlsx fixture, edit, save, reopen on host equal |
| T2 | Gingham: charts, sort/filter, freeze panes, ODS (calamine read, MIT), conditional formats | gate: chart renders from a range |
| T3 | **Brocade** word processor model: rich text (paragraph and character styles, lists, tables, images) on sex-textedit, paged layout with sex-shape | host: layout goldens |
| T4 | Brocade: DOCX open/save (docx-rs, MIT; the old Kaleidoscope importer reused), ODT read, PDF export | host: round trip keeps styles; PDF parses |
| T5 | Brocade: spelling (spellbook), track changes, comments, headers/footers, page numbers, print | gate: tracked insert accepted |
| T6 | **Burlesque** presentations: slides, layouts, text/images/shapes, PPTX/ODP open, presenter view, PDF export | gate: open pptx fixture, present, export PDF |

### U. Communication and reading: Valentine, Flirt, Date, Crush, Bodice, Gossip, Hookup (10)

Write set: new `apps/valentine`, `apps/flirt` (from #216), `apps/calendar` (Date), new
`apps/crush`, `apps/bodice`, `apps/gossip`, `apps/hookup`, `apps/rendezvous`, `apps/swingers`;
new `crates/sex-mail`, `crates/sex-dav`.

| # | Slice | Test |
| --- | --- | --- |
| U1 | **Valentine** mail: IMAP (IDLE), SMTP, JMAP; accounts, folders, threads, search; HTML mail drawn by sex-html with remote content blocked; attachments through Loom | gate: local IMAP/SMTP server fixture, send then receive |
| U2 | Valentine: OpenPGP (rPGP, MIT/Apache) sign/encrypt/verify, OAuth2 sign-in for big providers, offline cache, notifications | gate: encrypted mail round trip |
| U3 | **Flirt** messenger: IRC (TLS, SASL) and the UI on Weft; Talk's TCP/UDP tool becomes a Spindle command | gate: join a local IRC server, exchange messages |
| U4 | Flirt: Matrix with end-to-end encryption (vodozemac, Apache-2.0), rooms, media, notifications | gate: local Matrix test server (conduit) round trip, encrypted room |
| U5 | **Date**: month/week/day views, RRULE recurrence, reminders via Bell, several calendars, CalDAV sync (`sex-dav`), invitations; world clock, alarms, timers | gate: CalDAV fixture sync both ways; alarm fires |
| U6 | **Crush** contacts: CardDAV sync, vCard import/export, used by Valentine, Flirt and Voyeur | gate: CardDAV fixture sync |
| U7 | **Bodice** e-books (EPUB 2/3 via sex-html, CBZ, FB2; library, reading position, fonts, night mode) and **Gossip** feeds (RSS/Atom, podcasts played by Qupid) | gate: EPUB fixture opens at saved page; feed fixture lists items |
| U8 | **Hookup** phone link (KDE Connect protocol, written from its open spec): notifications, clipboard, file send, media control | gate: protocol test peer pairs and sends a file |
| U9 | **Rendezvous** maps: OpenStreetMap vector tiles, search (Nominatim), routing (OSRM API), offline regions | gate: local tile fixture renders; search finds a place |
| U10 | **Swingers** BitTorrent (librqbit, Apache-2.0): magnet links from Qupid/Kaleidoscope, sequential download for streaming into Qupid, per-program bandwidth via Choke | gate: local tracker fixture, file hash equal |

### V. Creative apps: Batik, Gestalt, Strip, Serenade, Flash (7)

Write set: new `apps/batik`, `apps/shapes` (Gestalt, from #216), new `apps/strip`,
`apps/serenade`, `apps/flash`; new `crates/sex-raster` (layers and filters, shared).

| # | Slice | Test |
| --- | --- | --- |
| V1 | **Batik** image editor core: layers, selections, brushes with pressure, undo, PNG/JPEG/WebP/ORA save | gate: draw stroke, save, pixels equal reference |
| V2 | Batik: adjustments and filters (levels, curves, blur, sharpen) non-destructively, text layers, transforms | host: filter goldens |
| V3 | Batik: PSD and XCF import, tablet input (USB HID digitizer in sexusb) | host: PSD fixture layers equal |
| V4 | **Gestalt** vector editor: SVG open/edit/save (usvg/resvg), paths, nodes, text, export PNG/PDF | gate: edit node, saved SVG renders as reference |
| V5 | **Strip** video editor: timeline, cuts, transitions, titles, audio tracks, export through FFmpeg encoders | gate: cut two clips, export, duration and hash of frames correct |
| V6 | **Serenade** audio recorder/editor: record (mic grant), waveform, cut, effects, export | gate: record test tone via QEMU audio, trim, export |
| V7 | **Flash** camera: USB Video Class driver in sexusb, photo and video capture to Loom, camera grant | gate: QEMU `usb-video`-style fixture frames arrive |

### W. Hardware services and guests: Cufflinks, Needle, Lurk, Drag (7)

Write set: new `servers/cufflinks`, `servers/needle`, `apps/lurk`, `servers/sexusb` (BT and
scanner transports only, with the USB owner).

| # | Slice | Test |
| --- | --- | --- |
| W1 | **Cufflinks** Bluetooth: native USB HCI transport in sexusb, Intel firmware load (AX201 on the x17, firmware table row), host stack in a ring-3 server (no BlueZ); scanning and pairing | host: HCI event goldens; x17 receipt |
| W2 | Cufflinks: HID (keyboards, mice) and BLE | x17 receipt |
| W3 | Cufflinks: A2DP audio (SBC, AAC via FFmpeg) into Bell; tray and Tatami page | x17 receipt; host: SBC goldens |
| W4 | **Needle** driverless printing: IPP Everywhere and AirPrint discovery (mDNS), PWG raster/PDF jobs, queue UI; no CUPS | gate: ipp-everywhere test printer receives the job |
| W5 | Needle scanning: eSCL/AirScan driverless scanners | gate: eSCL fixture returns a page |
| W6 | **Lurk** remote desktop: VNC client and server (RFB), RDP client later; screen and input grants | gate: VNC client views the QEMU host test server |
| W7 | **Drag** guests: a VT-x/SVM virtual machine monitor as a confined server running Linux or Windows guests with virtio devices. Needs a kernel mechanism (VMX root, EPT), so it is a **major change for Andreas to approve**; start with a design doc | review only, then gates per phase |

## 5. Priority

1. **First wave (start now):** M (Weft) — every other UI slice benefits; O (Quil, the
   furthest from its leader); N (Kaleidoscope: shields, passwords, isolation, devtools); T1
   (Gingham on IronCalc: the fastest big win, a real spreadsheet in one or two PRs).
2. **Second wave (when M3–M5 land):** Q (Loom, Vellum, Gaze, Aftercare, Corset), R (system
   apps, starting with the Privacy page), P (Spindle tabs/splits/sessions), U1–U5 (mail,
   chat, calendar).
3. **Third wave:** rest of T and U, S (shell extras and accessibility), V (creative), W
   (Bluetooth and printing need x17 receipts; Drag needs Andreas's decision).

## 6. Count

M 10, N 12, O 12, P 9, Q 15, R 10, S 8, T 6, U 10, V 7, W 7 = **106 slices**, in 11
workstreams. All new app names in this document are proposals pending Andreas's OK.
