# Files preserves deferred request task stamps

Files previously retained the caller domain and three words during nested block waits, discarding the kernel-stamped caller task. Its direct main-listen path also discarded the task. Both nested sources (DiskFS and volume reads), bounded defer queue, and main replay envelope now retain it.

The five existing payload tuple indices, priority/FIFO behavior, capacity and ordinary request admission remain unchanged. Only a separate task field is added; no kernel, BOOT or authority change is included. The fixed queue costs another 1,120 bytes (140 envelopes grow from 40 to 48 bytes). Legacy unstamped push/pop helpers exist only in host test builds; production requires stamped entry points.

Ten host queue cases pass, including old/new tasks with identical payloads, terminal priority and cancellation replacement. A separately compiled control that discards the task stamp fails at runtime. CI source checks bind both nested ingress sites, direct ingress and replay construction to their actual metadata.

This is a transport prerequisite, not an authorization claim. Ordinary program workers have task IDs distinct from their bound main run, so blanket equality against the main task would change their Files semantics. The later single-thread recorder role must check the retained actual task against its admitted binding and service epochs. First activation may retain a BOOT task; an already-activated reusable row's rebirth must receive a distinct run task. Native authority proof remains part of that integration.

Validation: full `scripts/service_custody_host_gate.sh` passes, including the production queue controls and existing nested service compositions. `cargo check --offline -p sexfiles --target x86_64-sex.json -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem` passes. The new metadata does not yet grant recorder access.
