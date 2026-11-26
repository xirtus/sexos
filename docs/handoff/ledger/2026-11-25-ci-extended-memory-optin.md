# CI extended memory opt-in — 2026-11-25

The full CI workflow's 12 GiB Denim QEMU proof ran on every manual dispatch.
App PRs use manual dispatch to run the full build and guest gates because the
workflow's pull-request event targets `master` only. In runs 45520474911 and
45522492863, the heap and 1 GiB proof passed; the 12 GiB step then held the
runner for roughly 110 and 98 minutes before GitHub reported a runner shutdown.
The runner did not report a guest assertion failure or preserve the memory
artifact after shutdown.

Manual dispatch now has an `extended_memory` input, false by default. The 12
GiB proof remains mandatory on the daily schedule and is available on any
manual dispatch that explicitly selects it. Every manual app run still builds
the full image and runs the heap, 1 GiB Denim, and other QEMU gates. The
extended job budget applies only to the nightly or opted-in run.

Branch: `codex/ci-extended-memory-optin`. No kernel, Wi-Fi, or tether code
changed. GitHub CI validates the workflow; no local full build or QEMU run.
