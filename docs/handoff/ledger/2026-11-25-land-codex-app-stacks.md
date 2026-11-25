# 2026-11-25 — Land Codex's native-app stacks

Codex ran out of usage with 284 stacked draft PRs open (codex/* branches). This
change lands every stack in one merge so CI runs once:

- Assuit (section running text, PDF page choices, tables, lists, proofing, print)
- Spreadeagle (ODS charts/filters/freeze/validation, chart editing, AutoFilter)
- Vellum (forms, choice fields, highlights, notes, Unicode/case/hyphen Find, Shibari signing)
- Kaleidoscope (site settings, scripts per origin, downloads, reader tables, autofill, private history)
- Date (crash-safe save, Bell notices, CalDAV conflicts, undo/redo)
- Quil (LSP client, git panel staging, syntax)
- Loom / SexFiles (tabs, multi-select, batch copy/move, tar formats, previews, trash recovery, redo)
- Valentine mail (IMAP/SMTP through Multipass broker, staged attachment export)
- Flirt branding, CI memory-proof opt-in

Integration fixes on top of Codex's work:
- Files request kinds: Valentine keeps 52-58; `REPLACE_IF_SHA256` = 59, `CREATE_WITH_DATA` = 60 (ABI hash regenerated).
- Assuit: "this section" running text never worked (shared story id); fixed. Insert Picture read cap; engine name removed from an error.
- Valentine: IMAP untagged responses bounded at 128 MiB per command.
- Loom: trash redo can no longer overwrite an existing file; trash undo checks the original path is free.
- Kaleidoscope: four site-scripts records now use the private-diagnostics gate.
- Vellum: host build (`pdf_sign` import), undo tests, fixture regeneration, picker ignores clicks during prompts.

Known gaps left open (from review): Shibari signer registration is not pinned to the
Shibari image; Shibari approval shows "Program row N"; organizer moved-in event skipped
for REPLACE_IF_SHA256 into home; ODS embedded chart XML read without a size cap.
