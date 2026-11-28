# 2026-11-28: Maya calendar math and day signs come from dreamroot-core

The Maya calendar (PR #865) became its own public app, Dreamroot
(https://github.com/xirtus/dreamroot), a Linux/macOS/Windows planning calendar.
Its `dreamroot-core` crate (`no_std`, no dependencies) now holds the shared
math and artwork, and SexOS uses it by git rev instead of keeping a copy:

- `sex_time::{LongCount, Tzolkin, Haab, MayaDate}` and `sex_time::dreamspell`
  re-export `dreamroot_core`. Reading them from a timestamp goes through the
  new `sex_time::CivilDay` trait (`from_timestamp`, `from_unix`); bring it into
  scope with `use sex_time::CivilDay as _;`. `LongCount::days_of` and
  `LongCount::to_timestamp` became `sex_time::long_count_days` and
  `sex_time::long_count_timestamp`.
- `sex_icons::maya::SOURCE` is `dreamroot_core::glyphs::SOURCE`; the traced
  data, the SVGs, the original drawings and `maya_glyphs.py` moved to the
  Dreamroot repo. `assets/maya/ATTRIBUTION.md` stays here (CC BY 2.5 credit).

To change the calendar math or the glyphs: change dreamroot-core first, then
bump the `rev` in `crates/sex-time/Cargo.toml` and `crates/sex-icons/Cargo.toml`
together.
