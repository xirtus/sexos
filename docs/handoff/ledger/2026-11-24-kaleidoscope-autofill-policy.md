# Kaleidoscope password fill eligibility

- Branch: `codex/kaleidoscope-autofill`, stacked on
  `codex/kaleidoscope-site-settings`.
- Gap: browser password fill has no safe page/form policy yet; the Chastity SDK
  transport alone cannot decide which page or controls receive credentials.
- Change: a shared browser policy returns an exact HTTPS origin only for a
  top-level page, and identifies one username/password pair in a same-origin
  POST form. It refuses ambiguous forms, GET, cross-origin or downgraded
  actions, frames, non-HTTPS pages and disabled/new-password controls.
- Test: host policy fixtures run in GitHub CI. The full image and browser QEMU
  gates run there too; existing browser screenshots are preserved in
  `sexos-browser-proof`.
- Next slice: explicit user request, Chastity consent, short-interval client
  polling, on-screen field visibility, cancellation on navigation/tab switch,
  and a TLS QEMU fixture that proves an approved fill without printing secrets.
- Scope: `sex-web` policy, its host gate, this gap list and one ledger. No
  credential handling or new browser control is claimed in this PR.
