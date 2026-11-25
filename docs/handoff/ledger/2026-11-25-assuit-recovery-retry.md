# Assuit: retry failed recovery writes

- **Gap:** A failed idle recovery write cleared Assuit's pending timer before Files had promoted a recovery copy. With no later edit, it never retried, leaving DOCX or ODT changes unprotected across a crash.
- **Change:** Keep the pending edit until `write_recovery` succeeds; schedule a bounded five-second retry after an error. A successful ordinary save still clears the timer.
- **Evidence:** The focused host test injects a Files write failure, confirms no early retry, then retries the same edit without a keystroke. It reopens the recovery copy and saves/reopens DOCX and ODT with text and a linked footnote. The existing two-boot `program_assuit_recovery_gate.py` captures the restored page after a guest power cut.
- **CI:** Run the Assuit host test and the existing QEMU recovery screenshot gate on the exact PR head. Full builds and QEMU run only on GitHub CI.
- **Limits:** The test injects the write error at Assuit's recovery boundary; it does not fault the Files server itself or prove the power cut lands inside Files' rename.
