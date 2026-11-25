# Vellum highlight colors — 2026-11-25

Vellum now cycles four highlight colors with Ctrl+Shift+H. Ctrl+H and region Enter use the selected color for both the annotation's PDF `/C` value and rendered appearance. Yellow remains the initial choice.

The focused editor test checks a green highlight's render and incremental save/reopen. The QEMU edit gate selects green in the guest, captures `pdf-highlight-green.ppm`, then checks that the guest-saved PDF still contains the green highlight. This uses the existing Files save path and does not add authority or compatibility behavior.
