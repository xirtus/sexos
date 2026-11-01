# Round 150: readable Spindle sustained acceptance

The readable Spindle image from [Round 146](SPINDLE_READABLE_TILES_2026_11_01.md)
passes paired sustained workflow acceptance at both desktop sizes, followed by
fresh independent read-only artifact replays. This qualifies frozen source
`348cc9ab4d2988eb58c1e4dcacf8f7b4149d6ad7`, rather than the newer Browser output
image from Round 149.

| Profile | Model | Raw session seconds | Cycles | Verified captures |
| --- | --- | ---: | ---: | ---: |
| 1024×768 | e1000 | 1800.0040542919887 | 87 | 209 |
| 1280×800 | e1000e | 1800.0015799589455 | 84 | 203 |
| Total | | | 171 | 412 |

Both native runs and both fresh replays exit successfully with strict acceptance
true. The requested raw window remains 1,800 seconds. The cycle/idle checks use
the corrected Round 146 timing that records the final serial read before the
cycle ends; the acceptance bounds themselves are unchanged. These are workflow
acceptance observations, not guest CPU or scheduler idle benchmarks.

The image includes the 2× Spindle text threshold, viewport-aware prompt layout,
and all eight actual BOOT-reserved Spindles. Both sessions independently exercise
Quil through target 23 and Spindle through target 29, including history handoff,
resource cleanup and reserve exhaustion. The remaining native obligation still
includes Quil pending-I/O terminal teardown. This sustained window does not add
a power-loss or cold-reboot proof; the separate short cold-boot evidence remains
in the source-image report.

ISO SHA256:
`2ac80573a5240fc0d30dd2900fb59da19513b0f098d33013b8bc985f94e41b5e`.
Kernel SHA256:
`08bd29a9099ad90a8fa91698b973b18c1d9f6aa5e65dadbf6291f9e0f2064b82`.

The [evidence](assets/sexos-readable-spindle-alpha-paired-2026-11-01-evidence.json)
and [raw archive](assets/sexos-readable-spindle-alpha-paired-2026-11-01-raw.tar.gz)
retain the frozen source/image, both sessions and their external replays. All
1,635 archive members were reopened and checked by byte size and SHA256. Archive
size: 13,541,286 bytes; SHA256:
`2c7fb61678ba938d32915ec8de8afb40895e3f9028d5d7c2e9b5afbd9b937bd8`.

The architecture inventory remains **2 Open, 28 Partial, System Proven false**.
GLOBAL_VAS remains addressing only, typed sex-pdx carries authority, PKU enforces,
and BOOT alone creates capabilities. No process address spaces or runtime
capability minting are introduced. The next implementation step binds Display's
bitmap store once to its held lifecycle tuple and authenticates the incoming
main task before bitmap effects.
