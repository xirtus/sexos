# SexOS development: round 108 acceptance and rounds 109–119

31 October 2026. Continues [rounds 80–200](SEXOS_DEVELOPMENT_ROUNDS_80_200_2026_10_30.md) from the [round 108 checkpoint](ROUND108_CHECKPOINT_2026_10_31.md). SexOS remains a single-address-space microkernel: GLOBAL_VAS supplies addressing only, BOOT alone creates capabilities, typed sex-pdx supplies authority and PKU enforces held authority. Nothing here introduces a Unix/POSIX mechanism, a per-process address space or a runtime capability mint.

## Summary

This ledger's original HEAD, test counts and obligation inventory below describe
the rounds 108–119 handoff. The appended Browser milestone identifies its
initial and integrated images separately. Later primary development is outside
the explicitly identified cohorts.

- **Round 108 is accepted.** The connected desktop session passes natively at both desktop sizes on a kernel identical to the checkpoint image, including same-image cold recovery and independent replay. Every failure on the way was a checker defect; production source did not change.
- **Rounds 109–115 close or narrow seven authority blockers.** Two ambient kernel paths are gone (slot-0 framebuffer disclosure, kernel-privileged SyncCall). Replies now need an accepted invocation. Capability revocation is real and runs natively at app retirement. Three architectural obligations now have executable checks.
- **Rounds 116, 118 and 119 add user features.** The SexTime calendar crate, Quil find (Ctrl+F) and a Spindle `sextime` calculator command.
- **Round 117 passes the 1800-second sustained alpha session** on the latest image at 1280x800. Its runner first needed five stale races fixed; none were OS defects.
- **Obligation inventory: 4 open / 26 partial** (was 7 / 23). System Proven remains false.

### Evidence level

Rounds 80–107 were accepted against a very strict bar: frozen source/runner cohorts, raw archives with per-member hashes, ELF instruction scans and independent replay of every capture. Rounds 109–119 do **not** all meet that bar. Each states its evidence level below:

- **host**: production code compiled in host tests, usually with a compiled guard-removal control that must fail.
- **sealed**: a sealed GREEN_MASTER build and boot of the changed image.
- **native**: a native QEMU workflow on that image.

Treat 109–119 as *completed with scoped evidence*, not as accepted under the 80–107 definition. Re-qualifying them under that definition is open work.

The round numbers below are the order the work was done. They do not match the plan table in the 80–200 document: work moved forward from plan rows 119–127 (authority), 129 (time input), 182 (find) and 117 (sustained session). Plan rows 110 (physical target profile), 111 (storage torn writes) and 112–116 (lifetimes, reserves, pressure, packaging) were not started.

## Round ledger

| Round | Change | Evidence | Commit |
|---:|---|---|---|
| 108 | Connected desktop session accepted | native ×2 sizes, 4 boots, 34 frames, replay; 2,104 Rust + 836 Python | `fa026c6c`, `26e0afda` |
| 109 | Slot-0 bridge narrowed to 3 decoded caller-scoped ops; ambient 0xE3 framebuffer disclosure and 0xBB hook removed | host + compiled control, sealed | `4112b33c` |
| 110 | Kernel-privileged SyncCall edge removed; IPC capabilities name targets only | host source guard, sealed | `0a33ef98` |
| 111 | Every reply needs a kernel-recorded obligation from an accepted invocation | host (production router) + control, sealed, native connected ×2 with 0 denials | `16339df0`, `b02e255f` |
| 112 | Executable single-VAS check; NO_PER_PROCESS_TABLES open → partial | static gate + 5 injected negatives | `956e6879` |
| 113 | shell_app_cleanup host gate restored (silently broken since clipboard custody) | host + control | `9a2264a5` |
| 114 | Real capability revocation; app retirement revokes the terminal incarnation's table; PDX_REVOCATION open → partial | host (production table, concurrency) + control, sealed, native (9/9/7 revoked) | `26e4d77d` |
| 115 | Paired PKU-on/off authority parity; PKU_ENFORCEMENT_ONLY open → partial | native paired boot (179 identical records), host source guard | `c8cf7ee7` |
| 116 | SexTime civil calendar crate (Gregorian + Long Count) | host, no_std target build, 5,008-sample Python oracle | `1a2d1dfb` |
| 117 | Sustained 1800 s alpha session passes; five runner races fixed | native, 1 profile (wide), 80 cycles | `1ac9b3bc`, `5e054cb7`, `b649b519`, `d6608523` |
| 118 | Quil Ctrl+F find | host + native (prompt pixels, forward/wrapped/missing receipts) | `fc23ccfc` |
| 119 | Spindle `sextime <unix-seconds>` calculator | host + native (exact epoch record) | `8e482405` |

