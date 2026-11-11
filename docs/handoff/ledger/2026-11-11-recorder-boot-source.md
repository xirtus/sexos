# Recorder BOOT profile source integration

Opt-in SEXOS_RECORDER=1 now requires exactly one /programs/record module, retains its exact private BOOT bytes, and binds only reserved reusable row42/key5 to that source. All program rows are checked for key uniqueness before any filtering. The tiny hello idle boot image remains the first activation; only the pinned codec image can replace it. Default builds grant no capture bank.

The complete source profile constructs a 1MiB paired bank at slot95: Display query/read/write, recorder query/read, no DMA and no grants to other program rows. Explicit Files-to-Shell slot6 refuses collisions. The builder requires the existing encoder marker before building/scanning the recorder, copies that exact ELF into programs, and adds it to each Limine entry before the install image is assembled. Recorder requests a 1MiB ELF stack; heap remains separately bounded64MiB.

Generic activation receipts expose only source-constraint presence (bit8), with strict state/unknown-bit/identity decoding. Shell ordinary launch selection excludes actual source-pinned receipts, and recorder admission requires the pinned receipt's exact catalog target/surface. Existing source comparison checks the private kernel copy, not a filename; source bytes/hash are not exposed by the receipt. ABI hashes regenerated.

SOURCE ONLY: no enabled image has been built or booted. Current-head CI validates default build and actual host controls; native exact ordinary-replacement42 rejection preserving identity/generation/bytes, full recorder Start/Stop/Files sync, cross-key denial, KEEP/restart, indicator publication and revocation still required before usable-feature claims. No hardware qualification.
