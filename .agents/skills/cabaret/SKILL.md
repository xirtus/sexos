---
name: cabaret
description: >
  Guide for Cabaret, the SexOS live video broadcast studio and recorder. Covers
  the headless CLI (`schema`, `status --json`,
  `list`, `inspect`, `ctl`, `apply`), OBS WebSocket v5 (`:4455`), `cabaret-ops`
  Python/Lua/JS line-RPC, 12 safe-Rust plugins, Wide-Gamut `LinearRec2020`/HDR10
  compositing, Burlesque `LivePresentationSource`, and Splicy timeline handoff.
---

# Cabaret

Cabaret (`apps/cabaret`, `/bin/cabaret`) is the SexOS live broadcast studio and recorder built on Weft (`crates/cabaret-core`, `crates/cabaret-plugins`, `crates/cabaret-proto`).

Spindle bare-name alias `broadcast` resolves to `/bin/cabaret`.

## Headless CLI

Run subcommands directly (or discover capabilities via `cabaret schema`):

| Command | Usage | Purpose |
|---|---|---|
| `schema` | `cabaret schema` | Emit machine-readable JSON tool schema (`obs-ws-v5`, `defaultPort: 4455`, `rpcVersion: 1`) |
| `status` | `cabaret status [--json]` | Inspect streaming, recording, virtual cam, and active scene state |
| `list` | `cabaret list [scenes\|inputs\|devices]` | List scenes, inputs, filters, or attached Stream Deck devices |
| `inspect` | `cabaret inspect [target]` | Inspect detailed properties for a scene, input, or script |
| `ctl` | `cabaret ctl <action> [args...]` | Execute a single control action mapped to an `ObsRequest` |
| `apply` | `cabaret apply <json>` | Apply a declarative JSON state patch or atomic batch of `ObsRequest`s |

### `ctl` Actions

- **Query / Scene**: `version`, `scene <scene_name>`, `preview <scene_name>`, `studio-mode <bool>`, `transition`
- **Audio**: `mute <input_name> [bool]`, `unmute <input_name>`, `toggle-mute <input_name>`, `volume <input_name> <volume_db>`
- **Scene Items & Filters**: `item-enable <scene_name> <scene_item_id> <bool>`, `filter-enable <source_name> <filter_name> <bool>`
- **Outputs**: `stream-start`, `stream-stop`, `stream-toggle`, `record-start`, `record-stop`, `record-pause`, `replay-save`, `vcam-toggle`

## Protocols & Scripting

- **OBS WebSocket v5 (`:4455`, RPC v1)**: SHA-256 + Base64 challenge/response auth, OpCodes, event subscription bitmasks, and typed requests/events compatible with Stream Deck, Bitfocus Companion, and Touch Portal (`crates/cabaret-proto/src/obs_ws.rs`).
- **`cabaret-ops` Line-RPC**: Out-of-process scripting over single-line JSON-RPC (`cabaret-ops 1` header) supporting `obspython` (`python`), `obslua` (`lua`), and `quickjs` (`js`) runtimes with atomic batch commit/rollback and watchdog timeout reporting (`crates/cabaret-proto/src/script_rpc.rs`).

## Native Safe-Rust Plugins (`crates/cabaret-plugins`)

All 12 plugins are `#![forbid(unsafe_code)]`, `no_std + alloc`:

1. `adv_switcher` — Advanced Scene Switcher automation rules
2. `downstream_keyer` — Downstream keyer overlays
3. `local_captions` — Local live captions
4. `move_transition` — Smooth geometry/property move transitions
5. `multi_canvas` — Vertical/multi-canvas simultaneous layouts
6. `overlays` — Broadcast HUD and status overlays
7. `shader_filter` — Programmable pixel shader filters
8. `source_record` — Per-source isolated recording
9. `spectralizer` — Audio spectrum visualizer source
10. `streamfx_3d` — 3D transform / blur / SDF effects
11. `studio_link` — Low-latency studio audio/video link
12. `time_warp` — Frame-history time-warp / instant replay

Plus `wasm_fx` (`crates/sex-wasm` sandboxed video FX bridge).

## Wide-Gamut `LinearRec2020` & HDR10 Compositing

- Linear-light FP32 RGBA compositor (`FrameBuffer`, up to `16384` px) with `crates/sex-color` integration.
- Supports `ColorSpace::LinearRec2020`, `ColorSpace::Hdr10Pq`, `ColorSpace::Hlg`, and scene profiles `ColorSpaceProfile::Rec2020Pq` / `Rec2020Hlg`.
- Video filters include `VideoFilter::HdrToneMap` (HDR scene-referred to SDR) and `VideoFilter::SdrOnHdr` (SDR placement onto HDR canvas).

## Burlesque & Splicy Integrations

- **Burlesque (`LivePresentationSource`)**: `SourceKind::PresentationDeck` drives `burlesque_core::LivePresentationSource` step-through presentations (`FullSlide`, `TransparentSlide`, `LowerThird` overlay modes) rendered directly as straight RGBA8 frames into Cabaret's compositor.
- **Splicy Timeline Handoff**: `QcPlayerState` provides J/K/L shuttle (`-2x..+4x`), exact frame stepping, `mark_in` / `mark_out`, and `export_splicy_clip()` producing a `SplicyClipSpec` (`source_path`, `mark_in_us`, `mark_out_us`) for `crates/sex-splice`.
