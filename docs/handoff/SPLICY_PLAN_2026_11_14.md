# Splicy, the SexOS video editor: plan (2026-11-14)

Andreas asked for Splicy to be a priority and to learn from
[Scarlet](https://github.com/Forward-Future/scarlet) (MIT). This is the short plan; the slice
list it refines is V5, V11 and V12 in `APP_PARITY_PLAN_2026_11_14.md`.

## What Scarlet does well, and what Splicy takes from it

| Scarlet | Splicy |
| --- | --- |
| Original footage is never touched; the edit is a cut list | Same. `sex-splice` has no I/O; sources are only referenced |
| Every edit is a list of typed timeline ops (add, split, ripple delete, ...) | `Op`, applied all-or-nothing by `Timeline::apply_all` |
| Cuts ripple every track so sound stays in sync | `Op::RippleDelete` cuts all tracks and moves markers |
| Edit by transcript; remove pauses, fillers, retakes (`dialogue-pass`) | `transcript`: `cut_words`, `pause_cuts`, `filler_cuts`, `repeated_takes` |
| Requested edits apply; suggestions wait for the editor | `suggest`: staged as **source** cuts, so they still land after other edits (Scarlet needs an 800-line rebase for this) |
| Earlier versions stay in history; undo/revert | `history`: undo, redo, revert as a new version |
| Preview renders only the changed section | `render::changed_window` finds it; `render::plan` is what the renderer follows |
| OTIO export to Resolve | `otio::export` |
| Agent edits through Claude Code/Codex | Later: assistant is a confined program, off by default, that only stages suggestions |
| Floating-point seconds | Integer microseconds and per-mille speed, so cuts are exact and goldens compare bit for bit |

Not taken: Tauri/TypeScript/web UI, PostHog telemetry, cloud transcription.

## Slices

| Slice | What | Gate |
| --- | --- | --- |
| **S1 (this PR)** | `crates/sex-splice`: timeline, ops, edit by transcript, suggestions, history, render plan, OTIO export | 12 host tests in CI |
| **S2a (this PR)** | `sex_splice::export`: timeline to frames (rounded up so each lands in its own source frame) and sample-exact sound pieces | 3 host tests |
| **S2b (this PR)** | `sex_media::export`: WebM VP9+Opus, MP4 H.264+AAC, MP4 AV1+Opus (Andreas: all three). Export profile `FFMPEG_RECORDING=2` builds libvpx (`scripts/build_libvpx.sh`) for SexOS | host gate `scripts/splicy_export_host_gate.py`: all three formats, 60 frames, 2 s sound, the cut on exactly frame 30 |
| S2c | x264 and libaom builds for SexOS (scripts like build_libvpx.sh; FFmpeg `--enable-gpl` for x264), so MP4 works on SexOS too, not only on the host | build + guest gate |
| S3 | The Splicy program: transcript pane (delete text to cut), timeline strip, player, suggestion cards, `\` to hear the cut and the join | gate: delete a sentence on screen, its frames and audio are gone |
| S4 | Local transcription: Whisper on candle (MIT/Apache) as a confined program writing `Transcript` | host: transcript goldens on a generated tone/speech fixture |
| S5 | Assistant (Claude API, off by default, confined): reads transcript, stages `Suggestion`s only | host: scripted assistant replies become pending suggestions |
| S6+ | V11 pro list: unlimited tracks, keyframes, proxies, titles, colour, OTIO import, render queue | per V11 |

Each slice is small and lands on its own; no parallel agents.
