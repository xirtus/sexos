# 2026-11-28 Intel display: refresh changes without a modeset (stage 1a)

Thread "Build a full Intel graphics driver". First stage of the full native
Tiger Lake graphics driver (modesetting, external monitors, PSR, acceleration).

- `crates/sex-gpu-intel/src/timing.rs`: heap-free EDID detailed timings (base
  block + CTA extension DTDs) in the transcoder readout's form; `same_raster`.
- `refresh.rs`: transcoder M/N read/write; a same-raster refresh change is a
  rescale of data/link M (N kept), written in the first half of the frame so
  all four latch at one vblank. Refusals: not DP SST, different raster, link
  bandwidth (data M/N > 99.5 %), CDCLK (pixel rate > 2 x CDCLK), watermarks.
- `watermarks.rs`: PLANE_WM/CUR_WM/BUF_CFG read/write; firmware's levels
  scaled to a faster pixel rate (upper bound of both PRM methods), deep levels
  that no longer fit the DBUF share turn off, level 0 must fit. Latched by
  re-arming SURF/CUR_BASE before the M/N change.
- sexdisplay `intel.rs`: receipt adds `[display.intel.link]` (M/N, link use,
  CDCLK, DBUF, watermark levels) and `[display.intel.edid.timing]`; after the
  takeover the panel moves to its fastest same-raster mode
  (`[display.intel.refresh.boot]`, `[display.intel.refresh]`). The switch is
  checked by re-measuring the refresh and the pipe's FIFO-underrun IIR bit;
  either failing restores firmware's M/N and watermarks
  (`[display.intel.refresh.revert]`). PSR on: refused (stage 3 owns it).
- Gate: `scripts/kms_host_gate.sh` (new `crates/sex-gpu-intel/tests/refresh.rs`).