## Round details

### 108: connected desktop session accepted

The corrected runner passes at 1280x800/e1000e and 1024x768/e1000. Each profile runs one live boot and one same-image, terminal-disk cold boot, with 17 frames per profile. The live boot covers: an authenticated HTTPS image held while Quil typing completes, Fit/1:1 inspection and Clear, the plain-HTTP reader with real wheel scroll, hidden-Copy refusal, clipboard replacement, an exact 2049-byte SaveAs, Linen reopen, the Spindle history barrier, same-instance BringHere and ordered exact-incarnation Close. The kernel is byte-identical to the checkpoint; the ISO differs only in packaging bytes. `connected_desktop_gate.py --check` replays the wide cohort independently. [Evidence](assets/sexos-alpha-r108-2026-10-31-evidence.json), [raw archive](assets/sexos-alpha-r108-2026-10-31-qualified-raw.tar.gz).

Four runner defects were fixed:

- Shell's shared existing-app focus receipt (`launcher.action`) was treated as a launcher palette antecedent. Linen reopen emits it with no palette.
- The append edit was matched against the reopen's load receipt. It is now joined to the exact revision the single Save carries.
- Replay dropped the reader and copy anchors.
- Close windows sampled the serial boundary separately from the F11 action, which raced with background output.

![Quil after Linen reopen](assets/sexos-alpha-r108-quil-reopened-wide-2026-10-31.png)

### 109–115: authority

- **Slot-0 bridge (109).** Any domain could call slot 0 op `0xE3` and get the framebuffer address and geometry, bypassing the BOOT display holder check that sexdisplay uses through `0xE6`. Slot 0 now selects effects only through `decode_bridge_op`: the BOOT display query, a bounded serial record, and the caller's own init argument. A compiled control that restores `0xE3` fails.
- **SyncCall (110).** An IPC capability's `entry_point` was executed with a direct `call` at ring 0. Nothing constructed such a capability, but any future grant would have let a capability choose kernel code. IPC capabilities now carry no entry point and deny as invocation edges.
- **Reply obligations (111).** Syscall 29 let any domain publish a reply to any domain ID. The callee's protection domain now holds a finite 32-entry obligation table:
  - A typed enqueue grants an obligation before the call is visible, and withdraws it if the enqueue fails.
  - The obligation is spent only when the reply is published; a full slot keeps it for the retry.
  - Retirement clears it.
  - Obligations are counted per caller, not per invocation.
- **Single VAS (112).** `scripts/no_per_process_tables_gate.sh` scans every workspace and arch Rust source. CR3/TTBR loads and mapper roots must be one of the bound sites that reinstall the single boot root. Process-model identifiers are rejected.
- **Revocation (114).** `CapabilityTable` moved to `kernel/src/capability_table.rs` with `revoke`, `revoke_all` and `holds_none`. The old `verify_revocation` stub always returned true. A sealed table still cannot insert, so a revoked slot never refills. Retirement revokes the terminal incarnation's table and checks it empty; natively, Spindle/Browser/Quil revoke 9/9/7.
- **PKU parity (115).** `scripts/pku_parity_gate.py` boots one image twice from identical disks, with and without `pku`, and compares authority-decision records. A source guard keeps kernel-mediated authorization on the software key mask, never hardware PKU state. Without PKU, nothing stops direct user loads or stores in GLOBAL_VAS: hostile-code isolation remains a release blocker.

