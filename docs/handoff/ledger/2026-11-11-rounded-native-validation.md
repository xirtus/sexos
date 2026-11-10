# Rounded desktop native validation follow-up

Candidate 85cc4014 built as UI-only ISO ff3e74d54788200d3444edafff823813cd5ff6cd25d5080d9dafb215b5a14459. Root verified the sealed ISO and three ELF hashes. Native evidence remains under sexos-evidence/rounded-desktop/native85cc.

The original run and burst-repeat-2328 both stop after eight of ten characters from one QMP input-send-event containing twenty PS/2 edges. Preserve these failures. QEMU v11.0.2 hw/input/ps2.c declares a sixteen-byte queue and drops additions when full; this is consistent with the trace, not independently verified installed-binary identity. No kernel ring-full or input rejection record was observed.

A separately saved paced-control-2330 reaches all ten characters and full selection on the identical ISO, but fails its pixel oracle. Investigation finds the document viewport diagnostic uses a stale fixed y+36 while the actual child geometry uses title26+inset8 = y+34. All 2,376 tested text/selection/caret pixels match exactly at the actual child origin, two pixels above the logged origin. This is diagnostic evidence, not a passing full gate.

Shell now obtains this diagnostic origin from the same catalog-backed child geometry function used to position the actual child, including current UI scale. It retains the standalone origin for documents without a shell child. No display coordinates, application messages, input authority or rendering assertions change.

Fresh candidate build, unchanged pixel assertions, HiDPI, Web child pointer, stack audit and matched performance remain required before merging #158. The paced runner is a separate control; do not relabel either burst failure as a pass.
