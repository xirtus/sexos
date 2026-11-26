# Kaleidoscope reactive login autofill

- Chastity-approved autofill now sends `input` and `change` events for both controls after the browser writes their values. The page script receives node IDs only; credentials are absent from the dispatch message and event data.
- The browser rechecks tab, document owner, exact HTTPS origin, visible pair and control identity after listeners run. A listener that submits or replaces the form cannot leave a stale “filled” status on a new page.
- A host `sex-js` test verifies reactive listeners see both updated values and no secret event payload. The existing two-app QEMU gate now disables Sign in until those events run, waits for the reactive readiness marker, captures Chastity consent and filled browser screenshots, and confirms the same-origin POST. Full build and QEMU run are GitHub CI work.
