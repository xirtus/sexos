# Vellum Unicode Find entry

- **Gap:** Find accepted only ASCII through `sex_app::typed`, although Shell
  already delivered layout and input-method characters through `sex_app::text`.
- **Change:** Find consumes that existing text path, retains its 64-character
  bound, and removes a grapheme on Backspace. No kernel or input protocol change.
- **Checks:** Focused host test covers a dead-key `é`, input-method `好`,
  combining accent deletion, and Ctrl-shortcut rejection. The existing PDF
  QEMU Find gate now switches to German, types `café` through a dead key,
  checks one hit, and captures `pdf-find-unicode.ppm`.
- **Scope:** Unicode normalization between composed and decomposed PDF text
  is still a separate gap. Full build and QEMU evidence is required from
  exact-head GitHub CI.
