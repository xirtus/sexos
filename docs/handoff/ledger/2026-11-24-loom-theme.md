# Loom follows the appearance setting

The thumbnail grid and disk usage map now read SexOS theme tokens. Loom polls
the settings generation and repaints an open view when Tatami changes it.
The text list uses the desktop text surface, which already follows Tatami.

Cached picture thumbnails are RGB pixels blended onto a dark photo well. The
well stays dark in every theme, while grid chrome and labels change. This
keeps both freshly made and cached transparent pictures visually consistent
without rewriting cache files or their helper contract.

GitHub CI runs the full build and QEMU gates. The new guest gate changes Light
to Dark with Tatami while Loom remains open and checks both grid surfaces; it
uploads `loom-light.ppm` and `loom-dark.ppm`. The existing thumbnail gate
checks decoded and cached picture colours over two Loom launches.
