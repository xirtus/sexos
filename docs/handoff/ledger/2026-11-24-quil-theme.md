# Quil follows SexOS appearance settings

- Branch: `codex/quil-theme` (stacked on `codex/quil-atomic-save`).
- Gap: Quil used a fixed dark palette even when Settings selected light or high
  contrast, and did not react to accent changes.
- Change: derive the editor, tab, tree, overlays, cursor and syntax colors from
  SexOS theme tokens. Poll the settings generation while open and repaint when it
  changes. Keep syntax colors legible in all three modes.
- Test: the Quil QEMU gate requires the default Light theme serial record and
  checks the light background in `quil-light.ppm`. CI runs the full image build,
  host Quil engine tests and the QEMU editor gate.
- Scope: `apps/quil`, its lockfile dependency, the editor gate, gap list and
  this ledger. No kernel, networking driver or tether changes.
