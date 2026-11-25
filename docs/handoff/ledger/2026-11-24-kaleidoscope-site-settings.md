# Kaleidoscope site settings, 2026-11-24

Add Ctrl+Shift+E to show a local settings page for the current web site. The page
shows the origin, current container and Shields state. Existing Ctrl+Shift+S and
Ctrl+Shift+Backspace work while this page is open; the latter clears that site's
cookies, storage and compiled code. The page has a link back to the site. No web
page action can toggle settings just by linking to an `about:` URL.

The browser privacy QEMU gate exercises the page and both controls and captures
`site-settings.png`. Full build and QEMU verification run in GitHub CI.
The gate imports Pillow's `Image` before converting the QEMU PPM capture to PNG;
the first CI run reached that conversion and failed on the missing import.
