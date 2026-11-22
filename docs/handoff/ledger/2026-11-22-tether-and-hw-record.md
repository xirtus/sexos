# Spindle tether and hw record (x17 first boot), 2026-11-22

For the Alienware x17 R1's first start (no serial port), two Spindle helpers:

- **`hw record [PATH|off]`** rewrites the hardware report every 30 s, into `hw-record-1.txt`
  and `hw-record-2.txt` in turn (one whole copy survives a hang mid-write). Each copy (and
  `hw save`) now also carries `uptime_ms=` and Spindle's newest 300 lines.
- **`tether HOST[:PORT]|off`** (default port 7177): Spindle connects *out* over the network
  service's flow 1 and sends `SEXOS-TETHER 1`, the kernel's boot log head, every retained
  Spindle line and then each new one. Lines from the host run as if typed, shown as
  `remote> LINE`; `install go`, `update`, `format`, `mkfs` are refused from the tether.
  While tethered, `fetch`/`update`/`pkg` wait (the shared region carries one call at a time).
- **Host side:** `scripts/tether_host.py` (`serve`, `send CMD`, `tail`, `status`), sessions in
  `tether-logs/`. Gate: `scripts/tether_gate.py` (ci.yml boot job); host test
  `scripts/tests/test_tether_host.py`. Runbook: X17_BARE_METAL_RUNBOOK_2026_11_18.md 2, 3b.
