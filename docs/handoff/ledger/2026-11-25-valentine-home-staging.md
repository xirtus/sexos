# Valentine encrypted-home staging — 2026-11-25

- Gap: the Files staging ticket only worked on primary SexFS, while a Loom Save grant may point into encrypted home.
- Change: the ticket and recovery logic now operate on each SexFS volume. Home setup and password unlock clean abandoned stages through home's deep stack and encrypted block device before issuing a new ticket. Lock invalidates home tickets until the next unlock. Primary and home tickets remain separate, and ordinary Files access cannot reach either staging namespace. Removable volumes still refuse staged operations.
- Evidence: the focused SexFS publish power-cut/remount and existing-name tests pass. Full build, encrypted-home QEMU power-cut coverage and screenshot are pending exact-head GitHub CI.
- Limit: Valentine still uses direct identity-bound writes. A follow-up must call staged BEGIN/CHUNK/PUBLISH (and ABORT on failure), then prove the UI path against both primary and home grants.
