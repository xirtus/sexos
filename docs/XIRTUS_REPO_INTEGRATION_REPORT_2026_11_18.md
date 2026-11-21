# Xirtus repositories that could benefit SexOS

**Review date:** 18 November 2026  
**Scope:** 316 repositories returned by the connected GitHub account for `user:xirtus`, including 45 private repositories. I screened the full name/visibility inventory, then read repository metadata and selected READMEs/manifests for the most relevant candidates. This is an integration assessment, not a build, security, or license audit of every repository. Private links below require account access.

## Recommendation

Build a **native file-organization service inspired by Doumi** first. Its rules have a clear everyday use, and SexOS already has a file service with list, create, remove, rename, read, write, and sync operations. The service should make files land in predictable places, with reviewable rules and recovery. Keep the existing Linux/BSD/macOS/Windows Doumi daemon and GUI as separate front ends.

Use [`sexlife`](https://github.com/xirtus/sexlife) as the home for large creator applications, following its own README. Keep SexOS's kernel, servers, SDK and small built-ins in `sexos`. Reconcile existing `Moan`, `Splicy`, and `Polyamour` sources before copying anything: their READMEs describe existing SexOS integration or mirrors, so treating them as fresh ports would create duplicate code.

| Priority | Repository | Useful SexOS outcome | Integration route | Main dependency or caution |
| --- | --- | --- | --- | --- |
| **1** | [Doumi](https://github.com/xirtus/Doumi) | Integrated native organization service for SexFiles | Extract or adapt rule semantics; put policy in a capability-scoped user-space service; give users an inbox, previews, undo and clear destination rules | No native filesystem event subscription found in the inspected SexFiles API. Host daemon uses `notify`, Tokio, SQLite and platform APIs. |
| **2** | [sexlife](https://github.com/xirtus/sexlife) **private** | Clear packaging boundary for large apps | Make it the app-suite integration target and document SDK/toolkit versions; retain source history when moving apps | Its README says `Foreplay` is a submodule and office apps move later. Current repository is mostly structure, not a ready app bundle. |
| **3** | [Moan](https://github.com/xirtus/Moan) + [Splicy](https://github.com/xirtus/Splicy) + [Polyamour](https://github.com/xirtus/Polyamour) | Audio, video and 3D creator suite | Reconcile current SexOS branches/worktrees with their repositories; make one authoritative source and repeatable build gates | Moan says it is kept in step with SexOS; Polyamour calls itself a mirror; Splicy says it builds inside SexOS. Avoid importing a second copy. |
| **4** | [MOSSlanding](https://github.com/xirtus/MOSSlanding) | Standalone voice creation in sexlife and voice generation inside Moan | Share one voice-job contract and asset library; first connect SexOS to a local/LAN worker, then consider native inference | Current app runs Python model code on Apple Silicon or NVIDIA; Moan's inspected import path accepts bounded WAV files. Model execution and larger import need separate work. |
| **5** | [Foreplay](https://github.com/xirtus/Foreplay) + [3JSE](https://github.com/xirtus/3JSE) | Game creation and playable content | Continue the Foreplay/sexlife path; reuse engine data formats and game content where practical | 3JSE is a pre-alpha TypeScript/WebGPU engine. A native SexOS runtime needs a separate renderer and input bridge. |
| **6** | [BrootLite](https://github.com/xirtus/brootlite) **private** / [brootlite-stream](https://github.com/xirtus/brootlite-stream) | Reference material for Qupid | Reuse only features or protocols Qupid still needs | The `feat/qupid-brootlite-native` worktree contains the native `apps/play` player described as Qupid in its user documentation. The separate `apps/qupid` source is still an old UI stub. Do not start another media player port. |
| **Later** | [Blackglass](https://github.com/xirtus/Blackglass) | Candidate for a future native games collection | Evaluate when the games runtime and content pipeline are ready | It is not a current SexOS priority. |

## Other promising ideas, with lower near-term payoff

- [Wanswift](https://github.com/xirtus/Wanswift): creative workflow with a macOS/Python runtime. A later SexOS app could use a local or LAN inference worker after network, video and job-control APIs are ready.
- [arcade](https://github.com/xirtus/arcade): catalog of small games. Use a simple title as an input, audio and rendering smoke test. Check each game's asset license and renderer requirements individually.
- [Lugosi](https://github.com/xirtus/Lugosi) **private**: conceptually adjacent OS/web-shell work, but its TypeScript web architecture is a reference for interaction design, not a direct kernel or server import.
- [Cisi](https://github.com/xirtus/Cisi): tiny CLI/skill content that might fit a terminal or developer-tools package, but it does not unlock a core SexOS capability.
- [evidence](https://github.com/xirtus/evidence) **private**: use as historical gate receipts and provenance. Its README identifies it as an archive, so it is evidence infrastructure rather than an app to integrate.

## Do not count these as ready SexOS ports

Many account repositories are forks of substantial third-party systems, including `voicebox`, `zoxide`, `atuin`, `filebrowser`, `airi`, `smithay`, `Hyprland`, `servo`, and emulator projects. A fork in the inventory does not imply a maintained Xirtus implementation or an easy native port. The account also contains project backups, assets, sites and domain-specific apps. They may supply ideas or content, but the OS's capability and ABI constraints still apply.

## First integration slice: Doumi

1. Define a minimal portable rule schema: path scope, extension/name/size predicates, and actions `move`, `rename`, or `archive`. Keep command execution disabled until SexOS has an explicit executable capability model for rules.
2. Extract or reimplement the pure matcher with fixture tests. The current `doumi-core` manifest includes host-centric crates (`dirs`, `walkdir`, `trash`, `notify-rust`, `xattr`), so a direct dependency in the native image is unlikely to work.
3. Build a SexOS worker that receives only BOOT-granted access to selected folders through SexFiles. Existing programs are confined to one program folder, so cross-folder organization needs a dedicated broker interface with explicit source and destination grants. Use list/stat/read and create/rename/sync, with path validation and operation receipts. Start on demand from the file manager or shell; later add a post-commit event stream.
4. Add a native rule editor after the worker has a stable schema. Present a dry-run preview before moving files; ensure a rule cannot escape its granted root or recursively reprocess its own outputs.
5. Gate the slice in QEMU with a fixture directory: matching file moves once, nonmatching file stays, escaping destination fails, repeated scan is idempotent, and interruption leaves recoverable state.

## SexOS constraints that affect every candidate

SexOS is an experimental single-address-space, capability-controlled OS, not a Linux binary target. The local README reports a QEMU x86_64 boot and unstable syscall/ABI surface. Its architecture requires BOOT_DAG-sanctioned capabilities, `sex-pdx` authority, and deterministic input handling. The inspected SexFiles operation list offers file operations but no watcher/subscription operation. Host GUIs, POSIX daemons, Swift/GTK apps, WebGPU engines, Python model servers, and FFmpeg/libmpv integrations therefore need a port, a service boundary, or a later platform milestone rather than a package copy.

## Sources checked

- SexOS local [README](../README.md), [architecture](../ARCHITECTURE.md), [`sex-pdx` file protocol](../crates/sex-pdx/src/fs.rs), and [SexFiles service](../servers/sexfiles/src/fs_service.rs).
- GitHub repository metadata and READMEs/manifests linked above, especially [Doumi core](https://github.com/xirtus/Doumi/blob/main/crates/doumi-core/Cargo.toml), [Doumi daemon](https://github.com/xirtus/Doumi/blob/main/crates/doumi-daemon/Cargo.toml), [Rook engine](https://github.com/xirtus/Rook/blob/master/crates/rook-engine/Cargo.toml), and the [sexlife README](https://github.com/xirtus/sexlife/blob/main/README.md).

Repository descriptions and README feature claims were treated as project claims. The rankings are my assessment of usefulness and integration effort, not proof that the software runs on SexOS today.
