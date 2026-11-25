# Assuit spelling check

The native view now uses WordCraft's proofing engine: F7 selects the next spelling or grammar issue, the status line shows suggestions, Up/Down chooses one, Enter applies it, and Esc closes. The app's own title and status line expose the shortcut without upstream branding.

A host test corrects `helo` to `hello` and verifies the saved DOCX. The Assuit QEMU gate types `helo`, opens F7, and captures `assuit-spelling.ppm`. Full build and guest proof run on GitHub CI. No kernel, Wi-Fi driver, or tether changes.
