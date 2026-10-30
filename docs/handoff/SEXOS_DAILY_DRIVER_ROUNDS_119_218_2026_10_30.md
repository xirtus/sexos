# SexOS daily-driver continuation: rounds 119–218

Requested and reaffirmed 30 October 2026. Finish rounds19–118, then immediately continue with these100 substantive engineering rounds. Round118 is a checkpoint, not permission to stop. This is an execution queue, not completed work or a release promise. Current accepted implementation is through70;71 is undergoing qualification. The [live ledger](ALPHA_100_ROUND_CONTINUATION_2026_10_29.md) remains the acceptance record.

The goal is a useful, reliable daily-driver desktop with apps and a coherent, attractive UI. The first release boundary is a trusted-builtins QEMU alpha; a full daily driver additionally needs real connectivity, sustainable app lifetimes, recovery, protection appropriate to its threat model and a named physical target. A round count cannot grant any release label. A general modern web browser, arbitrary third-party binary compatibility and native ARM support are separate large projects unless explicitly brought into scope.

## Constitution and execution rules

One GLOBAL_VAS defines addressing. sex-pdx alone defines authority. PKU accelerates enforcement; it cannot supply missing authority or substitute for an invocation check. BOOT_DAG alone sanctions capability genesis before seal. Sext controls runtime memory use through existing sanctioned authority. Preserve exact subject/task/incarnation bindings and terminal identities. No per-process address spaces, Unix process/file-descriptor model, ambient path authority, runtime capability minting, silent mutation replay or resurrection of dead subjects. Every feature must respect [ARCHITECTURE.md](../../ARCHITECTURE.md); unresolved constitutional obligations stay visible.

Each numbered entry must produce a reviewable implementation or substantive acceptance investigation with its own result. A test run, screenshot, document edit or arbitrary split of the same implementation is not a round. Freeze source/kernel/image/checker bindings, test the relevant actual functions and native workflow, retain failed candidates and record limits. Commit and push accepted batches. Use Astra sparingly for difficult authority/concurrency reviews; routine changes stay inline.

Carry unfinished118 blockers forward before dependent features. Inspect the actual118 candidate and reorder entries when evidence requires it; record the dependency and final scope. Do not pretend a foundational gate passed because an earlier queue assigned it a number. If physical equipment or peer services are unavailable, work on independent software while keeping those gates unqualified. This plan does not authorize a new purchase or sending messages to other people.

## 119–128: protection and memory authority

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|119|Audit the exact118 candidate against the constitution, threat model and release obligations; reproduce the highest-impact unresolved authority defect.|Current source-bound reproduction and a prioritized defect inventory; no inherited proofs.|
|120|Close the first remaining raw-pointer or numeric-selector route that bypasses typed sex-pdx authority.|Actual dispatch rejects wrong-kind, aliased, stale and cross-subject inputs before effects.|
|121|Constrain access to capability tables, dispatch records and authority metadata under the chosen enforcement mechanism.|A caller cannot edit the graph through globally nameable memory; trusted-kernel assumptions explicit.|
|122|Resolve hostile WRPKRU and executable-code enforcement assumptions for supported app loading.|Concrete execution policy and hostile native probes; absence of PKU cannot silently authorize access.|
|123|Implement the missing software authority boundary for the supported no-PKU profile, or explicitly restrict that profile's supported threat model pending a constitutional solution.|Same invocation rights/rejections with and without PKU; any containment gap remains a release blocker, not a changed rule.|
|124|Audit shared aliases, per-incarnation data exposure and terminal scrub across every app/service buffer.|Whole-buffer markers cannot appear in another authorized session; retirement scrubs before successor activation.|
|125|Close remaining Sext request/completion identity and stale-fault gaps under real queue pressure.|Wrong pager, wrong leaf, reused request, terminal target and late completion have no mapping effect.|
|126|Make bounded memory exhaustion and reclamation useful to apps without granting allocator authority to the kernel or clients.|Actual pool exhaustion/recovery; no double ownership, stale translation, data leak or runtime mint.|
|127|Exercise the supported hostile-app containment boundary on the actual image.|Unauthorized loads/stores/invocations and malformed messages are contained; logs and exact threat limits retained.|
|128|Qualify the integrated protection candidate during useful desktop work.|Native notes/browser/system workflows plus containment and cleanup; unresolved architectural gaps remain open.|

