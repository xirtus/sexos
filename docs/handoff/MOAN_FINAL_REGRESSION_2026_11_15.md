# Moan final colored-build regression

Staged Moan ELF `a2f6a58a…` passes the actual app, project reboot, three-format
export/partial cancellation and microphone-consent regression gates. Exact
inputs, source revisions, disks, raw captures and failures are bound in
`MOAN_FINAL_REGRESSION_2026_11_15.json`. These are private development app installs
over the preserved v9 normal ISO, not a newly sealed image or installer proof.

- The app produces exactly two seconds of 400 Hz output, then real pointer
  split/undo and dirty Close/Discard/retirement pass.
- A saved edited audio project reopens in a new boot and remains editable; Undo
  returns to the clean saved state. Cancel preserves the live dirty session beyond
  Shell's close deadline.
- Actual FFmpeg WAV/FLAC/ShineMP3 exports independently decode with the original
  exact lossless sample and bounded MP3 checks. Close during a long WAV export
  retains only a partial file and emits no false success.
- The QEMU microphone fixture requires explicit consent before recording and
  retains at least one second after Stop. It does not prove physical input
  fidelity, monitoring, hardware latency or host microphone access.

A supplemental two-boot v3 project is an independent headroom/persistence check.
Three real 12-second stereo imports each have channel level 200%, with master 50%.
The synchronized 6,912,274-byte archive independently verifies internal checksum,
separate whole-file saved hash, version/master/track metadata, all clip bounds and
every original PCM byte. It reopens cold with the same values. Actual captured
output retains 400/600 Hz, RMS 12726.56/16969.45, peaks 17997/24000 and matching negative
peaks; every 10 ms measured window stays active. The unscaled bus exceeds i16 on
both channels, so post-clamp master scaling would fail. The actual exported
672,000-frame WAV independently decodes to exactly 3× each original sample followed
by 96,000 zero frames, with no skipped samples or amplitude tolerance in that
comparison. A cold-opened real UI screenshot is preserved.

The identified checker repairs remain separate from app behavior. Spindle's
first asynchronous toggle can report the old focus before its window arrives;
the helper now waits for the eventual target rather than toggling repeatedly.
A legitimate own minimize permits one restore; contradictory minimize/focus
publication is refused. Own Input make/release and positive semantic completion
remain mandatory; FocusNext behavior is unchanged.

A later audit caught that `g.wait` matches are relative to the requested serial
start. Both new Moan refresh fences now add that start before inspecting the
fresh timeline-to-viewport burst. Faithful mock fixtures fail both old versions
and pass the corrected versions. The first complete four-gate semantic result is
retained with that earlier checker limitation; a fresh real app run and the
supplemental two-boot/audio/export case separately qualify the corrected fence.
The slider checker also reads the post-pointer value before generating keyboard
adjustments, since a no-op End legitimately emits no ValueChanged receipt.
Fourteen desktop-focus and nine Moan-input regressions pass. No audio tolerance,
app source, kernel or toolkit was changed by these checker repairs.

The supplemental attempts preserve an early session-busy launch (now fenced on
exact seed retirement/session-ended/later empty-session SYNC), a redundant End
receipt wait, and a mistaken comparison of internal versus whole-file hashes.
Both actual hashes and saved data were valid. Original logs, disks, archives and
checker snapshots are retained; none is relabelled complete.

`scripts/moan_master_project_gate.py` maintains the exact successful supplemental
oracle, with a repository-relative root instead of the local fixed path. It takes
`--iso`, `--source-disk`, `--seed-elf` (qualified developer-only moanrack) and a fresh
`--out`. The other four scripts retain their original substantive assertions and
now open the real Region controls before accessing its fields. Prior 8d46 live
mix/plugin artifacts and later restricted-role work remain separate cohorts.
