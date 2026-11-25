# Vellum form save sync — 2026-11-25

Vellum staged and renamed edited PDF bytes, then ignored Files' final sync
result. A failed sync could still clear the filled form's edit history and
recovery copy and report a successful save. The viewer now requires a
successful sync before it reloads the file, clears recovery, and records
`pdf.saved`. On sync failure it keeps the in-memory edits and recovery state,
reports the unconfirmed save, and permits a retry.

A focused host test fills a real AcroForm, injects a failing sync, checks that
the editor stays dirty and its recovery PDF remains, then retries and reopens
the saved value. The GitHub CI QEMU edit gate requires the guest sync record
before the success record and captures `pdf-form-sync-confirmed.ppm` after the
filled form saves. The guest gate does not inject a Files sync failure.

Branch `codex/vellum-form-save-sync` stacks on
`codex/vellum-search-layout-breaks`. No kernel, Wi-Fi, or tether code changed.
