# Completed wide sustained workflow

The earlier frozen alpha image completes **85 distinct mixed-app cycles over
1809.016 seconds** at 1280×800/e1000e, followed by terminal cleanup and fresh
independent read-only replay of **206 immutable verified captures**. This accepts
the version-2 sustained workflow on that exact image, not the newer optimized
image or general daily-driver readiness.

The session measures 547.557 seconds of active work and 1261.459 seconds of
intentional idle, with the existing per-cycle idle bound. Cycles join actual
Quil edits, exact immutable saves, current/save pixels, Spindle Stat/echo/history
and Linen refresh. The terminal phase reopens exact saved documents through
BOOT-reserved successors 16/17/18, preserves the third Quil edit, retires a
pending Spindle job and completes the exact-incarnation cleanup checks.

An ordinary Quil Close was declined while its save was pending. The confirmed
Close occurred after that immutable save completed: its scope is
`safe_completed_save_close`. **Quil pending-I/O terminal teardown remains
unproved.** The machine evidence retains this obligation explicitly.

[Machine evidence](assets/sexos-alpha-wide-1800-2026-10-31-evidence.json)
links the [raw archive](assets/sexos-alpha-wide-1800-2026-10-31-raw.tar.gz),
including frozen image/checkers, every capture/prefix, terminal disk, complete
serial trace, host logs and fresh replay. Every archive member is size/hash
verified. All 1,143 frozen inputs match committed source `20c50957`.

- ISO: `79a48af561534484182f02cb641f38054962996c77f96f619e46a7bf4b9d2b3f`.
- Kernel: `7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.
- Supporting checks: 42 registered host/static checks, 2,628 passing Rust
  executions and 902 Python tests on the same frozen runner cohort.

The [failed compact attempt and corrected launcher proof](ALPHA_LAUNCHER_PROOF_2026_10_31.md)
retain their separate outcomes. The corrected compact session on the optimized
image remains live and must earn its own terminal result/replay; its subsequent
wide qualification is still needed. No result is reassigned to newer source.

The [pending device-I/O Close fixture](QUIL_PENDING_IO_CLOSE_PLAN_2026_10_31.md)
is the next lifetime proof. Its large ASCII baseline must be built through
Quil editing and completed clipboard operations: Spindle's fill commands write
binary data without a supported document wrapper and are unsuitable.

Inventory stays **2 Open / 28 Partial / System Proven false**. The BOOT-only
capability genesis, typed sex-pdx authority, PKU enforcement and single global
address space remain unchanged. Physical/storage/reserve and global epoch work
continues toward the full OS goal.
