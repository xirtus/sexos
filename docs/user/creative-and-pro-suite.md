# Creative & Professional Suite

To achieve parity with class-leading professional creative software on Linux, macOS, and Windows, SexOS is establishing a dedicated suite of high-performance creative and productivity tools. In accordance with `docs/DESIGN_GUARDRAILS.md` §6 and `docs/handoff/APP_NAMES_2026_11_14.md`, every application features a strictly themed canonical identity.

---

## Office & Document Production

| Application | Conceptual Role | Theme & Heritage | Description |
|---|---|---|---|
| **Assuit** | Word Processor | Fabric (Egyptian metal-thread embroidery; Andreas's pick) | Full-featured rich-text word processor with document styles, typography layout, headers/footers, and PDF/DOCX import and export. |
| **Spreadeagle** | Spreadsheet Suite | Romance / Pun (Andreas's pick, PR #219) | Grid calculation engine with mathematical and financial formulas, cell formatting, charts, and multi-sheet workbooks. |
| **Burlesque** | Presentations | Romance / Performance (stage show) | Slide deck editor and presentation tool with vector layouts, smooth transitions, and presenter display support. |

---

## Digital Audio & Visual Arts

| Application | Conceptual Role | Theme & Heritage | Description |
|---|---|---|---|
| **Polyamour** | 3D Creation Suite | Romance (many polygons, many loves; Andreas's pick, PR #235) | Full 3D modeling, mesh editing, texturing, rigging, animation, and hardware-accelerated raytracing engine (Blender class). |
| **Splicy** | Video Editor | Film / Pun (splicing film; Andreas's pick, PR #232) | Multitrack non-linear video editing suite with timeline trimming, transitions, title cards, and multi-stream audio mixing. |
| **Moan** | Digital Audio Workstation (DAW) | Audio / Pun (Andreas's pick, PR #244) | Frame-accurate audio multitrack recording and editing workstation (Logic Pro / REAPER class) supporting soft synths, plugins, and mastering. |
| **Serenade** | Audio Recorder | Romance (a song of devotion) | Minimalist high-fidelity audio capture utility for voice memos, podcasts, and acoustic instrument recordings. |
| **Batik** | Raster Painting | Fabric (wax-resist cloth dyeing) | Digital painting and image retouching suite with pressure-sensitive brush engines, layer blending modes, and color grading. |
| **Flash** | Camera Capture | Photography / Pun | Direct webcam and video input preview and recording utility supporting USB Video Class (UVC) devices. |

---

## Reading & Information Discovery

| Application | Conceptual Role | Theme & Heritage | Description |
|---|---|---|---|
| **Bodice** | E-Book Reader | Romance / Fabric (PR #230) | Distraction-free digital book reader supporting EPUB and comic book archive (CBZ) formats with custom typography and night mode. |
| **Gossip** | Feeds & Podcasts | Information / Pun (PR #230) | RSS and Atom news aggregator and podcast manager with background episode downloads and playback sync. |

---

## System Engineering & Virtualization

| Application | Conceptual Role | Theme & Heritage | Description |
|---|---|---|---|
| **Drag** | Virtual Machines | Pun / Disguise (PR #241) | Hardware-accelerated hypervisor monitor (Intel VT-x / AMD-V) enabling seamless execution of unmodified Linux and guest operating systems within confined SexOS organs. |
| **Corset** | Disk & Partition Manager | Fabric (shapes the body; UI of BDSM) | Graphical storage disk partitioner, filesystem formatter, RAID array manager, and volume health inspector. |
| **Aftercare** | Backup & Recovery | Recovery / Care | Automated and manual backup tool creating atomic, differential **sexfiles** snapshots with one-click full system rollback. |
| **Climax** | Torture & Benchmark | Performance / Pun (`apps/bench`) | High-intensity CPU, memory, and GPU hardware stress testing and thermal throttling benchmarking suite. |
| **Leather** | Developer SDK | Fabric / Tough gear (Andreas's pick) | Software Development Kit (SDK) and build toolchain for compiling, sandboxing, and cryptographically signing native SexOS packages. |

---

## Shared Architecture

All creative applications in SexOS share the same foundational design:
1. **The Weft Retained Toolkit**: Unified widgets, consistent typography via `sex-font`, and shared dark/light accent theming from **[Tatami](apps/tatami.md)**.
2. **Standardized Tab Model**: Tabs across Assuit, Spreadeagle, Polyamour, and Splicy follow identical keyboard navigation (`Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`) with tear-off and window docking.
3. **Collar Sandbox Confinement**: Media files are imported through zero-copy capability grants; creative tools cannot access your system partition or network without explicit authority.
