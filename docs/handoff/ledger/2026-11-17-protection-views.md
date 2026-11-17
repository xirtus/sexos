# Protection views with PCID (2026-11-17, kernel plan Phase 5b, decision D2)

- **One translation, several roots.** `kernel/src/views.rs`: view 0 is the canonical
  tables every mapping is made in; each shared-key program row (targets 44-49, key 8)
  gets a view of its own. A view copies only its root and the PDPT/page-directory pages
  above the spans it opens (`views_rules::open`); every page table is shared, so a leaf
  written once is seen everywhere and the translation stays global.
- **Rows no longer take turns.** A row's spans (arena, regions, and its BOOT image and
  stack spans, checked at BOOT to hold only its own pages) are closed in the canonical
  tables and every other view, open in its own. Switching rows is one CR3 write with no
  flush and no PDE edits. `program_tenancy` lost its tenant, its full TLB flush and its
  present-bit toggling of BOOT pages; shared rows now spread over the program cores like
  other programs (`smp::home_core`) instead of all sitting on the last one.
- **PCID.** Each view is tagged with its index. INVLPG and CR3 reloads reach only the
  current PCID, so `smp::tlb` now calls `views::invalidate`, which also marks this core's
  other views stale; a stale view is flushed alone on its next entry. This replaces
  `x86_64::instructions::tlb::flush_all`, which would rewrite CR3 with a mangled PCID.
  Without PCID support every view switch flushes, which stays correct. Records keep their
  own present-bit turns on the guest core, with the same PCID-aware invalidation.
- **Isolation without PKU between rows.** Row separation is by page-table permission, not
  by key, so it holds on CPUs without PKU. Giving every organ a view when PKU is absent is
  the next step (not in this change).
- **Static rule.** `no_per_process_tables_gate.sh` sanctions `views::set_cr3` as the second
  CR3 site; the NO_PER_PROCESS_TABLES obligation text names views.
- **Proof.** Host: `scripts/protection_views_host_gate.sh` (7 tests on the real
  `views_rules.rs`). QEMU (CI `profiles` / `views`): `program_pool_gate.py` on 4 cores
  (six programs drawn, no `[program.pool.switch]`, at most half of view switches flush
  when PCID is on, shared rows seen on two or more cores) and
  `program_isolation_probe.py` with `SEXOS_PROGRAM_ISOLATION_PROBE=4` and PKU off (row 45
  reads row 44's image while 44 is live; only 45 dies).
- **Kernel growth:** +191 non-blank lines (views 176, rules 67; tenancy -42).
- **Known limit:** private table copies follow canonical changes made by
  `boot_memory::arena_leaf` at once and anything else at the next program start
  (`views::resync`). A `GLOBAL_VAS` mapping made at run time is invisible inside a row's
  view until then; WS2 can call `views::resync` from `memory::manager` to close this.
