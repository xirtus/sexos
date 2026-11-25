# Vellum staged PDF saves

Vellum now stages edited PDF and thumbnail bytes in a new file, syncs them, and replaces the destination through Files' rename. Ordinary paths stage beside the destination; an exact `@/file.pdf` grant stages in Vellum's own folder, because the grant does not authorize a temporary sibling. Failed writing no longer truncates an existing PDF or removes an existing thumbnail. The edited PDF still uses Vellum's incremental update so the original PDF bytes remain at the front of the saved file. A cross-volume exact grant needs a future Files operation for atomic replacement.

Host tests verify replacement and stale staging data. GitHub CI owns the full build, `pdf` and `vellum-edit` tests, QEMU viewer/edit gates and screenshots. No local full build or QEMU run. No kernel, Wi-Fi driver, or tether changes.
