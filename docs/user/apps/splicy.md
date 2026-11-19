# Splicy (Video Editor & Media Export)

> **Identity Note**: **Splicy** (`crates/sex-splice`, `crates/sex-media/src/export`, `apps/play`, `apps/flash`) is the SexOS non-linear video editor and media export engine. Its canonical name draws from the physical craft of cutting and splicing film reels, following the project's playful pun theme (Andreas's pick, PR #232).

**Splicy** brings modern, non-linear video editing, precision timeline trims, AI-assisted transcript cutting, and multi-format video rendering to SexOS. Designed with an immutable non-destructive core where source footage is never modified, Splicy provides frame-accurate cuts, multi-track audio mixing, OpenTimelineIO interchange, and accelerated video encoding to WebM (VP9/Opus) and MP4 (H.264/AAC and AV1/Opus).

![Splicy Video Export](../assets/splicy-export.png)
*Splicy video capture, playback, and export running in SexOS at 1280×800, displaying real-time timeline rendering and video playback.*

---

## The Splicy Ecosystem

Splicy's video pipeline spans three tightly integrated components:

1. **Splicy Core (`crates/sex-splice`)**: The timeline engine managing tracks, clip ranges, trim operations, edit suggestions, and version history.
2. **Media Export & Codecs (`crates/sex-media/src/export`)**: The hardware-accelerated encoding bridge powering WebM and MP4 multiplexing and compression.
3. **Companions (`apps/play` & `apps/flash`)**:
   - **Play (`run play @video.mp4`)**: Conconfined media player running sandboxed FFmpeg decoders to play back video and audio with zero crash risk to the desktop.
   - **Flash (`run +net flash`)**: Live USB UVC webcam capture utility with real-time viewfinder display and direct-to-disk WebM (VP9 + Opus) video recording.

---

## Supported File Formats & Codecs

| Category | Format / Container | Video Codec | Audio Codec | Description |
|---|---|---|---|---|
| **WebM Export** | `.webm` | VP9 | Opus | Royalty-free, highly compressed modern web and streaming format. |
| **Universal MP4** | `.mp4` | H.264 (AVC) | AAC | Maximum compatibility across legacy devices, browsers, and mobile hardware. |
| **Next-Gen MP4** | `.mp4` | AV1 | Opus | State-of-the-art compression efficiency with high-fidelity Opus sound. |
| **Timeline Interchange**| `.otio`, `.fcpxml`, `.edl`, `.srt` | N/A | N/A | Full NLE handoff bundle (`Ctrl+H`) with FCPXML 1.10, OpenTimelineIO, CMX 3600 EDL, SubRip captions, and a per-project compatibility audit `README`. |

---

## Non-Linear Editing Operations

Splicy features professional trim modes that keep video and multi-track audio synchronized:

| Trim Mode | Operation | Behavioral Effect on Timeline |
|---|---|---|
| **Ripple Trim** | In / Out Cut | Trims the head or tail of a clip and automatically ripples subsequent clips forward or backward so no gap remains. |
| **Roll Trim** | Boundary Shift | Moves the edit point between two adjacent clips simultaneously, extending one while shortening the other. |
| **Slip Trim** | Content Slide | Changes the in and out points of the source footage within a clip while keeping its position and duration on the timeline fixed. |
| **Slide Trim** | Position Shift | Moves a clip along the timeline while adjusting the adjacent clips to accommodate the shift. |

---

## Unique Splicy Innovations

### 1. Integer Microsecond Precision & Acoustic Alignment (`sex_splice::align`)
Unlike legacy video editors that measure time using floating-point seconds, Splicy represents all timeline points and media spans in integer microseconds (`type Micros = i64`, where 1 second = 1,000,000 µs):
- Zero accumulated floating-point rounding errors across long projects and bit-identical golden renders.
- **Viterbi CTC Forced Alignment & Room-Tone Snapping**: Refines coarse ASR word spans with integer CTC alignment, measures each recording's adaptive noise floor, snaps cut edges onto exact whole frames inside room-tone valleys, refuses cuts that split sounding speech, detects skipped speech holes and acoustic `"uh"/"um"` hesitations, and aligns dual-system audio via waveform envelope cross-correlation.

### 2. Editing by Transcript, Centered Join Crossfades & Two-State `\` Audition
Splicy integrates spoken dialogue directly with non-destructive source ranges:
- Delete words or sentences (`S` + `Delete`) to execute synchronized ripple cuts across picture and sound tracks.
- **Centered Audio-Only Join Crossfades (`sex_splice::crossfade`)**: Smooths dialogue joins using real source handles without shifting picture cuts or runtime, reporting exact handle diagnostics (`JoinFadeReport`).
- **Two-State `\` Audition**: Press `\` once to hear the join in context, and press `\` again to audition the exact removed footage (`AuditionMode::HearRemoved`).

### 3. Source-Anchored Rebasing, Inheritable Skills & Agent JSON-RPC (`sex_splice::{rebase, skills, ipc}`)
- **Source-Anchored Rebasing**: Rebases concurrent assistant cut batches onto an actively edited timeline (`rebase_cuts`), supports partial batch apply (`apply_cut_subset`), and tracks `"N need you"` review flags (`EditFlag`).
- **Inheritable `SKILL.md` Workflows**: Supports `extends: dialogue-pass` skill chains and verbatim-quoted standing-rule learnings (`LearningProposal`).
- **Typed JSON-RPC 2.0 & Foreplay Bridge**: Exposes atomic `batch.execute`, `query.analyze_pacing`, `query.find_gaps`, push events, and `foreplay.cutscene_edl` J/L split-edit export.

### 4. Smooth CPU Scrubbing, Bottom VU Strip & Fuel-Metered WASM FX
- **Quarter-Res Scrubbing & Dual-Limit `FrameCache`**: Composites at quarter resolution (`16×` fewer pixels) during playback/scrubbing and full resolution when paused, backed by an LRU `FrameCache` enforcing both entry and pixel-byte caps (`16 MiB` `LowMemoryMode` for tight guests).
- **Full-Width Bottom VU Strip (`V`)**: Per-track and master stereo peak-hold meters sit in an optional horizontal bottom strip so metering never narrows the 16:9 video viewer at `1280×800`.
- **Zero-Ambient-Authority Confinement & WASM Effects (`sex_wasm::frame_fx`)**: Splicy runs inside isolated PKU domains, and third-party frame effects execute in `wasmi` with a `50M` per-frame fuel budget, `64 MiB` memory cap, deterministic timestamps, and zero filesystem or network authority.
