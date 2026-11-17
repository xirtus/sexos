# Denim D4: Mesh shows compressed memory

Mesh (apps/monitor) now shows, when Denim holds anything, a "compressed" line: the pages
kept packed, the pages Denim's store takes for them, and the ratio; and a PACKED column with
each program's packed pages. The numbers come from a new SYS_STATS op, `OP_PACKED` (6):
`system_stats::packed(pd)` gives a run's packed pages, and with pd 0 every run's and the
pages Denim's arena holds now (grown less the pages it gave back). Serial:
`[monitor.packed] packed_pages= store_pages= ratio_tenths=` and `[monitor.row.packed]`.
The Denim gate opens Mesh once pages are out and checks both.
