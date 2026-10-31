# Round 147: sustained acceptance of the eight-reserve image

The Round 145 eight-Spindle image completes both sustained desktop sessions.
Each stops cleanly and passes a fresh source/image-bound read-only artifact
replay. This is the older **502dda45** image, with the 256-pixel Spindle text
threshold and original cycle-timing code. The newer readable-tile image needs
its own acceptance; no older result is relabelled.

| Profile | Actual soak seconds | Cycles | Verified captures |
|---|---:|---:|---:|
|1024×768 /e1000|1800.0004786670906|86|207|
|1280×800 /e1000e|1808.372962540947|86|207|
|Total|two completed sessions|172|414|

Both perform real input/save/load, Spindle stat/history and Linen refresh, then
all eight Quil and all eight Spindle incarnations, controlled TCP verification
through positive RELEASE, complete private/surface cleanup and exhausted
launchers. Exact final document bytes and source-bound pixels are replayed.
The original strict idle bound passes in both artifacts. Its known timestamp
ordering conservatively misclassifies some host log work as idle; these results
are workflow acceptance, not guest CPU/idle performance measurements. The future
fix does not rewrite these raw records or change the bound.

Source `502dda457c9fc8e22733918f20c8b37750cee022` matches 1,172 distinct committed blobs:
576 production +596 runner. Existing same-image qualification supplies 43 host
gates, 2,730 reported Rust passes plus eight extracted lifecycle cases, 1,017 Python
tests and the separate short-workflow writer 29→fresh 12 cold recovery. These long
session disks were not themselves cold-reboot tested. Terminal Quil Close had
completed its save; this is not relabelled pending device-I/O teardown.

ISO SHA256: `8ad3339e4be9bd815443719b3723086da03f3b36cc147e0022f32d989c449def`

Kernel SHA256: `1b0a81238e5d92460f123e8df97afd44f438166842a03d51057b60130a2f9e82`

Architecture SHA256: `4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`

[Machine evidence](assets/sexos-eight-spindle-alpha-paired-2026-11-01-evidence.json)
and [raw archive](assets/sexos-eight-spindle-alpha-paired-2026-11-01-raw.tar.gz)
contain 1,683 members /13,703,111 compressed bytes. Every member size/hash was
verified after reopening. Archive SHA256:
`c2ed84b1f0edb678aacd6b8582ca88c15f2d6ec71a73c104d4e1a79b319e1d4c`.

GLOBAL_VAS supplies naming; BOOT creates authority, typed sex-pdx authorizes and
PKU enforces it. Eight Spindles remain finite and Browser still has one actual
incarnation. No unrestricted daily-driver, physical power-loss, SMP, hostile
containment or global-epoch proof is established. Inventory stays
**2 Open / 28 Partial / System Proven false**.

Next: [held Browser identity and relaunch authority](BROWSER_RELAUNCH_AUTHORITY_PLAN_2026_11_01.md),
plus separately qualified sustained evidence for the readable-tile/timing image.
