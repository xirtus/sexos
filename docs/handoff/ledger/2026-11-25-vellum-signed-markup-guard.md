# Vellum signed markup guard — 2026-11-25

Vellum's annotation engine now refuses add, remove, and restore operations when the PDF already holds a signature. The document remains unchanged until DocMDP and FieldMDP permissions can be evaluated. Vellum logs a refused highlight and shows the reason in its status bar.

A focused host test checks that add and remove fail without pending changes on a signed fixture. The QEMU edit gate tries a region highlight after signing, captures `pdf-signed-markup-refused.ppm`, and checks that the signature's covered content remains intact. A host check repeats the refusal on the guest-signed PDF. No kernel, network, or credential handling changes.
