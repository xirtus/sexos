# Assuit follows SexOS appearance settings

- Branch: `codex/assuit-theme` (stacked on `codex/assuit-spellcheck`).
- Gap: page chrome was fixed to a light palette, even when Settings selected
  dark or high contrast.
- Change: derive the desk, shadow, status bar, style picker and selection
  colors from SexOS theme tokens; poll the settings generation and redraw on
  theme or accent changes. The document page stays white as paper, with a dark
  caret on the page.
- Test: the Assuit QEMU gate requires the Light theme record and checks desk
  pixels in `assuit-opened.ppm`. GitHub CI runs the full image build, host
  document tests and QEMU gate.
- Scope: the word processor app, lockfile, QEMU gate, gap list and this ledger.
