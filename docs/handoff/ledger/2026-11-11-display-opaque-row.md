# Display opaque row shading reuse

Base: eeb39e6a1efdb55e27e09e621502ce650c6973f1 (PR158 remains draft).

## Preserved failure

Root's matched three-pair nativeeeb3/perf-ab/latency.json result is FAIL:
p95 candidate/baseline 1.2552156071376965, CPU 1.2343560133480822.
The sealed nativeeeb inputs and results are unchanged. Matched baseline763
may be reused; this change needs its own candidate rebuild and matched run.
No performance recovery is claimed from source inspection.

## Change and boundaries

raster_row already proves each selected interval opaque before attempting
fast_content_run. When that helper rejects chrome, bitmap or partial-fill
work, shade the interval with composite_pixel_source_unmasked using the same
immutable Scene's already-selected source. This avoids repeating coverage's
child-group lookup and rounded coverage calculation and resolving the row
source for every pixel. All ordinary source painting and contributor-bit
updates remain. Nonopaque intervals keep the actual lower-owner AA chain;
busy row scratch keeps the scalar fallback. No canonical order, mask, layer,
stack-scratch ownership or damage-boundary changes; no kernel growth.

## Validation scope

Extended actual extracted raster_row host comparisons with a translucent
bitmap over partial fill and a lower owner beneath AA corners, both focused
and unfocused at 320x240 and 640x400. A probe explicitly requires an opaque
interval whose fast-content helper rejects it. Existing checks compare full
rows, partial damage, contributor bits and scratch-busy fallback; existing
rounded-layer controls retain independent expected AA colors.

git diff --check passes. Rust host gate, rebuilt ELF/native 1x/2x pixel gates,
and matched performance gates are PENDING: JIT owns the heavy slot. No compiler
or VM was started for this source review. No new 64KiB stack proof is claimed.
