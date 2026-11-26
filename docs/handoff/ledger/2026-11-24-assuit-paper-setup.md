# Assuit paper setup

- Branch: `codex/assuit-paper-setup` (stacked on `codex/assuit-print-range`).
- Gap: Assuit could open documents with their paper size, but had no native control to change it.
- Change: Ctrl+Shift+L accepts Letter, Legal or A4 and portrait or landscape. WordCraft applies the section's paper settings, which Assuit saves in DOCX or ODT.
- Tests: a host test rejects unsupported choices and saves/reopens A4 landscape in both formats. The Assuit QEMU gate records the command and captures prompt and page screenshots. Full build and QEMU run on GitHub CI.
- Scope: Assuit book/UI, shortcut reference, QEMU gate, gap list and ledger. No kernel or driver changes.
