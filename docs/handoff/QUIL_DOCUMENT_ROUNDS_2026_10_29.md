# Quil document rounds 14–17

29 October 2026. This continues the [alpha plan](ALPHA_DAILY_DRIVER_NEXT_SESSION_2026_10_29.md). SexOS retains one GLOBAL_VAS, BOOT-only capability genesis, sex-pdx authority, immutable lifecycle catalogs and terminal incarnation identities. Architecture text remains unchanged. No Astra agents were used.

## Round 14 — main-loop loading

Added a separate register-only document exchange (0x4D, tags 0x55/0xE5). Only actual live BOOT Quil subjects 9/16 may use it. Their pre-existing storage route grants access to the shared document workspace: legacy document 2 and user slots 3–14. Object numbers are data within that existing service scope, not capabilities. Foreign subjects cannot acquire a document session. SexFiles owns each generation-bound selection and immutable staged snapshot; Quil receives bounded chunks into private staging. Exact sender/sequence checks, retained requests and receipts under Full, nonwrapping sequences and cooperative-turn deadlines protect the exchange. A complete load replaces the editor only if its captured revision still matches. Close scrubs staging; terminal service retirement precedes cleanup acknowledgment.

The live palette and Linen load paths now use this exchange. The synchronous helpers remained for startup diagnostics, and this historical image still ran the old unconditional RamFS fixture at boot. Round 15 migrates live save paths; round 17 skips that normal-boot fixture and retains only explicitly enabled legacy diagnostics. Source hashing now includes servers/quil/src, which the earlier broad source inventory omitted.

Accepted evidence: [round 14 binding](assets/sexos-quil-r14-2026-10-29-evidence.json), [trace](assets/sexos-quil-r14-2026-10-29-trace.json). Sealed BUILD PASS/GREEN_MASTER (90 seconds), 38 architectural host gates, 551 Rust test executions and 255 Python tests. Native QMP input edited while loading, retained the newer editor revision, retried successfully, opened document 2 from Linen, and closed with a load pending through terminal service/Sext cleanup. These are historical source/image bindings, not a whole-system protection proof.

## Round 15 — revision-bound Save and Save As

Completed. Save and Save As upload immutable editor revisions through the main loop. Save As adopts its separately named object's identity only after commit completion. A newer Linen selection supersedes the prior load through a fresh own-session abort. Cancellation acknowledges quiescence, not rollback. The interim round-15 adapter used QP01; round 16 replaces that live write path with durable banks.

Accepted evidence: [round 15 binding](assets/sexos-quil-r15-2026-10-29-evidence.json). BUILD PASS/GREEN_MASTER (90 seconds), 38 host gates and 554 Rust test executions. Native input edited during Save and the completion retained dirty state. Two named objects retained distinct bytes after Quil 9 Close, BOOT activation of Quil 16, refresh of Linen and actual object selection/reopen. The selection walkthrough exposed and fixed a race where a newer Open had previously been discarded while an older load could still publish.

## Round 16 — durable transactions

Completed. QJ02 stores two independently checked banks inside each existing V4 workspace object. It uses SexFiles' existing BOOT block route and allocator, with no new disk reservation or runtime capability. Fixed capacity is established before bank reuse; V4 growth now orders data, descriptor and manifest through actual flush barriers. Initial QP01 conversion writes bank 1 beyond the complete bounded legacy payload. A save invalidates the inactive bank and flushes, writes and flushes its payload, checks it, publishes a checked generation/length/object-incarnation/hash header, flushes and reads it back. Acknowledgment follows those steps. The previous complete bank survives an interrupted update.

Bindings retain the exact service subject and slot incarnation. Save checks the identity previously opened by the client instead of accepting a freshly recycled slot through a new INFO response. Delete/create counters stop at exhaustion instead of wrapping. Another subject cannot address a session; protected slots 0/1 are excluded. Documents intentionally share the BOOT-authorized workspace between original and reserved Quil incarnations; this is not per-app secret document isolation.

Accepted evidence: [round 16 binding](assets/sexos-quil-r16-2026-10-29-evidence.json). BUILD PASS/GREEN_MASTER (90 seconds), 38 host gates, 559 Rust test executions, 255 Python tests. Native Boot A saved two distinct documents, closed/relaunched Quil and reopened them. Boot B used the same sealed image and copied disk; both recovered byte-for-byte through Linen. An independent read-only decoder checked the QJ02 payloads on disk. Production journal host tests cut every modeled read/write/flush operation, including torn journal-sector writes, and recovered only complete old/new payloads; they also cover empty/12 KiB documents, repeated commits and wrong identities.

