# Open source SexOS can integrate, adapt or learn from (2026-11-08)

Andreas's ask: SexOS is not POSIX, Linux or Unix, so a lot has to be built from scratch.
Where it doesn't have to be, there is a huge free software library to use. This report
says what to take, in what order, and how it fits the single address space design.

Research only. No code was changed. Versions and project status are as of late 2026 and
from memory plus spot checks; check each crate's current release and licence file before
vendoring.

## 1. The two things that unlock everything else

Before any single crate, two platform steps decide how much of the ecosystem SexOS can use.

### 1a. Give SDK programs a heap (`alloc`) — low effort, very high impact

Today `sex-app` programs are `#![no_std]` with no allocator: `peek` holds a page in a
`static mut [u8; 65536]`. Some servers (Quil, Linen, sexstore) already set a
`#[global_allocator]`. Almost every useful `no_std` crate below needs `alloc`
(`Vec`, `String`, `Box`): font shaping, PNG/JPEG, tiny-skia, rustls, wasmi, Taffy.

Do: a `sex_app::heap` with a `#[global_allocator]` over memory the program's capability
grants, growable by asking `sext` for more frames. `talc` (MIT, `no_std`, fast) or the
existing `linked_list_allocator` work. This is a day or two and turns "write it from
scratch" into "add a dependency" for most of this report.

### 1b. A Rust `std` target for SexOS — medium/high effort, strategic

`std` in Rust is not POSIX. It has a per-platform layer (`library/std/src/sys/pal/*`)
that maps threads, time, files, sockets and the allocator to whatever the OS offers.
Redox (`x86_64-unknown-redox`), Hermit (`x86_64-unknown-hermit`), Motor OS, Xous,
Fuchsia and UEFI all have one. A `x86_64-unknown-sexos` target with files over sexfs,
TCP over sexnet, threads over SexOS tasks and time over `sex-time` would let SexOS run
`std` crates unchanged: html5ever, Blitz, Boa, Symphonia, regex, serde_json, image-rs,
egui, Servo eventually.

It stays honest about the design: `std::fs::File::open` can resolve through the
program's own folder capability; no Unix process model is implied. Unsupported calls
return `Unsupported`, as on UEFI.

Learn from: Hermit's PAL (smallest and cleanest), Motor OS (Rust-first microkernel
with std), Redox (largest). Start as an out-of-tree fork of `std` built with
`-Zbuild-std`, like the existing `x86_64-sex.json` custom target.

### 1c. Later: a C toolchain plus a libc for the Guests organ

For C libraries with no Rust equivalent (FFmpeg, QuickJS, dav1d, libopus, SQLite, Mesa),
a C ABI layer is needed. Redox's `relibc` (MIT, written in Rust) is the closest fit and
there is already an empty `relibc/` placeholder here. Run such code in the Guests organ
(see `PROGRAM_SLOTS_DESIGN_2026_11_07.md`), so a C guest never shares a key with Core.
This is the "POSIX compat as an app" step of the approved plan, not a change to the core.

## 2. Ranked shortlist (impact vs effort, fit with SASOS)

