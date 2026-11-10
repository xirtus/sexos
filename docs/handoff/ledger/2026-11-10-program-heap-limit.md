# Explicit program heap limits

Adds an opt-in `sex_app::heap::set_limit(bytes)` before the first allocation. Ordinary programs retain the existing unlimited growth path and syscall sequence. Configuration and growth use the same allocator lock; repeated/late configuration, zero or non-page-aligned limits, and already-owned heap pages are rejected.

The limit covers heap pages only. Executable/static allocations and the main stack remain separate; this is not a kernel resource quota. No recorder, capture grant, BOOT assignment, or kernel change is included.

Bounded growth charges every valid accepted chunk, including a large request that succeeds partially before a later call fails. Each query and returned span must match the original base and exactly accounted range. Malformed responses permanently fence subsequent fallback/retry instead of allowing Talc to extend an existing heap into a changed span. A normal denied loan can recover later without losing its accepted-page accounting.

Validation: the production source runs against actual Talc 5.1.1 with 11 host cases, including allocation preservation, exact fallback, partial recovery, repeated configuration, changed bases, malformed responses, and unaccounted ranges. Three separately compiled negative controls must fail at runtime: forgetting total charges, recovering after a malformed span, and forgetting the original query-base anchor. The host gate is part of CI. Design guardrails pass. The first changed-base control reproduced a real WIP defect; the fixed source anchors both base and accepted page count.

Target validation: `cargo check --offline -p sex-app --target x86_64-sex-app.json -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem` passes with the default global-heap feature enabled.
