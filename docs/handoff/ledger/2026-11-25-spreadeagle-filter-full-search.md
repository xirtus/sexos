# Spreadeagle full-range filter search

- Gap: the native AutoFilter menu preview stops at 1,000 values or 10,000 rows.
  A partial query for a later value could not find it, although an exact query
  could.
- Change: search the full sparse GridCraft column and formula spill values for
  up to 128 distinct case-insensitive partial matches. Search results enter the
  same native menu, and checked choices survive later queries. Ctrl+Space marks
  several results before Enter applies them. Large result sets ask for a more
  specific query instead of growing the menu without limit.
- Proof: host tests exercise two late matches beyond both preview limits,
  multi-value checks, XLSX reopen, and the existing 200-function catalog. The
  guest filter gate opens a fixture with two late values, captures
  `spreadeagle-filter-partial-search.ppm`, and applies both through the menu.
- Review fix: blank-matching queries use a one-bit-per-row coverage map rather
  than a tree node per occupied row, and retain at most 128 text matches while
  checking for blanks. A dense-column regression rejects a false blank, then
  confirms a blank after removing one interior value; 300 matching values
  confirm the menu remains bounded.
- Local checks: all 59 host tests, Python gate syntax, and `git diff --check`
  passed. The custom SexOS target needs `-Zbuild-std`; full image and QEMU tests
  run on GitHub CI only.
