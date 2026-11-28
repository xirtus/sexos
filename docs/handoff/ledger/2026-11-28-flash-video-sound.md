# 2026-11-28: Flash videos record microphone sound

- Flash's WebM (VP9 + Opus) videos now carry sound (`Config.sound = true` at all three record starts).
- New `apps/flash/src/mic.rs` (`Listen`): opens the program's Bell sound ring, `listen(true)` while a video records, and hands every heard 48 kHz stereo block to `Exporter::sound` after each picture. Bell's chosen input is used (headset jack today; built-in DSP mics once the SOF stages land, `docs/handoff/SOF_DMIC_PLAN_2026_11_28.md`).
- Until the mic is on (permission pending or no microphone), Flash writes silence for the time the pictures cover, so the file always has an in-step sound track.
- Logs: `[flash.mic.start] ring=`, `[flash.mic.state] mic=` (sound_ring MIC_* value), `[flash.mic.stop] heard= silence= lost=`.
- Checked: `scripts/flash_host_gate.sh` PASS (release build + FakeUvcCamera smoke).
