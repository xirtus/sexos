# Ribbon boundary developer fixtures

Original GPL-3.0-only source-built CLAP ABI modules, using the retained MIT CLAP
headers in vendor/clap-fixture/include. These are explicit qualification probes,
not production effects. They are never normal build/install stages.

`python3 scripts/build_ribbon_boundary.py --out target/ribbon-boundary-NEW`
builds swap, UD2 crash, infinite hang, and separate parent-memory read/write
modules. It records every source/header digest and compiler/linker identity, then
runs the unchanged forbidden-byte scanner on every result. Output must be fresh.

Read/write addresses are encoded as sixteen finite nibble-valued input samples.
The qualifier must hold a REAL mapped, writable, 4096-byte-aligned parent marker
until the exact helper is retired. The runtime oracle requires a fault at that
EXACT parent address and an unchanged parent marker. An arbitrary invalid address,
UD2, or serial-only status cannot establish parent-memory isolation. If access
succeeds the plugin deliberately returns distinctive 13/11 PCM, making an isolation
failure unmistakable. Native tests execute both accesses against a real native
marker to establish that the probe actually touches the supplied address.

The ordinary swap exercises one stereo input/output at 48kHz with 256-frame maximum
blocks. Native tests cover actual ABI lifecycle and both 1/256-frame blocks.

These host/build results are not guest isolation evidence. The new restricted
coordinator and production supervisor API still need a separately bound image and
real QEMU crash/restart/reuse gates. Silent provider fences require external causal
barriers and unchanged side-effect state; enqueue success or timeout absence alone
must never become an authenticated denial claim.
