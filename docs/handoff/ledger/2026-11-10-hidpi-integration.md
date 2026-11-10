# HiDPI integration follow-up (2026-11-10)

- **Scope:** refresh PR #77 onto current master without changing its native-mode,
  padded-scanout, buffer-age or scale behavior. No kernel growth beyond PR #77.
- **Failed proof:** the first crash proof captured the boot-log header and background
  with zero body ink. The console scroll path cleared the full body before redrawing
  retained lines, exposing a blank intermediate frame. PR #119 replaces this with
  batched, row-by-row background-and-text drawing. Refresh #77 after #119 merges;
  do not weaken the crash proof's requirement for visible log text.
- **Host validation after the master refresh:** kernel release cross-build;
  display-frame-output host gate (48 cases, 14 runtime mutation controls and four
  compiler trait controls); display-scene host gate (23 cases and 15 runtime mutation
  controls); EDID/scale tests (three); BOOT display-role tests (two); architecture
  guardrails (six). All passed. These are host/build results, not native scanout or
  hardware containment claims.
- **Still required:** green CI on the refreshed exact head, including the full crash
  proof and native display gate. Real GPU/monitor testing remains the x17 R1 gate.
  HiDPI text and pointer are implemented; bar/title/border geometry remains a
  separate follow-up requiring coordinated Shell work areas and hit targets.
