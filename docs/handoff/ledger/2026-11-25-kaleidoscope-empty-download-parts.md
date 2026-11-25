# Kaleidoscope empty download part cleanup

Kaleidoscope creates a hidden `Downloads/.NAME.part` before writing a download.
A hard stop at that point can leave a zero-byte file with no resumable metadata.
On public-window startup, after loading and validating resume records, it now
removes only unreferenced, empty parts with that exact browser naming pattern.
It preserves nonempty hidden parts for possible manual recovery.

The `sex-web` host test covers the naming, size, directory, and protected-record
boundaries. The GitHub CI downloads QEMU gate stages an empty and a nonempty
hidden part, checks the cleanup receipt and Files listing, and saves
`browser-download-empty-parts-cleaned.ppm` beside its other panel screenshots.

No kernel, network driver, tether, or filesystem service behavior changes.
