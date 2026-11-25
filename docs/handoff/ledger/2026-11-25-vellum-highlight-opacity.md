# Vellum highlight opacity — 2026-11-25

Ctrl+Shift+O cycles 100%, 75%, 50%, and 25% opacity for the next highlights. Vellum writes the value to the PDF annotation and appearance stream, and reports it through the annotation reader. The initial value remains 100%.

The focused editor test compares opaque and 50% render pixels and verifies save/reopen. The QEMU edit gate captures a green 75% highlight and checks the guest-saved `/CA` through the editor reader. No authority or compatibility behavior changes.
