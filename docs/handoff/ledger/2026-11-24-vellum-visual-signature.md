# Vellum typed visual signature

Ctrl+Shift+G prompts for a name and places it as a visible FreeText annotation in the chosen region or near the lower part of the page. The bar calls it a **visual signature** and explicitly says it is not a digital signature. It saves through Vellum's existing incremental PDF update. No key material or cryptographic signature field is created.

A host test verifies the markup survives save/reopen and does not appear as a digital signature field. The GitHub CI QEMU edit gate types a name, captures `pdf-visual-signature.ppm`, saves, reopens, and checks the FreeText annotation. No local full build or QEMU run; no kernel, Wi-Fi driver, or tether changes.
