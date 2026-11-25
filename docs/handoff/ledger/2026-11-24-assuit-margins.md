# Assuit page margins

Ctrl+Shift+M opens a page-margin prompt. It accepts WordCraft's normal, narrow,
moderate, wide and mirrored presets or four numeric inch values in top, bottom,
left, right order. WordCraft checks that the section still leaves room for text
and keeps the margins in DOCX and ODT saves.

The host test rejects invalid input and verifies custom margins after DOCX and
ODT reopen. The Programs QEMU gate applies narrow margins and captures the
prompt and paginated page in `assuit-margins-prompt.ppm` and
`assuit-margins.ppm`. Full builds and QEMU run only on GitHub CI.
