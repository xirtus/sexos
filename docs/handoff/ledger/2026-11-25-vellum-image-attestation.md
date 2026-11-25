# Vellum signing client image attestation

Files pins SHA-256 of the canonical staged Vellum ELF at build time and hashes the exact bytes it copied into the program image. Its successful load reply carries an approval bit only on an exact match. Shell binds that bit to the activated program row and task; the PDF signing broker refuses `BEGIN` from every other run and clears the binding on exit or row reuse.

The release build now stages Vellum before Files and records it as a consumed input, so a changed artifact invalidates the build receipt. A host test checks the stage ordering, Rust unit tests cover altered image bytes, reply decoding and row reuse, and the PDF guest gate requires the attested signing run and captures Shibari's approval window.

This change authenticates the signing client image only. Shibari still signs client-supplied CMS attributes after approval and cannot yet verify or display the exact PDF covered by the signature. New digital signing remains unsuitable for sensitive documents until that document-bound consent protocol lands. Existing signed-PDF inspection and signed-edit guards are unchanged.