| # | What | Crate / project | Licence | `no_std`? | Impact | Effort |
|---|---|---|---|---|---|---|
| 1 | Heap for SDK programs | `talc` | MIT | yes | unlocks most of this list | S |
| 2 | Runtime font rendering at any size | `skrifa` + `read-fonts` (Google fontations) or `ttf-parser`, raster by `zeno`/`fontdue` | MIT/Apache | yes (+alloc) | real typography, any font | S–M |
| 3 | Text shaping (ligatures, kerning, scripts) | `harfrust` (HarfBuzz's official Rust port, successor of `rustybuzz`) | MIT | yes (+alloc) | correct text in every language | S |
| 4 | 2D canvas for window pixel buffers | `tiny-skia` | BSD-3 | yes (+alloc, `libm`) | anti-aliased shapes, gradients, rounded windows, shadows | S |
| 5 | Image viewer + browser images | `zune-png`, `zune-jpeg` (+`miniz_oxide`) | MIT/Apache/Zlib | yes (+alloc) | PNG/JPEG everywhere | S |
| 6 | QEMU devices fast and complete | `virtio-drivers` (rcore-os) | MIT | yes | virtio blk/net/gpu/input/sound in days | S–M |
| 7 | TLS 1.2 + 1.3, more ciphers | `rustls` (no_std since 0.23) with `rustls-rustcrypto` or `graviola` provider | Apache/MIT/ISC | yes (+alloc) | opens most of the web | M |
| 8 | Wasm guests (third-party apps, in-organ isolation) | `wasmi` now, `wasmtime` (min-platform/no_std) later | MIT/Apache; Apache-2.0+LLVM | yes | safe untrusted programs without a hardware key each | M |
| 9 | Layout engine for the browser and the UI | `taffy` (flexbox, grid, block) | MIT | yes (+alloc) | real CSS layout | M |
| 10 | Rust `std` target | fork of `std` PAL | MIT/Apache | — | unlocks the std ecosystem | L |
| 11 | HTML/CSS renderer | Blitz (html5ever + Stylo + Taffy + Parley) | MIT/Apache (Stylo MPL-2.0) | needs std | real web pages with CSS | M after #10 |
| 12 | JavaScript | QuickJS-NG (C) or Boa (Rust) | MIT; MIT/Unlicense | needs libc / std | scripts on pages | M–L |
| 13 | Audio decode | Symphonia | MPL-2.0 | needs std | MP3/AAC/FLAC/Vorbis/WAV | S after #10 |
| 14 | Video | `rav1d` (AV1), openh264 / FFmpeg in Guests | BSD-2; BSD-2; LGPL/GPL | std / libc | video playback | L |

S = days, M = 1–3 weeks, L = a month or more.

Suggested order: 1 → 2+3 → 4 → 5 → 6 → 7 → 9 → 8 → 10 → 11 → 13 → 12 → 14.
The first five turn the desktop from "text grid" to "looks like a modern OS" with only
`no_std` crates and no platform change.

## 3. By area

### Fonts and text

- **Parsing and outlines:** `skrifa`/`read-fonts` (Google, used in Chrome; MIT/Apache;
  `no_std`; variable fonts and TrueType hinting) is the best long-term choice.
  `ttf-parser` (MIT/Apache, zero-allocation, `no_std`) is the simplest.
- **Rasterizing:** `zeno` (MIT/Apache, `no_std`), `fontdue` (MIT/Apache/Zlib,
  `no_std`+alloc, very fast), `ab_glyph_rasterizer` (Apache-2.0). `swash` (MIT/Apache)
  bundles scaling, hinting and colour emoji.
- **Shaping:** `harfrust` (MIT). Needed for ligatures in Geist Mono, Arabic, Indic, emoji
  sequences.
- **Line breaking, bidi, segmentation:** `unicode-linebreak`, `unicode-bidi`,
  `unicode-segmentation` (MIT/Apache, `no_std`), or ICU4X (Unicode-3.0, `no_std`) for
  full internationalisation.
- **Layout:** `parley` (Linebender, MIT/Apache) and `cosmic-text` (MIT/Apache, has a
  `no_std` feature) do paragraphs, selection and cursor movement; use one for Jot and the
  browser rather than writing an editor text engine.
- **Today's approach:** `scripts/make_font.py` pre-renders Geist into a coverage table at
  one size. Moving rasterization to runtime with a glyph cache per organ (shared read-only
  pages, which a single address space makes cheap) gives every size, zoom and font.
- **Fonts to ship (all free):** Geist / Geist Mono (OFL, already in), Inter (OFL),
  Noto Sans/Serif/CJK and Noto Color Emoji (OFL), JetBrains Mono, IBM Plex, Source Sans/
  Serif/Code, Fira (all OFL), DejaVu (Bitstream Vera licence). Twemoji (CC-BY 4.0 art,
  MIT code) for emoji.

### 2D graphics, compositing and GUI

- **`tiny-skia`** (BSD-3): Skia's CPU rasterizer subset in pure Rust; paths, strokes,
  gradients, patterns, anti-aliasing, blending. The right base for per-window pixel
  buffers (plan step 2) and window decorations.
- **`vello_cpu` / `vello_hybrid`** (Linebender, MIT/Apache): newer, faster CPU/hybrid
  renderers from the Vello team; watch, adopt when stable.
- **`embedded-graphics`** (MIT/Apache, `no_std`, no alloc): fine for boot screens and the
  panic screen.
- **Geometry:** `kurbo` (curves), `peniko` (brushes), `lyon` (tessellation), all MIT/Apache.
- **Vector icons:** `resvg`/`usvg` (MIT/Apache) render SVG; rasterize icon sets at build
  time first, at runtime after #10. Icon sets: Lucide (ISC), Phosphor (MIT), Tabler (MIT),
  Material Symbols (Apache-2.0).
- **Toolkits:**
  - **Slint** — licence choice of GPL-3.0, royalty-free or commercial; GPL-3.0 fits
    SexOS's own licence. Has a `no_std` software renderer built for microcontrollers,
    a declarative UI language and a designer. The fastest way to good-looking Settings,
    Files and dialogs. Strong candidate for the SDK's widget layer.
  - **egui** (MIT/Apache): immediate mode, outputs triangles; needs `std`-ish support in
    practice. `crates/egui-hello` and `apps/cosmic-*` are placeholder stubs outside the
    workspace today.
  - **Xilem/Masonry, iced, libcosmic** (MIT/Apache; libcosmic MPL-2.0): `std` heavy,
    good after #10, good to learn widget design from.
- **Compositor design to learn from:** Fuchsia Scenic/Flatland (BSD), Haiku's app_server
  (MIT), Redox Orbital (MIT), SerenityOS WindowServer (BSD-2).

### Images

- PNG/JPEG: `zune-png`, `zune-jpeg` (MIT/Apache/Zlib, `no_std`+alloc, fast, fuzzed).
  `miniz_oxide` (MIT/Apache/Zlib, `no_std`) for DEFLATE, also needed for gzip HTTP.
- GIF: `gif` crate (MIT/Apache). WebP: `image-webp` (MIT/Apache, pure Rust).
  QOI: `qoi` (MIT/Apache). JPEG XL: `jxl-oxide` (MIT/Apache). AVIF: `rav1d` decode +
  `avif-parse` (MPL-2.0, a fork of Mozilla's mp4parse).
- After #10, `image` (image-rs, MIT/Apache) wraps them all.
- Scaling: `fast_image_resize` (MIT/Apache).

### Audio

- Decoders: Symphonia (MPL-2.0; MP3, AAC, ALAC, FLAC, Vorbis, WAV, MKV/MP4 demux).
  Smaller `no_std`-friendly picks: `claxon` (FLAC, Apache-2.0), `lewton` (Vorbis,
  MIT/Apache). Opus: libopus (BSD-3, C) via the Guests organ until a mature Rust decoder
  exists.
- Resampling and mixing: `rubato` (MIT), `dasp` (MIT/Apache).
- Drivers: Intel HDA spec is public; Redox `ihdad` (MIT) is a readable Rust HDA driver;
  `virtio-drivers` covers virtio-sound for QEMU first.

### Video and FFmpeg

- Containers: `mp4parse` (Mozilla, MPL-2.0, used in Firefox), `matroska-demuxer`
  (MIT/Apache).
- AV1: `rav1d` (BSD-2), the Rust port of dav1d, near dav1d's speed.
- H.264: Cisco openh264 (BSD-2) or FFmpeg's decoders.
- FFmpeg itself: LGPL-2.1+ by default, GPL with some options. Both are compatible with
  SexOS's GPL-3.0. Needs a libc (#1c) and is the natural first big guest. Run it in the
  Media organ behind a typed "decode this stream into this buffer" capability.
- GPU video decode (VA-API style) is much later and hardware specific.

### HTML, CSS and JavaScript

Recommended path, cheapest first:

1. **Now (`no_std`):** keep the text reader, add a tolerant tokenizer and a small CSS
   cascade, lay out with `taffy`, draw with tiny-skia and the font stack, show images with
   zune. A readable "reader mode" browser with real typography and images.
2. **After the `std` target:** **Blitz** (DioxusLabs, MIT/Apache) is a lightweight
   renderer made of Servo's Stylo (MPL-2.0) + html5ever + Taffy + Parley + Vello. Real CSS
   without all of Servo. The single best fit for SexOS's browser.
3. **JavaScript:** QuickJS-NG (MIT, small C, ES2023, easy to port once a libc exists) is
   the quickest. Boa (MIT/Unlicense, Rust, high Test262 conformance) is the pure-Rust
   path after #10. Nova (Rust, data-oriented) is one to watch.
4. **Long term:** Servo (MPL-2.0) as the full engine once `std`, threads and a GPU or fast
   CPU renderer exist.

Also worth knowing: litehtml (BSD-3, C++) is a tiny HTML/CSS layout library; Ladybird
(BSD-2, C++) is the best example of a from-scratch engine to learn from. NetSurf's core
is GPL-2.0, see licence notes.

### Networking and TLS

- `smoltcp` (0BSD) is already in. Keep it.
- **`rustls`** supports `no_std`+alloc since 0.23. With a pure-Rust provider
  (`rustls-rustcrypto`, still alpha, or `graviola`, fast on x86_64) it replaces
  `embedded-tls` and adds TLS 1.2, more cipher suites and session resumption, which the
  alpha table lists as gaps. `webpki-roots` and `rustls-webpki` are already used.
- HTTP: `reqwless` (MIT/Apache, `no_std`) for a small client; `httparse` (MIT/Apache,
  `no_std`) for parsing. HTTP/2 (`h2`) and QUIC (`quinn`) after #10.
- DNS: hickory-dns (MIT/Apache) after #10; own small resolver is fine until then.
- Wi-Fi is hard everywhere. Wired + virtio first; later learn from OpenBSD's drivers
  (ISC/BSD) and use vendor firmware blobs, which are redistributable.

### Filesystems and storage

- sexfs stays SexOS's own. For interchange with other OSes: `fatfs` (MIT, `no_std`) for
  USB sticks and the EFI partition, `ext4-view` (MIT/Apache, `no_std`) to read Linux
  disks, an ISO 9660 reader for install media.
- Compression and checksums: `lz4_flex` (MIT), `ruzstd` (MIT, zstd decoder), `blake3`
  (CC0/Apache), `crc32fast` (MIT/Apache), all `no_std`-capable.
- Databases for apps: `redb` (MIT/Apache, embedded key-value), SQLite (public domain, C)
  in Guests, or Turso/Limbo (MIT, pure-Rust SQLite rewrite) after #10.
- Designs to learn from: RedoxFS (MIT), ZFS/bcachefs ideas (COW, checksums), Twizzler's
  persistent objects (see OS section).

### Drivers

- `virtio-drivers` (MIT, `no_std`): blk, net, gpu, input, sound, console. Lets QEMU run
  with fast paravirtual devices and gives sound almost for free.
- rust-osdev crates (MIT/Apache): `x86_64`, `acpi`, `aml`, `pci_types` (already in),
  `xhci` (register definitions), `uefi`, `pc-keyboard`.
- Redox drivers (MIT): `e1000d`, `rtl8168d`, `nvmed`, `ahcid`, `xhcid`, `ihdad`,
  `ps2d`. MIT means code can be adapted with attribution. Best single source for real
  hardware (Realtek and Intel NICs, NVMe, HDA).
- ixy / ixy.rs (MIT/BSD): small user-space NIC drivers, written to be read.
- Linux drivers are GPL-2.0-only: read datasheets and behaviour, but don't copy code
  (see licence notes). Genode's approach (wrap Linux drivers in a small emulation
  environment, "DDE") is the escape hatch for Wi-Fi and GPUs later, inside a Guests-like
  organ.
- Display on real hardware: UEFI GOP framebuffer first. Native Intel/AMD GPU is a long
  project; Mesa (MIT) is the eventual userspace.

### Wasm runtimes (fit with "organs")

The organs design wants many tasks per hardware key with inner isolation from Rust or
sandboxed runtimes. Wasm is that sandbox:

- **`wasmi`** (MIT/Apache, `no_std`+alloc): interpreter, used by Polkadot; small, easy.
  Start here for the Guests organ.
- **`wasmtime`** (Apache-2.0 with LLVM exception): supports `no_std` via its
  "min-platform" embedding; Cranelift JIT or the Pulley interpreter. Use for speed later.
- **WAMR** (Apache-2.0, C): AOT, small; option once a C toolchain exists.
- **WASI preview 2 / component model** as a stable ABI for third-party programs: typed
  interfaces map well onto SexOS capabilities (a WASI `filesystem` handle is a folder
  capability). This also gives SexOS thousands of existing programs compiled to WASI.

### Everyday building blocks (all `no_std`-capable unless noted)

- Serialisation for IPC: `postcard` (MIT/Apache); `rkyv` (MIT) zero-copy archives fit
  shared-memory IPC in a single address space particularly well.
- `serde`/`serde_json` (alloc), `heapless` (already in), `hashbrown`.
- Terminal emulation: `vte` (MIT/Apache, `no_std` parser), `alacritty_terminal`
  (Apache-2.0, std) for a full terminal model.
- Text buffers: `ropey` (MIT) for Jot.
- Markdown: `pulldown-cmark` (MIT). PDF viewing: hayro (Linebender, MIT/Apache, pure Rust).
- Time and zones: `jiff` (MIT/Unlicense), IANA tzdb (public domain).
- Regex: `regex-automata` (MIT/Apache, alloc).
- Crypto: RustCrypto (already in), `ed25519-dalek` (BSD-3) for signed installs.

## 4. OS projects to learn from

| Project | Licence | What SexOS should take |
|---|---|---|
| **Theseus** (Rust) | MIT | Closest relative: single address space, single privilege level, Rust-enforced isolation, live evolution of crates, "state spill" freedom. Read their papers on cells and fault recovery. |
| **Twizzler** (Rust) | BSD-3 | Single-address-space-style global object IDs and persistent memory, invariant pointers. Ideas for sexfs objects and shared data. |
| **Redox** (Rust) | MIT | Drivers, relibc, `std` port, Orbital, package tooling. Most code SexOS can directly adapt. |
| **Hubris** (Rust, Oxide) | MPL-2.0 | Static task tables fixed at build time (like boot-only capability genesis), supervisor that restarts tasks, IPC leases. |
| **seL4 / Microkit** | GPL-2.0 kernel, BSD userland | Capability derivation and revocation done right; proofs as the gold standard. |
| **Fuchsia / Zircon** | BSD/MIT/Apache | Handles and rights, VMOs, FIDL typed IPC, component framework, Scenic compositor. |
| **Genode / Sculpt OS** | AGPL-3.0 or commercial | Proof a capability microkernel OS can be a daily driver; Linux driver reuse via DDE; per-component resource trading. |
| **Singularity / Midori** (papers) | — | Language-based isolation in one address space, contract-typed channels. Directly relevant to organs. |
| **Haiku** | MIT | A coherent, fast desktop built from scratch; Media Kit and translators (codec plugin design). |
| **SerenityOS / Ladybird** | BSD-2 | How a small team builds an entire coherent desktop and web engine; LibGfx, LibWeb, LibJS design. |
| **Asterinas** (Rust) | MPL-2.0 | "Framekernel": tiny unsafe core, safe rest; Linux ABI as a compatibility layer. |
| **Tock** (Rust) | MIT/Apache | Grants, capsules, language isolation inside one privilege level. |
| **Hermit, Motor OS** (Rust) | MIT/Apache | How to add a new OS to Rust `std` (#1b). |
| **Managarm** | MIT | Async microkernel that runs a real desktop with Wayland. |

## 5. Licence notes (SexOS is GPL-3.0-only)

- **Compatible, can be combined and shipped inside SexOS:** MIT, BSD-2/3, ISC, Zlib, 0BSD,
  Apache-2.0 (compatible with GPLv3, not GPLv2), MPL-2.0 (file-level copyleft, explicitly
  GPL-compatible), LGPL-2.1+/3, GPL-2.0-or-later, GPL-3.0, Unicode-3.0, CC0, public domain.
  Keep each project's licence and copyright notices, e.g. in `vendor/` and an
  `assets/licences` listing.
- **Fonts under SIL OFL 1.1:** fine to bundle. Can't sell the fonts on their own; if a
  font is modified and it has Reserved Font Names, the modified version needs a new name.
- **Not compatible, read only:** GPL-2.0-only code (the Linux kernel and its drivers, some
  older projects; NetSurf's browser core). Don't copy code from it into SexOS; specs,
  datasheets and observed behaviour are fine.
- **AGPL-3.0 (Genode):** GPLv3 allows combining with AGPLv3, but the AGPL part keeps its
  network-use obligations. Learn from it rather than import it.
- **Firmware blobs:** separately licensed per vendor, redistributable as data, not code.
- **A decision for Andreas later, not now:** with the SDK under GPL-3.0, every program
  built on `sex-app` must be GPL-compatible. If SexOS wants proprietary or permissively
  licensed third-party apps (as Linux allows via the syscall boundary), the SDK crates
  would need a permissive licence or a linking exception. Wasm guests sidestep this,
  since they talk to SexOS only through an interface.

## 6. What this buys, in plain terms

- **Weeks 1–2:** heap for programs, runtime fonts with shaping, tiny-skia windows with
  rounded corners and shadows, PNG/JPEG viewer. SexOS starts to look modern.
- **Weeks 3–6:** virtio devices and sound in QEMU, rustls (most HTTPS sites), Taffy
  layout plus images in the browser, Slint-based Settings and Files, wasmi guests.
- **Months 2–4:** `std` target, Blitz browser with real CSS, Symphonia music player,
  QuickJS or Boa, relibc and the first C guests (FFmpeg, SQLite).
- Throughout: adapt Redox drivers for real hardware when the PKU PC arrives.

## 7. Lessons from browser projects

### What each one teaches

| Project | Licence | Lesson for SexOS |
|---|---|---|
| **Ladybird** (C++) | BSD-2 | A from-scratch engine that renders the real web, built by a small team by working through Web Platform Tests and Test262 one failure at a time. Splits into WebContent, RequestServer and ImageDecoder processes: the decoder that parses hostile images runs alone with no other rights. SexOS can copy that split as separate tasks with typed capabilities. |
| **Servo** (Rust) | MPL-2.0 | Parallel style and layout (Stylo), html5ever, a "constellation" that owns one pipeline per site. Its crates (Stylo, html5ever, cssparser, selectors, url) are what Blitz builds on. Embeddable since 2025. |
| **Chromium** | BSD-3 | Site isolation: each site gets its own sandboxed renderer. V8 uses **PKU** to keep JIT code write-protected. SexOS already has PKU, so it can give JIT engines the same protection natively. |
| **Firefox / Gecko** | MPL-2.0 | **RLBox**: C libraries (fonts, XML, codecs) compiled to Wasm and run inside the browser in a sandbox. SexOS can do the same with any C library, with no native libc. |
| **WebKit** | LGPL/BSD | JavaScriptCore's tiered JITs; WebKit runs on many small platforms, which is a reference for a minimal porting layer. |
| **Blitz** (Rust) | MIT/Apache | A lightweight HTML/CSS engine assembled from Servo parts. The best ready-made engine for SexOS once `std` exists (section 1b). |
| **Dillo** (C) | GPL-3.0+ | A tiny, fast browser with no JS. Its licence is compatible with SexOS's, and it shows how much a small engine can render. |
| **NetSurf** (C) | GPL-2.0 core, MIT libs | Its CSS (libcss) and HTML (hubbub) libraries are MIT and usable; the browser itself is GPL-2.0 and should only be read. |
| **litehtml** (C++) | BSD-3 | HTML/CSS layout only; the host draws. Shows the smallest useful engine interface. |
| **"Let's build a browser engine"** (Robinson, Rust) and **browser.engineering** (book) | MIT/Apache; free online | How to write tokenizer, cascade, layout, painting and JS bindings step by step. Good training for whoever works on the SexOS browser. |

### Design rules to take

1. **Split the browser into tasks inside a Browser organ:** network (only the network
   capability), image and media decoders (no rights at all, just "bytes in, pixels out"),
   one content task per site, and the window/compositor side. A crash or exploit in one
   costs one tab, and Display never sees untrusted code.
2. **Hostile parsers get nothing:** every decoder of web data runs where a bug can't reach
   files, network or other sites. Wasm (RLBox style) is the cheap way to get that inside
   the organ without spending a hardware key per tab.
3. **Measure with real test suites:** Web Platform Tests (BSD-3), Test262 (BSD) and the
   html5lib tests. They are behaviour tests, which matches the plan to replace
   source-text gates.
4. **Zero-copy pixels:** content tasks paint into a per-window pixel buffer that the
   compositor maps read-only, which a single address space makes free.

## 8. Lessons from media players and media frameworks

| Project | Licence | Lesson for SexOS |
|---|---|---|
| **mpv** | GPL-2.0+ or LGPL-2.1+ build | Clean layers: demux → decode → filter → video out, with audio as the master clock and display-synced playback. libmpv's render API lets a host draw video into its own surface. libplacebo (LGPL-2.1) does high-quality scaling, colour management and HDR. |
| **VLC / libvlc** | LGPL-2.1+ | Everything is a plugin (access, demux, codec, output); plays anything because each format is one module. |
| **GStreamer** | LGPL-2.1 (many Rust plugins MIT/MPL) | Media as a graph of typed elements with negotiated formats. Maps well onto SexOS tasks connected by typed capabilities. |
| **PipeWire** | MIT | Graph-based audio and video routing in shared memory, low latency, per-client permissions. The right model for a SexOS sound service. |
| **FFmpeg** | LGPL-2.1+ / GPL | libavformat (containers), libavcodec (codecs), libswscale/libswresample. Everyone builds on it. |
| **Haiku Media Kit** | MIT | A native OS media API with codec "add-ons" system-wide, so every app plays every format. |
| **Android media / Stagefright** | Apache-2.0 | The warning: media parsers caused a flood of remote exploits, so Android moved extractors and codecs into separate sandboxed processes. |
| **Rust players** (`rodio`, `cpal`, Symphonia, Ruffle) | MIT/Apache, MPL-2.0 | Pure-Rust playback and decoding; Ruffle shows a complete media runtime in Rust on wgpu. |

Design rules to take:

1. **One system media service in a Media organ** with a typed API ("open this file or
   stream, decode into this buffer, play on this output"), so every program plays every
   format, Haiku style.
2. **Decoders sandboxed:** native Rust decoders inside the organ; C codecs (FFmpeg, libvpx,
   libopus) compiled to Wasm and run there, RLBox style, until a native libc guest exists.
3. **Audio clock is master,** video frames are dropped or repeated to match it (mpv).
4. **Pull-model audio through shared ring buffers** (PipeWire, CoreAudio): the mixer pulls
   from each client's buffer, no copies.

## 9. How to get HTML + JS + three.js running on SexOS

three.js needs five things: a modern JavaScript engine (ES2020+ with modules), a `<canvas>`
element, `requestAnimationFrame`, **WebGL 2** (WebGL 1 support was dropped in r163) or
**WebGPU** (its newer renderer), plus `fetch`/image decoding for textures and models. A full
HTML engine is not needed to run it. That gives a staircase where each step is usable:

### Step A: a JavaScript runtime with a canvas, no HTML (weeks)

Like Node or Deno with only a window. A SexOS app `js` that runs a script with these
globals: `console`, timers, `requestAnimationFrame`, `fetch` (over sexnet with `+net`),
`TextEncoder`, `URL`, a minimal `document.createElement('canvas')` and Canvas 2D.

- **Engine:** QuickJS-NG (MIT, ES2023, small C). Two ways to run it:
  - compile it to **Wasm with wasi-sdk** (Apache/MIT) and run it under wasmi/wasmtime.
    This needs no native libc on SexOS at all, so it can happen right after the Wasm
    runtime lands; or
  - native via relibc later, for speed.
  Boa (Rust, MIT/Unlicense) is the pure-Rust alternative once `std` exists.
- **Canvas 2D:** map the Canvas API onto tiny-skia (paths, fills, strokes, gradients,
  `drawImage` with zune-decoded images, text with the font stack).
- **Web API code to borrow:** Deno's implementations of URL, streams, `fetch`, `TextEncoder`
  and events are MIT; many are written in JS and run on any engine.

Result: 2D canvas demos and games, and a scripting language for SexOS itself.

### Step B: WebGL 2 in software, then three.js (1–3 months)

Two routes. Recommend the first.

1. **Pure Rust (fits SexOS best):** `naga` (wgpu's shader translator, MIT/Apache) parses
   GLSL ES 3.0 shaders. Compile them to native code with Cranelift (Apache-2.0+LLVM
   exception, already inside wasmtime) or run them through a simple interpreter first. A
   tile-based, multi-core triangle rasterizer (like `euc`, MIT/Apache, which runs shaders as
   Rust closures) draws into the window's pixel buffer. Implement the WebGL 2 calls three.js
   actually uses (buffers, VAOs, textures, framebuffers, draw calls) rather than all of
   OpenGL ES.
2. **Port the C++ stack:** ANGLE (BSD-3, what Chrome uses to turn WebGL into Vulkan) on top
   of SwiftShader (Apache-2.0, Chrome's CPU Vulkan) or Mesa's lavapipe (MIT). Proven and
   complete, but needs a C++ toolchain and runtime first, so it's months later.

Then three.js (MIT) runs unmodified: `new THREE.WebGLRenderer({ canvas })`. A software
renderer on 8+ cores manages simple scenes at interactive rates; heavy scenes need the GPU.

### Step C: the same JS inside real web pages (after `std`)

Blitz (HTML/CSS) plus the Step A engine plus DOM bindings gives pages with scripts. DOM
in Rust can follow Servo's `script` crate design. A shortcut for prototyping: run a DOM
written in JS (linkedom or happy-dom, both MIT) inside the engine and sync only the
rendered tree to Blitz. Events, forms, `fetch`, `<img>`, `<video>` (from the Media
service) and `<canvas>` with Step B's WebGL then make sites like three.js examples load
as pages.

### Step D: speed (long term)

- **A JIT:** V8 (BSD-3) or SpiderMonkey (MPL-2.0). SexOS needs a "JIT memory" capability:
  pages a task may write, then flip to executable, never both. PKU makes that cheap,
  exactly as V8 already uses it on Linux. Until then, V8's `--jitless` mode or Hermes (MIT,
  no JIT) are options.
- **WebGPU:** `wgpu` (MIT/Apache, the WebGPU implementation in Firefox and Deno) on Vulkan.
  `deno_webgpu` (MIT) shows how to bind it to JS. three.js's WebGPURenderer then works.
- **GPU:** in QEMU, virtio-gpu with Venus (Vulkan) or virgl (OpenGL) forwards to the host
  GPU through Mesa's drivers (MIT); on real hardware, Mesa's radv (AMD) and anv (Intel)
  Vulkan drivers, which need a DRM-like kernel interface of SexOS's own design. This is
  the biggest single project in this report; software rendering keeps everything working
  until it lands.

### What to build first

1. Wasm runtime + wasi-sdk (also unlocks C libraries without a libc).
2. QuickJS-NG on Wasm → `js` app with console, timers, `fetch`.
3. Canvas 2D on tiny-skia → 2D demos run.
4. naga + software WebGL 2 subset → three.js "spinning cube" example runs in a window.
5. Then Blitz pages with scripts, and a JIT, as section 1b and Step D land.

Sources spot-checked for current status: [harfrust on docs.rs](https://docs.rs/crate/harfrust/0.13.2),
[wasmtime min-platform example](https://sources.debian.org/src/rust-wasmtime/26.0.1%2Bdfsg-3/examples/min-platform/README.md),
[wasmtime platform support](https://docs.rs/crate/wasmtime-cli/45.0.0/source/docs/stability-platform-support.md),
[Blitz](https://github.com/DioxusLabs/blitz),
[rustls-rustcrypto](https://docs.rs/rustls-rustcrypto/latest/src/rustls_rustcrypto/lib.rs.html).