## 129–138: explicit epochs and replay

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|129|Inventory all nondeterministic input to the actual candidate and define the epoch/input/capability-graph recording format.|Keyboard, pointer, timer, storage completions, network bytes and hardware snapshots have explicit provenance.|
|130|Implement bounded epoch admission and overflow/backpressure policy for one real input path.|No hidden input, silent loss or partially committed record; repeatable overflow outcome.|
|131|Move keyboard/pointer delivery to recorded snapshot→transform→swap→commit boundaries.|Same captured input produces exact semantic events and focus transitions.|
|132|Record storage outcomes and asynchronous completion ordering as explicit input.|Replaying reads/failures uses captured outcomes without new uncontrolled device effects.|
|133|Record network ingress, connection failures and timer expiry inputs.|Replay does not depend on current peer state or host wall clock.|
|134|Bind UiIR publication to actual epoch and per-sender causal sequences.|Reject out-of-epoch/stale publication; no cross-epoch partial visibility.|
|135|Make shell/lifecycle transitions consume committed epoch input and publish deterministic state.|Same terminal/activation stream yields identical subjects, surfaces and reserve accounting.|
|136|Implement bounded checkpoint storage and graph/version compatibility checks.|Interrupted checkpoints reject or recover completely; incompatible replay fails clearly.|
|137|Replay a useful mixed desktop session from a clean recorded epoch.|Compare observable state, document bytes, resource ownership and semantic output; explain every excluded nondeterministic signal.|
|138|Qualify replay under recorded queue pressure, cancellation and faults.|Actual repeated replay agrees; any remaining hidden state stays a constitutional blocker.|

## 139–148: scheduler progress and multicore

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|139|Reproduce remaining wake/park races using actual scheduler transitions and adversarial ordering.|Every queued event has one runnable or safely parked owner; no lost wake.|
|140|Close post-stack-release ownership and terminal wake races.|No runnable reference to reclaimed task/stack; exact cleanup joins.|
|141|Bound service work turns where measurements still show input starvation.|Logical work budget and native responsiveness under load; no hard time bound inferred from host timing.|
|142|Replace remaining indefinite device/service waits with explicit bounded pending states.|Input/Close progress, timeout ownership and late-completion quarantine.|
|143|Design multicore epoch/lane ordering around the constitution and current lock/custody graph.|A documented happens-before argument and executable race model; no Unix scheduler substitution.|
|144|Bring up a second execution lane with explicitly sanctioned tasks and interrupt routing.|Real multi-vCPU execution, separate lane state and deterministic input admission.|
|145|Close cross-lane IPC publication and wake ordering defects.|Queue Full/empty/terminal interleavings preserve causal order and exact ownership.|
|146|Implement cross-lane translation/enforcement updates required by Sext reclamation.|No core retains usable stale translations after acknowledged revocation; actual shootdown evidence.|
|147|Stress simultaneous app work, paging, I/O and terminal cleanup across lanes.|No double cleanup, stack reuse race, data corruption or silent dropped request.|
|148|Qualify useful multicore desktop work and replay limits.|Measured multi-vCPU sessions and repeated cleanup; physical SMP remains separate until tested.|

