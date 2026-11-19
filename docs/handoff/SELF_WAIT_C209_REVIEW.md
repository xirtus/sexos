# Timed-wait source review: c20934ae

Reviewed raw master `c20934ae11a5a8cbb609825acc52adc4baa4d9b8` against
`ba8ebc28` (the same kernel tree as `5b4f647a` and `1dc3382f`).
The exact source receipt is [SELF_WAIT_C209_REVIEW.json](SELF_WAIT_C209_REVIEW.json).

Five existing kernel files change: `boot_memory.rs`, `device_lease.rs`,
`devmgr.rs`, `hal/acpi.rs`, and `init.rs`. SerialIO leases and touchpad routing
are BOOT setup. Power profile idle code writes energy preference MSRs; it does
not alter the tick clock or interrupt/arm/park ordering. The new power-button
notification enqueues then unparks only the capability-authenticated claimant.
Only Shell holds REQUEST; the added Display grant has SENSE alone. Net's main
task identity and wait capability remain unchanged. SDK lib adds only the
touchpad module declaration; Net and USB main sources are unchanged.

All six deadline-reference owners are byte-identical. The bounded scan still
finds exactly those six owners across 139 kernel Rust files. The admission adds
only the exact new init, SDK lib, and whole-tree hashes, preserving all previous
pins. Existing unknown-source, injected-writer, cancellation, SMP, ambiguous
return, topology and clock negatives remain required. An actual git-archive of
this commit passes the source contract; adding a new writer or changing a
pinned owner fails closed. No kernel edits or builds were performed for this
review. This mechanical source qualification does not establish runtime results,
SMP behavior, general Rust verification, or any future combined kernel cohort.
