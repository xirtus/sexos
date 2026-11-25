# Vellum signing source conflict

- Gap: after Shibari approval, Vellum could replace the PDF path with a signature over the older bytes it opened, discarding another run's intervening changes.
- Change: stage the complete signed PDF, then compare the destination with the opened revision immediately before Files rename. A mismatch or missing source refuses promotion and leaves the current PDF untouched.
- Proof: a focused host test changes the source between open and save, checks refusal and unchanged bytes, then checks that a matching source can be promoted. The existing PDF guest signing gate captures `pdf-digital-signature.ppm` and verifies the saved signed bytes independently.
- Limit: comparison and rename are separate Files operations. An atomic conditional replacement is still needed to close the final race; this PR prevents changes already visible at promotion time from being overwritten.
- CI: full build, focused test, and QEMU PDF edit gate run on GitHub CI only.
