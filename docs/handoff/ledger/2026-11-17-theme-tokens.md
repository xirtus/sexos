# One set of theme tokens for every program (2026-11-17)

- **`crates/sex-theme`:** no_std tokens (window, surface, field, controls, border,
  text, accent, selection, focus, danger, shadow, scrim, radius, font size,
  spacing) for light, dark and high contrast, from a mode and an accent. Weft's
  palette is now built from it (pixel-identical: the gallery renders the same).
- **Settings:** new `appearance.theme` (Light, Dark, High contrast; key 24) next
  to `appearance.accent`, whose default is now SexOS pink `#d6336c` (Weft's own
  default, and the first accent Settings offers). Nothing read the old blue default.
- **Programs follow it:** `sex_app::settings::{appearance, theme}` give any program
  the tokens; `sex_graphics::theme` re-exports them for programs that draw with
  sex-graphics. Every Weft program reads the theme when it opens and follows a change
  within a second (`[weft.theme] mode=.. accent=..`). Files' thumbnail grid takes the
  dark tokens with Settings' accent (picture grids stay dark so pictures read true).
- **Proof:** `cargo test -p sex-theme -p weft`, `scripts/settings_host_gate.sh`.
