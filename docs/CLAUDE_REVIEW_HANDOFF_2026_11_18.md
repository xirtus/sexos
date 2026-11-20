# SexOS file organization and voice: Claude review handoff

Date: 18 November 2026. **Review only; nothing in this handoff has been merged or committed.**

## Where the changes are

1. Main checkout: `/Users/xmacbookm5/Projects/Sex` on `master`. New work includes `crates/sex-organize`, `crates/sex-voice`, `crates/sexfs/src/organizer.rs`, `crates/sexfs/tests`, `apps/voice`, `scripts/spindle_organize_gate.py`, `scripts/program_voice_gate.py`, the MOSSlanding bridge under `crates/sex-voice/adapters`, and changes in `apps/spindle`, `crates/sex-app`, `crates/sex-pdx`, `crates/sexfs`, `servers/sexfiles`, `Cargo.toml`, `Cargo.lock`, and `sexos_build_spec.toml`.
2. Isolated Moan checkout: `/Users/xmacbookm5/Projects/Sex/.worktrees/moan-workstation` on `codex/moan-workstation`. Review changes in `apps/moan`, its local `crates/sex-app` network client and its copied `crates/sex-voice` contract. This branch has **not** been brought into the main checkout.

The main checkout had extensive unrelated pre-existing dirty/deleted `docs/handoff/assets` content and other untracked work before these slices. Do not stage, reset, or interpret the whole `git status` as this change set. Review the paths above explicitly.

## Behavior implemented

### Files

- `organize --dry-run` previews PDF and PNG placement from `/Inbox`; `organize` performs a no-clobber move after checking source object identity. Unknown types remain in Inbox.
- `organize auto on/off` controls an opt-in durable marker. Spindle compares the committed SexFiles generation and reconciles Inbox when it changes, including after reboot. The BOOT Spindle caller alone can invoke the organizer control protocol.
- Moves and a rolling 32-entry undo ledger commit atomically. `organize undo` checks the moved object's identity and source collision, restores it, and disables automatic mode so it remains in Inbox.

### Voice

- `sex-voice` defines bounded SXVC v1 submit/status/cancel/result frames. The Python bridge offers fixture synthesis or forwards to the existing standalone MOSSlanding backend. The original MOSSlanding repository was not changed.
- `apps/voice` runs with `run +net voice http://HOST:PORT [VOICE_ID]`, submits typed text, polls, cancels, fetches a 206 range WAV in 32 KiB chunks, verifies SHA-256, and stores `.wav`, `.sxvc`, and `.txt` in `/data/voice`.
- Moan offers a Voice panel with Generate, Retry, Cancel, an editable worker URL, fixture or actual worker submission, status polling, checked ranged download, WAV validation and 48 kHz conversion. A completed take enters as a new audio track. Version 4 project files retain the PCM and generation provenance so reopen does not regenerate the clip. Each generated take also writes a synced, no-clobber recovery project in Moan's private folder; snapshot failure leaves the in-memory clip intact and is reported.

## Verification

- `make full` in the main checkout passed with the updated sealed ABI hash and staged `programs/voice`.
- `cargo test -p sexfs --test organizer --offline`: 3 passed, including remount, stale/collision, 33 moves and 32 undos.
- `cargo test -p sexfs --test checked_move --offline`: 2 passed.
- `cargo test -p sex-organize --offline`: 7 passed.
- `cargo test -p sex-voice --offline`: 3 passed.
- `python3 -m unittest discover -s crates/sex-voice/adapters -p 'test_*.py'`: 4 passed.
- `python3 scripts/program_voice_gate.py --iso sexos-v1.0.0.iso --source-disk /tmp/sexos-organizer-blank.img --out /tmp/sexos-voice-gate-final-a`: passed; guest generated a 52-byte fixture WAV and saw all three saved files after reboot. Evidence is in `/tmp/sexos-voice-gate-final-a`.
- `python3 scripts/spindle_organize_gate.py --iso sexos-v1.0.0.iso --source-disk /tmp/sexos-organizer-blank.img --out /tmp/sexos-organizer-gate-final-c`: passed 21 command cases over three boots, covering opt-in mode, undo, and reboot persistence. Evidence is in `/tmp/sexos-organizer-gate-final-c`.
- Moan branch `cargo test -p moan --no-default-features`: passed. Native target check and release build passed. `scripts/moan_voice_gate.py` passed against `.worktrees/plan/sexos-v1.0.0.iso`, whose kernel supports x87/SSE state. The guest submitted to the worker, downloaded a 206 WAV, logged `[moan.voice.imported]`, wrote and synced a 303-byte version 4 recovery project, rebooted, and logged `[moan.project.opened]` with one track and one clip. Evidence: `/tmp/sexos-moan-voice-oldiso9/result.json` and its serial logs.

## Review focus

1. Inspect checked move, atomic move/undo journal, BOOT-only protocol guard, and generation reconciliation for identity, collision, restart, and authorization errors.
2. Inspect `sex-app::net` request construction, bounds, timeouts, partial HTTP reads, and cleanup on early failure. Verify Voice's range/length/hash checks and failure behavior around `.partial` files.
3. Inspect Moan's resampling bounds, project version migration, provenance alignment with tracks and undo/redo, worker job state, and responsiveness while network calls are active.
4. Check that the eventual Moan merge reconciles its copied `sex-voice` and modified `sex-app` with the main checkout versions instead of taking either side wholesale.
5. Restore or port the kernel's x87/SSE task support before claiming Moan works on the current main ISO. The current main ISO launches the same Moan ELF but traps `INVALID OPCODE` at a Weft `movss` before `[moan.ready]`; Files shows the same symptom. The older `.worktrees/plan` ISO admits the identical Moan ELF and reaches the voice worker. That worktree has `kernel/src/fpu.rs` and scheduler x87/SSE state, while the current main checkout lacks them. The kernel prerequisite is outside these file/voice edits and has not been applied to main.

## Current scope limits

Automatic organization runs inside active Spindle, with fixed PDF/PNG rules for Inbox. There is no separate Organizer protection domain, post-commit event stream, user rule editor, general watched-folder scope, or full conflict UI. The Voice and Moan clients need a broad `+net` grant and a reachable worker; there is no endpoint-scoped broker, common voice library, native model inference, or reference-recording UI. Moan still accepts at most one minute of audio. The Moan guest gate verifies its automatic recovery project and cold reopen; manual Save through the Files picker remains unproven because the compatible older ISO's picker does not answer the newer request. The fixture guest gate does not establish that a real MOSS model has run on the host. These are explicit follow-on decisions, not claims of completion.

Related design and repo-priority context: `docs/FILE_ORGANIZATION_AND_VOICE_PLAN_2026_11_18.md` and `docs/XIRTUS_REPO_INTEGRATION_REPORT_2026_11_18.md`.
