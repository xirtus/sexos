# Kaleidoscope download promotion — 2026-11-25

Streaming downloads use hidden `.part` files. The previous completion path
renamed the part before syncing Files and ignored the sync result. The browser
could report and record success without confirmed durable bytes or name.

The completion path now asks Files to make the staged bytes durable, renames
the part, then asks Files to make the new name durable. A failure at any step
stops the success log and completed-history entry. After a failed second sync,
the file may still be visible in Downloads; the browser reports that durability
could not be confirmed and offers a retry.

The host test checks promotion ordering and stops at each failure. The GitHub
CI QEMU download gate checks the ordered guest records before completion and
captures `browser-download-durable.ppm`. Branch
`codex/kaleidoscope-download-promotion` stacks on
`codex/kaleidoscope-download-pause`. No kernel, Wi-Fi, or tether code changed.
