# 2026-11-27: fetch resumes cut downloads; update names a truncated image

**Problem.** Over Wi-Fi a download could stop part way: smoltcp aborts a flow after 30 s
without packets (`OPEN` flows' `set_timeout`), or the peer resets it. Spindle's `fetch`
took any end of the flow (`STATE_CLOSED` as well as the peer's FIN) as the end of the
file and printed "Saved N bytes". `update` then handed Files a short image, and
`sex_disk::update::apply` reported it as `BadSignature`.

**Change.**
- `apps/spindle/src/fetch_command.rs`: reads Content-Length (and a 206's Content-Range).
  A body that ends short of it, or with no length ends by a reset/timeout instead of a
  FIN, is resumed up to 6 times with `Range: bytes=N-` (206 appends; a server that ignores
  Range and answers 200 has the file cut and refetched). Silence past the 60 s deadline
  in the body resumes too. Still short: `fetch: download incomplete (why): got X of Y
  bytes`, `[spindle.fetch.incomplete]`, result false. New records:
  `[spindle.fetch.length]`, `[spindle.fetch.resume]`.
- `servers/sexfiles/src/updater.rs`: an image smaller than the index's `bytes` is refused
  before any disk work with "The image download is incomplete: X of Y bytes arrived"
  (`[files.update.truncated]`). Apply failures now say which step failed and use
  update wording (the x17's 2026-11-27 attempt printed the installer's "shrink another
  partition" text for an update `NoSpace`); `[files.update.failed]` carries `step=`.
- `scripts/fetch_resume_gate.py` (CI, after the tether gate): FIN-cut + 206 resume,
  reset-cut + 200 restart, always-cut reported incomplete, plain download; `sum` FNV
  checked against the host's.

**Open.** The x17's last OTA (version 3/5 image from `:8791/ota`) downloaded whole
(33554432 bytes) and failed with `NoSpace` during apply; the new `step=` tells which
part (menu over 64 KiB vs. ESP FAT run vs. slot size) on the next try.
