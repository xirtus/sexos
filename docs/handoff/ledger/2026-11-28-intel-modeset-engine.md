# 2026-11-28 Intel display: full DP modeset engine (stage 1b)

Thread "Build a full Intel graphics driver". Follows #875 (seamless refresh).

- `crates/sex-gpu-intel`:
  - `dpll.rs` combo DPLL0/1: refclk (DSSM), DP divider table (19.2/38.4 and
    24 MHz), CFGCR0/1 encode + port-clock decode, DPCLKA_CFGCR0 routing,
    PRM enable/disable sequence.
  - `phy.rs` combo PHY swing/pre-emphasis tables (TGL DP HBR, HBR2, eDP HBR3)
    and the PRM's per-lane programming order (keeper, loadgen, SUS clock,
    training off, DW5/DW2/DW4/DW7, training on).
  - `training.rs` VESA DP 1.4a link training (CR with TPS1, EQ with
    TPS2/3/4, adjust requests, bounded tries, eDP LINK_RATE_SET) over a
    `Source` trait.
  - `pcode.rs` pcode mailbox: watermark latencies (adjusted per PRM) and
    Gen12 SAGV block time. `watermarks::compute` derives levels from them.
  - `modeset.rs` link choice, M/N, and the DP SST enable/disable sequences on
    a combo port.
- sexdisplay `intel.rs`: `set_refresh` covers every mode at the panel's own
  size. Same raster -> seamless M/N (stage 1a). Different raster -> full
  modeset (`[display.intel.modeset.plan]`, `[display.intel.modeset]`),
  checked by the sink's lane status (DPCD 0x202-0x204), measured refresh and
  FIFO underrun; any failure relights firmware's mode with firmware's
  watermarks (`[display.intel.refresh.revert] modeset=1`).
- Gate: `scripts/kms_host_gate.sh` (new tests/training.rs, tests/modeset.rs).
- Not yet: resolution changes (Display buffers are boot-sized), Type-C/TBT
  ports (DKL PHY, FIA), HDMI, CDCLK changes, DBUF reallocation — stage 2.
