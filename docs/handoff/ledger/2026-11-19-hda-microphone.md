# Microphone on Intel HD Audio laptops and desktops (2026-11-19, real-PC hardware)

- **What changed:** Bell records from real Intel HD Audio codecs, not only from
  virtio sound. Every program that already asks for the microphone (`play --listen`,
  Moan, voice apps) now hears the laptop's built-in microphone or a headset jack,
  with the same per-program leave (`volume ROW mic on`) and wipe-on-exit rules.
- **Routing:** `servers/sexbell/src/mic.rs` (pure) ranks input pins from their
  default configuration: a plugged-in jack microphone first (presence detect read
  when recording starts), then the built-in microphone, then an unsensed jack, then
  line in. Microphone pins get 80 % VRef bias (100 % or 50 % when that is all the
  pin offers). `hda.rs` finds a path from an input converter through mixers and
  selectors to the pin, unmutes only that path (other mixer inputs stay muted so
  playback does not loop back), and records 48 kHz stereo on the controller's first
  input stream into its own 8-period ring (64 KiB, inside the existing 256 KiB pool).
- **Boot log:** `[bell.hda.input]` per recordable pin, `[bell.mic.card] input=N`
  for HDA too, `[bell.hda.mic]` when recording starts (pin, converter, kind, bias).
- **Gates:** `tests/bell_mic.rs` in `scripts/sound_host_gate.sh`;
  `scripts/sound_listen_gate.py --card hda` (QEMU intel-hda + hda-duplex) in the
  CI media job.
- **Not yet:** Intel DMIC arrays on the PCH (SOF/DSP; many thin laptops wire the
  built-in microphone there rather than to the codec), switching input while
  recording when a headset is plugged in, input gain control in Settings.
