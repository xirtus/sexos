# sex-codecs

Fast, memory-bounded, fuzzed image decoders for **WebP**, **AVIF**, **HEIC**, and **GIF animation** in SexOS.

## Overview
`sex-codecs` is a pure `#![no_std]` + `alloc` library crate providing zero-copy, memory-safe image decoding for SexOS confined user applications (such as the **Gaze** picture viewer and the **Kaleidoscope** browser).

- **Strictly Confined & Safe**: `#![forbid(unsafe_code)]` across the entire crate. Decoders run strictly inside the requesting application's domain, never in the Display server or microkernel.
- **Quota & Memory Discipline**: Every decoder validates announced dimensions and pixel memory quotas through [`Limits::admit`] before allocating pixel memory. Allocations use `try_reserve_exact` to return `Error::OutOfMemory` rather than aborting.
- **Target Compatibility**: Runs cleanly on both host platforms (macOS/Linux x86_64 and AArch64) and the SexOS userspace target (`x86_64-sex-app.json`).

## Supported Formats

| Format | Decoding Capabilities |
| :--- | :--- |
| **WebP** | Lossy VP8, Lossless VP8L, `VP8X` extended container, `ALPH` alpha, animated WebP (`ANIM` + `ANMF`) with per-frame blending and disposal. |
| **GIF** | Streaming GIF87a/89a with LZW decompression, global/local palettes, 4-pass interlacing, all 4 disposal modes (0, 1, 2, 3), and loop count. |
| **AVIF** | ISOBMFF / MIAF container parsing (`ftyp`, `meta`, `pitm`, `iloc`, `iinf`, `iprp`, `iref`), AV1 Sequence & Frame Header parsing, intra frame reconstruction, multi-tile `grid` assembly, auxiliary alpha plane merging, and `irot`/`imir` transforms. |
| **HEIC** | ISOBMFF / MIAF container parsing, `hvcC` SPS/PPS parsing, HEVC IDR/CRA intra slice decoding, multi-tile `grid` assembly, auxiliary alpha plane merging, and `irot`/`imir` transforms. |
| **PNG / JPEG** | Delegated seamlessly to `sex-image`. |

## API Example

```rust
use sex_codecs::{decode, decode_into, AnimationDecoder, Limits};

// 1. One-shot decode of a still image (WebP, AVIF, HEIC, GIF, PNG, JPEG)
let image = decode(bytes, &Limits::PHOTO)?;
assert_eq!(image.pixels.len(), image.width * image.height);

// 2. Decode into preallocated caller buffer
let mut canvas = vec![0u32; 1920 * 1080];
let (w, h) = decode_into(bytes, &Limits::SURFACE, &mut canvas)?;

// 3. Streaming multi-frame animation (GIF, WebP, or single-frame stills)
let mut anim = AnimationDecoder::open(bytes, &Limits::PHOTO)?;
let mut frame_buf = vec![0u32; anim.width() * anim.height()];
while let Some(timing) = anim.next_frame(&mut frame_buf)? {
    display_frame(&frame_buf, timing.duration_ms);
}
```
