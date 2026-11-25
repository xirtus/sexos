# Spreadeagle ODS list validation — 2026-11-25

Spreadeagle's XLSX list rules worked, but saving as ODS silently removed them. ODF `table:content-validations` now carries literal lists and current-sheet range sources, and validated blank cells get their rule name in `content.xml`. ODS import restores the same rules to GridCraft, so dropdown choices and stop-style entry checks work after reopen.

The focused host test saves an XLSX workbook as ODS, checks its XML, reopens it, tests both list types and edits a live source. The Spreadeagle QEMU gate repeats save/reopen and captures `spreadeagle-ods-validation.ppm` after rejecting an invalid entry. Full builds and QEMU run on GitHub CI.

Limit: ODS export returns an error for unsupported non-list validation and list ranges above 100,000 cells instead of silently discarding them. Charts, frozen panes, and AutoFilter state remain separate ODS gaps.
