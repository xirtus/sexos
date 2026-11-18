# Native file organization and voice creation for SexOS

**Design and implementation record — 18 November 2026.** The design below describes the broader product direction. The implementation checkpoint at the end distinguishes the reviewable slices now in the checkouts from later work.

## 1. Files go where they belong

### Product behavior

Give each user an **Inbox** as the default destination for unclassified imports, downloads and desktop drops. A native **Organize** panel explains the matched rule, proposed destination and origin of each file. Rules may put invoices in Documents, recordings in Audio, project media beside its project, and screenshots in Pictures. Unknown or conflicting files remain in Inbox. A user can pin a file in place, override a rule, undo a move, or ask to organize a folder now. The desktop and common folders therefore stay tidy without silently guessing at valuable data.

System files, application private stores and active project files are outside the default watch scope. Their owner must explicitly opt in. Organization is a user-level policy, not a kernel filesystem policy.

### Architecture

```text
import/download/file-manager write
        → SexFiles commit + ordered change receipt
        → Organizer service (BOOT-granted source/destination scopes)
        → deterministic rule evaluation + planned operation
        → SexFiles checked move/rename + commit
        → operation journal → UI explanation / undo
```

The **Organizer** is a separate protection-domain service reached via `sex-pdx`. SexFiles remains the sole file authority. The organizer receives narrow grants for particular source and destination roots. A rule file cannot manufacture access to another root. Current ordinary programs are confined to a single program folder in `servers/sexfiles/src/fs_service.rs`; cross-folder organization therefore needs a specific broker capability and a checked move interface, not a path trick or a broad filesystem grant.

Use Doumi's readable rule ideas and pure predicates where portable. Keep host watchers, GTK, Tokio, SQLite, shell execution and platform directory conventions out of the native service. A rule contains a version, source scope, predicates, priority, destination template and action. Matching uses a stable order: explicit user override, project affinity, specific rule, general rule, Inbox fallback. Conflicts choose no automatic move and appear in the review queue.

### File-event and transaction work

1. **First slice:** manual `Organize now` on an authorized Inbox. This proves rules and file operations without relying on a new event API.
2. **Next:** add a bounded post-commit SexFiles event stream carrying operation ID, object identity/generation, old/new path, and commit sequence. Events are delivered after a durable commit and enter the organizer as ordered input. On restart, reconcile from the last sequence; a full scan remains the fallback. Do not emit a new placement event for an already completed organizer operation.
3. **Checked move:** validate source identity immediately before action, ensure both roots are granted, reject path escape and collisions, then commit a rename when the filesystem permits it. For a cross-store move, copy to a temporary object, verify length/hash, sync, publish destination, then remove source. Journal the decision and outcome so restart can finish or roll back. Keep the original until the new object is durable.
4. **Recovery:** keep a bounded undo log with original path, destination, identity and rule version. Undo checks that the destination is still the same object and that the original name is free. Otherwise offer a conflict choice rather than overwrite.

**First QEMU gate:** import six fixtures (matching, unmatched, conflicting, pinned, name collision and escaping destination); verify exactly one placement per eligible file, preserved bytes, an understandable reason, undo, and the same result after an interrupted/restarted scan. Include capability-denial and symlink/path traversal cases supported by the actual filesystem model.

## 2. MOSSlanding as a standalone app and Moan instrument

### One job interface, two user experiences

Keep the existing desktop MOSSlanding app usable on macOS/Linux/Windows. Define a versioned **voice job contract** independent of its current UI: `create_voice`, `synthesize`, `continue`, `cancel`, `status`, `result`. Inputs are text, language, model/version, seed, reference-audio asset ID and generation settings. Outputs are immutable audio assets with sample rate, channels, duration, content hash, generation parameters and provenance. Store voices and takes in a shared SexOS voice library through explicitly granted file capabilities.

Build a native sexlife **Voice** app over that contract: reference recording/import, text editor, voice library, preview, take comparison and export. Moan uses the same contract through a **Generate Voice** action on an audio track. It creates a new clip referencing the returned asset, leaving the session responsive while the job runs; cancel and retry are visible. A new take becomes a separate clip so the user can compare or revert.

### Execution order

