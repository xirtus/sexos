# Optimized-image sustained workflow at both sizes

The optimized clock image completes **172 mixed-app cycles and 418 immutable
verified captures** across two terminal sessions: compact 1024×768/e1000 runs
86 cycles over 1800.001925 seconds; wide 1280×800/e1000e runs 86 cycles over
1800.010017 seconds. Fresh independent replay accepts both version-2 workflows.

These are the completed sessions previously recorded as live in the launcher
handoff. They bind to source `96f4436c27a2b85400a7af3acf1bbe3014a17b16`, ISO
`9ad6768dcc70a0554999b5b2f57ceebcc0cd66e32500c1ceee79deb1e9ff8aca` and kernel
`7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.
Every one of the 574 production and 569 runner inputs matches both the frozen
snapshot and the committed Git blob. Supporting checks on that runner cohort
are 42 registered host checks, 2,630 passing Rust executions and 910 Python tests.

The workflows cover edits, exact durable saves, Spindle history, Linen refresh,
BOOT-reserved successor recovery, terminal cleanup and fresh launcher selection
after reserve exhaustion. Confirmed Quil Close follows completed storage:
`safe_completed_save_close`. Their original pending-device obligation remains
unchanged in the archived records. The separately bound
[pending-device qualification](PENDING_DEVICE_IO_CLOSE_2026_10_31.md) supplies
that edge on the same production image.

[Machine evidence](assets/sexos-alpha-paired-1800-2026-10-31-evidence.json) binds
the [raw archive](assets/sexos-alpha-paired-1800-2026-10-31-raw.tar.gz): 1,642
members, 13,100,401 bytes, every member size/hash verified. It retains both
terminal sessions, frozen image/source, immutable frames and prefixes, disk
snapshots, host logs, fresh replays and the independent committed-source audit.

This accepts the earlier optimized image's sustained workflow. The later
catalog integration needs its own qualification. Physical storage/power-loss,
SMP, global epoch/replay and unrestricted daily-driver readiness remain outside
this result. Inventory remains **2 Open / 28 Partial / System Proven false**.
