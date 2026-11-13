# Noto fallback fonts

Drawn by Display when Geist lacks a character (`crates/sex-font/src/fallback.rs`).
All are Noto fonts by Google and Adobe under the SIL Open Font License 1.1 (`OFL.txt`),
taken from Ubuntu's `fonts-noto-core`, `fonts-noto-cjk` and `fonts-noto-color-emoji`
packages by `scripts/gen_fallback_fonts.py`:

- `NotoSansMono-Regular.ttf`, `NotoSansSymbols2-Regular.ttf`: unchanged.
- `NotoSansCJK-SC-Subset.otf`: Noto Sans CJK SC cut to GB2312, JIS X 0208, KS X 1001 and
  Big5 level 1 characters, kana, Hangul compatibility jamo and full-width forms (a
  modified version, as the OFL allows; the reserved name is kept in its name table only
  as the source's).
- `emoji-36.sxe`: the single-scalar emoji of Noto Color Emoji as 36x36 4-bit coverage,
  shaded by lightness, for monochrome text. Format in `fallback.rs`.
