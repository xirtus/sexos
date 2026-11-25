# Date follows SexOS appearance settings

- Branch: `codex/date-theme` (stacked on `codex/date-shortcuts`).
- Gap: Date's calendar, help panel, clock and side bar used fixed dark colors,
  regardless of the system appearance setting.
- Change: derive native surface, text, selection, border and accent colors from
  SexOS theme tokens. Poll the settings generation and repaint on theme or
  accent changes, including light, dark and high contrast. Calendar-specific
  event colors remain user data; their rendered text is adjusted for contrast
  against a theme-aware background tint.
- Test: the calendar QEMU gate requires the default Light theme record,
  checks its background in `date-light.ppm` and preserves the screenshot in
  `sexos-productivity-proof`. GitHub CI runs the full image and guest gate.
- Scope: Date view, dependency lockfile, QEMU gate, gap list and this ledger.
  No kernel, Wi-Fi driver or tether changes.
