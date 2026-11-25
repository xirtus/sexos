# Kaleidoscope resume metadata recovery

- Gap: a readable but malformed `resumable.tsv` prevented the browser from
  checking its valid `.previous` copy. The synced partial download then lost
  its resume link and occupied space as a hidden orphan.
- Change: decode the live metadata, then decode the previous copy if the live
  record is invalid; keep only records whose hidden part still exists.
- Proof: parser regression rejects malformed live metadata and accepts the
  valid previous record. The browser QEMU gate interrupts a download, writes
  a valid previous record and malformed live record through Spindle, restarts
  Kaleidoscope, verifies the resume panel and Range/ETag completion, and
  captures `browser-download-resume-recovered.ppm`.
- Scope: this does not recover parts that never received durable metadata.
  Focused `sex-web` test, `web` x86_64-none check, Python syntax, and diff
  check passed locally. Full build and QEMU proof run on GitHub CI.