1. Factor MOSSlanding's Python/model runner behind the job contract and preserve its standalone desktop UI as one client. Add fixture jobs that return a small deterministic WAV without loading model weights.
2. Add a SexOS native Voice UI and a capability-scoped client to a **user-configured local/LAN worker**. Do not grant generic network access to Moan; a broker exposes only the configured worker and job operations. Handle worker absence, reconnect, cancellation, output size and version mismatch explicitly. This phase depends on SexOS network and audio paths being proven in the target boot profile.
3. Extend Moan's import path from its inspected one-minute WAV limit to a bounded chunked asset import or direct immutable source reference. Resample and channel-map at the boundary, then add an audio region through Moan's existing project model. Save the source asset ID and generation metadata with the project so reopening does not regenerate speech.
4. Only after that, investigate native inference on SexOS. MOSS-TTS currently relies on Python, MPS/CUDA and large weights. A native worker would need a supported accelerator/runtime, model packaging and bounded memory accounting; it is a separate milestone, not a prerequisite for a useful Voice app or Moan integration.

**First end-to-end gate:** submit and cancel fixture jobs from Voice and Moan; import a completed WAV into Moan; save/reopen the project; confirm clip identity, samples, metadata, and no duplicate generation. Then run a real MOSS-TTS worker on a supported host and test timeout, network loss and worker restart.

## Recommended order

1. Finish the Organizer's manual Inbox workflow and checked move/undo semantics.
2. Add post-commit SexFiles events and make organization automatic for opted-in ingress folders.
3. Define the voice job contract and build a fixture worker.
4. Ship sexlife Voice and Moan's generated-clip workflow using a local/LAN model worker.
5. Evaluate native model execution after the runtime and hardware gates are measured.

Qupid remains the SexOS media player (`apps/play` in the inspected feature worktree); Splicy remains the video editor. Blackglass belongs on the future games list. None of these needs a new port as part of this plan.

## Implementation checkpoint, 18 November 2026

The reviewable implementation now spans the main checkout and the isolated `codex/moan-workstation` worktree:

- `crates/sex-organize` provides a `no_std` rule engine with deterministic priority, pin/override handling, conflict review, and destination validation. The current Spindle policy recognizes PDF and PNG files in `/Inbox`, routing them to `/Documents` and `/Pictures`; other files stay in Inbox.
- SexFiles provides identity-checked, no-clobber moves plus atomic move-and-journal and undo operations. Only the BOOT Spindle caller can invoke organizer control operations. An opt-in marker and rolling 32-record undo ledger survive remounts. `organize --dry-run`, `organize`, `organize auto on/off`, and `organize undo` expose the workflow. Automatic reconciliation polls the durable filesystem generation from Spindle; undo also switches automatic mode off so the restored file stays in Inbox.
- `crates/sex-voice` defines the bounded SXVC v1 job and asset protocol. `crates/sex-voice/adapters/mosslanding.py` bridges that contract to the existing MOSSlanding HTTP backend or a deterministic fixture. The native `apps/voice` client sends jobs through a `+net` grant, downloads the result in checked 32 KiB ranges, validates its SHA-256 hash, and saves a WAV, result frame, and prompt in its private SexFiles folder.
- The Moan worktree adds a Voice panel with Generate, Retry, and Cancel; polls the same worker; checks ranged WAV output, hash, and PCM16 format; resamples to Moan's 48 kHz timeline; and stores both audio samples and generation provenance in version 4 editable projects. It also syncs a no-clobber recovery project after generated takes. This branch remains separate from the main checkout for review.

This is a bounded first product slice. The current automatic organizer lives in Spindle and runs while Spindle is active; there is no separate Organizer protection domain or post-commit event stream yet. The file rules are built in; the full Doumi-style user rule editor, watched project scopes, and a general conflict UI are future work. Voice and Moan currently require `run +net` and a reachable user-configured worker; there is no endpoint-scoped broker, shared voice library, native model inference, or microphone/reference recording UI. Moan still enforces its one-minute timeline limit. Moan's guest gate passed on the compatible `.worktrees/plan` ISO; the current main ISO lacks the kernel x87/SSE support required by Moan and Files and traps before their ready receipts. That kernel prerequisite must be restored before claiming main-ISO Moan integration. These limits are deliberate review points before merging or expanding the surface.

See `docs/CLAUDE_REVIEW_HANDOFF_2026_11_18.md` for exact checkout boundaries, build and guest-gate evidence, and suggested review questions.
