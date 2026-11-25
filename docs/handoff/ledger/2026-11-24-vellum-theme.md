# Vellum follows SexOS appearance settings

- Branch: `codex/vellum-theme`, stacked on `codex/vellum-visual-sign`.
- Gap: the desk, panel and find bar used fixed dark colors in every system theme.
- Change: derive viewer chrome from shared SexOS theme tokens, poll settings
  generation while the window is open and repaint after theme or accent changes.
  PDF page pixels, search hit colors and annotation outlines stay attached to
  the document rendering rather than the window chrome.
- Test: the PDF find QEMU gate requires the default Light theme record and
  checks the desk color in `pdf-theme.ppm`; CI runs full image and guest gates.
- Scope: Vellum app, lockfile, PDF find gate, gap list and this ledger.
