# Burlesque (Keynote-Class Presentations)

> **Identity Note**: **Burlesque** (`apps/burlesque`, `/programs/burlesque`) is the Keynote-class presentation suite for SexOS, named in keeping with the cabaret, theatrical romance, and fabric aesthetic of the operating system.

**Burlesque** delivers desktop presentation authoring and live delivery on SexOS. Built on the `#![no_std]` `burlesque-core` engine and the Weft windowing system, Burlesque provides 6 designer themes, live animated slide transitions, editable speaker notes, dual-pane Presenter View with timers, memory-bounded OOXML (`.pptx`) and OpenDocument (`.odp`) import/export, and vector PDF-1.4 generation.

![Burlesque Themes](../assets/burlesque-themes.png)
*Burlesque authoring interface showcasing Keynote-class typography, slide layouts, and designer palettes.*

---

## Launching Burlesque

- **From Spindle**:
  ```text
  run burlesque                     # Open with default blank presentation
  run burlesque @presentation.pptx  # Open OOXML PPTX presentation with custody grant
  run burlesque @deck.odp           # Open OpenDocument ODP presentation
  run burlesque @talk.md            # Open Markdown presentation
  ```
- **From Loom**: Double-click any `.pptx`, `.odp`, or presentation `.md` file.
- **From Voyeur**: Press **`Super+D`**, type `burlesque`, and press **`Enter`**.

---

## Screenshots & Visual Walkthrough

### Designer Themes
Burlesque comes out of the box with 6 distinct themes engineered for typographic contrast and high-impact projection:
1. **Burlesque Velvet**: Deep cabaret plum (`#180828`), warm ivory, crimson rose, champagne gold.
2. **Editorial Light**: Warm newsprint alabaster (`#FAF7F2`), obsidian ink, terracotta vermilion, deep emerald.
3. **Midnight Slate**: Midnight navy slate (`#0B1120`), ice white, electric sky cyan, indigo glow.
4. **Champagne Ivory**: Luminous ivory silk (`#FBF8F1`), warm charcoal, burnished amber gold, warm bronze.
5. **Cyber Prism**: Void black-indigo (`#09071A`), luminescent aqua, hot neon magenta, cyan laser.
6. **Swiss Modern**: International stark white (`#FAFAFA`), dense typographic black, Swiss signal red.

![Burlesque Themes](../assets/burlesque-themes.png)
*Theme Picker active with Editorial Light theme applied across slide deck.*

### Live Animated Slide Transitions
Burlesque renders real-time software transition frame interpolation:
- **Dissolve / Fade**: Smooth 256-level cross-dissolve alpha blending.
- **Push Left / Push Right**: Horizontal slide pushes with subpixel rendering.
- **Wipe Right**: Sharp linear wipes across canvas boundaries.
- **Zoom In**: Dynamic scaling transition from 20% to 100% viewport.
- **Magic Move**: Automatic object morphing across slide boundaries by interpolating centroids, bounding boxes, font scales, and opacity.

![Burlesque Transitions](../assets/burlesque-transition.png)
*Interpolated frame rendering during a live slide transition playback.*

### Intra-Slide Build Animations & Magic Move
Burlesque provides Keynote-class intra-slide animations and automatic object morphing:
- **Magic Move Transition**: Elements across consecutive slides sharing a `magic_key` (or matching element type and content) dynamically interpolate their position, bounding box, font scale, opacity, and rotation across transition frames. Unmatched incoming elements fade in while outgoing elements smoothly fade out.
- **Intra-Slide Build Effects**: Elements within a slide can animate sequentially or automatically with configurable delays and durations:
  - `Appear`: Instant element revelation.
  - `DissolveIn` / `FadeOut`: Smooth 256-level opacity ramps.
  - `FlyInLeft` / `FlyInBottom`: Directional entry transitions.
  - `PopScale`: Bounded scale-up pop effect from 80% to 100%.
  - `WipeRight`: Horizontal progressive reveal clip.
- **Hero Badge Identifiers**: Elements tagged with magic move keys display visual indicator badges in the slide outline and properties panel.

![Burlesque Magic Move](../assets/burlesque-magic-move.png)
*Magic Move transition interpolating element bounding boxes and font sizes between slide variants.*

### Dual-Card Presenter View & Speaker Notes
Toggle Presenter View (**`F5`** or toolbar button) to enter speaker mode:
- **Left Card**: Large Current Slide display with slide counter and transition badge.
- **Top-Right Card**: Real-time Next Slide preview (or "End of Presentation" card).
- **Bottom-Right Card**: High-contrast Speaker Notes card with word-wrapped Geist Sans rendering.
- **Header Bar**: Deck title, running elapsed timer (`MM:SS [RUN]`), and animated progress bar.

