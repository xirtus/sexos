# Assuit proofing controls — 2026-11-24

Assuit now exposes the WordCraft default proofing language through Ctrl+Shift+K. The prompt accepts en-US and en-GB (also US and UK); the default and selected text are updated, and the default is stored in DOCX. During Ctrl+Shift+Y's spelling review, A keeps the selected word in `assuit-custom-dictionary.txt` in Assuit's own program folder. Assuit loads that list at open and checks that the word reached disk before reporting success.

Focused host tests cover invalid language input, DOCX language round trip, writing a custom word, removing it from memory, and loading it in a new session. The Programs QEMU gate exercises both native controls and captures `assuit-dictionary-added.ppm`, `assuit-proofing-language-prompt.ppm`, and `assuit-proofing-language.ppm` in GitHub CI.

The dictionary is app-owned rather than written beside an exact-file-granted document. This uses the existing WordCraft proofing engine and SexOS Files-backed app storage; it adds no kernel or POSIX compatibility layer. A full dictionary manager, other language choices, and live spelling marks remain open.