## 149–158: durable user data and recovery

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|149|Implement capability-backed document import/export custody beyond built-in sample objects.|Explicit object selection/grant; no ambient pathname authority or caller-address copy.|
|150|Extend document payload/encoding limits through bounded service-owned extents and explicit metadata.|Exact larger/UTF-8 payload recovery; malformed encodings and exhaustion handled without weakening current guarantees.|
|151|Implement capability-mediated user collection creation and navigation.|Selection incarnation binds every mutation; stale collection/object handles reject.|
|152|Add rename/move with atomic authority and persistent identity handling.|Old-or-new recovery and no stale name selecting a different object.|
|153|Implement deliberate delete with recoverable retention and explicit capacity limits.|No accidental cross-object deletion; retained data/reclaim policy verified after reboot.|
|154|Expose full storage and allocation pressure as useful UI states.|No false Saved state, silent truncation, uncontrolled retry or loss of the previous committed document.|
|155|Audit allocator/manifest crash consistency across competing creation, growth, deletion and reuse.|Actual cut/reboot campaign plus exact independent disk inspection; torn-sector assumptions explicit.|
|156|Implement user-selected backup/snapshot export through an explicit storage capability.|Byte-exact independent restore; incomplete or untrusted backup rejected safely.|
|157|Provide a BOOT-sanctioned recovery workflow for damaged metadata or interrupted upgrade.|Read-only inspection first, explicit selected repair, retained original evidence and no ambient repair authority.|
|158|Qualify repeated save/reboot/recovery with realistic workspace sizes.|Exact documents/history/collections survive; physical device FLUSH behavior requires separate evidence.|

## 159–168: sustainable app lifetimes and resource budgets

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|159|Measure a realistic daily session's launch/close, surface, memory and service demand.|Actual workload budget and exhaustion points; finite BOOT identity pools labeled honestly.|
|160|Design a larger finite BOOT-sanctioned incarnation reserve with explicit metadata/backing costs.|All possible authority sanctioned before seal; no dead identity reuse or runtime mint.|
|161|Implement bounded activation of existing reserved subjects with Sext-controlled backing use.|Fresh terminal-safe identities; no authority created by launch parameters.|
|162|Expand cleanup to every newly supported app/resource family.|Exact five-role or expanded role joins, whole-buffer scrub and private-frame ownership transfer.|
|163|Make reserve pressure visible and keep enough sanctioned capacity for safe recovery actions.|Launcher counts match actual activatable identities; exhaustion cannot resurrect an old subject.|
|164|Implement dirty-close choice and cancellation for document apps.|Keep editing, deliberately discard or complete verified Save; ambiguous mutation stays uncertain.|
|165|Recover app state into fresh sanctioned incarnations after a contained app failure.|Private state restored only through authorized validated objects; stale replies cannot affect the successor.|
|166|Implement sanctioned service-failure containment/recovery for one essential service.|Exact dependency/cleanup handling; no arbitrary service restart or duplicate ownership.|
|167|Persist user-selected session layout and reopen intent as data.|Fresh objects/subjects are revalidated at next boot; persisted IDs do not become capabilities.|
|168|Qualify a realistic day of launch/close/recovery and memory pressure.|Useful session fits the finite reserve with margin; sustained exact cleanup and no cumulative leaks.|

## 169–178: coherent desktop and accessibility

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|169|Measure remaining startup, focus, typing, scroll and pointer latency on the current candidate.|Per-action distributions, first-use outliers and exact glyph/pixel correlation.|
|170|Fix the largest measured shell/compositor admission bottleneck.|Same-input source-bound comparison plus pixel/erasure equivalence and no focus regression.|
|171|Implement coherent desktop spacing, typography and shared layout metrics.|Actual wide/narrow layouts; no clipped prompt, title, status or primary control.|
|172|Provide launcher search and keyboard navigation across available sanctioned apps.|Exact live logical app names, launch eligibility, focus and exhaustion states.|
|173|Add predictable workspace/tile manipulation and keyboard equivalents.|Focus remains on visible live content; pointer targets and restore geometry verified.|
|174|Implement accessible focus, contrast and scalable text settings through authorized UI state.|Readable actual pixels at supported sizes; no color-only essential status.|
|175|Complete long-document navigation, selection and caret visibility.|Exact text/selection boundaries across scroll/resize; no hidden input target.|
|176|Provide safe clipboard-like transfer through explicit origin/destination capabilities and user intent.|No ambient cross-app text access; bounded data lifetime, cancellation and terminal scrub.|
|177|Make loading, offline, error, uncertain and recovery states visually consistent.|Every state corresponds to actual service receipts; input and Close remain responsive.|
|178|Qualify complete keyboard-only and pointer workflows and visual consistency.|Useful end-to-end tasks at multiple sizes; screenshots accompany actual interaction and data proofs.|