![Burlesque Presenter View](../assets/burlesque-presenter-notes.png)
*Dual-card Presenter Display with elapsed timer, live next-slide preview, and speaker notes.*

### Rehearse Timings HUD & WebM Video Export
Presenters can practice live delivery with full timing telemetry and export presentations as standalone video files:
- **Rehearse Timings HUD**: Floating presentation head-up display showing:
  - **Slide Dwell Timer**: Elapsed seconds and minutes on the current active slide.
  - **Total Deck Timer**: Cumulative elapsed rehearsal duration.
  - **Pacing Controls**: Quick keyboard actions (`[Space]` advance & record, `[R]` restart slide timer, `[Esc]` finish & commit).
- **Automatic Deck Timing Commit**: When rehearsal finishes, recorded dwell times are committed directly into slide metadata (`rehearsed_dwell_ms`) and persisted in Markdown comments (`<!-- timing: dwell_ms=... -->`).
- **Splicy WebM Video Export**: Exports the entire presentation deck—including intra-slide builds, rehearsed dwell times, and slide transitions—as an animated VP9 WebM video stream (`/data/burlesque/deck.webm`) built on a `sex-splice` `Timeline` (`export_pictures`) and encoded through Splicy's `sex_media::export::Exporter` (`Format::WebmVp9Opus`).

![Burlesque Rehearse Timings & Video](../assets/burlesque-rehearse-video.png)
*Rehearse Timings HUD overlaid on live slide preview with dual dwell timers and pacing controls.*

### PPTX, ODP, and Vector PDF Workflows
Burlesque provides full interoperability with enterprise presentation suites:
- **Save PPTX / Open PPTX**: Standards-compliant OOXML `.pptx` container reading and generation.
- **Save ODP / Open ODP**: OpenDocument `.odp` container reading and generation.
- **Export PDF**: Multi-page vector PDF-1.4 export with exact layout preservation and font embedding.

![Burlesque PPTX, ODP, and PDF](../assets/burlesque-pptx-odp-pdf.png)
*Exporting vector PDF documents and roundtripping PPTX and ODP slide archives in `/data/burlesque/`.*

---

## Keyboard Shortcuts & Toolbar Commands

| Shortcut | Command | Action |
|---|---|---|
| **`Left`** | `burlesque.prev` | Navigate to previous slide. |
| **`Right`** | `burlesque.next` | Navigate to next slide. |
| **`Ctrl+N`** | `burlesque.add-slide` | Add a new slide to the presentation. |
| **`Delete`** | `burlesque.delete-slide` | Delete the current slide. |
| **`Ctrl+T`** | `burlesque.cycle-theme` | Cycle through the 6 Keynote-class themes. |
| **`Space`** | `burlesque.play-transition` | Play animated transition for current slide. |
| **`F5`** | `burlesque.presenter` | Toggle dual-pane Presenter View on/off. |
| — | `burlesque.magic-move` | Set transition to Magic Move. |
| — | `burlesque.add-build` | Add an intra-slide build animation step to selected element. |
| — | `burlesque.step-build` | Step to next intra-slide build animation state. |
| — | `burlesque.rehearse` | Toggle live Rehearse Timings session on/off. |
| **`Ctrl+E`** | `burlesque.export-pdf` | Export multi-page vector PDF to `/data/burlesque/deck.pdf`. |
| — | `burlesque.export-webm` | Export presentation to animated WebM video (`/data/burlesque/deck.webm`). |
| — | `burlesque.save-pptx` | Export presentation to `/data/burlesque/deck.pptx`. |
| — | `burlesque.import-pptx` | Import presentation from `/data/burlesque/deck.pptx`. |
| — | `burlesque.save-odp` | Export presentation to `/data/burlesque/deck.odp`. |
| — | `burlesque.import-odp` | Import presentation from `/data/burlesque/deck.odp`. |

---

## Security & Architectural Guarantees

1. **Pure Rust `#![no_std]` and `#![forbid(unsafe_code)]`**: The presentation core (`burlesque-core`) is completely panic-free, bounded in memory allocation, and contains zero unsafe code.
2. **ZIP Bomb DOS Protection**: PPTX and ODP import features strict 32 MiB per-member limits, 128 MiB total uncompressed archive limits, and a maximum of 2,048 archive entries.
3. **Hardware Isolation**: Burlesque executes within an unprivileged PKU protection domain, holding explicit access only to its `/data/burlesque/` storage directory and granted presentation documents.
