# 2026-11-28: Opening the same store program again skips the disk read, proven by hash

Safe version of Gemini's `LAST_LOADED` (from #696, left out of #830 because it trusted a
remembered name and skipped both the read and the check, so anything else that wrote the
image region could be launched as that program; it also cached the store directory for
good, which an OTA update rewrites).

- `servers/sexfiles/src/program_store.rs`: `LAST_IMAGE` remembers the (name, len, hash)
  of the store program the image region was last filled with. It is cleared at the start
  of every load (tickets, `/bin`, media and failures all leave it empty) and set again
  only after a read whose FNV hash matched the directory. When the same entry is asked
  for again and the region still hashes to the directory's value, the bytes are already
  there: no drive commands, `[files.program.read] ... kept=1`. Any mismatch reads from
  the disk exactly as before. The directory is still read every time.
- Cost on a hit: one hash pass over the image, which the disk path also does after its
  read. Cost on a miss: none. The kernel still checks whatever arrives.
- No ABI change.
