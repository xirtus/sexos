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
- **Kernel growth:** +221 non-blank lines (views, rules, two `views::mapped` calls in memory::manager; tenancy -42).
- **Run-time mappings:** private copies follow canonical changes from
  `boot_memory::arena_leaf` (`views::mirror`) and from `GLOBAL_VAS` `map_*` calls
  (`views::mapped`, only the entries the mapping reaches), and `views::resync` runs at each
  program start as a backstop.
- **2026-11-19 follow-up: every program row in a view.** `organs::program_key` returns
  `SHARED_PROGRAM_KEY` (8) for every row, so keys 5 and 6 are no row's (key 5 goes to the
  PKU gate frame, plan P6.2). A row's workers spread over every program core like any
  row's. Since several cores can now run one view, a span a view opens at run time is
  followed by one shootdown on every core (`views::flush_everywhere`), sent after the
  pool lock is released. Kernel lines: net -8.
- **2026-11-19 K7c (kernel pass 2): INVPCID.** Where the CPU has PCID and INVPCID
  (`[boot.views] invpcid=1`), an invalidation drops the address at once in every view
  the core has run (INVPCID type 0, `views_rules::invpcid_views`) and a whole flush is one
  INVPCID type 3 for every PCID. No view is marked stale, so a view switch flushes only on
  a core's first entry into a view; the pool gate checks `flushed <= first entries`.
  Without INVPCID the stale marks stay as before. Global (kernel) translations still go by
  INVLPG.
