# Vellum reuses recently rendered pages

- Branch: `codex/vellum-page-cache`, stacked on `codex/vellum-theme`.
- Gap: moving back to a page discarded its pixels and reran Hayro, even at the
  same zoom. A long PDF search or outline jump therefore paid the full render
  cost repeatedly.
- Change: retain up to two retired owned pixel pages under a 48 MiB cache cap;
  the shown page remains separate. A revisit with the same page and scale takes
  its pixels from the cache. Edits and saves discard cached pages because PDF
  bytes changed. The existing `[pdf.page]` record remains for every shown page,
  with a `[pdf.page.cache]` record on reuse.
- Test: the PDF find QEMU gate jumps from page 1 to page 7 and back, requires a
  cache hit for page 1 and preserves `pdf-revisit.ppm`. Full builds and QEMU run
  on GitHub CI only.
- Scope: Vellum viewer, PDF find gate, gap list and this ledger. No kernel,
  Wi-Fi driver or tether changes.
