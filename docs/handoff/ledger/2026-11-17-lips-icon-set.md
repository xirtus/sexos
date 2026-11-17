# The SexOS icon set in the lips brand style (2026-11-17)

- **One source:** `assets/icons/icons.txt` holds every icon (73 line, 23 app,
  9 file-kind; the line icons include tray icons for wifi, battery, bluetooth, power
  and volume-off) on a 24-unit grid
  in Weft's path notation, now with `F` for filled subpaths.
  `scripts/brand/icons.py` writes `assets/icons/svg/{line,app,file}/*.svg`, PNGs at
  16 to 256 px in `assets/icons/png/` and `assets/icons/preview.png`.
- **Style:** app icons are the glyph in the bright rose-gold sweep on a charcoal
  badge with a faint rose rim; file icons sit on a charcoal sheet with a folded corner;
  folders are rose gold. Colours from docs/BRAND.md (`sex_icons::Palette::BRAND`).
- **`crates/sex-icons`:** no_std, no heap: parses the source, strokes paths into
  polygons and rasterises them with `sex_font::raster`, so native drawing matches the
  SVGs. `app_for(program)` maps aliases (`files` -> loom), `file_for(name, dir)` maps
  extensions.
- **Users:** Weft's built-in icons are now this set (`Icons::draw_app`,
  `Icons::draw_file`, `Painter::image_alpha`); Files' thumbnail grid shows file and
  folder icons instead of kind words.
- **Proof:** `cargo test -p sex-icons -p weft`; `cargo run -p sex-icons --example sheet`.
