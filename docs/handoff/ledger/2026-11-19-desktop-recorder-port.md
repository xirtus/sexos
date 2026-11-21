# Desktop recorder ported onto current master (replaces PR #164)

Codex's opt-in desktop recorder (draft PR #164, `codex/desktop-recording`) was 1,415 commits
behind master. Its Files half had already landed in #163. This change brings over the rest
against today's tree:

- `apps/record`: the recorder program (capture, encode session, Files output).
- Shell: `desktop_recording::Controller` (start/stop, the stop hit target in the top bar,
  hide fencing). Its messages are routed after the restricted-helper filters, so a
  restricted helper still cannot reach it.
- Display: `recording_capture` (frame bank publication after commit) and
  `recording_indicator` (the protected red marker, painted after the cursor). Its messages
  are handled after `restricted::denied`, so restricted helpers can't capture.
- Kernel: with `SEXOS_RECORDER=1` only, BOOT retains the `/programs/record` image for the
  private row 42 and pairs the capture bank with Display. Activation replies now carry
  `ACTIVATION_SOURCE_PINNED`. Boot regions grow from 192 to 256 for the capture bank.
- Boot image: `make_boot_image.py` copies `programs/record` when an entry names it, and
  counts it toward the image's size (the image already grows to fit since #399).

All of it stays off unless the image is built with `SEXOS_RECORDER=1` and the recording
FFmpeg profile, so the default image only gains dead code paths. Host fixtures that the
old branch carried were updated to today's tree (`access_view` in the cursor extract, the
app catalog in the activation receipt test, master's catalog in the scene fixture).

Not done yet: an armed QEMU gate (`scripts/desktop_recording_guest_gate.py`) in CI, and a
check that row 42 and PKU key 5 still suit the recorder once #465 frees keys 5 and 6.
