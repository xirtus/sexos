# Assuit exact and minimum line heights

- **Gap:** Assuit exposed paragraph line multiples, but could not set fixed or minimum heights in points even though WordCraft stores and renders both.
- **Change:** Ctrl+Alt+S accepts `exact HEIGHT BEFORE AFTER` and `minimum HEIGHT BEFORE AFTER` with bounded point values. The prior three-number multiple form stays available.
- **Evidence:** A focused host test checks invalid inputs, rendered change, and exact/minimum DOCX and ODT save/reopen. The Assuit QEMU gate captures the native results; full build and QEMU run in GitHub CI.
- **Limit:** More complex paragraph layout controls remain outside this change.
