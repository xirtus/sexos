# Kaleidoscope chrome appearance

Kaleidoscope's own tabs, address field, vertical strip, suggestions, find bar,
and Shields badge now draw from SexOS appearance tokens. It reads the active
Settings theme on launch and polls the generation every 500 ms to repaint when
light, dark, high contrast, or accent changes. Site CSS and article pixels keep
the site's own colors.

The `sex-web` host test compares rendered chrome across all three modes while
checking identical site pixels. The browser QEMU gate changes Settings from
light to dark with Kaleidoscope open, checks the actual bar pixels, and captures
`web-light.ppm` and `web-dark.ppm`. Full builds and QEMU run in GitHub CI.
