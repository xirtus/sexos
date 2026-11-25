# Vellum thumbnail rendering cache

Hayro recommends reusing its render cache for pages from the same PDF. Vellum
previously recreated it for each thumbnail. The visible thumbnail batch now
shares one cache for each pair, then releases it. This bounds temporary decoded
resources on tall screens; the existing owned-pixel cache still governs retained
pages. No parser or storage authority changes.

The host test compares pixels on two pages rendered with a shared cache against
fresh-cache renders. The ten-page QEMU fixture checks the thumbnail panel,
records the cold batch duration in `[pdf.thumbs]`, and captures `pdf-thumbs.ppm`
in `sexos-documents-proof`. It uses no fixed elapsed-time pass threshold.
