# Round 155: Browser source-grant readiness

Read-only source audit, 2026-11-01. Actual Browser remains PD15/task15. No successor rows, grants, activation, guest execution, or repository edits performed. Paths below are repository-relative; line anchors describe the current Round152 source.

## Current authority and exact records

`kernel/src/init.rs:815–855` resolves Net and Browser registered main-task pointers, requires both nonnull, and constructs three separate typed grants. It does **not** infer task from PD. Current resulting tuples:

| Holder | Holder slot | Capability | Consumer | Provider/profile |
|---|---:|---|---|---|
| Net13/task13 | 60 | EntropySource | Browser15/task15 | RDSEED, at most8 attempts |
| Net13/task13 | 61 | ClockSource | Browser15/task15 | q35 CMOS UTC, at most4 coherent-pair attempts, precision1s |
| Net13/task13 | 62 | TrustPolicy | Browser15/task15 | immutable profile1, current root376 bytes |
| SilkBar6/task6 | 61 | ClockSource | SilkBar6/task6 | independent direct civil-time grant |

BOOT emits `[boot.entropy.source]`, `[boot.clock.source]`, `[boot.trust.policy]`, and descriptive `[boot.network.tls]` records; each currently names consumer15/task15. Each actual insertion checks holder-local vacancy first. The TLS record is not a fourth source capability. The separate `[boot.silkbar.clock.source]` and fixed `STATUS_BAR={6,6}` assertion are intentional civil-role constraints, not Browser task inference.

**Provider PD13 holds distinct BOOT grants per new consumer. “13” is the provider domain, not a grant count.** Supporting two future consumers means six additional capabilities held by Net13, alongside the existing three; neither Browser receives raw source authority.

`kernel/src/{entropy_source,clock_source,trust_source}.rs` resolve the exact typed capability from the current holder and full-width decoded slot; check current task pointer equals the PD's registered main and its context points back to the PD; use actual `(*task).id`. SDK `*_source::invoke` validates exact grant holder, nonzero consumer, canonical kind/reserved words, and redacts errors. No production consumer15 or task==PD authorization was found in these mechanisms. Existing syscalls72/73/74 and SDK wrappers already accept arbitrary held slots; no new syscall is necessary.

Packing differs intentionally: entropy/clock INFO encode consumer PD in the high32 bits and task in the low32; trust encodes PD low32/task high32. Reuse each typed decoder; never use one generic guessed decoder.

## Actual service blockers and invariants

`servers/sexnet/src/main.rs:5214–5219` caches one `entropy_info(60)` and one `clock_info(61)`. `:5267` always advances entropy through slot60; trust dispatch `:5424` always queries62; certificate-time `:5445` always snapshots61; entropy dispatch `:5468` supplies the singleton Info. Future callers therefore correctly fail the existing consumer-equality checks, even if generic network lifecycle admission recognizes them. Merely adding catalog rows cannot enable these sources.

Fresh typed trust/time/entropy requests already use `network_request_owner(req.caller_pd,req.caller_task)` (`:42`, `:5421`, `:5442`, `:5465`). That helper checks a nonzero incoming stamped task against `network_owner`'s held catalog/LIVE task; it does not infer task from PD. Maintenance and deferred replies correctly use the previously captured identity, not an invented new incoming task (`:5260–5267`, `:5367–5377`). Preserve this split. Wire requests0x20E/0x20F/0x210 provide finite correlation, not consumer/source/slot selectors.

`entropy.rs` compares exact Info.consumer on BEGIN and data operations, retains the exact active owner and monotonically allocated tickets, and scrubs samples/read receipts on failure/retire. Async collection must select the source belonging to that **active owner**, not the last request or newly focused Browser. CANCEL/RELEASE must remain possible when source availability is lost.

`certificate_time.rs:12–69` has one service-global high-water and permanent rollback poison. Its six per-identity replay slots are separate from that global state. `retire` only removes exact owner receipts. **Do not create a Machine per Browser or reset it when selecting another source/retiring an app.** Equal/forward samples across identities remain allowed; a backward sample poisons existing pending/cached successes globally and remains poisoned until a real boot.

`trust_policy.rs:24–46` validates exact returned consumer/profile before publishing policy, coalesces duplicate queries, and retires only exact identity receipts. `main.rs:5396–5415` retires entropy/time/trust inside the validated lifecycle cleanup closure before ACK. Submitted DMA remains driver-owned.

A concrete capacity caveat should be addressed in selection: all three machines allocate one of six retained owner fences **before** source authorization. Eight live Spindles have legitimate Net routes, and `network_request_owner` admits their held identities. Six distinct unsupported source requests can occupy all six fences with DENIED receipts until retirement, blocking a Browser. Normal Spindle probes do not do this, but a source-roster selector should reject absent source membership before consuming the authorized-source receipt budget. Do not “solve” this by evicting another owner's pending receipt. If denial delivery must be reliable, retain it separately without consuming the supported-consumer fences.

## Smallest safe next phase, still one actual Browser

