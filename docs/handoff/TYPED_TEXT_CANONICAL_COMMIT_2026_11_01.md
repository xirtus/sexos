# Round 170 — typed text candidates and canonical commit

Status: **qualified**. Source `da40f6f3267bcadf34ed3c55b469a19159affc6f`.
GLOBAL_VAS addressing, BOOT-only capability genesis, sex-pdx authority and PKU
remain unchanged. No Unix/POSIX model, per-process address space, new syscall,
new capability kind or runtime capability mint is introduced.

## Result

Typed text previously replaced canonical Surface bytes/selection/layout during
intake. Round 169 fenced completion eligibility but a failed output attempt still
left those canonical fields changed. Main now seals a validated Stage candidate
without mutating canonical text. Scene capture projects its bytes, color and
selection into **owned scene storage**, using the same actual UTF8/layout routine
as canonical commit. No producer pointer or borrowed Stage view survives capture.

The scene records descriptive owner/task/SID/sequence contexts for projected
slots. Successful CPU publication copies those contexts into its result; ordinary
SceneLease Drop erases scene payload and context storage before Main finishes.
Only a successful Full result with an exact still-pending Stage context can
update canonical text and make Applied eligible. Actual committed diagnostics
move to that boundary; intake emits a prepared diagnostic. Failure retains the
sealed candidate and the old canonical text for the next Full attempt.

Projection checks active Surface, owner and SID at the same index, and fails with
whole-scene scrub on mismatch. Canonical application repeats that association
check and copies only text/color/selection and derived layout, preserving other
Surface fields. Reset/rebind/terminal retirement cancel pending data. Context is
**descriptive under the single-Main closed-intake window**, not a capability or a
reset-resistant replay token. Surface ownership remains PD based; no Task=PD
inference or exact-task reply isolation is added.

A later legacy 0xFA/0xFB write on the same Surface cannot overwrite an unreleased
atomic candidate: Main checks ownership, then denies that mixed protocol state
before legacy mutation. Legacy transport admission remains distinct from service
application; no positive service receipt or new inline NACK is invented. Existing
modern outboxes serialize atomic transfers. Other legacy operations and other
producer intake remain outside this transaction slice.

## Evidence

- All 46 registered frozen host checks and 1,132 Python tests pass. Retained stdout
  logs record **3,210** successful Rust executions, including repeated SDK
  fixtures; this is not a unique or exhaustive test count.
- Actual Stage 24 cases cover read-only validated UTF8/selection views, exact
  contexts and release/reset/retirement exclusion. Actual Main/Scene 21 cases and
 12 compiled defect controls plus two source controls exercise owned projection,
  canonical immutability on failure, exact publication/canonical/Applied order,
  crossed/cancelled contexts and lifecycle erasure. All prior cases remain.
- Actual compositor 52 cases preserve 49 existing pixel cases and add pending
  scalar selection, width/clipping, owned-copy stability and hidden-parent glyph
  composition. Cursor 5 cases/two compiled controls/four source controls pass.
- Frame 39 cases include 10 actual Main pipeline cases. Seven pipeline and three
  paint-plan runtime defects, four ownership/compiler defects and seven source
  defects reject. The actual failed CPU output path preserves canonical text;
  successful output carries the captured contexts while canonical text still
  waits for Main finish. Binding checker 13 cases/five compiled defects/15 source
  controls verify the split intake/commit flow and no intervening raw intake.
- Eleven qualified boots/ 70 retained captures pass fresh frozen CLI checks and
  full clock pixel API replay: 16 clock/date, 34 connected/cold-recovery and 20 Browser
  captures at 1024×768/e1000 and 1280×800/e1000e. All 11 BOOT inventories and aggregate
  typed-release witnesses pass. Capture-backed glyph checks are distinct from
  metadata/admission markers; neither proves a hardware latch.
- All 1,224 frozen files/**12,293,089 bytes** match exact Git blobs and manifests.
 583/586 prior production files are unchanged; Main, scene and text Stage change,
  with no additions. Kernel and all 67 ABI input bytes remain identical to 169.
  Nine exact ISO ELFs contain **325,066 decoded instructions / 0 matched FP-vector**.

Current preflight diagnostics are retained UNQUALIFIED: the first scene harness
requested imported layout constants as if locally defined; the second exposed
fixture u16/usize selection casts. The compositor's52 actual pixel cases passed,
then its outer checker rejected the obsolete in-intake committed-marker contract.
The checker was migrated to the actual postpublication canonical flow. The final
sealed source independently passes all checks; no prior round's native acceptance
or failed source is reused as acceptance. No extra/restarted guest is counted.

The first packaging preflight inherited the old compositor case pin 49 instead
of the measured 52 and stopped before archive creation. The exact helper/log are
UNQUALIFIED. Its pin was corrected against the frozen host result; native evidence
was unchanged and no guest was rerun for this packaging metadata fix.

## Identity, cost and limits

ISO SHA256 `5be3183a73811a619ae5149684b3a3f60fe493998badefe560bbae4dc94ef9f2`.
Kernel SHA256 `26bace17d464d1ea04667a6ff1dc47514d76be472342ca4ed396000f62a20f89`.
ABI67 SHA256 `3b98b23398b7a309a74d5d10526e9234fa372ff448c79f51aa4f4b9f44fdc36a`.

Display ELF 291,760 bytes (+3,080). Static owned scene 60,096 (+512), text stages
9,088, bitmap stores 55,992 and frame-store metadata 88. Frame backing remains
8 MiB physical buddy reservation per boot. Post-seal kernel heap remains
11,851,408 used / 256,584,048 free; app-private inventory 2465 pages. These are not
additive disjoint total-RAM measurements; physical free frames remain unmeasured.

Measured local prologues: `_start` 4,728 (+32), `raster_batch` 2,344 and
`render_prepared` 1,304 bytes, each plus 48 register-push bytes. They exclude
transitive callees, interrupts and dynamic stack. This slice does not establish
speedup, hard real-time behavior, or a sufficient complete-stack bound.

Raw archive: **11,554,814 bytes /1,601 members**, SHA256
`4fe87bc0fd97cbb397ff0382ec756a709de0af57481bbf47258216bdf313062b`. Packager and Root independently reopen every member and verify
byte counts/hashes. [Raw evidence](assets/sexos-text-candidates-2026-11-01-raw.tar.gz).
[Preview](assets/sexos-text-candidates-2026-11-01.png) is a lossless conversion of
an existing qualified compact session capture, not an additional capture/boot.

Typed canonical text now has a bounded candidate/publication/commit path. Bitmap,
bar, focus, geometry, other raw intake and complete-system state still need their
remaining transaction work. Dirty is output obligation data, not a causal input
stream. No whole-system EPOCH_COMMIT/replay, hardware latch/atomic flip, current
1800s soak, Alpha, daily-driver, full-all-app reserve, hostile isolation, hardware/
SMP or physical-erasure claim. **2 Open / 28 Partial / System Proven false** remains.

## Next slice

Round 171: reduce conservative Full settlement using captured candidate identity
and explicit output coverage, including unchanged/hidden/empty settlement. Preserve
canonical commit ordering and lifecycle cancellation, and measure input/Close
responsiveness rather than assuming a speedup. Continue remaining source-state,
causal-input, interruption, hardware and replay work afterward. Under the active
80–200 plan, accepted 80–170 is 91/121; 171–200 remain 30 substantive rounds.
