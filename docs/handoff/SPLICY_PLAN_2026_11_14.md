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
| **S2c (done)** | libaom for SexOS (`scripts/build_libaom.sh`, official release tarball checked by SHA-256), so AV1 MP4 export works on SexOS | export-profile FFmpeg builds; Play links at 21 MB (limit 32 MiB) |
| **S2d (done)** | x264 for SexOS (`scripts/build_x264.sh`, commit-pinned archive from code.videolan.org; Andreas supplied the source); the export profile's FFmpeg is GPL | export-profile FFmpeg builds; Play links at 22 MB (limit 32 MiB), clean layout |
| **S3a (done)** | `apps/splicy`: `run splicy @talk.webm` with `talk.words` beside it (`START_MS END_MS WORD` per line). Transcript pane (click, arrows, Shift to select, S for the sentence, Delete cuts picture and sound together, cut words stay struck through), timeline strip with joins, player showing the picture at the caret, clean-up suggestion cards (first takes, fillers, long pauses; Enter takes, Esc skips), undo/redo, `\` shows the pictures around the nearest join, Ctrl+E exports `talk-edit.webm` with picture and sound | `scripts/splicy_host_gate.sh`: 7 host tests, including a click on the painted word, S, Delete, then the export walk shows no cut frame and lays no cut sound; PIE build without key bytes, under 32 MiB |
| **E1 (this PR)** | Editing layout, dark: toolbar with workspace tab and timecode, transcript and suggestion cards left, viewer with transport timecode right, timeline below with track headers (colour tab, eye / M S), timecode ruler, zoom (`=`/`-`, wheel over the timeline), coloured clips with filmstrip pictures (video) and waveforms (sound), joins, playhead; a click on the timeline moves the playhead and the transcript caret | host: 9 tests (filmstrip pictures follow the source after a cut, waveform louder where the sound is, zoom, click to time); QEMU `scripts/program_splicy_gate.py`: real VP9/Opus clip, scan finds pictures and loudness, screenshots show label, waveform and picture, cut a sentence, export 120 frames |
| **X-E (done)** | Splicy Studio exports WebM (VP9 + Opus), MP4 AV1 (+ Opus) and MP4 H.264 (+ AAC) with Splicy's own pure-Rust encoders; Export panel lists them; `--export=` for scripts | host `scripts/splicy_studio_export_host_gate.py` (FFmpeg decode, picture count, PSNR, loudness); QEMU `program_splicy_studio_gate.py` exports all three on SexOS |
| E2 | Source and program viewers, skimming (hover scrubs), J/K/L, in and out points | QEMU screenshots |
| E3 | Media browser with thumbnails of the folder's clips; insert, overwrite, blade, ripple, magnetic snapping; more tracks | QEMU |
| E4 | Inspector (volume, speed, position, opacity) and stereo level meters | QEMU |
| E5 | Workspaces: Editing (FCP-like), Shotcut-like panels, Transcript-first | QEMU screenshots of each |
| E6 | Sound playback and audio scrubbing (sex-app sound ring) | QEMU: played frames match the edit |
| E7 | FilmCraft (storytold/filmcraft, MIT OR Apache-2.0, pure Rust, ~370k lines, egui 0.37): which engine crates build for SexOS, then take what fits (as Spreadeagle takes GridCraft's engine); its egui UI once sex-egui (#220) lands | build report |
| S4 | Local transcription: Whisper on candle (MIT/Apache) as a confined program writing `.words` | host goldens |
| S5 | Assistant (off by default, confined): stages suggestions only | host |

Andreas (2026-11-15): Splicy should be a world-class alternative to Final Cut Pro, Shotcut, Premiere and FilmCraft, keeping Scarlet's transcript editing. Scarlet (MIT) features live in `sex-splice`, so later Scarlet improvements port there.

Each slice is small and lands on its own; no parallel agents.