1. Add a bounded descriptive Browser-source plan plus an immutable validated provider selection helper. Current production plan contains only the actual Browser15 row and slots60/61/62. Host-only fixtures can contain30/31 with unequal PD/task IDs. The plan describes expected slots; only typed current-domain INFO plus held lifecycle catalog establish usable associations. Require Browser family, exact consumer PD/actual task, exact source/profile/root/version, distinct triples and unique identities. Missing/wrong-kind/foreign-consumer records fail closed; never rewrite Info.consumer to fit a request.
2. Refactor BOOT construction through that plan, obtaining actual registered holder/consumer tasks. Before each grant assert exact holder-local vacancy; after construction read back the actually inserted typed capability and compare its decoded INFO metadata to registered task/catalog identity. BOOT can test the pure typed INFO mechanism over that readback; it must not fake Net as the current scheduled task to bypass the runtime wrapper. Net startup then performs the real holder-context syscalls. Emit every consumer's source records before sealing. At this phase emit exactly today's original grants; no latent30/31 grant or runtime mint.
3. Net initializes the finite source associations from its held queries. Fresh requests select by authenticated owner; entropy advance selects by `seed.owner()` and retains its slot through contention; trust/time closures capture the selected exact slot. Keep one entropy machine, one clock machine and one trust machine. Do not change shared protocols or weaken the kernel holder checks.
4. Update source/evidence parsers and exercise unequal-task, unavailable, contention, retirement, cross-owner replay/poison controls. Only after this and the remaining independent Browser activation/presentation/paging dependencies are satisfied should real30/31 rows and their six grants be introduced.

Suggested bounded file groups: new SDK descriptive source-plan module + tests; `kernel/src/init.rs` construction/selfchecks; new Net immutable selection helper + actual-source tests and `servers/sexnet/src/main.rs` integration; then shared source-evidence parser and its TLS/HTTPS consumers. Existing pure provider mechanisms largely need tests, not redesign.

## Slots and actual activation prerequisites

Existing60/61/62 stay stable in Net13. Prospective Net-only triples70/71/72 and73/74/75 appear outside the current catalog lifecycle slots and explicit source/device assignments inspected here, but are **candidates, not reserved grants**. Require a full holder-local genesis inventory assertion at implementation, including image-created capabilities and all generic grant loops. Slot numbers are not syscall numbers; the same numeric slot in another PD is unrelated.

All service participants, including Net13, receive each app's lifecycle slot via `kernel/src/app_resources.rs:89–97`. Thus proposed future lifecycle slots154 and158 would also occupy Net13's table, despite close/activation slots being Shell-local. Check candidate source slots against these future lifecycle entries too. Never treat proposed Shell slots154–161 as globally free provider slots or copy another holder's table.

`kernel/src/init.rs:693–705` grants SLOT_NET18 to the original Browser. Successor generic edges `:790–806` currently give non-Spindles Display/Files/Shell only; the later `:858–864` network loop is explicitly Spindle-only. Real Browser successors therefore also need explicit catalog-driven Net18 BOOT edges, plus their existing generic lifecycle grants. A source-plan refactor alone is not activation readiness. Bell/pager/private backing and renderer-bound instances remain separately qualified dependencies.

## Evidence and qualification matrix

`kernel/src/clock_source.rs:11,46–48` owns a single global successful-snapshot ordinal for **all** source slots/holders, including SilkBar6. Preserve it across source selection and app retirement. `scripts/clock_source_evidence.py:9–47` currently accepts only Net13/task13 slot61 and SilkBar6/task6 slot61 plus one Browser BOOT record. Generalize by a strictly validated BOOT grant inventory: every global marker must map to an exact holder/task/slot and sanctioned profile, with contiguous global ordinals; then join the selected consumer's service sequence and host time brackets. Never filter other slots before validating the whole stream.

Entropy sample count is also global, but its kernel diagnostic is deliberately sparse (first4/powers-of-two). Do not demand one source marker per seed word or reset count per consumer. Trust receipts need exact consumer/profile; trust metadata has no clock ordinal.

`entropy_seed_gate.py`, `certificate_time_gate.py`, `trust_policy_gate.py`, `tls_prepare_gate.py`, `tls_exchange_gate.py`, `tls_lifecycle_gate.py`, responsiveness and HTTPS checkers still contain intentional original15/task15/SID300/slots60–62 receipt and geometry assumptions. Existing tests remain original-only; successor qualification needs explicit selected identity joins, all BOOT source records, exact cleanup windows, and source-bound image/runner closure—not broad “any Browser” regexes.

Minimum controls before new activation:
- Actual production selector/kernel helpers: holder13/task130 with consumers15/task150,30/task300,31/task310; wrong task/slot/type/consumer/profile/duplicate/truncation/high bits deny before provider effects.
- Original native Browser: seed32 and positive RELEASE, current UTC under the held grant, immutable trust, controlled TLS exchange and exact body; direct raw source queries from Browser still denied.
- Later native15→30→31: each task independently attested at BOOT, fresh retained correlation/tickets, correct distinct provider slots, exact old-owner cleanup before reuse, no old seed/receipt/egress inheritance; terminal release/cancel and reply Full controls.
- Cross-owner time: global ordinal includes interleaved SilkBar reads; retire predecessor at T, successor at T or T+1 allowed; T-1 poisons all consumers and remains poisoned after retirement. No last-good or tick fallback.
- Unavailable RDSEED/RTC, malformed source, trust mismatch, wrong stamped request task, unauthorized-source caller saturation, delayed replies/cleanup, and no-provider-effects negative controls.

No host fixture establishes runtime unequal-task BOOT execution. No source/pixel/log evidence here qualifies hostile-code isolation, physical entropy quality, authenticated RTC, IOMMU/DMA drain, SMP, or post-reboot epoch security. The design remains immutable BOOT genesis, sex-pdx-mediated transitions, one GLOBAL_VAS, and userland policy; no Unix/process tables or runtime capability minting.

Round153 precedes this generalization with a denial-only retained outbox. Unsupported held consumers must receive canonical retained errors without consuming authorized source-machine owner fences or creating another clock/high-water machine. No pending owner receipt may be evicted.

Round153 receipt isolation is qualified. Round154 first fixes the legacy authorized-source polling RETRY gap without losing lease expiry/redaction; generic TCP/DNS liveness and packet custody remain separate. Per-consumer source selection follows in155.
