# SilkBar Candy & Lavender UI Polish

Upgrades SilkBar from a flat grey/charcoal strip into a smooth, modern
candy-lavender frosted glass top bar matching the rest of SexOS's Candy UI,
with zero kernel changes and zero ABI/layout drift.

## Changes

- **`crates/sex-theme/src/lib.rs`**: Added `candy_lavender` palette tokens
  (`DARK_BAR_TOP`/`DARK_BAR_BOTTOM`, `LIGHT_BAR_TOP`/`LIGHT_BAR_BOTTOM`,
  specular highlights, orchid hairlines, frosted pill surfaces, and orchid
  accent/glow) with a WCAG AAA contrast test (`candy_lavender_silkbar_meets_wcag_aa`).
- **`servers/sexdisplay/src/chrome_candy.rs`**:
  - Upgraded `bar_glass_pixel` to render a smooth vertical candy-lavender
    frosted gradient in both Dark (deep-plum/lavender-velvet top → translucent
    lavender-lilac sheen) and Light (airy pastel-lavender top → soft
    orchid-lilac sheen) modes, with a crisp 1px pastel-lavender specular
    highlight along `y == 0` and a soft orchid bottom hairline at `y == bar_h - 1`.
  - Added `candy_pill_pixel` — a zero-heap, half-pixel SDF anti-aliased rounded
    frosted lavender pill renderer with crisp pastel-lavender rim highlight and
    optional orchid glow (adapts cleanly to Dark, Light, and HighContrast).
- **`servers/sexdisplay/src/silkbar_anim.rs`**:
  - Added 1px perimeter anti-aliasing and candy-lavender/orchid tints to
    `slot_capsule_pixel` hover/active/press capsules.
  - Upgraded `animated_workspace_pill_pixel` with a soft glowing orchid/lavender
    outer halo and subtle candy top-half sheen on the active workspace capsule.
- **`servers/sexdisplay/src/main.rs`**:
  - Added `bar_pill_bg` and wired smooth frosted candy-lavender pills behind the
    launcher, active window title/notice badge, workspace cluster strip,
    right-side status cluster (battery text + Wi-Fi/Net, Volume, Battery, Bell),
    and clock pill without changing any `silkbar-model` hit-test geometry or
    render/clock markers.
- **`tests/chrome_candy.rs`** & **`scripts/virtio_gpu_gate.py`**:
  - Added `candy_lavender_silkbar_gradient_and_pills` host unit test verifying
    vertical gradient, specular/hairline edges, lavender hue, and smooth rounded
    pill corners across Dark, Light, and HighContrast modes.
  - Updated `virtio_gpu_gate.py` expected Light SilkBar sample pixel `(300, 14)`
    from flat grey `(221, 220, 224)` to candy-lavender glass `(207, 198, 228)`.

## Verification

- `cargo test -p silkbar-model -p sex-theme`
- Host tests `tests/chrome_candy.rs` and `tests/desktop_theme.rs`
- `python3 scripts/check_architectural_truth.py`
- `PYTHONPATH=scripts:scripts/tests PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts/tests/test_silkbar_clock_gate.py`
- `cargo check --target x86_64-sex.json -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem -p sexdisplay -p silkbar`
