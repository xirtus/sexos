# Moan workspace and live mixer qualification

The intermediate `8d46f756` app passes four actual 1280×800 screenshots and nine
independent captured-audio measurements. The complete hashes and preserved failure
paths are in `MOAN_WORKSPACE_LIVE_RUNTIME_2026_11_15.json`. This is a development
app installed on a private disk over the preserved v9 normal image, not a newly
sealed production image or a restricted-plugin-role qualification.

Arrange has three real original MIDI tracks, Instruments verifies the actual
Weft status text and every painted RGB pixel in its status rectangle, Step uses
four real selected notes, and Mix exposes eight distinct real channel strips plus
master. Native horizontal level/pan controls remain an explicit toolkit limit;
there is no decorative vertical fader or fake audio channel.

A source-built twelve-second stereo fixture contains 400 Hz left / 600 Hz right,
with amplitudes 6000 / 8000. Two actual Files imports establish the baseline.
Three separate playback passes exercise master50%, hard-right pan, solo,
channel50%, mute and unmute through actual controls. Captured RMS matches each
independent expected channel sum within4%; pitch remains within1 Hz. Every10 ms
window inside each measured plateau must remain active. One Undo restores the
whole live pass before the next baseline. The final actual measurements are
400.0/600.0 Hz throughout and approximately8484/11313 baseline RMS,4242/5656 at
master50% or solo,2121/2828 at solo plus channel50%. Track edits remain blocked
while playing; safe mixer controls affect future blocks. The checker deliberately
measures after the existing2.1 s queued ring/card-tail allowance and does not claim
instantaneous updates, hard realtime or physical output latency.

Five tests execute the actual checker Driver methods without launching QEMU,
covering held/reversed key pairing, newest-value no-op behavior, bounded real
pointer coordinates and the real pointer49/keyboard-step2 parity trap. The first
three identified checker failures are preserved alongside their exact source,
serial, private disk, captures and notes. No audio thresholds were weakened.

Reproduce using the source-built developer seed ELF; it is never a shipped app:

```sh
PATH=/opt/homebrew/opt/llvm/bin:/opt/homebrew/bin:$PATH \
python3 scripts/moan_workspace_gate.py \
  --iso /absolute/preserved/normal.iso \
  --source-disk /absolute/read-only/blank-seed.img \
  --seed-elf /absolute/qualified/moanrack \
  --out /absolute/new-output-directory
```

The maintained runner replaces fixed local paths with CLI arguments; the exact
runner used for this pass is retained and independently hashed as
`target/workspace-live-proof-v4/checker-source.py`. Current101 app/core/plugin
host tests, including overloaded positive/negative master-headroom and unchanged
unity behavior, pass. The guest ELF builds and passes the unchanged forbidden-byte
scanner. Later track-color, restricted-role, narrow-layout and legacy-regression
cohorts must be recorded separately rather than relabeling these captures.
