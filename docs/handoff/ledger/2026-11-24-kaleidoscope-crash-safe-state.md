# Kaleidoscope crash-safe profile and session state

- **Scope:** `apps/web` profile files (history, bookmarks, cookies, storage,
  container cookies/storage, tab layout) and session restore. No kernel, driver,
  network, or authority changes.
- **Change:** Write a sibling `.next`, sync it through Files, rename the current
  file to `.previous`, then promote `.next` and sync again. On a missing,
  oversized, or invalid UTF-8 live file, read the previous copy. A stale staged
  file is removed before the next save and is never read during startup.
- **Authority:** All paths remain inside Kaleidoscope's existing Files grant.
  Private tabs still keep their cookies and storage only in memory; session
  serialization still omits private tabs.
- **Verification:** A host fault test executes the actual Rust helper with a
  Files stand-in and interrupts each replacement step. The GitHub CI QEMU
  session gate moves saved session and history to their previous names, leaves
  bogus staged files, relaunches, verifies both recovered, and captures a
  restored-window screenshot. Full build and QEMU run only on GitHub CI.
- **Limit:** A syntactically valid UTF-8 file with corrupted application fields
  can still be parsed partially. The previous copy protects interrupted Files
  replacement, not unrelated media corruption or arbitrary semantic damage.
