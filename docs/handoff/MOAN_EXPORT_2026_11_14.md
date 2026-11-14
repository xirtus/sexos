# Moan real audio export, ChatGPT round 181

Moan exports its processed stereo timeline through FFmpeg inside the app's own
organ. WAV, FLAC and MP3 select actual codecs and muxers; changing a filename is
not used as a substitute for encoding. The project workstream supplies standard
Weft format controls; exporting still uses the normal Files Save picker.

## Codec contract

- Input: interleaved signed16 stereo PCM, 48 kHz, including the existing two-second
  effect-tail render after the last clip. No resampling or channel remixing occurs.
- WAV: FFmpeg `pcm_s16le` encoder and `wav` muxer, exact decoded PCM.
- FLAC: FFmpeg `flac` encoder and muxer, exact decoded PCM and sample count.
- MP3: FFmpeg `libshine` wrapper and `mp3` muxer, stereo192kbps. Shine uses fixed
  point. MP3 is lossy and its codec framing introduces delay/padding; this slice
  does not promise gapless metadata or sample-exact MP3 trimming. The oracle
  bounds decoded length difference to fewer than2304 samples (48ms) and checks
  both channels' pitch and level. WAV/FLAC remain sample-exact alternatives.

`crates/moan-export` owns the Rust/C boundary. `Encoder::frame_capacity()` reports
its codec's block size. Submit full blocks, except the final block may be short;
finish requires exactly the declared input frame count. Bounds:3662seconds,
8192input frames per call,1MiB queued output,32KiB file-write turns,4GiB file size.
A caller cannot submit another block while encoded output is pending. Codec errors
are terminal. The encoder is single-thread-owned, matching the port's disabled
FFmpeg threading. Input is copied into codec-owned frame storage before return.

Custom AVIO callbacks only append owned, bounded output records or move an output
cursor. They never open paths, access device grants, call Files, or block on IPC.
The file owner drains ordered writes, including trailer/header seek patches, and
acknowledges exact turns. Tokens prevent a stale short-write acknowledgement from
consuming another turn. Drop frees codec state without writing a trailer or I/O.

The app verifies encoder availability, asks the Files picker, exclusively creates
the chosen destination, renders/encodes/drains, finishes/drains and syncs before
emitting `[moan.export] ... synced=1`. Existing files are never replaced. Failed
or cancelled output remains a named partial; no saved receipt is emitted for it.
Close is polled between codec/write turns and follows the project workstream's
unsaved-project guard. Failure cleanup makes a best-effort sync of partial output.

## Pinned port and licence

The default guest and native oracle use FFmpeg7.1.2, commit
`f893221c8d89cb798b829bebe71d55e1a3f242fd`, and Shine3.1.1, commit
`97f188efc3dc90315b79a2af1f477c0c18d85e82`.
Primary source: [FFmpeg7.1.2 audio encoding API example](https://github.com/FFmpeg/FFmpeg/blob/n7.1.2/doc/examples/encode_audio.c)
and [Shine3.1.1](https://github.com/toots/shine/tree/97f188efc3dc90315b79a2af1f477c0c18d85e82).

Shine's original `COPYING` is GNU Library GPLversion2. Its section3 expressly
allows selecting ordinary GPLversion2 or a newer version. This build selects
ordinary GPLversion3 for its separately compiled Shine copy. The source is not
copied into Rust and has no upstream source changes. `build_shine.sh` retains
`COPYING.upstream-LGPL2`, the project's `COPYING.GPL3`, and a selection/provenance
`NOTICE` under `share/licenses/shine` beside the resulting archives. The downloaded
source checkout retains the original licence and notices. FFmpeg's own selected
source stays LGPL2.1-or-later (no FFmpeg GPL components enabled); the combined Moan
program with the selected Shine copy is GPL3. This is an app userspace encoder,
not a Linux driver bridge or kernel component.

`build_ffmpeg.sh` enables these encoders/muxers in both normal and recording
profiles, copies the Shine archive and licence notices, and records `source-pin`
and `sexos-audio-export-v1`. The default FFmpeg source is checked against the pin.
An explicit developer `FFMPEG_TAG` override is recorded but is not qualified by
the default oracle. The setup cache includes the export marker, source pin and
licences; its rebuild guard requires the marker and Shine archive. Compiler-object
and final-program scans enforce the existing forbidden-instruction-byte gate.

## Evidence

`bash scripts/moan_export_host_gate.sh` builds a minimal native instance of the
same pinned FFmpeg/Shine profile, then runs the actual C/Rust encoder tests. It
requires clang/LLVM tools, make, pkg-config and host ffmpeg/ffprobe. The independent
host decoder can be a newer FFmpeg; the encoder is always the pinned profile.
Host tests verify real headers/codecs/rate/channels, byte-exact WAV/FLAC decoding,
MP3 pitch/level/duration, non-codec-aligned input length, repeated13-byte short
write acknowledgements, seek-based header patches, stale/zero acknowledgements,
invalid size/seek, codec frame bounds, premature finish and abort behavior.

The actual guest app links against the full pinned SexOS FFmpeg profile and passes
its final forbidden-instruction scan. Compile success is not a QEMU result.

`scripts/moan_export_gate.py --iso sexos-v1.0.0.iso --source-disk BLANK.img --out OUT`
launches the real Files picker and Moan, clicks Tone, selects each format through
standard Weft commands, saves all3 through the picker, then cancels a long WAV
export with Close and explicitly answers the unsaved-project dialog. It extracts
actual SexFS files read-only, independently probes/decodes them, verifies exact
lossless source PCM and MP3 waveform/length, checks retained partial size and
absence of a false saved receipt, and writes `result.json`. QEMU validation must
pass in CI before merge. The host fixture and a guest compile do not establish
that this guest file-export path passed.