## 179–188: useful native apps and app contracts

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|179|Define the native app manifest/interface around static BOOT sanctions and typed services.|App metadata requests no ambient filesystem/network/display authority; compatibility/version limits explicit.|
|180|Make adding a trusted built-in app reproducible through the sealed build graph.|Exact manifest→domain/routes/budget binding; undeclared dependencies reject before boot.|
|181|Extend Quil to useful multi-document editing with explicit object custody.|Independent dirty states, exact Save targets and no stale-tab mutation.|
|182|Add bounded find/replace or equivalent high-value editing workflow.|Exact Unicode/byte semantics and verified persisted result; no silent destructive replay.|
|183|Make Linen practical for user collections, metadata, import/export and recovery.|Keyboard/pointer operations target exact selected object incarnations.|
|184|Make Spindle system inspection useful without introducing a Unix shell/process model.|Typed capability-aware commands, nonblocking progress and explicit error/authority reporting.|
|185|Provide an authorized image/document viewer for one bounded useful format.|Malformed data contained; render/input/cancel/resource bounds tested.|
|186|Implement a small settings app for actual supported display/input/session preferences.|Settings reflect real behavior, survive reboot and cannot mint authority.|
|187|Provide useful system/resource diagnostics tied to real owners and service state.|No fabricated connectivity, capacity or completion; users can identify exhausted budgets.|
|188|Qualify a real user task spanning editor, collections, viewer, settings and system inspection.|Exact data transfer/save/reboot, coherent focus and complete app cleanup.|

## 189–198: real networking and online use

Carry this block forward earlier if networking remains absent at118. Existing normal SexNet is passive/offline at the70 checkpoint. Cached Kaleido content is not connectivity evidence.

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|189|Close remaining service-owned NIC/DMA ring admission and progress gaps.|Only exact BOOT leases; bounded descriptors, device completion validation and no kernel runtime packet owner.|
|190|Prove exact real Ethernet/ARP/IP exchange with a controlled peer.|Actual captured peer/guest bytes; malformed/oversize and no-device cases fail safely.|
|191|Provide real bounded TCP stream progress through explicit endpoint capabilities.|Exact bidirectional payload, segmentation/reordering/close/reset and resource reclamation.|
|192|Implement network configuration with honest link/address/route status.|No synthetic success; missing link, conflicting configuration and recovery visible.|
|193|Implement bounded DNS resolution through the authorized network service.|Actual query/response bytes, compression/length validation, timeout and cache lifetime.|
|194|Implement useful HTTP retrieval with response custody and cancellation.|Real server body, headers/length/redirect limits; no raw-pointer handoff or false online claim.|
|195|Implement TLS with explicit trust roots, entropy and time authority.|Real interoperable encrypted connection; certificate failures and missing trusted time handled honestly.|
|196|Integrate useful online Kaleido browsing/document viewing within bounded supported formats.|Actual remote content and user navigation; unsupported modern-web behavior stated clearly.|
|197|Exercise disconnect/reconnect, peer failure, hostile responses and concurrent desktop I/O.|Input/Close progress; safe retry only where protocol/idempotence permits; no duplicate mutation.|
|198|Qualify sustained online tasks and exact downloaded/saved data.|Reproducible controlled/live-peer evidence, bounded resources and honest trust/privacy limits.|

## 199–208: named hardware and everyday device support

