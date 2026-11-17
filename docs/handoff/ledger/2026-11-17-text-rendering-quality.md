# Gamma-correct, hinted, darkened interface text (2026-11-17)

- **sex-font `Render`:** `Font::draw_rendered` adds light hinting (baseline,
  x-height and cap height snapped to whole pixels, vertical only, below 32 px, so
  advances and line breaks are unchanged) and stem darkening (up to 88/256 px at
  8 px, fading to none at 32 px, drawn as two passes whose larger coverage is kept,
  so the advance does not change). `Render::PLAIN` is bit-identical to `draw`, so
  Display's pixel oracles are untouched.
- **sex-font `gamma`:** `blend` mixes coverage in linear light through integer
  sRGB tables (`scripts/fonts/gen_gamma_tables.py`) with DirectWrite-style
  contrast: heavier for dark ink, lighter for light ink, so a word keeps one
  weight in the light and dark themes. `Render::ui_for(ink)` darkens dark ink only.
- **Weft:** every Weft program's text uses `Render::ui_for` and the linear-light
  blend (`Fonts::glyph_for`, `Painter::text`).
- **Proof:** `cargo test -p sex-font` (render, gamma tests),
  `cargo run -p sex-font --example text_quality -- out.ppm [scale]` draws the
  before/after sheet at 100 % and 200 %.
