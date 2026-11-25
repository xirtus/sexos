# Kaleidoscope asks Chastity for a password fill

- Branch: `codex/kaleidoscope-autofill-live`, stacked on
  `codex/kaleidoscope-autofill`.
- Gap: Kaleidoscope had no native path from an explicit user action through
  Chastity's consent broker to a login form.
- Change: Ctrl+Shift+K requests credentials for the shown top-level HTTPS page
  only when one visible username/password pair belongs to a same-origin POST
  form. The browser polls Chastity's client, then rechecks the origin, active tab,
  document and controls before placing the approved pair in that page's live
  DOM. Navigation, tab changes and close cancel pending requests. Credentials
  are not written to browser profile or session files or serial output.
- Test: the CI browser QEMU gate uses a temporary, CI-only TLS trust root and a
  fictional Chastity vault. It proves tab-switch and navigation cancellation,
  an approval, exactly one same-origin POST with the approved values, screenshots
  of consent and the filled form, and no secret in serial output. The preceding
  policy PR's host fixtures run in the same CI workflow. The shared-image host
  assertion permits only the named Browser TLS fixture rebuild while continuing
  to reject extra image rebuilds in all shared jobs; its five tests pass.
- Scope: browser app, browser CI gate, gap list and this ledger. No kernel,
  Wi-Fi driver or tether changes.
