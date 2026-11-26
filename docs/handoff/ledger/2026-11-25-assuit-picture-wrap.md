# Assuit picture text flow

- Gap: inserted pictures could be resized and cropped, but there was no native
  control to place them beside text or between text blocks.
- Change: Ctrl+Shift+W offers inline, square, and top-and-bottom flow for the
  selected picture. The prompt rejects other values before editing. These modes
  use WordCraft's floating-object model and preserve the image and its anchor.
- Proof: a focused host test saves and reopens all three modes in DOCX and ODT,
  including refusal when no picture is selected. The Assuit QEMU gate applies
  square flow and captures `assuit-picture-square-wrap.ppm`.
- Checks: focused host test, Python gate syntax, and diff check passed locally.
  Full build and QEMU proof run only on GitHub CI.
