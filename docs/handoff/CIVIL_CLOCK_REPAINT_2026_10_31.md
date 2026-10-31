# Civil-clock repaint suppression

31 October 2026. Fresh clock receipts no longer request a complete top-strip
repaint when both displayed text rows are identical. Every canonical receipt
still advances the receiver's sequence and refreshes observation health. The
actual rendered rows, including source/precision and stale/failure labels,
determine whether new pixels are needed.

This changes only `servers/sexdisplay/src/main.rs` among the 574 production
inputs. The source sampler, BOOT grants, strict identity/framing checks, clock
model, guest ABI and kernel are unchanged. The event loop ORs the clock's dirty
decision into existing dirty state, preserving repaint requests from other UI
work. Suppressed admissions do not manufacture completed-draw receipts.

## Measured effect

The source-bound raw checker recomputes each decision from actual admissions and
local health transitions. Its native results are:

| Workflow | Fresh admissions | Clock repaint requests | Unchanged admissions | Avoided requests |
| --- | ---: | ---: | ---: | ---: |
| Boot probe | 228 | 21 | 207 | 90.8% |
| Compact app workflow | 193 | 42 | 151 | 78.2% |
| Wide app workflow | 236 | 53 | 183 | 77.5% |
| Compact TLS responsiveness | 80 | 14 | 66 | 82.5% |
| Wide TLS responsiveness | 92 | 20 | 72 | 78.3% |

These percentages describe admissions that avoid requesting clock-driven
top-strip work. They are not CPU, total redraw, input-latency or wall-time
speedup measurements. The renderer deduplicates identical completed stamps;
the evidence's `completed_draw_receipts` counts those observations, not every
top-strip render invocation. Other UI activity may still require a real draw
after an unchanged clock admission.

## Verification

One frozen source/image/checker cohort passes GREEN_MASTER, all 42 registered
host/static checks with 2,630 passing Rust test executions, and all 906 Python
tests. Compiled mutations of the actual authority guard and actual repaint
decision are rejected. Tests cover equal-second health renewal, duplicate
idempotence/expiry, fresh recovery after staleness, source-label changes,
rollback, exhaustion and enabling civil presentation.

Six native boots preserve **68 verified captures**, all independently replayed:

- Compact and wide full app rehearsals: 40 captures, real find/replace/undo,
  save while editing, minimize/restore, Linen refresh, Spindle operations and
  BOOT-reserved successors 16/17/18, with exact terminal durable document bytes.
- Invalid-source profiles at both sizes: eight captures proving unavailable
  presentation and removal of numeric time.
- TLS responsiveness at both sizes: 20 captures with actual wire progress,
  Quil input/Save, durable payloads and clock cells before/after Browser Close.

The two app rehearsals are short and have `acceptance=false`. Both observe
`safe_completed_save_close`; actual Quil pending-I/O terminal teardown remains
unproved. Five ELF files extracted from this exact ISO decode 229,004
instructions with zero inspected vector/FP matches; this is a static scan with
the limits recorded in the machine evidence.

[Machine evidence](assets/sexos-civil-clock-repaint-2026-10-31-evidence.json)
links the [raw archive](assets/sexos-civil-clock-repaint-2026-10-31-raw.tar.gz),
which retains frozen inputs/image, native frames/serial/disk evidence, source
and pixel joins, fresh replay, raw repaint measurements, host logs and scalar
disassemblies. Every archive member is size/hash verified.

- ISO: `9ad6768dcc70a0554999b5b2f57ceebcc0cd66e32500c1ceee79deb1e9ff8aca`.
- Kernel: `7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.

## Continuing alpha work

The separately running compact and wide 1800-second sessions use the earlier
alpha-evidence image. Their live trees are excluded from this completed archive.
Each needs terminal cleanup and fresh replay before acceptance. The optimized
image needs its own sustained qualification before any latest-image claim.
The next lifecycle target is native confirmed Close while Quil I/O is actually
pending, with exact cancellation/reclamation evidence rather than completed-save
receipts.

Inventory remains **2 Open / 28 Partial / System Proven false**. GLOBAL_VAS
supplies addressing only; BOOT sanctions capability genesis, sex-pdx supplies
runtime authority and PKU enforces it. This remains the SexOS SASOS design;
global epoch commit, deterministic replay and the broader physical/storage
release gates remain open or partial as recorded in the main plan.