The crash model assumes ordered FLUSH and atomic sectors for V4 allocator/manifest metadata. Physical torn allocator metadata, dishonest device flushes and arbitrary hardware power loss are not established. A failed or cancelled request may already have committed; no rollback is promised.

## Round 17 — readable document workflow

Completed. Quil has visible New/Open/Save/AS controls, an actual command menu, dirty/loading/saving/error/uncertain/quiescent states and app-local Ctrl N/O/S/A/R actions. Open focuses Linen; AS creates a separate copy. Ctrl R quiesces an uncertain exchange, then the user explicitly reopens to inspect saved data. It never automatically replays a mutation.

Text and caret follow the current owned surface geometry and fit the fixed 512-byte render budget. Display submission retains one immutable active batch and the latest desired text, with four attempts per loop turn. Close clears both. Admission remains distinct from actual pixel presentation; native gates compare complete text/caret glyphs against framebuffer pixels.

The original Quil 9 now receives an explicit BOOT shell route, matching its reserved successor's existing route. Shell custody allows only the two exact live document operations with exact task arguments. Main-loop READY asks for current geometry after startup. Normal boot skips the old unconditional synchronous RamFS demo fixture; explicitly enabled legacy diagnostics remain separate from live user jobs.

The compact gate exposed older fixed display/pointer assumptions. Immutable BOOT display metadata now supplies real dimensions/pitch to the compositor and shell before rendering; only the display subject can query its existing framebuffer address. Tablet coordinates remain absolute through SexInput and normalize against current shell geometry. Shared integer font/inset/line tokens drive rendering and toolbar hit tests. The same sealed image has normal and compact menu entries using the same kernel/modules. The supported Limine key is [RESOLUTION](https://raw.githubusercontent.com/limine-bootloader/limine/v7.x/CONFIG.md).

The final-image reboot also exposed a real Collar gap: its old seed-object grants covered only IDs 1–6, so the second newly created note could not open. Added an explicit revocable workspace-document policy for Linen over its existing BOOT storage scope. It matches only complete published snapshot members for document slots 2–14 and only Open/Link operations. The workspace rule rejects foreign subjects, missing/unpublished entries and inactive policy; explicit object denials take precedence. Existing exact grants retain their own scopes. It is shell policy data, never a minted kernel capability. Protected proof slots 0/1 are rejected before the legacy zero encoding could alias them to document 2.

Named limits: QEMU, trusted built-ins, ASCII notes up to 12 KiB, fixed workspace slots, finite BOOT successor reserve, contiguous framebuffer pitch. Cold allocation and service-side I/O still need latency measurement and further scheduling work. Networking, physical hardware, SMP and hostile-code containment remain separate gates.

## Final verification and next session

The final sealed image passed BUILD and GREEN_MASTER for 90 seconds. All 38 architectural host gates passed, with 570 Rust test executions and 258 Python tests. Both normal 1280×800 and compact 1024×768 workflows passed eight full text/caret glyph stages, actual keyboard/USB-tablet actions, edit-during-save dirty retention, pointer Save and Save As, zoom/retile, disappearing Quil ink on minimize, and restored glyphs. Both notes then reopened byte-for-byte on a fresh boot of the same image, including the newly authorized published workspace object beyond the old seed IDs.

The full native Close/relaunch regression passed all five app incarnations. Sext retired and zeroed 299 private pages after all five service acknowledgments and translation flushes. The history reboot regression also uses this final image and retains BOOT-sanctioned recovery with fresh private handles. See the [final evidence binding](assets/sexos-quil-r17-2026-10-29-evidence.json) and [native trace](assets/sexos-quil-r17-2026-10-29-trace.json). Each earlier round's evidence remains bound to its historical sources and image.

Start next session at [round 18 in the alpha plan](ALPHA_DAILY_DRIVER_NEXT_SESSION_2026_10_29.md). The 30-minute mixed-app candidate soak, measured cold/warm storage and UI latency, and a sustainable BOOT incarnation reserve remain release gates. This work does not tag a general daily driver or establish physical power-loss, SMP or hostile-code containment.

Final ISO SHA-256: `8c9a7dfd576ec7a2861539d8c8037386568dbb51479807cc215de47f2bbb43b8`. The final normal and compact UI runs each observed 20 actual NVMe flush completions; the full Close/relaunch run observed 22. These counts describe the recorded runs, not a latency guarantee.

Native captures: [1280×800 restored editor](assets/sexos-quil-r17-ui1280-restored-2026-10-29.png), [1024×768 restored editor](assets/sexos-quil-r17-ui1024-restored-2026-10-29.png), [compact dirty-during-save state](assets/sexos-quil-r17-ui1024-newer-edit-2026-10-29.png). Full glyph checks and source/image/disk hashes are retained in the evidence binding. No Astra agents were used.
