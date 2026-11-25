# Kaleidoscope site data counts

- Branch: `codex/kaleidoscope-site-data-counts`, stacked on
  `codex/kaleidoscope-download-cancel` (#661).
- The native site settings page now shows cookies matching its host plus local
  storage item and byte counts for the current container. It refreshes after
  the existing clear-site command. Counts do not expose cookie values.
- Focused host test verifies `Jar::count_site` matches the cookies the clear
  operation removes while leaving a different site untouched.
- The browser privacy QEMU gate loads a real site that sets one cookie and one
  local-storage item, captures `site-settings.ppm`, and asserts the counts return
  to zero after clearing. Full build and QEMU run on GitHub CI only.
