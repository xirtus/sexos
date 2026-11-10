## Sound follows the headphone jack (2026-11-10)
- By default Bell follows the HD Audio headphone jack (pin sense, looked at no more than
  twice a second while sound plays): while headphones in the default set of outputs are
  plugged in, its speakers go quiet; when they come out, the speakers play again
  (`servers/sexbell/src/jack.rs`, host tests `tests/bell_jack.rs` in
  `scripts/sound_host_gate.sh`).
- Picking outputs (`volume output N|all`) stops following; `volume output auto`
  resumes it. The outputs word has `OUTPUTS_AUTO` (bit 12); `SET_OUTPUTS` takes `AUTO`.
- QEMU's codecs have no headphone jack, so in the OS this is only checked for no change
  (`sound_output_gate.py`); real laptops are the test.
