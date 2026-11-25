# Quil: stage an explicit new text file

- Base: #777 (`codex/quil-cross-file-definition`). Branch: `codex/quil-stage-new`.
- Gap: Quil's Git panel listed files outside the index but could stage only tracked modifications or deletions.
- Change: N jumps among new-file candidates. S stages the selected UTF-8 file up to 64 KiB using a checked loose blob and sorted v2/v3 index entry; U restores the previous index if unchanged. Paths under `.git`, unsafe paths, extensions and conflicts are refused. Ignore rules are not evaluated, so candidate status remains qualified.
- Evidence: focused core regression checks the blob, index order, duplicate and unsafe path refusal; the QEMU editor gate stages and undoes a candidate and captures the panel. Full build, QEMU, and screenshot require exact-head GitHub CI.
- Limit: packed objects, extended indexes, ignore evaluation, broad unstaging, commits and branch switching remain open. Quil's existing read-then-rename index replacement can race a simultaneous Git writer; this slice does not claim atomic index compare-and-swap.