### 116, 118, 119: features

- **SexTime (116).** `crates/sex-time` implements the [SexTime proposal](SEXTIME_AND_WEB_MILESTONES_2026_10_31.md): epoch 2012-12-21T00:00:00Z (Unix 1,356,048,000), civil seconds without leap seconds, normalized nanoseconds, checked conversions, the Long Count under GMT 584283 on UTC days, no b'ak'tun wrap, and rejection before 0.0.0.0.0. No clock ABI or source trust changed.
- **Quil find (118).** Ctrl+F opens `Find: query#` in the status line. The query is at most 32 printable bytes. Enter selects the next exact match and wraps once; Esc closes and erases the query. Find is private editor state, and paste is declined while find is active. [Prompt](assets/sexos-r118-quil-find-prompt-wide-2026-10-31.png).
- **Spindle sextime (119).** `sextime 1356048000` prints `SexTime 0 s`, `UTC 2012-12-21T00:00:00Z` and `Long Count 13.0.0.0.0 (GMT 584283)`. It is a calculator, not a clock.

### 117: sustained alpha session

`quil_workflow_gate.py --soak-seconds 1800 --find` (with `alpha_session.py`) could not reach its soak on either the baseline or the current image. Five stale races were fixed, all in the runner:

- It typed the document name while emulated Ctrl was still held, so every letter arrived as a Ctrl chord.
- Spindle focus toggling re-read focus before the toggle took effect and toggled back. Two intermediate fixes waited for specific records that interleaved serial output can split. The final fix waits for any focus change after the key was sent.
- Linen reopen assumed every row loads, but non-document rows correctly report a failed load.

**Result: pass.** The session ran on ISO `e70a222a7a46770c95a9722afa828eacfcd243e512caa3f8e741e82e30464a8a`, whose production source equals HEAD. It completed 80 active cycles over 1800 s. Each cycle typed in Quil with pixel checks, saved with a concurrent Linen rescan, refreshed Linen and ran a Spindle storage stat, a history-saved command and, in cycle 1, `sextime`.

The run then completed:

- A Quil Close while an admitted save was pending.
- BOOT successor relaunch, with both saved documents reopened byte-exactly.
- A pending Spindle job Close and the Spindle successor's history reload.
- The full cleanup check.

The run recorded zero denied replies, and capability revocation fired for all five retired incarnations (9, 16, 15, 12, 17).

| Latency (s) | n | p50 | p95 | max |
|---|---:|---:|---:|---:|
| Quil key receipt | 80 | 0.144 | 0.197 | 0.206 |
| Quil key to verified pixels | 80 | 0.914 | 1.053 | 1.086 |
| Warm save receipt | 80 | 0.170 | 0.203 | 0.224 |
| Linen refresh | 162 | 0.151 | 0.195 | 0.217 |
| Spindle storage stat | 80 | 1.919 | 1.974 | 1.985 |
| App Close to pool reuse | 3 | 0.309 | 0.309 | 0.310 |

Host-measured times include QMP and screendump overhead and are not a hard real-time bound. Only the wide profile ran; compact and a second fresh-image repeat remain open. Earlier attempts are diagnostic only:

- The first full attempt failed at cycle 22 under heavy concurrent host load.
- Two later attempts failed on interleaved-record waits.

[Result](assets/sexos-r117-alpha-1800s-2026-10-31-result.json.gz) (sha256 of the uncompressed JSON `4ffec7c95866638a50dedbf64422eae4d58b1dfbe21806cb2c0059093b72a4f9`), [raw serial/frames archive](assets/sexos-r117-alpha-1800s-2026-10-31-raw.tar.gz).

## Verification state at handoff

