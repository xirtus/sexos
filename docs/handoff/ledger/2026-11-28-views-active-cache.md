# 2026-11-28: Protection view switch skips its CR3 read when the view is unchanged

Safe version of Gemini's active-view cache (from #699, left out of #829).

- `kernel/src/views.rs` `enter`: with interrupts off, compares the view index in
  `CoreLocal::active_view` (gs:[56]) first and returns at once when it matches; it is
  stored after the CR3 check or write. A task switch between two tasks in the same view
  now costs no CR3 read.
- Why this is safe where the first version was not: the check runs with interrupts off
  (no migration between the read and the compare), the cache is written only after CR3
  really holds that view, and view roots are made once in `views::init` and never
  change. Every other CR3 writer keeps the root CR3 holds (`invalidate`,
  `hal::tlb_flush_local`, the VMX host CR3, which is read from CR3). The current view is
  never marked stale (`views_rules::stale_after_invalidate`), so the skip loses no flush.
- `kernel/src/core_local.rs`: field doc only; whole-file pin re-taken in
  `scripts/check_self_wait.py`.
- No ABI change.
