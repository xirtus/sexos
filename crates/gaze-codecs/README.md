# gaze-codecs

Picture formats and photo metadata for Gaze (`apps/view`), beyond what
`sex-image` decodes. Output is `sex_image::Image` (`u32` 0xAARRGGBB).

| Feature | Adds | Needs std | Dependency licence |
| --- | --- | --- | --- |
| (none) | `sniff`, EXIF, orientation, RAW JPEG-preview extraction | no (`no_std` + alloc) | sex-image only |
| `jxl` | JPEG XL decoding | yes | jxl-oxide: MIT OR Apache-2.0 |
| `svg` | SVG rendering | yes | resvg/usvg: Apache-2.0 OR MIT; tiny-skia: BSD-3-Clause |
| `raw` | Full camera RAW development (half or full size) | yes | **rawler: LGPL-2.1** |

## Licences

This crate is GPL-3.0-only, like all SexOS code. With the `raw` feature it
links **rawler 0.8.0** (from the dnglab project), licensed **LGPL-2.1**. SexOS
accepts LGPL-2.1 for libraries it links: only SexOS's own code has to be
GPL-3.0. rawler's own dependencies are MIT, Apache-2.0, BSD or Zlib.
`rawloader` (also LGPL-2.1) is not used.

## RAW files

`decode` shows the camera's embedded JPEG preview by default, which is the
fast path. Set `Options::raw_develop` to `Some(RawSize::Half)` or
`Some(RawSize::Full)` to develop the sensor data instead. A RAW file without
a usable preview, or a CR3, is developed at half size.

## Tests

`cargo test --locked -p gaze-codecs --all-features`. All fixtures (EXIF
blocks, PNGs, JPEG XL, DNG, RAW containers) are generated at test time. JPEG
and WebP pictures come from sex-image's test fixtures.