Select a platform from hardware actually available to the user. Do not claim native ARM from x86 QEMU on Apple Silicon. Implementation can proceed in simulation, but equipment-dependent acceptance stays open.

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|199|Inventory and select one named x86_64 target and supported boot/recovery medium.|Exact CPU/GPU/storage/input/NIC devices and access constraints recorded.|
|200|Boot the sealed candidate repeatedly on that target with recoverable diagnostics.|Actual firmware/boot handoff and source-bound hardware logs; existing host OS/data preserved.|
|201|Complete real display/scanout and supported scaling/resolution handling.|Readable actual desktop, correct buffer/pitch/ownership and no framebuffer corruption.|
|202|Complete everyday keyboard/pointer support and disconnect/reconnect.|Native real-device input, modifiers, layout and usable fallback behavior.|
|203|Qualify actual storage/DMA, flush and error handling on the target.|Exact persistent bytes, repeated recovery and device-specific limits; simulation assumptions revisited.|
|204|Qualify target networking through the sanctioned service and real peer.|Actual physical link/packets/TLS workflow; unsupported controllers reject safely.|
|205|Implement capability-backed basic audio on a supported device if required by the daily workload.|Real playback/control and bounded buffers/cleanup; no feature claim from a mock.|
|206|Implement supported power/thermal/battery status and safe shutdown.|Real state, completed data flush, no fabricated battery or thermal reading.|
|207|Add suspend/resume only if device ownership and recovery can be proven; otherwise complete the next blocking supported hardware path.|No stale DMA/IRQ/mapping after resume; unsupported suspend is explicit.|
|208|Qualify a sustained useful session on the named hardware.|Real app/network/storage/input work, repeated shutdown/reboot and honest physical SMP/protection limits.|

## 209–218: installation, updates and daily-driver release gates

|Round|Concrete work|Acceptance boundary|
|---:|---|---|
|209|Define the supported daily-driver workload, threat model, target and release criteria from actual results.|Every advertised feature maps to evidence; unmet constitutional/reliability gates block the corresponding label.|
|210|Implement an explicit-target installation workflow with recoverable media and disk authority.|Preview exact selected target; no unintended host-data modification; interrupted installation recovers.|
|211|Implement verified sealed-image update selection and rollback.|BOOT sanctions remain intact; failed update returns to a complete known image with user data preserved.|
|212|Provide data/schema compatibility and migration recovery across supported versions.|Old/new image tests, complete backup and no silent irreversible partial migration.|
|213|Run extended app/memory/surface/service budget pressure with realistic usage.|No cumulative leaks or premature finite-reserve exhaustion; clear pressure/recovery UI.|
|214|Run actual interruption/error campaigns across Save, export, update, network and shutdown.|Old-or-new durable state, explicit uncertainty and exact reproducible recovery.|
|215|Close remaining measured responsiveness/accessibility defects in the daily workload.|Current-source timing distributions and usable keyboard/pointer controls under load.|
|216|Re-audit protection, epochs, replay, SMP and device authority on the exact release candidate.|No stale proof inherited after source changes; all exclusions and unsolved gaps explicit.|
|217|Perform repeatable acceptance sessions on QEMU and the named physical target, with exact reboots.|Frozen complete artifacts, independently inspected user data and successful recovery; no test-only fast path.|
|218|Package the demonstrated release, recovery instructions, reproducible build and remaining-blocker report.|Use the strongest label actually proved. If full daily-driver gates remain open, retain an alpha/beta label and continue a concrete blocker queue;218 does not fabricate completion.|

## Checkpoints and continuation

At118, record the exact accepted image, remaining release blockers and the final adjusted119 start; immediately start that work. At each ten-round boundary, reassess dependencies, measured progress and daily-user value. Keep software work moving while external hardware gates await evidence. Record accepted rounds individually in the ledger with links to source/image/host/native artifacts. This100-round plan adds no completed round by itself.

Do not call the OS a full daily driver while ordinary user-data recovery, app reserve sustainability, useful real networking, supported hardware operation or a required constitutional gate is unresolved. Conversely, do not defer useful visible improvements merely because a larger research obligation remains open: implement both with honest boundaries and retain the user's requested sequential continuation through218.
