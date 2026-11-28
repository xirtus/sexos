# 2026-11-28: Microphone input picker (system default + per-app choice)

Andreas: anything that takes sound (Moan, Flash, ...) must be able to use the internal mic.

- `sex_pdx::sound_mixer`: input kinds `INPUT_AUTO` 0, `INPUT_HEADSET` 1, `INPUT_BUILT_IN` 2, `INPUT_USB` 3; mixer actions `INPUTS` (13) and `SET_INPUT` (14, desktop callers only, like `SET_OUTPUTS`); the `inputs` word (present bits 0..8, recording kind bits 8..12, person's choice bits 12..16), `input_name`.
- `sex_pdx::sound_ring`: `INPUT` (104, program writes the kind it wants while it listens, 0 = the person's choice) and `INPUTS` (112, Bell writes the inputs word). `sex_app::sound::Sound::set_input` / `inputs`; `open` resets `INPUT` to 0.
- Bell `listen_pump`: present = USB mic, plugged headset jack mic, built-in (DSP mics, else the card's own input). Picks the first listening program's named input that is present, else the person's choice, else USB > headset > built-in. Logs `[bell.mic.source] input= usb= dsp=`, `[bell.mic.inputs] present= used= chosen=`, `[bell.mic.choose] input=`.
- Spindle: `volume inputs`, `volume input auto|headset|built-in|usb` (works over the tether).
- Moan: Transport menu "Record From: System Default / Built-in Mic / Headset Mic / USB Mic" (`Studio::record_input`, `moan::input`); Record calls `set_input` before `listen(true)`.
- Flash uses the person's choice (no per-app pick yet). A SilkBar quick-settings picker is a follow-up.
