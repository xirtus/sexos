# SexOS desktop growth plan

The target is a dependable single-address-space desktop with useful apps and a
coherent visual experience. `GLOBAL_VAS`, immutable BOOT_DAG capabilities and
sex-pdx remain the architectural boundary. A task lifecycle is scheduler custody
and capability-authorized service state; it does not introduce Unix processes,
per-process page tables or ambient resource authority.

## Current delivery

1. Two boot-authorized sext clients share a bounded frame pool with independent
   quotas. Explicit release invalidates old mappings before frame reuse; stale
   messages cannot reclaim a replacement. Actual guest faults recover.
2. Task-exit cleanup reclaims all of a dead
   client's demand mappings only after its execution and stack custody are gone,
   including frames whose fault completion was interrupted. Never reclaim another
   live app's page or use a cleanup replay to free its replacement.
3. Dependable document opening. Reboot validation exposed a foreign-service
   reply being decoded as Linen metadata, producing duplicate object IDs and
   routing Enter to the wrong item. Reply origins are now authenticated, complete views are staged privately, and
   the saved-document → Linen → Quil path passes guest reboot verification.

All three items are implemented and guest-verified in this wave: ordinary finish
and fault rejection reclaim 4 resident pages; the survivor reuses 3 while retaining
its sentinel; reboot tests restore and open the exact 242-byte saved document.
Detailed scope and evidence are in the [delivery report](SEXT_MULTI_CLIENT_PAGING_AND_SAFE_RECLAMATION_2026_10_28.md).

## Observed visual baseline

The [native guest screenshot](assets/sexos-desktop-baseline-2026-10-28.png) shows
large sparse content areas, tiny text, narrow/clipped app content and inconsistent
window geometry. It is a regression baseline, not a polished-desktop claim.
Prioritize shared layout/clipping, readable typography and focus contrast before
adding more decorative surfaces. Validate those changes with screenshots and
keyboard/pointer walkthroughs at the supported resolutions.

## Next visible milestones

| Priority | User-visible gain | Concrete delivery | Acceptance evidence |
|---|---|---|---|
| 1 | Opening an item always opens that item | Typed, correlated Linen snapshots; stable unique selection IDs; preserve the previous complete view during refresh; clear loading/empty/error states | Repeated keyboard and pointer opens during concurrent Bell/display traffic, correct document ID and saved content after reboot |
| 2 | Apps feel responsive while services work | Keep input and rendering serviced during storage waits; bounded asynchronous app requests; cancellation/error state instead of frozen UI | Type, move and switch focus during multi-block saves; no lost input, stalled cursor or mismatched reply |
| 3 | App failures leave the desktop usable | Capability-scoped app lifecycle notifications, cleanup completion, surface ownership retirement and honest recovery affordances | Terminate a test app with live pages and a window; its surface retires, other apps keep state, resources return, no stale focus or ghost window |
| 4 | A coherent, beautiful desktop | One palette/spacing/type scale shared by shell, Linen, Quil, Spindle and Bell; clear active/inactive focus; aligned title bars; legible empty and error states | Native guest screenshots at supported sizes, keyboard focus walkthrough, readable selection/caret states and contrast checks |
| 5 | Daily work survives interruption | Document dirty/saving/saved/error states, transactional save and session restore through existing SexFiles/Linen authority | Edit → save → reboot → reopen exact content; interrupted save preserves the previous complete version |
| 6 | Several useful apps work together | Quil documents, Linen object browsing, Spindle commands, bounded background services and explicit app-to-object intents | Scripted end-to-end desktop sessions with multiple windows, input, app failure and reboot persistence |

Deliver one tested vertical slice at a time. Avoid cosmetic claims based only on
serial markers: visible changes need guest screenshots and input walkthroughs.
Avoid lifecycle claims based only on an exited flag: cleanup needs ownership,
translation invalidation, generation checks and successful reuse evidence.

## Boundaries that remain explicit

The current pool and clients are fixed at boot. Physical APs remain parked; a
local invalidation is not an SMP shootdown. Trusted pager aliases remain mapped.
At the initial baseline, task cleanup did not reclaim ELF images, user stacks,
display surfaces or service-owned objects. The 29 October delivery below adds
these for three enrolled apps. Driver DMA, capability slots, kernel stacks and
control-object backing remain retained. Dynamic app restart
requires a separately designed incarnation/authority protocol; dead domain IDs
must not silently acquire a new runtime identity. Full epoch/replay, hostile-code
containment and expandable physical-memory management remain separate work.


## App cleanup and responsiveness delivered on 29 October

The next implementation now spans native Close, five service acknowledgements,
private ELF/user-stack detach and Sext zero/reuse. Linen refresh is asynchronous;
actual app registration and first presentation have explicit feedback, normal
boot input is no longer synthetic, and body/title text is more readable. Linen
renders real filenames and selection; native app content fits its frame during
tiling and resizing, and minimized child surfaces preserve their text.
See [the implementation and integration report](APP_RESOURCES_AND_DESKTOP_2026_10_29.md)
for actual evidence and limits. BOOT app identities remain terminal after Close;
relaunch and safe incarnation authority are the next lifecycle milestone.
