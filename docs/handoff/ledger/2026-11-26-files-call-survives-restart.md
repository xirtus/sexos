# A program's Files call survives a Files restart (Gemini's app retry, reworked)

Gemini's sex-app retry helper (#819) was taken back out in #821. It gave up after 8
not-ready refusals, and a merely busy Files returns those too, so Polyamour's save and
Splicy's export failed under load. This keeps the useful half.

- `sex_app::files::once` still waits as long as Files refuses with not-ready (busy, or
  restarting before it took the call).
- When Files restarts after taking the call, the kernel answers the caller with
  `PEER_RESTARTED` (`service_restart.rs`). On master that answer went to the mailbox and
  the program waited forever. Now a request that changes nothing (stat, list, read,
  statfs, args, ping, I/O stats) is sent again, up to 4 times. Any other request fails as
  unavailable, because the old run may or may not have done it.

No ABI change.
