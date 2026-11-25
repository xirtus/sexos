# Kaleidoscope JavaScript site setting

- Ctrl+Shift+Y on a page or its site settings view toggles JavaScript for that exact origin. The site settings view shows the current choice. A blocked page does not fetch external page scripts, create a script runtime for inline scripts or handlers, or run extension content scripts there; the browser's built-in pages remain usable.
- The choice is stored as a bounded list in `Web/scripts-off.tsv` through the browser's staged, synced state replacement. If saving fails, the in-memory change is rolled back. Different schemes, ports and subdomains keep separate choices.
- The focused host test checks exact-origin isolation and saved-list round trip. The privacy QEMU gate toggles the setting, captures `site-scripts-off.ppm`, proves a script-bearing page loads without a script runtime or console action, then enables it and observes execution. The extension QEMU gate also arms a later `executeScript` request and checks its explicit rejection while the origin is blocked. Full build and QEMU run on GitHub CI.
- Camera, microphone, notifications and pop-up prompts remain open because the current browser has no safe permission hook for them.
