# 2026-11-26: Tether `push-file` & Streaming WAV Playback (`play <path>`)

## Summary
- **Tether `push-file` (`SEXOS-PUSHFILE <remote_path> <size> <fnv64_hex>`)**:
  - `scripts/tether_host.py`: added `build_pushfile_frame()`, `@PUSHFILE` staged binary inbox delivery, and `push-file <local_path> <remote_path>` CLI subcommand (up to 16 MiB).
  - `crates/sex-pdx/src/crash_log.rs`: added `FNV64_OFFSET`, incremental `fnv64_update()`, `PUSHFILE_MAX_BYTES`, `PushfileHeader`, and `parse_pushfile_header()`.
  - `apps/spindle/src/fs_commands.rs` + `apps/spindle/src/tether_command.rs`: added zero-heap, zero-big-static streaming receiver (`PushfileRx`) that creates/truncates the destination path via `sexfiles`, streams incoming TCP chunks directly from `sexnet`'s shared region into `sexfiles`'s shared region with natural TCP backpressure, computes running FNV-1a 64-bit checksums, and replies `push-file: ok <path> (<size> bytes)`.
- **Real PCM WAV Playback (`play <wav_path>` / `play stop`)**:
  - `crates/sex-pdx/src/wav.rs`: pure `no_std` RIFF/WAVE PCM header parser (skips `LIST`/`JUNK`/`fact` chunks, supports 8/16-bit, mono/stereo, 8..=192 kHz), streaming 16.16 fixed-point linear-interpolating resampler to 48 kHz stereo `i16`, and `pack_pcm6`/`unpack_pcm6` wire helpers.
  - `crates/sex-pdx/src/lib.rs`: assigned `OP_BELL_PCM = 0xCF` in Bell's reserved `0xC8-0xCF` opcode range (6 stereo 48 kHz `i16` frames per PDX call).
  - `servers/sexbell/src/sound.rs` + `servers/sexbell/src/main.rs`: added 16,384-frame (~341 ms) direct PCM FIFO (`PCM_RING`) mixed through Bell's master gain into the active audio card (Realtek ALC289 HDA, VirtIO, USB, Bluetooth).
  - `apps/spindle/src/play_command.rs` + `apps/spindle/src/main.rs`: added zero-heap streaming `play <wav_path>` command (~160 bytes state) that reads 4 KiB WAV chunks directly in `sexfiles`'s shared region, resamples on the fly, and streams tick-paced `OP_BELL_PCM` packets to `sexbell`.
- **Guardrails**: Zero kernel changes; zero heap allocations (`DummyAllocator` compliant) and zero large static buffers in Spindle.

## Verification
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts/tests/test_tether_pushfile.py scripts/tests/test_tether_hotswap.py`
- `cargo test -p sex-pdx --lib wav` and `cargo test -p sex-pdx --lib pushfile_tests`
- `python3 scripts/check_architectural_truth.py`
- `cargo check --target x86_64-sex.json -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem -p spindle -p sexbell`
