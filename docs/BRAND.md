# SexOS brand

The SexOS mark is a pair of rose-gold lips on warm charcoal, from Andreas' prototype (2026-11-16).

| File | Use |
| --- | --- |
| `assets/brand/sexos-lips.svg` | The mark, transparent, in its rich gradient. The master. |
| `assets/brand/sexos-lips-bright.svg` | Lighter gradient for small sizes on dark glass. |
| `assets/brand/sexos-lips-mono.svg` | One colour (`currentColor`), for print or single-ink use. |
| `assets/brand/sexos-lips-badge.svg`, `-badge-N.png` | The mark on a charcoal square, as in the prototype: icons, avatars. |
| `assets/brand/sexos-wallpaper.svg`, `-WxH.png` | The desktop wallpaper, the same layout Display draws. |

Colours: charcoal `#2e2a2b`; wallpaper gradient `#332e2f` to `#1c1919`; the mark runs left to right `#5e3f37`, `#9c6e5e`, `#f5aa8d`, `#ffc4a6` (bright: `#b07a66`, `#e2a084`, `#ffcaae`).

Rules:
- `scripts/brand/lips.py` is the one source of the shape. Change the curves there and run it (`--png` also renders the PNGs); it rewrites the SVGs and `crates/sex-pdx/src/brand_lips.rs`, so native drawing never drifts from the vector.
- Native code draws the mark with `sex_pdx::brand` (`draw`, or `coverage_row` with a cache), never a hand-copied bitmap.
- Our programs show only our own branding (see `DESIGN_GUARDRAILS.md`): the lips go where other systems put their own mark.

Where it shows: the Silkbar launcher button, the default wallpaper ("SexOS"), the kernel's boot screen header, the lock screen, the README and the website.