- Sealed GREEN_MASTER on the final image; ABI hash updated for each ABI-input change. Production source at HEAD equals the 1800 s alpha image.
- Six routine Rust host gates: 1,909 + 56 + 9 + 35 + 67 + 38 executions pass after round 111, plus the new authority, reply, revocation, single-VAS, SexTime, find and sextime gates.
- Python runner suite: 841 tests pass (836 plus 5 for PKU parity).
- `check_architectural_truth.py --run-host`: all 40 registered host/static checks pass. Two had been failing: `shell_app_cleanup` was stale before this work and fixed in 113, and the Quil clipboard harness was broken by 118 and fixed.

## Known pre-existing issues found

- `scripts/arm64_phase{2..5}_gate.sh` compare against a backup archive outside the repository and fail on any kernel change; they were already stale.
- The `shell_app_cleanup` host gate had been failing since clipboard custody was added (fixed in 113). Registered host checks are not all run routinely; run `check_architectural_truth.py --run-host` before each acceptance.
- The Quil status chip highlight is fixed-width and overlaps a longer status such as the find prompt (cosmetic).

## Remaining release gates

- **Open (4):** DETERMINISTIC_REPLAY, EPOCH_COMMIT, CAUSAL_DOMAIN_ORDER, UIIR_AUTHORIZED_STREAM. The epoch/UiIR transport and the recorder do not exist. Per-sender sequences need a listen ABI change: `PdxMessage` gains a field, which touches about 60 fixture literals.
- **Partial (26):** includes PDX_SOLE_AUTHORITY (slot-0 serial records remain an unsanctioned diagnostic channel), PKU_ENFORCEMENT_ONLY (no no-PKU hostile isolation), PDX_REVOCATION (no user-initiated or partial revocation, no revocation of others' capabilities naming a retired target), and the memory, scheduler and diskfs obligations listed in `spec/`.
- **Not started:** physical target profile and boot (plan 110, 199–200), storage torn-write campaign (111, 155), larger BOOT reserve (159–163), SMP lanes (143–148) and epoch replay (129–138).
- **Not claimed:** public web/CA, crash/power-loss, hostile-code containment, physical hardware, SMP, or a general daily driver.

## Next steps

1. Re-qualify 109–119 under the 80–107 acceptance bar: freeze one cohort and run the full native regression set at both sizes, with raw archives.
2. Wire SexTime into the Browser clock panel or SilkBar once a validated civil-time source is routed there. Unknown or synthetic sources must stay visible as such.
3. Per-sender causal sequences (listen ABI v2) as the first CAUSAL_DOMAIN_ORDER step.
4. Plan rows 110–116: physical profile, torn-write recovery, lifetimes and reserves.

## Reproducing

```
make full
GATE_DIR=/tmp/gm ./scripts/master_runtime_gate.sh --keep-log
python3 scripts/freeze_native_evidence.py --build /tmp/gm --out /tmp/fz --iso sexos-v1.0.0.iso --kernel iso_root/sexos-kernel
cd scripts && python3 connected_desktop_gate.py --iso /tmp/fz/tested.iso --binding /tmp/fz/source-binding.json --source-disk <copy of /tmp/gm/nvme.img> --out /tmp/conn
python3 quil_workflow_gate.py --iso /tmp/fz/tested.iso --source-disk <disk> --out /tmp/alpha --soak-seconds 1800 --find
python3 pku_parity_gate.py --iso /tmp/fz/tested.iso --disk <disk> --out /tmp/parity
```

When an ABI input changes, recompute `abi_version_hash` in `sexos_build_spec.toml` from the `ABI_INPUTS` list in `scripts/entrypoint_build.sh`, using bash.

## Independent Browser SexTime milestone — 31 October

### Initial checkpoint `bf5de8db`

The Browser T action now displays a bounded last-query Clock panel with Gregorian
UTC, native SexTime seconds and Maya Long Count. Q clears its calendar values.
Source/precision/snapshot labels remain explicit, and invalid or unavailable
states remove stale dates. Rendering respects actual Reader/Copy precedence
and joins each text frame to its immutable held Display receipt.

Qualified on `codex/browser-sextime`, with committed primary base `f8058b31`:
sealed GREEN_MASTER; 40 registered checks with 2,581 passing Rust test executions;
851 Python tests; two rejected compiled calendar mutations; Browser ELF scan
of 92,261 instructions with zero inspected vector/FP matches; eight native
cohorts at both desktop sizes, 10 boots, 46 pixel-verified captures, independent
raw replay. Two additional Close screenshots are observational only. Native
Clock checks use full-screen views; default/tiny geometry has host scope.

The [Clock handoff](BROWSER_SEXTIME_SNAPSHOT_2026_10_31.md) and
[machine evidence](assets/sexos-browser-sextime-2026-10-31-evidence.json) identify
the exact image, original C/D bindings, input equivalence, raw archive and
diagnostics. Its frozen inventory remains 4 Open / 26 Partial / System Proven
false. This is an independently named milestone, not a new numbered round or
qualification of Claude's newer causal/dirty-close/UiIR work. Integrating and
verifying newer primary commits was the next step at that checkpoint.

### Integrated primary base `e5b95eb0`

The independent branch now incorporates the committed launcher deadline fix,
Quil dirty-close guard and BOOT-granted UiIR stream. A fresh single frozen
cohort qualifies the combined image: GREEN_MASTER; 41 registered checks with
2,590 passing Rust test executions; 854 Python tests; two rejected compiled
calendar mutations with identical recorded inputs. Browser, Shell, Display and
Quil ELF scans decode 222,704 instructions with zero inspected vector/FP matches.

Clock native checks also restore the exact original tile after T/Q/T, preserving
the second snapshot without a new query. Both default tile sizes keep the full
calendar/provenance rows readable. All eight Clock/connected cohorts pass
fresh independent replay: **10 boots, 50 verified captures, 52 raw screenshots**;
two Close screenshots remain observational only. UiIR BOOT pairing, attachments
and sampled committed focus records are observed across all ten boots, with
per-stream scope and no rendering-authority/global-epoch claim.

[Integrated evidence](assets/sexos-browser-sextime-integrated-2026-10-31-evidence.json)
links the 1,420-member size/hash-verified archive. Its exact inventory is
**2 Open / 28 Partial / System Proven false**, adopting the upstream Partial
UiIR/causal scope. Global epoch commit and deterministic replay remain Open.
Later primary commits and a sustained alpha on this newest image need fresh
qualification. Next independent UI work is a BOOT-routed, nonblocking validated
status-bar clock; the main physical/storage/lifetime/reserve plan continues.

## Independent status-bar civil-clock milestone — 31 October

The normal status bar now displays BOOT-held q35 UTC snapshots with source,
one-second precision and snapshot labels. Unknown/failure/stale/rollback/terminal
states hide numeric time. Exact PD6/task6 admission, canonical framing and an
immutable one-item publication outbox preserve authority and backpressure rules;
ticks and demo counters supply no UTC. The existing source wire epoch remains
unchanged, with full SexTime/calendar views available in Browser.

Qualified on `codex/silkbar-civil-time`, based on `876c30be`: one frozen cohort,
GREEN_MASTER, 42 registered checks with 2,628 passing Rust test executions,
880 Python tests, two rejected retained compiled guard-removal controls. Five
ISO-extracted application/service ELFs decode 228,757 instructions with zero
inspected vector/FP matches. Ten native cohorts pass independent replay:
12 boots, 70 verified captures, 72 raw screenshots; 22 captures verify the entire
clock cell, with 14 positive snapshots and 8 unavailable views. Two Close
screenshots are observational only. Real TLS work, Quil input/Save and Browser
Close remain responsive in the tested workflows.

The [status-bar handoff](SILKBAR_CIVIL_CLOCK_2026_10_31.md) and
[machine evidence](assets/sexos-silkbar-civil-clock-2026-10-31-evidence.json)
identify exact source/image hashes, raw archive and diagnostics. Inventory stays
2 Open / 28 Partial / System Proven false; no new general daily-driver claim.
Next is the compact 1800-second latest-image alpha workflow, then wide repeat and
repaint/source-traffic measurement, alongside the main physical/storage plan.

## Independent alpha evidence milestone — 31 October

The runner now retains every capture with unique sequence/name, exact serial
prefix and source-bound glyph proof. Linen/third-Quil/minimize frames use the
same registry. A read-only replay verifies all captures and exact terminal
document bytes after QEMU exit. Close facts distinguish pending-save decline
from actual terminal pending state; completed-save Close cannot claim pending
teardown. Per-cycle IDs/windows prevent duplicate samples from becoming a soak.

On the unchanged `73c06c82` production image, the new frozen runner cohort passes
42 registered checks, 902 Python tests and the compact rehearsal with 20 immutable
captures and fresh replay. The rehearsal is not sustained acceptance; observed
Quil terminal pending0 explicitly leaves pending-I/O teardown unproved.

The [alpha evidence handoff](ALPHA_EVIDENCE_2026_10_31.md) and
[machine evidence](assets/sexos-alpha-evidence-rehearsal-2026-10-31-evidence.json)
preserve the completed rehearsal separately from the live compact 1800-second
run. That longer run and subsequent wide repeat require terminal results and
independent replay. Inventory remains 2 Open / 28 Partial / System Proven false.

## Independent civil-clock repaint milestone — 31 October

Display now preserves every canonical clock admission/health renewal while
requesting top-strip repaint only when the two rendered rows change. Other UI
dirty causes survive. The native boot probe avoids a clock repaint request for
207 of 228 admissions (90.8%); full compact/wide app workflows avoid 78.2%/77.5%.
These are source-bound decision counts, not CPU or latency speedup claims.

One new frozen image passes GREEN_MASTER, 42 registered checks with 2,630
passing Rust test executions, 906 Python tests, two rejected compiled mutations,
six native boots with 68 verified captures and fresh independent replay. Five
exact ISO-extracted ELFs decode 229,004 instructions with zero inspected
vector/FP matches. Only Display changes among production inputs; the kernel,
ABI, source sampler and SASOS authority rules remain unchanged.

The [repaint handoff](CIVIL_CLOCK_REPAINT_2026_10_31.md) and
[machine evidence](assets/sexos-civil-clock-repaint-2026-10-31-evidence.json)
preserve the qualified short workflows and measured decisions. Separate earlier
image compact/wide 1800-second sessions remain live and excluded. Sustained
acceptance on the optimized image and actual Quil pending-I/O terminal teardown
still require qualification. Inventory stays 2 Open / 28 Partial / System Proven
false; the physical/storage/reserve plan continues.

## Alpha launcher-proof correction — 31 October

The first compact attempt completed 84 cycles over 1800.1757 seconds, then
failed the final legacy launcher-exhaustion wait. It remains diagnostic, with
an incomplete registry and explicit rejection by read-only replay. The runner
now selects Quil's row, requires fresh atomic post-selection status and verifies
the entire launcher through a uniquely retained frame and positive text Applied.
Replay joins that frame and exact offsets to third-incarnation cleanup.

Production is identical to the qualified repaint image. The corrected frozen
runner passes 42 checks/2,630 Rust executions, 910 Python tests and compact/wide
full rehearsals with 42 verified captures and fresh independent replay. The
[launcher handoff](ALPHA_LAUNCHER_PROOF_2026_10_31.md) and
[machine evidence](assets/sexos-alpha-launcher-proof-2026-10-31-evidence.json)
retain both the failed original cohort and qualified correction. A corrected
compact 1800-second run is live; the older wide run remains unchanged. Neither
is accepted before its actual terminal result and replay.

The next lifecycle work has a concrete
[pending device-I/O Close plan](QUIL_PENDING_IO_CLOSE_PLAN_2026_10_31.md), using
Quil's actual 12,288-byte logical limit, bounded runtime storage throttling and
same-trace write/Close/completion ordering. It is still planned. Inventory stays
2 Open / 28 Partial / System Proven false; no general daily-driver claim.
